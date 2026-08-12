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
pub enum RuntimeError {
    Capacity,
    InvalidOperation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OperationHandle {
    slot: u8,
    generation: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeCommand {
    Submit,
    Wait { handle: OperationHandle, deadline: u64 },
    Complete { handle: OperationHandle },
    Cancel { handle: OperationHandle },
    Timeout { handle: OperationHandle, now: u64 },
    Reclaim { handle: OperationHandle },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandError {
    Busy,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeResponse {
    Submitted(OperationHandle),
    Waiting(OperationHandle),
    Completed(OperationHandle),
    Cancelled(OperationHandle),
    TimedOut(OperationHandle),
    Reclaimed(OperationHandle),
    Rejected(RuntimeError),
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
    command: Option<RuntimeCommand>,
    response: Option<RuntimeResponse>,
}

impl Runtime {
    pub const fn new() -> Self {
        Self {
            operations: [const { OperationSlot::new() }; MAX_OPERATIONS],
            command: None,
            response: None,
        }
    }

    pub fn submit(&mut self, command: RuntimeCommand) -> Result<(), CommandError> {
        if self.command.is_some() || self.response.is_some() {
            return Err(CommandError::Busy);
        }
        self.command = Some(command);
        Ok(())
    }

    pub fn step(&mut self) -> bool {
        let Some(command) = self.command.take() else { return false };
        self.response = Some(match command {
            RuntimeCommand::Submit => match self.start() {
                Ok(handle) => RuntimeResponse::Submitted(handle),
                Err(error) => RuntimeResponse::Rejected(error),
            },
            RuntimeCommand::Wait { handle, deadline } => {
                if self.wait_operation(handle, deadline) {
                    RuntimeResponse::Waiting(handle)
                } else {
                    RuntimeResponse::Rejected(RuntimeError::InvalidOperation)
                }
            }
            RuntimeCommand::Complete { handle } => {
                if self.complete_operation(handle) {
                    RuntimeResponse::Completed(handle)
                } else {
                    RuntimeResponse::Rejected(RuntimeError::InvalidOperation)
                }
            }
            RuntimeCommand::Cancel { handle } => {
                if self.cancel_operation(handle) {
                    RuntimeResponse::Cancelled(handle)
                } else {
                    RuntimeResponse::Rejected(RuntimeError::InvalidOperation)
                }
            }
            RuntimeCommand::Timeout { handle, now } => {
                if self.timeout_operation(handle, now) {
                    RuntimeResponse::TimedOut(handle)
                } else {
                    RuntimeResponse::Rejected(RuntimeError::InvalidOperation)
                }
            }
            RuntimeCommand::Reclaim { handle } => {
                if self.reclaim_operation(handle) {
                    RuntimeResponse::Reclaimed(handle)
                } else {
                    RuntimeResponse::Rejected(RuntimeError::InvalidOperation)
                }
            }
        });
        true
    }

    pub fn take_response(&mut self) -> Option<RuntimeResponse> {
        self.response.take()
    }

    fn start(&mut self) -> Result<OperationHandle, RuntimeError> {
        let Some((slot, operation)) =
            self.operations.iter_mut().enumerate().find(|(_, operation)| operation.state == VACANT)
        else {
            return Err(RuntimeError::Capacity);
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

    fn wait_operation(&mut self, handle: OperationHandle, deadline: u64) -> bool {
        let Some(operation) = self.current_mut(handle) else { return false };
        if operation.state != READY {
            return false;
        }
        operation.deadline = deadline;
        operation.state = WAITING;
        true
    }

    fn complete_operation(&mut self, handle: OperationHandle) -> bool {
        let Some(operation) = self.current_mut(handle) else { return false };
        if operation.state != WAITING {
            return false;
        }
        operation.deadline = NO_DEADLINE;
        operation.state = COMPLETE;
        true
    }

    fn cancel_operation(&mut self, handle: OperationHandle) -> bool {
        let Some(operation) = self.current_mut(handle) else { return false };
        if !matches!(operation.state, READY | WAITING) {
            return false;
        }
        operation.deadline = NO_DEADLINE;
        operation.state = CANCELLED;
        true
    }

    fn timeout_operation(&mut self, handle: OperationHandle, now: u64) -> bool {
        let Some(operation) = self.current_mut(handle) else { return false };
        if operation.state != WAITING || operation.deadline > now {
            return false;
        }
        operation.deadline = NO_DEADLINE;
        operation.state = TIMED_OUT;
        true
    }

    fn reclaim_operation(&mut self, handle: OperationHandle) -> bool {
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
        let first = submit(&mut runtime);
        let second = submit(&mut runtime);
        assert_eq!(second.slot(), 1);
        assert_eq!(
            submit_response(&mut runtime, RuntimeCommand::Submit),
            RuntimeResponse::Rejected(RuntimeError::Capacity)
        );
        assert_eq!(
            submit_response(&mut runtime, RuntimeCommand::Wait { handle: first, deadline: 10 }),
            RuntimeResponse::Waiting(first)
        );
        assert_eq!(
            submit_response(&mut runtime, RuntimeCommand::Complete { handle: first }),
            RuntimeResponse::Completed(first)
        );
        assert_eq!(runtime.state(first), Some(OperationState::Complete));
        assert_eq!(
            submit_response(&mut runtime, RuntimeCommand::Reclaim { handle: first }),
            RuntimeResponse::Reclaimed(first)
        );
        let replacement = submit(&mut runtime);
        assert_eq!(replacement.slot(), first.slot());
        assert_ne!(replacement.generation(), first.generation());
        assert_eq!(
            submit_response(&mut runtime, RuntimeCommand::Cancel { handle: first }),
            RuntimeResponse::Rejected(RuntimeError::InvalidOperation)
        );
    }

    #[test]
    fn timeout_and_cancel_are_terminal() {
        let mut runtime = Runtime::new();
        let timed = submit(&mut runtime);
        assert_eq!(
            submit_response(&mut runtime, RuntimeCommand::Wait { handle: timed, deadline: 10 }),
            RuntimeResponse::Waiting(timed)
        );
        assert_eq!(
            submit_response(&mut runtime, RuntimeCommand::Timeout { handle: timed, now: 9 }),
            RuntimeResponse::Rejected(RuntimeError::InvalidOperation)
        );
        assert_eq!(
            submit_response(&mut runtime, RuntimeCommand::Timeout { handle: timed, now: 10 }),
            RuntimeResponse::TimedOut(timed)
        );
        assert_eq!(runtime.state(timed), Some(OperationState::TimedOut));
        assert_eq!(
            submit_response(&mut runtime, RuntimeCommand::Reclaim { handle: timed }),
            RuntimeResponse::Reclaimed(timed)
        );

        let cancelled = submit(&mut runtime);
        assert_eq!(
            submit_response(&mut runtime, RuntimeCommand::Wait { handle: cancelled, deadline: 20 }),
            RuntimeResponse::Waiting(cancelled)
        );
        assert_eq!(
            submit_response(&mut runtime, RuntimeCommand::Cancel { handle: cancelled }),
            RuntimeResponse::Cancelled(cancelled)
        );
        assert_eq!(runtime.state(cancelled), Some(OperationState::Cancelled));
        assert_eq!(
            submit_response(&mut runtime, RuntimeCommand::Timeout { handle: cancelled, now: 20 }),
            RuntimeResponse::Rejected(RuntimeError::InvalidOperation)
        );
        assert_eq!(
            submit_response(&mut runtime, RuntimeCommand::Reclaim { handle: cancelled }),
            RuntimeResponse::Reclaimed(cancelled)
        );
    }

    #[test]
    fn mailbox_is_one_entry_and_response_must_be_drained() {
        let mut runtime = Runtime::new();
        assert!(runtime.submit(RuntimeCommand::Submit).is_ok());
        assert_eq!(runtime.submit(RuntimeCommand::Submit), Err(CommandError::Busy));
        assert!(runtime.step());
        assert_eq!(runtime.submit(RuntimeCommand::Submit), Err(CommandError::Busy));
        let Some(RuntimeResponse::Submitted(handle)) = runtime.take_response() else { panic!() };
        assert_eq!(runtime.state(handle), Some(OperationState::Ready));
        assert!(runtime.submit(RuntimeCommand::Cancel { handle }).is_ok());
        assert!(runtime.step());
        assert_eq!(runtime.take_response(), Some(RuntimeResponse::Cancelled(handle)));
    }

    fn submit(runtime: &mut Runtime) -> OperationHandle {
        match submit_response(runtime, RuntimeCommand::Submit) {
            RuntimeResponse::Submitted(handle) => handle,
            _ => panic!(),
        }
    }

    fn submit_response(runtime: &mut Runtime, command: RuntimeCommand) -> RuntimeResponse {
        assert!(runtime.submit(command).is_ok());
        assert!(runtime.step());
        runtime.take_response().unwrap()
    }
}
