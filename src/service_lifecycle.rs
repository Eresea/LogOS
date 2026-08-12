const VACANT: u8 = 0;
const IN_FLIGHT: u8 = 1;
const COMPLETED: u8 = 2;
const CANCELLED: u8 = 3;
const RESTARTED: u8 = 4;
const INITIAL_EPOCH: u64 = 1;
const INITIAL_SLOT_GENERATION: u64 = 1;

pub const MAX_OPERATIONS: usize = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationState {
    InFlight,
    Completed,
    Cancelled,
    Restarted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StartError {
    Capacity,
    InvalidRequest,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OperationHandle {
    slot: u8,
    slot_generation: u64,
    service_epoch: u64,
    request_id: u64,
}

impl OperationHandle {
    pub const fn slot(self) -> usize {
        self.slot as usize
    }

    pub const fn slot_generation(self) -> u64 {
        self.slot_generation
    }

    pub const fn service_epoch(self) -> u64 {
        self.service_epoch
    }

    pub const fn request_id(self) -> u64 {
        self.request_id
    }
}

#[derive(Clone, Copy)]
struct OperationSlot {
    slot_generation: u64,
    service_epoch: u64,
    request_id: u64,
    state: u8,
}

impl OperationSlot {
    const fn new() -> Self {
        Self {
            slot_generation: INITIAL_SLOT_GENERATION,
            service_epoch: 0,
            request_id: 0,
            state: VACANT,
        }
    }
}

pub struct ServiceLifecycle {
    service_epoch: u64,
    operations: [OperationSlot; MAX_OPERATIONS],
}

impl ServiceLifecycle {
    pub const fn new() -> Self {
        Self {
            service_epoch: INITIAL_EPOCH,
            operations: [const { OperationSlot::new() }; MAX_OPERATIONS],
        }
    }

    pub const fn service_epoch(&self) -> u64 {
        self.service_epoch
    }

    pub fn start(&mut self, request_id: u64) -> Result<OperationHandle, StartError> {
        if request_id == 0 {
            return Err(StartError::InvalidRequest);
        }
        let Some((slot, operation)) =
            self.operations.iter_mut().enumerate().find(|(_, operation)| operation.state == VACANT)
        else {
            return Err(StartError::Capacity);
        };
        operation.service_epoch = self.service_epoch;
        operation.request_id = request_id;
        operation.state = IN_FLIGHT;
        Ok(OperationHandle {
            slot: slot as u8,
            slot_generation: operation.slot_generation,
            service_epoch: self.service_epoch,
            request_id,
        })
    }

    pub fn state(&self, handle: OperationHandle) -> Option<OperationState> {
        let operation = self.operations.get(handle.slot as usize)?;
        if operation.slot_generation != handle.slot_generation || operation.state == VACANT {
            return None;
        }
        Some(match operation.state {
            IN_FLIGHT => OperationState::InFlight,
            COMPLETED => OperationState::Completed,
            CANCELLED => OperationState::Cancelled,
            RESTARTED => OperationState::Restarted,
            _ => return None,
        })
    }

    pub fn complete(
        &mut self,
        handle: OperationHandle,
        service_epoch: u64,
        request_id: u64,
    ) -> bool {
        let current_epoch = self.service_epoch;
        let Some(operation) = self.current_mut(handle) else { return false };
        if operation.state != IN_FLIGHT
            || operation.service_epoch != current_epoch
            || service_epoch != current_epoch
            || operation.request_id != request_id
        {
            return false;
        }
        operation.state = COMPLETED;
        true
    }

    pub fn cancel(&mut self, handle: OperationHandle) -> bool {
        let Some(operation) = self.current_mut(handle) else { return false };
        if operation.state != IN_FLIGHT {
            return false;
        }
        operation.state = CANCELLED;
        true
    }

    pub fn restart(&mut self) -> usize {
        let mut restarted = 0;
        for operation in &mut self.operations {
            if operation.state == IN_FLIGHT {
                operation.state = RESTARTED;
                restarted += 1;
            }
        }
        self.service_epoch = next_epoch(self.service_epoch);
        restarted
    }

    pub fn reclaim(&mut self, handle: OperationHandle) -> bool {
        let Some(operation) = self.current_mut(handle) else { return false };
        if !matches!(operation.state, COMPLETED | CANCELLED | RESTARTED) {
            return false;
        }
        operation.service_epoch = 0;
        operation.request_id = 0;
        operation.state = VACANT;
        operation.slot_generation = next_epoch(operation.slot_generation);
        true
    }

    fn current_mut(&mut self, handle: OperationHandle) -> Option<&mut OperationSlot> {
        let operation = self.operations.get_mut(handle.slot as usize)?;
        (operation.slot_generation == handle.slot_generation && operation.state != VACANT)
            .then_some(operation)
    }
}

impl Default for ServiceLifecycle {
    fn default() -> Self {
        Self::new()
    }
}

const fn next_epoch(epoch: u64) -> u64 {
    let next = epoch.wrapping_add(1);
    if next == 0 { INITIAL_EPOCH } else { next }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restart_rejects_late_completion_and_requires_owner_reclaim() {
        let mut lifecycle = ServiceLifecycle::new();
        let old_epoch = lifecycle.service_epoch();
        let old = lifecycle.start(7).unwrap();

        assert_eq!(lifecycle.restart(), 1);
        assert_eq!(lifecycle.state(old), Some(OperationState::Restarted));
        assert!(!lifecycle.complete(old, old_epoch, old.request_id()));
        assert!(lifecycle.reclaim(old));

        let replacement = lifecycle.start(8).unwrap();
        assert_eq!(replacement.slot(), old.slot());
        assert_ne!(replacement.slot_generation(), old.slot_generation());
        assert_ne!(replacement.service_epoch(), old.service_epoch());
        assert!(!lifecycle.complete(old, lifecycle.service_epoch(), old.request_id()));
        assert!(lifecycle.complete(
            replacement,
            replacement.service_epoch(),
            replacement.request_id(),
        ));
    }

    #[test]
    fn completion_requires_current_epoch_and_request_identity() {
        let mut lifecycle = ServiceLifecycle::new();
        let handle = lifecycle.start(9).unwrap();
        assert!(!lifecycle.complete(handle, handle.service_epoch() + 1, handle.request_id()));
        assert!(!lifecycle.complete(handle, handle.service_epoch(), handle.request_id() + 1));
        assert!(lifecycle.complete(handle, handle.service_epoch(), handle.request_id()));
        assert_eq!(lifecycle.state(handle), Some(OperationState::Completed));
        assert!(lifecycle.reclaim(handle));
    }

    #[test]
    fn capacity_and_cancellation_are_bounded() {
        let mut lifecycle = ServiceLifecycle::new();
        assert_eq!(lifecycle.start(0), Err(StartError::InvalidRequest));
        let first = lifecycle.start(1).unwrap();
        let second = lifecycle.start(2).unwrap();
        assert_eq!(lifecycle.start(3), Err(StartError::Capacity));
        assert!(lifecycle.cancel(first));
        assert!(!lifecycle.cancel(first));
        assert!(lifecycle.reclaim(first));
        assert!(lifecycle.cancel(second));
        assert!(lifecycle.reclaim(second));
    }
}
