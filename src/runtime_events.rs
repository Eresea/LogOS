//! Generation-safe runtime events and dynamically sized event sets.

use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use logos_abi::{
    DIRECTORY_EVENT_FLAG_HARDWARE_KEYBOARD, DIRECTORY_FLAG_MORE, DIRECTORY_RECORDS_PER_PAGE,
    DirectoryRecord, DirectoryRecordKind, DirectoryRequest, DirectoryResponse, DirectoryStatus,
    EventHandle, EventSetHandle, ServiceHandle,
};

const NO_DEADLINE: u64 = u64::MAX;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HardwareEventSource {
    Network,
    Keyboard,
}

static NETWORK_EVENT_RAW: AtomicU64 = AtomicU64::new(0);
static NETWORK_SET_RAW: AtomicU64 = AtomicU64::new(0);
static NETWORK_PENDING: AtomicBool = AtomicBool::new(false);
static KEYBOARD_EVENT_RAW: AtomicU64 = AtomicU64::new(0);
static KEYBOARD_SET_RAW: AtomicU64 = AtomicU64::new(0);
static KEYBOARD_PENDING: AtomicBool = AtomicBool::new(false);

fn hardware_event_raw(source: HardwareEventSource) -> &'static AtomicU64 {
    match source {
        HardwareEventSource::Network => &NETWORK_EVENT_RAW,
        HardwareEventSource::Keyboard => &KEYBOARD_EVENT_RAW,
    }
}

fn hardware_set_raw(source: HardwareEventSource) -> &'static AtomicU64 {
    match source {
        HardwareEventSource::Network => &NETWORK_SET_RAW,
        HardwareEventSource::Keyboard => &KEYBOARD_SET_RAW,
    }
}

fn hardware_pending(source: HardwareEventSource) -> &'static AtomicBool {
    match source {
        HardwareEventSource::Network => &NETWORK_PENDING,
        HardwareEventSource::Keyboard => &KEYBOARD_PENDING,
    }
}

#[allow(dead_code)]
pub(crate) fn bind_hardware_event(source: HardwareEventSource, event: EventHandle) {
    hardware_set_raw(source).store(0, Ordering::Release);
    hardware_pending(source).store(false, Ordering::Release);
    hardware_event_raw(source).store(event.raw(), Ordering::Release);
}

pub(crate) fn clear_hardware_event(source: HardwareEventSource) {
    hardware_event_raw(source).store(0, Ordering::Release);
    hardware_set_raw(source).store(0, Ordering::Release);
    hardware_pending(source).store(false, Ordering::Release);
}

pub(crate) fn bind_hardware_wait_set(
    source: HardwareEventSource,
    event: EventHandle,
    set: EventSetHandle,
) {
    if hardware_event_raw(source).load(Ordering::Acquire) == event.raw() {
        hardware_set_raw(source).store(set.raw(), Ordering::Release);
    }
}

