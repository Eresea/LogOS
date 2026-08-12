use crate::service_lifecycle::{OperationState, ServiceLifecycle, StartError};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PingState {
    InFlight,
    Completed,
    Cancelled,
    Restarted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HealthError {
    Capacity,
    InvalidRequest,
    StaleOperation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PingHandle(crate::service_lifecycle::OperationHandle);

impl PingHandle {
    pub const fn request_id(self) -> u64 {
        self.0.request_id()
    }

    pub const fn service_epoch(self) -> u64 {
        self.0.service_epoch()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HealthCommand {
    Ping { request_id: u64 },
    CompletePing { handle: PingHandle },
    CancelPing { handle: PingHandle },
    Restart,
    Reclaim { handle: PingHandle },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandError {
    Busy,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HealthResponse {
    PingAccepted(PingHandle),
    PingCompleted(PingHandle),
    PingCancelled(PingHandle),
    Restarted { count: usize },
    Reclaimed(PingHandle),
    Rejected(HealthError),
}

pub struct HealthService {
    lifecycle: ServiceLifecycle,
    command: Option<HealthCommand>,
    response: Option<HealthResponse>,
}

impl HealthService {
    pub const fn new() -> Self {
        Self { lifecycle: ServiceLifecycle::new(), command: None, response: None }
    }

    pub const fn service_epoch(&self) -> u64 {
        self.lifecycle.service_epoch()
    }

    pub fn submit(&mut self, command: HealthCommand) -> Result<(), CommandError> {
        if self.command.is_some() || self.response.is_some() {
            return Err(CommandError::Busy);
        }
        self.command = Some(command);
        Ok(())
    }

    pub fn step(&mut self) -> bool {
        let Some(command) = self.command.take() else { return false };
        self.response = Some(match command {
            HealthCommand::Ping { request_id } => match self.start_ping(request_id) {
                Ok(handle) => HealthResponse::PingAccepted(handle),
                Err(error) => HealthResponse::Rejected(error),
            },
            HealthCommand::CompletePing { handle } => {
                if self.complete_ping(handle) {
                    HealthResponse::PingCompleted(handle)
                } else {
                    HealthResponse::Rejected(HealthError::StaleOperation)
                }
            }
            HealthCommand::CancelPing { handle } => {
                if self.cancel_ping(handle) {
                    HealthResponse::PingCancelled(handle)
                } else {
                    HealthResponse::Rejected(HealthError::StaleOperation)
                }
            }
            HealthCommand::Restart => HealthResponse::Restarted { count: self.restart() },
            HealthCommand::Reclaim { handle } => {
                if self.reclaim_ping(handle) {
                    HealthResponse::Reclaimed(handle)
                } else {
                    HealthResponse::Rejected(HealthError::StaleOperation)
                }
            }
        });
        true
    }

    pub fn take_response(&mut self) -> Option<HealthResponse> {
        self.response.take()
    }

    fn start_ping(&mut self, request_id: u64) -> Result<PingHandle, HealthError> {
        self.lifecycle.start(request_id).map(PingHandle).map_err(|error| match error {
            StartError::Capacity => HealthError::Capacity,
            StartError::InvalidRequest => HealthError::InvalidRequest,
        })
    }

    pub fn state(&self, handle: PingHandle) -> Option<PingState> {
        self.lifecycle.state(handle.0).map(|state| match state {
            OperationState::InFlight => PingState::InFlight,
            OperationState::Completed => PingState::Completed,
            OperationState::Cancelled => PingState::Cancelled,
            OperationState::Restarted => PingState::Restarted,
        })
    }

    fn complete_ping(&mut self, handle: PingHandle) -> bool {
        self.lifecycle.complete(handle.0, handle.service_epoch(), handle.request_id())
    }

    fn cancel_ping(&mut self, handle: PingHandle) -> bool {
        self.lifecycle.cancel(handle.0)
    }

    fn restart(&mut self) -> usize {
        self.lifecycle.restart()
    }

    fn reclaim_ping(&mut self, handle: PingHandle) -> bool {
        self.lifecycle.reclaim(handle.0)
    }
}

impl Default for HealthService {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_roundtrip_completes_and_reclaims_ping() {
        let mut health = HealthService::new();
        assert!(health.submit(HealthCommand::Ping { request_id: 1 }).is_ok());
        assert!(health.step());
        let Some(HealthResponse::PingAccepted(ping)) = health.take_response() else { panic!() };
        assert_eq!(health.state(ping), Some(PingState::InFlight));
        assert!(health.submit(HealthCommand::CompletePing { handle: ping }).is_ok());
        assert!(health.step());
        assert_eq!(health.take_response(), Some(HealthResponse::PingCompleted(ping)));
        assert_eq!(health.state(ping), Some(PingState::Completed));
        assert!(health.submit(HealthCommand::Reclaim { handle: ping }).is_ok());
        assert!(health.step());
        assert_eq!(health.take_response(), Some(HealthResponse::Reclaimed(ping)));
        assert_eq!(health.state(ping), None);
    }

    #[test]
    fn restart_rejects_old_ping_and_explicit_retry_completes() {
        let mut health = HealthService::new();
        assert!(health.submit(HealthCommand::Ping { request_id: 1 }).is_ok());
        assert!(health.step());
        let Some(HealthResponse::PingAccepted(ping)) = health.take_response() else { panic!() };
        assert!(health.submit(HealthCommand::Restart).is_ok());
        assert!(health.step());
        assert_eq!(health.take_response(), Some(HealthResponse::Restarted { count: 1 }));
        assert_eq!(health.state(ping), Some(PingState::Restarted));
        assert!(health.submit(HealthCommand::CompletePing { handle: ping }).is_ok());
        assert!(health.step());
        assert_eq!(
            health.take_response(),
            Some(HealthResponse::Rejected(HealthError::StaleOperation))
        );
        assert!(health.submit(HealthCommand::Reclaim { handle: ping }).is_ok());
        assert!(health.step());
        assert_eq!(health.take_response(), Some(HealthResponse::Reclaimed(ping)));

        assert!(health.submit(HealthCommand::Ping { request_id: 2 }).is_ok());
        assert!(health.step());
        let Some(HealthResponse::PingAccepted(retry)) = health.take_response() else { panic!() };
        assert_ne!(retry.service_epoch(), ping.service_epoch());
        assert!(health.submit(HealthCommand::CompletePing { handle: retry }).is_ok());
        assert!(health.step());
        assert_eq!(health.take_response(), Some(HealthResponse::PingCompleted(retry)));
        assert_eq!(health.state(retry), Some(PingState::Completed));
        assert!(health.submit(HealthCommand::Reclaim { handle: retry }).is_ok());
        assert!(health.step());
        assert_eq!(health.take_response(), Some(HealthResponse::Reclaimed(retry)));
    }

    #[test]
    fn mailbox_is_one_entry_and_response_must_be_drained() {
        let mut health = HealthService::new();
        assert!(health.submit(HealthCommand::Ping { request_id: 1 }).is_ok());
        assert_eq!(health.submit(HealthCommand::Restart), Err(CommandError::Busy));
        assert!(health.step());
        assert_eq!(health.submit(HealthCommand::Restart), Err(CommandError::Busy));
        let Some(HealthResponse::PingAccepted(ping)) = health.take_response() else { panic!() };
        assert!(health.submit(HealthCommand::CancelPing { handle: ping }).is_ok());
        assert!(health.step());
        assert_eq!(health.take_response(), Some(HealthResponse::PingCancelled(ping)));
        assert_eq!(health.state(ping), Some(PingState::Cancelled));
        assert!(health.submit(HealthCommand::Restart).is_ok());
    }
}
