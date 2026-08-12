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

pub struct HealthService {
    lifecycle: ServiceLifecycle,
}

impl HealthService {
    pub const fn new() -> Self {
        Self { lifecycle: ServiceLifecycle::new() }
    }

    pub const fn service_epoch(&self) -> u64 {
        self.lifecycle.service_epoch()
    }

    pub fn start_ping(&mut self, request_id: u64) -> Result<PingHandle, HealthError> {
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

    pub fn complete_ping(&mut self, handle: PingHandle) -> bool {
        self.lifecycle.complete(handle.0, handle.service_epoch(), handle.request_id())
    }

    pub fn cancel_ping(&mut self, handle: PingHandle) -> bool {
        self.lifecycle.cancel(handle.0)
    }

    pub fn restart(&mut self) -> usize {
        self.lifecycle.restart()
    }

    pub fn reclaim_ping(&mut self, handle: PingHandle) -> bool {
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
    fn ping_completes_and_reclaims() {
        let mut health = HealthService::new();
        let ping = health.start_ping(1).unwrap();
        assert_eq!(health.state(ping), Some(PingState::InFlight));
        assert!(health.complete_ping(ping));
        assert_eq!(health.state(ping), Some(PingState::Completed));
        assert!(health.reclaim_ping(ping));
        assert_eq!(health.state(ping), None);
    }

    #[test]
    fn restart_rejects_old_ping_and_explicit_retry_completes() {
        let mut health = HealthService::new();
        let ping = health.start_ping(1).unwrap();
        assert_eq!(health.restart(), 1);
        assert_eq!(health.state(ping), Some(PingState::Restarted));
        assert!(!health.complete_ping(ping));
        assert!(health.reclaim_ping(ping));

        let retry = health.start_ping(2).unwrap();
        assert_ne!(retry.service_epoch(), ping.service_epoch());
        assert!(health.complete_ping(retry));
        assert_eq!(health.state(retry), Some(PingState::Completed));
        assert!(health.reclaim_ping(retry));
    }
}
