//! Generation-safe runtime events and dynamically sized event sets.

use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};

use logos_abi::{EventHandle, EventSetHandle, ServiceHandle};

const NO_DEADLINE: u64 = u64::MAX;

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
        if !owner.is_valid() {
            return Err(EventError::Unauthorized);
        }
        let index = self.allocate_event_slot()?;
        let handle = EventHandle::new(index as u32, self.events[index].generation)
            .ok_or(EventError::Capacity)?;
        self.events[index].value = Some(EventRecord { owner, signaled: AtomicBool::new(false) });
        Ok(handle)
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
        Ok(())
    }

    /// Nonallocating signal path for IRQ producers.
    pub fn signal_irq(&self, event: EventHandle) -> Result<(), EventError> {
        self.event(event)?.signaled.store(true, Ordering::Release);
        Ok(())
    }

    pub fn signal(&mut self, owner: ServiceHandle, event: EventHandle) -> Result<(), EventError> {
        let record = self.event_mut(event)?;
        if record.owner != owner {
            return Err(EventError::Unauthorized);
        }
        record.signaled.store(true, Ordering::Release);
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
            let signaled = self.event(event)?.signaled.load(Ordering::Acquire);
            if signaled {
                self.event(event)?.signaled.store(false, Ordering::Release);
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
            if self.event(*event)?.signaled.load(Ordering::Acquire) {
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
        for set in &mut self.sets {
            if let Some(set) = set.value.as_mut() {
                if set.members.contains(&event) {
                    set.invalidated = true;
                    set.waiting = false;
                }
                set.members.retain(|member| *member != event);
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
        self.sets[index].value = None;
        self.sets[index].generation = next_generation(self.sets[index].generation);
        Ok(())
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

    fn event_mut(&mut self, handle: EventHandle) -> Result<&mut EventRecord, EventError> {
        let index = self.event_index(handle)?;
        self.events[index].value.as_mut().ok_or(EventError::Stale)
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
        events.add(owner, set, event).unwrap();
        events.signal_irq(event).unwrap();
        assert_eq!(events.wait_any(owner, set, 1, Some(10)), Ok(EventWait::Ready(event)));
        assert_eq!(events.wait_any(owner, set, 1, Some(10)), Ok(EventWait::Pending));
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