pub(crate) fn unbind_hardware_wait_set(
    source: HardwareEventSource,
    event: EventHandle,
    set: EventSetHandle,
) {
    if hardware_event_raw(source).load(Ordering::Acquire) == event.raw() {
        let _ = hardware_set_raw(source).compare_exchange(
            set.raw(),
            0,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }
}

#[allow(dead_code)]
pub(crate) fn signal_hardware_event(source: HardwareEventSource) {
    if hardware_event_raw(source).load(Ordering::Acquire) == 0 {
        return;
    }
    hardware_pending(source).store(true, Ordering::Release);
    let set = hardware_set_raw(source).load(Ordering::Acquire);
    if let Some(set) = EventSetHandle::from_raw(set) {
        wake_event_set(set);
    }
}

fn hardware_event_is_pending(event: EventHandle) -> bool {
    [HardwareEventSource::Network, HardwareEventSource::Keyboard].into_iter().any(|source| {
        hardware_event_raw(source).load(Ordering::Acquire) == event.raw()
            && hardware_pending(source).load(Ordering::Acquire)
    })
}

fn consume_hardware_event(event: EventHandle) {
    for source in [HardwareEventSource::Network, HardwareEventSource::Keyboard] {
        if hardware_event_raw(source).load(Ordering::Acquire) == event.raw() {
            hardware_pending(source).store(false, Ordering::Release);
        }
    }
}

fn bind_all_hardware_wait_sets(event: EventHandle, set: EventSetHandle) {
    bind_hardware_wait_set(HardwareEventSource::Network, event, set);
    bind_hardware_wait_set(HardwareEventSource::Keyboard, event, set);
}

fn unbind_all_hardware_wait_sets(event: EventHandle, set: EventSetHandle) {
    unbind_hardware_wait_set(HardwareEventSource::Network, event, set);
    unbind_hardware_wait_set(HardwareEventSource::Keyboard, event, set);
}

struct Slot<T> {
    generation: u32,
    value: Option<T>,
}

impl<T> Slot<T> {
    fn with_generation(generation: u32) -> Self {
        Self { generation: generation.max(1), value: None }
    }
}

struct EventRecord {
    owner: ServiceHandle,
    directory_flags: u16,
    signaled: AtomicBool,
}

struct EventSetRecord {
    owner: ServiceHandle,
    members: Vec<EventHandle>,
    waiting: bool,
    invalidated: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EventWait {
    Ready(EventHandle),
    Pending,
    Timeout,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EventError {
    Stale,
    Unauthorized,
    Capacity,
    Duplicate,
    NotMember,
    InvalidDeadline,
    Busy,
}

pub(crate) fn wake_event_set(set: EventSetHandle) {
    #[cfg(target_os = "uefi")]
    crate::arch::signal_event_set(set);
    #[cfg(not(target_os = "uefi"))]
    let _ = set;
}

pub struct RuntimeEventRegistry {
    events: Vec<Slot<EventRecord>>,
    sets: Vec<Slot<EventSetRecord>>,
    generation_seed: u32,
}

impl RuntimeEventRegistry {
    pub fn new() -> Self {
        Self::new_with_generation(1)
    }

    pub fn new_with_generation(generation: u32) -> Self {
        Self { events: Vec::new(), sets: Vec::new(), generation_seed: generation.max(1) }
    }

    pub fn create_event(&mut self, owner: ServiceHandle) -> Result<EventHandle, EventError> {
        self.create_event_with_flags(owner, 0)
    }

    #[allow(dead_code)]
    pub(crate) fn create_hardware_event(
        &mut self,
        owner: ServiceHandle,
        source: HardwareEventSource,
    ) -> Result<EventHandle, EventError> {
        let flags = match source {
            HardwareEventSource::Network => 0,
            HardwareEventSource::Keyboard => DIRECTORY_EVENT_FLAG_HARDWARE_KEYBOARD,
        };
        let event = self.create_event_with_flags(owner, flags)?;
        bind_hardware_event(source, event);
        Ok(event)
    }

    fn create_event_with_flags(
        &mut self,
        owner: ServiceHandle,
        directory_flags: u16,
    ) -> Result<EventHandle, EventError> {
        if !owner.is_valid() {
            return Err(EventError::Unauthorized);
        }
        let index = self.allocate_event_slot()?;
        let handle = EventHandle::new(index as u32, self.events[index].generation)
            .ok_or(EventError::Capacity)?;
        self.events[index].value =
            Some(EventRecord { owner, directory_flags, signaled: AtomicBool::new(false) });
        Ok(handle)
    }

    pub fn directory(
        &self,
        request: DirectoryRequest,
        response: &mut DirectoryResponse,
    ) -> DirectoryStatus {
        if !request.is_valid()
            || request.operation != logos_abi::DirectoryOperation::Events
            || !request.subject.is_valid()
        {
            return DirectoryStatus::Malformed;
        }
        *response =
            DirectoryResponse::empty(request.operation, DirectoryStatus::Ok, request.request_id);
        let mut seen = 0u64;
        let mut written = 0usize;
        for (index, slot) in self.events.iter().enumerate() {
            let Some(event) = slot.value.as_ref() else { continue };
            if event.owner != request.subject {
                continue;
            }
            if seen < request.cursor {
                seen += 1;
                continue;
            }
            if written == DIRECTORY_RECORDS_PER_PAGE {
                response.flags |= DIRECTORY_FLAG_MORE;
                response.cursor = request.cursor.saturating_add(written as u64);
                break;
            }
            let Some(handle) = EventHandle::new(index as u32, slot.generation) else {
                return DirectoryStatus::Malformed;
            };
            response.records[written] = DirectoryRecord {
                kind: DirectoryRecordKind::Event,
                rights: 0,
                flags: event.directory_flags,
                handle: handle.raw(),
                peer: event.owner,
                contract_id: 0,
                message_bytes: 0,
                queue_capacity: 0,
                event: EventHandle::EMPTY,
                name_len: 0,
                reserved: [0; 1],
                name: [0; logos_abi::MAX_SERVICE_NAME_BYTES],
            };
            written += 1;
            seen += 1;
        }
        response.count = written as u8;
        if response.flags & DIRECTORY_FLAG_MORE == 0 {
            response.cursor = request.cursor.saturating_add(written as u64);
        }
        DirectoryStatus::Ok
    }

    pub fn create_set(&mut self, owner: ServiceHandle) -> Result<EventSetHandle, EventError> {
        if !owner.is_valid() {
            return Err(EventError::Unauthorized);
        }
        let index = self.allocate_set_slot()?;
        let handle = EventSetHandle::new(index as u32, self.sets[index].generation)
            .ok_or(EventError::Capacity)?;
        self.sets[index].value =
            Some(EventSetRecord { owner, members: Vec::new(), waiting: false, invalidated: false });
        Ok(handle)
    }

    pub fn add(
        &mut self,
        owner: ServiceHandle,
        set: EventSetHandle,
        event: EventHandle,
    ) -> Result<(), EventError> {
        let event_owner = self.event(event)?.owner;
        if event_owner != owner {
            return Err(EventError::Unauthorized);
        }
        let set_record = self.set_mut(set)?;
        if set_record.owner != owner {
            return Err(EventError::Unauthorized);
        }
        if set_record.invalidated {
            return Err(EventError::Stale);
        }
        if set_record.waiting {
            return Err(EventError::Busy);
        }
        if set_record.members.contains(&event) {
            return Err(EventError::Duplicate);
        }
        set_record.members.try_reserve(1).map_err(|_| EventError::Capacity)?;
        set_record.members.push(event);
        bind_all_hardware_wait_sets(event, set);
        Ok(())
    }

    pub fn remove(
        &mut self,
        owner: ServiceHandle,
        set: EventSetHandle,
        event: EventHandle,
    ) -> Result<(), EventError> {
        let set_record = self.set_mut(set)?;
        if set_record.owner != owner {
            return Err(EventError::Unauthorized);
        }
        if set_record.invalidated {
            return Err(EventError::Stale);
        }
        if set_record.waiting {
            return Err(EventError::Busy);
        }
        let Some(index) = set_record.members.iter().position(|member| *member == event) else {
            return Err(EventError::NotMember);
        };
        set_record.members.remove(index);
        unbind_all_hardware_wait_sets(event, set);
        Ok(())
    }

    /// Nonallocating signal path for IRQ producers.
    pub fn signal_irq(&self, event: EventHandle) -> Result<(), EventError> {
        self.event(event)?.signaled.store(true, Ordering::Release);
        // The event and waiter records already exist. Waking here keeps the
        // producer path allocation-free while closing the signal-before-wait
        // race for IPC and hardware producers alike.
        self.for_each_waiter(event, |set, _| wake_event_set(set));
        Ok(())
    }

    pub fn signal(&mut self, owner: ServiceHandle, event: EventHandle) -> Result<(), EventError> {
        if self.event(event)?.owner != owner {
            return Err(EventError::Unauthorized);
        }
        self.event(event)?.signaled.store(true, Ordering::Release);
        // Task-context signaling has the same wake contract as IRQ signaling.
        // The waiter was published before the signal, so the existing event
        // object is sufficient and no collection growth is needed here.
        self.for_each_waiter(event, |set, _| wake_event_set(set));
        Ok(())
    }

    pub fn wait_any(
        &mut self,
        owner: ServiceHandle,
        set: EventSetHandle,
        now: u64,
        deadline: Option<u64>,
    ) -> Result<EventWait, EventError> {
        let deadline = deadline.unwrap_or(NO_DEADLINE);
        if deadline != NO_DEADLINE && deadline < now {
            return Err(EventError::InvalidDeadline);
        }
        let member_count = {
            let set_record = self.set(set)?;
            if set_record.owner != owner {
                return Err(EventError::Unauthorized);
            }
            if set_record.invalidated {
                return Err(EventError::Stale);
            }
            set_record.members.len()
        };
        for index in 0..member_count {
            let event = self.set(set)?.members.get(index).copied().ok_or(EventError::Stale)?;
            let signaled = self.event(event)?.signaled.load(Ordering::Acquire)
                || hardware_event_is_pending(event);
            if signaled {
                self.event(event)?.signaled.store(false, Ordering::Release);
                consume_hardware_event(event);
                self.set_mut(set)?.waiting = false;
                return Ok(EventWait::Ready(event));
            }
        }
        if deadline != NO_DEADLINE && now >= deadline {
            self.set_mut(set)?.waiting = false;
            Ok(EventWait::Timeout)
        } else {
            self.set_mut(set)?.waiting = true;
            Ok(EventWait::Pending)
        }
    }

    pub fn members(
        &self,
        owner: ServiceHandle,
        set: EventSetHandle,
    ) -> Result<&[EventHandle], EventError> {
        let record = self.set(set)?;
        if record.owner != owner {
            return Err(EventError::Unauthorized);
        }
        Ok(&record.members)
    }

    pub fn has_ready_event(
        &self,
        owner: ServiceHandle,
        set: EventSetHandle,
    ) -> Result<bool, EventError> {
        let record = self.set(set)?;
        if record.owner != owner {
            return Err(EventError::Unauthorized);
        }
        for event in &record.members {
            if self.event(*event)?.signaled.load(Ordering::Acquire)
                || hardware_event_is_pending(*event)
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub fn is_waiting(
        &self,
        owner: ServiceHandle,
        set: EventSetHandle,
    ) -> Result<bool, EventError> {
        let record = self.set(set)?;
        if record.owner != owner {
            return Err(EventError::Unauthorized);
        }
        Ok(record.waiting)
    }

    pub fn cancel_wait(
        &mut self,
        owner: ServiceHandle,
        set: EventSetHandle,
    ) -> Result<(), EventError> {
        let record = self.set_mut(set)?;
        if record.owner != owner {
            return Err(EventError::Unauthorized);
        }
        if record.invalidated {
            return Err(EventError::Stale);
        }
        let was_waiting = record.waiting;
        record.waiting = false;
        if was_waiting {
            wake_event_set(set);
        }
        Ok(())
    }

    pub fn for_each_waiter<F>(&self, event: EventHandle, mut callback: F)
    where
        F: FnMut(EventSetHandle, ServiceHandle),
    {
        for (index, slot) in self.sets.iter().enumerate() {
            let Some(record) = slot.value.as_ref() else { continue };
            if !record.waiting || !record.members.contains(&event) {
                continue;
            }
            let Some(set) = EventSetHandle::new(index as u32, slot.generation) else {
                continue;
            };
            callback(set, record.owner);
        }
    }

    pub fn destroy_event(
        &mut self,
        owner: ServiceHandle,
        event: EventHandle,
    ) -> Result<(), EventError> {
        let index = self.event_index(event)?;
        if self.events[index].value.as_ref().is_some_and(|record| record.owner != owner) {
            return Err(EventError::Unauthorized);
        }
        self.wake_waiters(event);
        self.events[index].value = None;
        self.events[index].generation = next_generation(self.events[index].generation);
        for (set_index, slot) in self.sets.iter_mut().enumerate() {
            let Some(set_handle) = EventSetHandle::new(set_index as u32, slot.generation) else {
                continue;
            };
            let set = &mut slot.value;
            if let Some(set) = set.as_mut() {
                if set.members.contains(&event) {
                    set.invalidated = true;
                    set.waiting = false;
                    unbind_all_hardware_wait_sets(event, set_handle);
                }
                set.members.retain(|member| *member != event);
            }
        }
        for source in [HardwareEventSource::Network, HardwareEventSource::Keyboard] {
            if hardware_event_raw(source).load(Ordering::Acquire) == event.raw() {
                clear_hardware_event(source);
            }
        }
        Ok(())
    }

    pub fn destroy_set(
        &mut self,
        owner: ServiceHandle,
        set: EventSetHandle,
    ) -> Result<(), EventError> {
        let index = self.set_index(set)?;
        if self.sets[index].value.as_ref().is_some_and(|record| record.owner != owner) {
            return Err(EventError::Unauthorized);
        }
        if self.sets[index].value.as_ref().is_some_and(|record| record.waiting) {
            wake_event_set(set);
        }
        if let Some(record) = self.sets[index].value.as_ref() {
            for event in &record.members {
                unbind_all_hardware_wait_sets(*event, set);
            }
        }
        self.sets[index].value = None;
        self.sets[index].generation = next_generation(self.sets[index].generation);
        Ok(())
    }

    pub fn ownership_count(&self, owner: ServiceHandle) -> usize {
        self.events
            .iter()
            .filter_map(|slot| slot.value.as_ref())
            .filter(|event| event.owner == owner)
            .count()
    }

    pub fn event_set_count(&self, owner: ServiceHandle) -> usize {
        self.sets
            .iter()
            .filter_map(|slot| slot.value.as_ref())
            .filter(|set| set.owner == owner)
            .count()
    }

    pub fn destroy_service(&mut self, owner: ServiceHandle) {
        let events: Vec<_> = self
            .events
            .iter()
            .enumerate()
            .filter_map(|(index, slot)| {
                let event = slot.value.as_ref()?;
                (event.owner == owner).then(|| EventHandle::new(index as u32, slot.generation))?
            })
            .collect();
        for event in events {
            self.wake_waiters(event);
            let _ = self.destroy_event(owner, event);
        }

        let sets: Vec<_> = self
            .sets
            .iter()
            .enumerate()
            .filter_map(|(index, slot)| {
                let set = slot.value.as_ref()?;
                (set.owner == owner).then(|| EventSetHandle::new(index as u32, slot.generation))?
            })
            .collect();
        for set in sets {
            if self.set(set).is_ok_and(|record| record.waiting) {
                wake_event_set(set);
            }
            let _ = self.destroy_set(owner, set);
        }
    }

    fn wake_waiters(&self, event: EventHandle) {
        self.for_each_waiter(event, |set, _| {
            wake_event_set(set);
        });
    }

    fn allocate_event_slot(&mut self) -> Result<usize, EventError> {
        if let Some((index, _)) =
            self.events.iter().enumerate().find(|(_, slot)| slot.value.is_none())
        {
            return Ok(index);
        }
        self.events.try_reserve(1).map_err(|_| EventError::Capacity)?;
        self.events.push(Slot::with_generation(self.generation_seed));
        Ok(self.events.len() - 1)
    }

    fn allocate_set_slot(&mut self) -> Result<usize, EventError> {
        if let Some((index, _)) =
            self.sets.iter().enumerate().find(|(_, slot)| slot.value.is_none())
        {
            return Ok(index);
        }
        self.sets.try_reserve(1).map_err(|_| EventError::Capacity)?;
        self.sets.push(Slot::with_generation(self.generation_seed));
        Ok(self.sets.len() - 1)
    }

    fn event_index(&self, handle: EventHandle) -> Result<usize, EventError> {
        let index = handle.index() as usize;
        let Some(slot) = self.events.get(index) else { return Err(EventError::Stale) };
        if slot.generation != handle.generation() || slot.value.is_none() {
            return Err(EventError::Stale);
        }
        Ok(index)
    }

    fn set_index(&self, handle: EventSetHandle) -> Result<usize, EventError> {
        let index = handle.index() as usize;
        let Some(slot) = self.sets.get(index) else { return Err(EventError::Stale) };
        if slot.generation != handle.generation() || slot.value.is_none() {
            return Err(EventError::Stale);
        }
        Ok(index)
    }

    fn event(&self, handle: EventHandle) -> Result<&EventRecord, EventError> {
        let index = self.event_index(handle)?;
        self.events[index].value.as_ref().ok_or(EventError::Stale)
    }

    fn set(&self, handle: EventSetHandle) -> Result<&EventSetRecord, EventError> {
        let index = self.set_index(handle)?;
        self.sets[index].value.as_ref().ok_or(EventError::Stale)
    }

    fn set_mut(&mut self, handle: EventSetHandle) -> Result<&mut EventSetRecord, EventError> {
        let index = self.set_index(handle)?;
        self.sets[index].value.as_mut().ok_or(EventError::Stale)
    }
}

impl Default for RuntimeEventRegistry {
    fn default() -> Self {
        Self::new()
    }
}

fn next_generation(current: u32) -> u32 {
    current.wrapping_add(1).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owner() -> ServiceHandle {
        ServiceHandle::new(4, 1).unwrap()
    }

    #[test]
    fn signal_before_wait_is_latched_and_consumed_once() {
        let owner = owner();
        let mut events = RuntimeEventRegistry::new();
        let event = events.create_event(owner).unwrap();
        let set = events.create_set(owner).unwrap();
        assert_eq!(events.ownership_count(owner), 1);
        assert_eq!(events.event_set_count(owner), 1);
        events.add(owner, set, event).unwrap();
        events.signal_irq(event).unwrap();
        assert_eq!(events.wait_any(owner, set, 1, Some(10)), Ok(EventWait::Ready(event)));
        assert_eq!(events.wait_any(owner, set, 1, Some(10)), Ok(EventWait::Pending));
        events.destroy_event(owner, event).unwrap();
        assert_eq!(events.ownership_count(owner), 0);
    }

    #[test]
    fn hardware_signal_uses_the_bound_dynamic_event() {
        clear_hardware_event(HardwareEventSource::Network);
        let owner = owner();
        let mut events = RuntimeEventRegistry::new();
        let event = events.create_event(owner).unwrap();
        let set = events.create_set(owner).unwrap();
        events.add(owner, set, event).unwrap();
        bind_hardware_event(HardwareEventSource::Network, event);
        bind_hardware_wait_set(HardwareEventSource::Network, event, set);
        signal_hardware_event(HardwareEventSource::Network);
        assert_eq!(events.wait_any(owner, set, 1, Some(10)), Ok(EventWait::Ready(event)));
        clear_hardware_event(HardwareEventSource::Network);
    }

    #[test]
    fn event_directory_exposes_owned_hardware_source() {
        clear_hardware_event(HardwareEventSource::Keyboard);
        let owner = owner();
        let mut events = RuntimeEventRegistry::new();
        let event = events.create_hardware_event(owner, HardwareEventSource::Keyboard).unwrap();
        let mut request = DirectoryRequest::new(logos_abi::DirectoryOperation::Events, 1);
        request.subject = owner;
        let mut response = DirectoryResponse::empty(
            request.operation,
            DirectoryStatus::Malformed,
            request.request_id,
        );
        assert_eq!(events.directory(request, &mut response), DirectoryStatus::Ok);
        assert_eq!(response.count, 1);
        assert_eq!(response.records[0].kind, DirectoryRecordKind::Event);
        assert_eq!(response.records[0].handle, event.raw());
        assert_eq!(response.records[0].flags, DIRECTORY_EVENT_FLAG_HARDWARE_KEYBOARD);
        clear_hardware_event(HardwareEventSource::Keyboard);
    }

    #[test]
    fn readiness_can_be_rechecked_after_wait_publication() {
        let owner = owner();
        let mut events = RuntimeEventRegistry::new();
        let event = events.create_event(owner).unwrap();
        let set = events.create_set(owner).unwrap();
        events.add(owner, set, event).unwrap();
        assert_eq!(events.wait_any(owner, set, 1, Some(10)), Ok(EventWait::Pending));
        events.signal_irq(event).unwrap();
        assert!(events.has_ready_event(owner, set).unwrap());
    }

    #[test]
    fn task_signal_wakes_a_published_waiter() {
        let owner = owner();
        let mut events = RuntimeEventRegistry::new();
        let event = events.create_event(owner).unwrap();
        let set = events.create_set(owner).unwrap();
        events.add(owner, set, event).unwrap();
        assert_eq!(events.wait_any(owner, set, 1, Some(10)), Ok(EventWait::Pending));
        events.signal(owner, event).unwrap();
        assert!(events.has_ready_event(owner, set).unwrap());
        assert_eq!(events.wait_any(owner, set, 2, Some(10)), Ok(EventWait::Ready(event)));
    }

    #[test]
    fn cancellation_clears_a_published_wait_and_preserves_stale_checks() {
        let owner = owner();
        let mut events = RuntimeEventRegistry::new();
        let event = events.create_event(owner).unwrap();
        let set = events.create_set(owner).unwrap();
        events.add(owner, set, event).unwrap();
        assert_eq!(events.wait_any(owner, set, 1, Some(10)), Ok(EventWait::Pending));
        assert_eq!(events.cancel_wait(owner, set), Ok(()));
        assert_eq!(events.is_waiting(owner, set), Ok(false));
        events.destroy_set(owner, set).unwrap();
        assert_eq!(events.cancel_wait(owner, set), Err(EventError::Stale));
    }

    #[test]
    fn registry_generation_seed_rejects_handles_from_a_previous_runtime() {
        let owner = owner();
        let mut first = RuntimeEventRegistry::new_with_generation(3);
        let event = first.create_event(owner).unwrap();
        let mut second = RuntimeEventRegistry::new_with_generation(4);
        assert_eq!(second.signal_irq(event), Err(EventError::Stale));
        assert_ne!(event, second.create_event(owner).unwrap());
    }

    #[test]
    fn timeout_and_teardown_reject_stale_handles() {
        let owner = owner();
        let mut events = RuntimeEventRegistry::new();
        let event = events.create_event(owner).unwrap();
        let set = events.create_set(owner).unwrap();
        events.add(owner, set, event).unwrap();
        assert_eq!(events.wait_any(owner, set, 10, Some(10)), Ok(EventWait::Timeout));
        events.destroy_event(owner, event).unwrap();
        assert_eq!(events.signal_irq(event), Err(EventError::Stale));
        assert_eq!(events.destroy_set(owner, set), Ok(()));
        assert_eq!(events.destroy_set(owner, set), Err(EventError::Stale));
    }

    #[test]
    fn waiting_state_is_visible_before_teardown() {
        let owner = owner();
        let mut events = RuntimeEventRegistry::new();
        let event = events.create_event(owner).unwrap();
        let set = events.create_set(owner).unwrap();
        events.add(owner, set, event).unwrap();
        assert_eq!(events.wait_any(owner, set, 1, Some(10)), Ok(EventWait::Pending));
        assert_eq!(events.is_waiting(owner, set), Ok(true));
        events.destroy_set(owner, set).unwrap();
        assert_eq!(events.is_waiting(owner, set), Err(EventError::Stale));
    }

    #[test]
    fn destroying_a_member_invalidates_the_published_wait() {
        let owner = owner();
        let mut events = RuntimeEventRegistry::new();
        let event = events.create_event(owner).unwrap();
        let set = events.create_set(owner).unwrap();
        events.add(owner, set, event).unwrap();
        assert_eq!(events.wait_any(owner, set, 1, Some(10)), Ok(EventWait::Pending));
        events.destroy_event(owner, event).unwrap();
        assert_eq!(events.is_waiting(owner, set), Ok(false));
        assert_eq!(events.wait_any(owner, set, 2, Some(10)), Err(EventError::Stale));
    }

    #[test]
    fn cross_owner_membership_and_signal_are_rejected() {
        let owner = owner();
        let other = ServiceHandle::new(5, 1).unwrap();
        let mut events = RuntimeEventRegistry::new();
        let event = events.create_event(owner).unwrap();
        let set = events.create_set(other).unwrap();
        assert_eq!(events.add(other, set, event), Err(EventError::Unauthorized));
        assert_eq!(events.signal(other, event), Err(EventError::Unauthorized));
        assert_eq!(events.destroy_event(other, event), Err(EventError::Unauthorized));
        assert_eq!(events.destroy_set(owner, set), Err(EventError::Unauthorized));
    }

    #[test]
    fn membership_is_frozen_while_a_waiter_is_published() {
        let owner = owner();
        let mut events = RuntimeEventRegistry::new();
        let first = events.create_event(owner).unwrap();
        let second = events.create_event(owner).unwrap();
        let set = events.create_set(owner).unwrap();
        events.add(owner, set, first).unwrap();
        assert_eq!(events.wait_any(owner, set, 1, Some(10)), Ok(EventWait::Pending));
        assert_eq!(events.add(owner, set, second), Err(EventError::Busy));
        assert_eq!(events.remove(owner, set, first), Err(EventError::Busy));
        events.signal_irq(first).unwrap();
        assert_eq!(events.wait_any(owner, set, 2, Some(10)), Ok(EventWait::Ready(first)));
        assert_eq!(events.remove(owner, set, first), Ok(()));
    }

    #[test]
    fn waiting_sets_are_enumerated_without_allocating() {
        let owner = owner();
        let mut events = RuntimeEventRegistry::new();
        let event = events.create_event(owner).unwrap();
        let set = events.create_set(owner).unwrap();
        events.add(owner, set, event).unwrap();
        assert_eq!(events.wait_any(owner, set, 1, Some(10)), Ok(EventWait::Pending));

        let mut found = None;
        events.for_each_waiter(event, |candidate, candidate_owner| {
            found = Some((candidate, candidate_owner));
        });
        assert_eq!(found, Some((set, owner)));
    }

    #[test]
    fn destroying_service_invalidates_owned_events_and_sets() {
        let owner = owner();
        let mut events = RuntimeEventRegistry::new();
        let event = events.create_event(owner).unwrap();
        let set = events.create_set(owner).unwrap();
        events.add(owner, set, event).unwrap();

        events.destroy_service(owner);

        assert_eq!(events.signal_irq(event), Err(EventError::Stale));
        assert_eq!(events.destroy_set(owner, set), Err(EventError::Stale));
    }
}
