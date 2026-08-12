const VACANT: u8 = 0;
const READY: u8 = 1;
const WAITING: u8 = 2;
const COMPLETE: u8 = 3;
const CANCELLED: u8 = 4;
const TIMED_OUT: u8 = 5;
const INITIAL_GENERATION: u64 = 1;
const NO_DEADLINE: u64 = u64::MAX;

pub const MAX_OPERATIONS: usize = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationState {
    Ready,
    Waiting,
    Complete,
    Cancelled,
    TimedOut,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SubmitError {
    Capacity,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OperationHandle {
    slot: u8,
    generation: u64,
}

impl OperationHandle {
    pub const fn slot(self) -> usize {
        self.slot as usize
    }

    pub const fn generation(self) -> u64 {
        self.generation
    }
}

#[derive(Clone, Copy)]
struct OperationSlot {
    generation: u64,
    state: u8,
    deadline: u64,
}

impl OperationSlot {
    const fn new() -> Self {
        Self { generation: INITIAL_GENERATION, state: VACANT, deadline: NO_DEADLINE }
    }
}

pub struct Runtime {
    operations: [OperationSlot; MAX_OPERATIONS],
}

impl Runtime {
    pub const fn new() -> Self {
        Self { operations: [const { OperationSlot::new() }; MAX_OPERATIONS] }
    }

    pub fn submit(&mut self) -> Result<OperationHandle, SubmitError> {
        let Some((slot, operation)) =
            self.operations.iter_mut().enumerate().find(|(_, operation)| operation.state == VACANT)
        else {
            return Err(SubmitError::Capacity);
        };
        operation.state = READY;
        operation.deadline = NO_DEADLINE;
        Ok(OperationHandle { slot: slot as u8, generation: operation.generation })
    }

    pub fn state(&self, handle: OperationHandle) -> Option<OperationState> {
        let operation = self.operations.get(handle.slot as usize)?;
        if operation.generation != handle.generation || operation.state == VACANT {
            return None;
        }
        Some(match operation.state {
            READY => OperationState::Ready,
            WAITING => OperationState::Waiting,
            COMPLETE => OperationState::Complete,
            CANCELLED => OperationState::Cancelled,
            TIMED_OUT => OperationState::TimedOut,
            _ => return None,
        })
    }

    pub fn wait(&mut self, handle: OperationHandle, deadline: u64) -> bool {
        let Some(operation) = self.current_mut(handle) else { return false };
        if operation.state != READY {
            return false;
        }
        operation.deadline = deadline;
        operation.state = WAITING;
        true
    }

    pub fn complete(&mut self, handle: OperationHandle) -> bool {
        let Some(operation) = self.current_mut(handle) else { return false };
        if operation.state != WAITING {
            return false;
        }
        operation.deadline = NO_DEADLINE;
        operation.state = COMPLETE;
        true
    }

    pub fn cancel(&mut self, handle: OperationHandle) -> bool {
        let Some(operation) = self.current_mut(handle) else { return false };
        if !matches!(operation.state, READY | WAITING) {
            return false;
        }
        operation.deadline = NO_DEADLINE;
        operation.state = CANCELLED;
        true
    }

    pub fn timeout(&mut self, handle: OperationHandle, now: u64) -> bool {
        let Some(operation) = self.current_mut(handle) else { return false };
        if operation.state != WAITING || operation.deadline > now {
            return false;
        }
        operation.deadline = NO_DEADLINE;
        operation.state = TIMED_OUT;
        true
    }

    pub fn reclaim(&mut self, handle: OperationHandle) -> bool {
        let Some(operation) = self.current_mut(handle) else { return false };
        if !matches!(operation.state, COMPLETE | CANCELLED | TIMED_OUT) {
            return false;
        }
        operation.state = VACANT;
        operation.deadline = NO_DEADLINE;
        operation.generation = next_generation(operation.generation);
        true
    }

    fn current_mut(&mut self, handle: OperationHandle) -> Option<&mut OperationSlot> {
        let operation = self.operations.get_mut(handle.slot as usize)?;
        (operation.generation == handle.generation && operation.state != VACANT)
            .then_some(operation)
    }
}

impl Default for Runtime {
    fn default() -> Self {
        Self::new()
    }
}

const fn next_generation(generation: u64) -> u64 {
    let next = generation.wrapping_add(1);
    if next == 0 { INITIAL_GENERATION } else { next }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operations_are_bounded_and_generation_safe() {
        let mut runtime = Runtime::new();
        let first = runtime.submit().unwrap();
        assert_eq!(runtime.submit().unwrap().slot(), 1);
        assert_eq!(runtime.submit(), Err(SubmitError::Capacity));
        assert!(runtime.wait(first, 10));
        assert!(runtime.complete(first));
        assert_eq!(runtime.state(first), Some(OperationState::Complete));
        assert!(runtime.reclaim(first));
        let replacement = runtime.submit().unwrap();
        assert_eq!(replacement.slot(), first.slot());
        assert_ne!(replacement.generation(), first.generation());
        assert!(!runtime.cancel(first));
    }

    #[test]
    fn timeout_and_cancel_are_terminal() {
        let mut runtime = Runtime::new();
        let timed = runtime.submit().unwrap();
        assert!(runtime.wait(timed, 10));
        assert!(!runtime.timeout(timed, 9));
        assert!(runtime.timeout(timed, 10));
        assert_eq!(runtime.state(timed), Some(OperationState::TimedOut));
        assert!(runtime.reclaim(timed));

        let cancelled = runtime.submit().unwrap();
        assert!(runtime.wait(cancelled, 20));
        assert!(runtime.cancel(cancelled));
        assert_eq!(runtime.state(cancelled), Some(OperationState::Cancelled));
        assert!(!runtime.timeout(cancelled, 20));
        assert!(runtime.reclaim(cancelled));
    }
}
