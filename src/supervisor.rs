//! Fixed health and restart policy for the live service graph.

use crate::process::ProcessState;
use logos_abi::ServiceId;

pub const MAX_SERVICES: usize = 6;
pub const HEARTBEAT_INTERVAL: u64 = logos_abi::SERVICE_HEARTBEAT_INTERVAL_TICKS;
pub const MISSED_HEARTBEATS: u8 = 3;
pub const MAX_RESTARTS: u8 = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServiceState {
    Stopped,
    Running,
    Unhealthy,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EndpointIdentity {
    pub generation: u16,
    pub service_epoch: u64,
}

#[allow(dead_code)]
#[derive(Clone, Copy)]
struct ServiceRecord {
    state: ServiceState,
    last_heartbeat: u64,
    missed_heartbeats: u8,
    restarts: u8,
}

#[allow(dead_code)]
impl ServiceRecord {
    const EMPTY: Self =
        Self { state: ServiceState::Stopped, last_heartbeat: 0, missed_heartbeats: 0, restarts: 0 };
}

/// Runtime supervisor policy for the real service graph.
///
/// Resource ownership remains in `ServiceRuntime`; this type only tracks
/// bounded health state and decides when the runtime must rebuild the graph.
#[allow(dead_code)]
pub(crate) struct LiveSupervisor {
    records: [ServiceRecord; MAX_SERVICES],
    recovery: bool,
}

#[allow(dead_code)]
impl LiveSupervisor {
    pub const fn new() -> Self {
        Self { records: [ServiceRecord::EMPTY; MAX_SERVICES], recovery: false }
    }

    pub fn register(&mut self, service: ServiceId, now: u64) {
        let record = &mut self.records[service.index()];
        record.state = ServiceState::Running;
        record.last_heartbeat = now;
        record.missed_heartbeats = 0;
    }

    pub fn poll(
        &mut self,
        now: u64,
        heartbeats: [u64; MAX_SERVICES],
        process_states: [Option<ProcessState>; MAX_SERVICES],
    ) -> Option<ServiceId> {
        for index in 0..MAX_SERVICES {
            let record = &mut self.records[index];
            if record.state != ServiceState::Running {
                continue;
            }
            if process_states[index].is_some_and(|state| !matches!(state, ProcessState::Running)) {
                record.state = ServiceState::Unhealthy;
                return ServiceId::from_index(index);
            }
            if heartbeats[index] > record.last_heartbeat {
                record.last_heartbeat = heartbeats[index];
                record.missed_heartbeats = 0;
            }
            if now.saturating_sub(record.last_heartbeat) < HEARTBEAT_INTERVAL {
                continue;
            }
            let missed = (now.saturating_sub(record.last_heartbeat) / HEARTBEAT_INTERVAL)
                .min(u64::from(u8::MAX)) as u8;
            record.missed_heartbeats = record.missed_heartbeats.saturating_add(missed.max(1));
            record.last_heartbeat = now;
            if record.missed_heartbeats >= MISSED_HEARTBEATS {
                record.state = ServiceState::Unhealthy;
                return ServiceId::from_index(index);
            }
        }
        None
    }

    pub fn prepare_restart(&mut self) -> bool {
        if self.recovery || self.records.iter().any(|record| record.restarts >= MAX_RESTARTS) {
            self.recovery = true;
            return false;
        }
        for record in &mut self.records {
            record.state = ServiceState::Stopped;
            record.last_heartbeat = 0;
            record.missed_heartbeats = 0;
            record.restarts += 1;
        }
        true
    }

    #[cfg(test)]
    fn state(&self, service: ServiceId) -> ServiceState {
        self.records[service.index()].state
    }
}

impl Default for LiveSupervisor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supervisor_quiesces_on_fault_and_rebinds_after_restart() {
        let mut supervisor = LiveSupervisor::new();
        for index in 0..MAX_SERVICES {
            supervisor.register(ServiceId::from_index(index).unwrap(), 1);
        }
        assert_eq!(
            supervisor.poll(
                HEARTBEAT_INTERVAL * u64::from(MISSED_HEARTBEATS) + 1,
                [1; MAX_SERVICES],
                [Some(ProcessState::Running); MAX_SERVICES]
            ),
            Some(ServiceId::Input)
        );
        assert!(supervisor.prepare_restart());
        assert_eq!(supervisor.state(ServiceId::Terminal), ServiceState::Stopped);
        supervisor.register(ServiceId::Terminal, 20);
        assert_eq!(supervisor.state(ServiceId::Terminal), ServiceState::Running);
    }

    #[test]
    fn supervisor_enters_recovery_after_bounded_restarts() {
        let mut supervisor = LiveSupervisor::new();
        for _ in 0..MAX_RESTARTS {
            assert!(supervisor.prepare_restart());
        }
        assert!(!supervisor.prepare_restart());
        assert!(supervisor.recovery);
    }
}
