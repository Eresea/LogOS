//! Generation-safe runtime events and dynamically sized event sets.

use alloc::vec::Vec;

use logos_abi::{EventHandle, EventSetHandle, ServiceHandle};

const NO_DEADLINE: u64 = u64::MAX;

struct Slot<T> {
    generation: u32,
    value: Option<T>,
}

impl<T> Slot<T> {
    fn empty() -> Self {
        Self { generation: 1, value: None }
    }
}

struct EventRecord {
    owner: ServiceHandle,
    signaled: bool,
}

struct EventSetRecord {
    owner: ServiceHandle,
    members: Vec<EventHandle>,
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
}

pub struct RuntimeEventRegistry {
    events: Vec<Slot<EventRecord>>,
    sets: Vec<Slot<EventSetRecord>>,
}

impl RuntimeEventRegistry {
    pub fn new() -> Self {
        Self { events: Vec::new(), sets: Vec::new() }
    }

    pub fn create_event(&mut self, owner: ServiceHandle) -> Result<EventHandle, EventError> {
        if !owner.is_valid() {
            return Err(EventError::Unauthorized);
        }
        let index = self.allocate_event_slot()?;
        let handle = EventHandle::new(index as u32, self.events[index].generation)
            .ok_or(EventError::Capacity)?;
        self.events[index].value = Some(EventRecord { owner, signaled: false });
        Ok(handle)
    }

    pub fn create_set(&mut self, owner: ServiceHandle) -> Result<EventSetHandle, EventError> {
        if !owner.is_valid() {
            return Err(EventError::Unauthorized);
        }
        let index = self.allocate_set_slot()?;
        let handle = EventSetHandle::new(index as u32, self.sets[index].generation)
            .ok_or(EventError::Capacity)?;
        self.sets[index].value = Some(EventSetRecord { owner, members: Vec::new() });
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
        let Some(index) = set_record.members.iter().position(|member| *member == event) else {
            return Err(EventError::NotMember);
        };
        set_record.members.remove(index);
        Ok(())
    }

    /// Nonallocating signal path for IRQ producers.
    pub fn signal_irq(&mut self, event: EventHandle) -> Result<(), EventError> {
        self.event_mut(event)?.signaled = true;
        Ok(())
    }

    pub fn signal(&mut self, owner: ServiceHandle, event: EventHandle) -> Result<(), EventError> {
        let record = self.event_mut(event)?;
        if record.owner != owner {
            return Err(EventError::Unauthorized);
        }
        record.signaled = true;
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
        let members = {
            let set_record = self.set(set)?;
            if set_record.owner != owner {
                return Err(EventError::Unauthorized);
            }
            set_record.members.clone()
        };
        for event in members {
            let record = self.event_mut(event)?;
            if record.signaled {
                record.signaled = false;
                return Ok(EventWait::Ready(event));
            }
        }
        if deadline != NO_DEADLINE && now >= deadline {
            Ok(EventWait::Timeout)
        } else {
            Ok(EventWait::Pending)
        }
    }

    pub fn destroy_event(&mut self, event: EventHandle) -> Result<(), EventError> {
        let index = self.event_index(event)?;
        self.events[index].value = None;
        self.events[index].generation = next_generation(self.events[index].generation);
        for set in &mut self.sets {
            if let Some(set) = set.value.as_mut() {
                set.members.retain(|member| *member != event);
            }
        }
        Ok(())
    }

    pub fn destroy_set(&mut self, set: EventSetHandle) -> Result<(), EventError> {
        let index = self.set_index(set)?;
        self.sets[index].value = None;
        self.sets[index].generation = next_generation(self.sets[index].generation);
        Ok(())
    }

    fn allocate_event_slot(&mut self) -> Result<usize, EventError> {
        if let Some((index, _)) =
            self.events.iter().enumerate().find(|(_, slot)| slot.value.is_none())
        {
            return Ok(index);
        }
        self.events.try_reserve(1).map_err(|_| EventError::Capacity)?;
        self.events.push(Slot::empty());
        Ok(self.events.len() - 1)
    }

    fn allocate_set_slot(&mut self) -> Result<usize, EventError> {
        if let Some((index, _)) =
            self.sets.iter().enumerate().find(|(_, slot)| slot.value.is_none())
        {
            return Ok(index);
        }
        self.sets.try_reserve(1).map_err(|_| EventError::Capacity)?;
        self.sets.push(Slot::empty());
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
    fn timeout_and_teardown_reject_stale_handles() {
        let owner = owner();
        let mut events = RuntimeEventRegistry::new();
        let event = events.create_event(owner).unwrap();
        let set = events.create_set(owner).unwrap();
        events.add(owner, set, event).unwrap();
        assert_eq!(events.wait_any(owner, set, 10, Some(10)), Ok(EventWait::Timeout));
        events.destroy_event(event).unwrap();
        assert_eq!(events.signal_irq(event), Err(EventError::Stale));
        assert_eq!(events.destroy_set(set), Ok(()));
        assert_eq!(events.destroy_set(set), Err(EventError::Stale));
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
    }
}
