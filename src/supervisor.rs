//! Fixed health and restart policy for the live service graph.

use alloc::vec::Vec;

use crate::process::ProcessState;
use logos_abi::ServiceHandle;

pub const HEARTBEAT_INTERVAL: u64 = logos_abi::SERVICE_HEARTBEAT_INTERVAL_TICKS;
pub const MISSED_HEARTBEATS: u8 = 3;
pub const MAX_RESTARTS: u8 = 3;
const STARTUP_GRACE_TICKS: u64 = HEARTBEAT_INTERVAL * 20;

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
    handle: ServiceHandle,
    state: ServiceState,
    last_heartbeat: u64,
    missed_heartbeats: u8,
    restarts: u8,
    startup_grace_until: u64,
}

#[allow(dead_code)]
impl ServiceRecord {
    const EMPTY: Self = Self {
        handle: ServiceHandle::EMPTY,
        state: ServiceState::Stopped,
        last_heartbeat: 0,
        missed_heartbeats: 0,
        restarts: 0,
        startup_grace_until: 0,
    };
}

/// Runtime supervisor policy for the real service graph.
///
/// Resource ownership remains in `ServiceRuntime`; this type only tracks
/// bounded health state and decides when the runtime must rebuild the graph.
#[allow(dead_code)]
pub(crate) struct LiveSupervisor {
    records: Vec<Option<ServiceRecord>>,
    recovery: bool,
    startup_grace_armed: bool,
}

#[allow(dead_code)]
impl LiveSupervisor {
    pub const fn new() -> Self {
        Self { records: Vec::new(), recovery: false, startup_grace_armed: false }
    }

    pub fn ensure(&mut self, handle: ServiceHandle) -> bool {
        let Ok(index) = usize::try_from(handle.index()) else { return false };
        if !handle.is_valid() {
            return false;
        }
        if index >= self.records.len() {
            let additional = index + 1 - self.records.len();
            if self.records.try_reserve(additional).is_err() {
                return false;
            }
            self.records.resize(index + 1, None);
        }
        match self.records[index] {
            None => true,
            Some(record) if record.handle == handle => true,
            Some(record) if record.state == ServiceState::Stopped => {
                self.records[index] = None;
                true
            }
            Some(_) => false,
        }
    }

    pub fn register(&mut self, handle: ServiceHandle, now: u64) -> bool {
        if !self.ensure(handle) {
            return false;
        }
        let index = handle.index() as usize;
        let record = self.records[index].get_or_insert(ServiceRecord::EMPTY);
        record.handle = handle;
        record.state = ServiceState::Running;
        record.last_heartbeat = now;
        record.missed_heartbeats = 0;
        record.startup_grace_until =
            if self.startup_grace_armed { now.saturating_add(STARTUP_GRACE_TICKS) } else { 0 };
        true
    }

    pub fn unregister(&mut self, handle: ServiceHandle) {
        let Ok(index) = usize::try_from(handle.index()) else { return };
        if self
            .records
            .get(index)
            .and_then(Option::as_ref)
            .is_some_and(|record| record.handle == handle)
        {
            self.records[index] = None;
        }
    }

    pub fn retain_slots(&mut self, handles: &[ServiceHandle]) {
        for slot in &mut self.records {
            let Some(record) = slot.as_ref() else { continue };
            if !handles.iter().any(|handle| handle.index() == record.handle.index()) {
                *slot = None;
            }
        }
    }

    pub fn poll(
        &mut self,
        now: u64,
        heartbeats: &[u64],
        process_states: &[Option<ProcessState>],
    ) -> Option<ServiceHandle> {
        for (index, record) in self.records.iter_mut().enumerate() {
            let Some(record) = record.as_mut() else { continue };
            if record.state != ServiceState::Running {
                continue;
            }
            if process_states
                .get(index)
                .and_then(|state| *state)
                .is_some_and(|state| !matches!(state, ProcessState::Running))
            {
                record.state = ServiceState::Unhealthy;
                return Some(record.handle);
            }
            if heartbeats[index] > record.last_heartbeat {
                record.last_heartbeat = heartbeats[index];
                record.missed_heartbeats = 0;
                record.startup_grace_until = 0;
            }
            if now < record.startup_grace_until {
                continue;
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
                return Some(record.handle);
            }
        }
        None
    }

    pub fn prepare_restart(&mut self) -> bool {
        if self.recovery
            || self.records.iter().flatten().any(|record| record.restarts >= MAX_RESTARTS)
        {
            self.recovery = true;
            return false;
        }
        for record in self.records.iter_mut().flatten() {
            record.state = ServiceState::Stopped;
            record.last_heartbeat = 0;
            record.missed_heartbeats = 0;
            record.restarts += 1;
        }
        self.startup_grace_armed = true;
        true
    }

    pub fn prepare_targeted_restart(&mut self, handle: ServiceHandle) -> bool {
        let Ok(index) = usize::try_from(handle.index()) else { return false };
        let Some(record) = self.records.get_mut(index).and_then(Option::as_mut) else {
            return false;
        };
        if record.handle != handle {
            return false;
        }
        if record.restarts >= MAX_RESTARTS {
            self.recovery = true;
            return false;
        }
        record.state = ServiceState::Stopped;
        record.last_heartbeat = 0;
        record.missed_heartbeats = 0;
        record.restarts += 1;
        self.startup_grace_armed = true;
        true
    }

    pub fn clear_startup_grace(&mut self) {
        self.startup_grace_armed = false;
    }

    #[cfg(test)]
    fn state(&self, handle: ServiceHandle) -> ServiceState {
        let Ok(index) = usize::try_from(handle.index()) else { return ServiceState::Stopped };
        self.records
            .get(index)
            .and_then(Option::as_ref)
            .filter(|record| record.handle == handle)
            .map_or(ServiceState::Stopped, |record| record.state)
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
    use logos_abi::ServiceId;

    fn handle(service: ServiceId) -> ServiceHandle {
        ServiceHandle::new(service.index() as u32, 1).unwrap()
    }

    #[test]
    fn supervisor_quiesces_on_fault_and_rebinds_after_restart() {
        let mut supervisor = LiveSupervisor::new();
        let services: Vec<_> = (0..10).map(|index| ServiceHandle::new(index, 1).unwrap()).collect();
        for service in services {
            assert!(supervisor.register(service, 1));
        }
        assert_eq!(
            supervisor.poll(
                HEARTBEAT_INTERVAL * u64::from(MISSED_HEARTBEATS) + 1,
                &[1; 10],
                &[Some(ProcessState::Running); 10]
            ),
            Some(handle(ServiceId::Input))
        );
        assert!(supervisor.prepare_restart());
        assert_eq!(supervisor.state(handle(ServiceId::Terminal)), ServiceState::Stopped);
        assert!(supervisor.register(handle(ServiceId::Terminal), 20));
        assert_eq!(supervisor.state(handle(ServiceId::Terminal)), ServiceState::Running);
    }

    #[test]
    fn restarted_service_gets_startup_grace() {
        let mut supervisor = LiveSupervisor::new();
        assert!(supervisor.register(handle(ServiceId::Storage), 1));
        assert!(supervisor.prepare_targeted_restart(handle(ServiceId::Storage)));
        assert!(supervisor.register(handle(ServiceId::Storage), 20));
        assert_eq!(
            supervisor.poll(
                20 + STARTUP_GRACE_TICKS - 1,
                &[0; 10],
                &[Some(ProcessState::Running); 10]
            ),
            None
        );
        assert_eq!(
            supervisor.poll(
                20 + STARTUP_GRACE_TICKS + HEARTBEAT_INTERVAL * u64::from(MISSED_HEARTBEATS) + 1,
                &[0; 10],
                &[Some(ProcessState::Running); 10]
            ),
            Some(handle(ServiceId::Storage))
        );
    }

    #[test]
    fn supervisor_enters_recovery_after_bounded_restarts() {
        let mut supervisor = LiveSupervisor::new();
        assert!(supervisor.register(handle(ServiceId::Flow), 1));
        for _ in 0..MAX_RESTARTS {
            assert!(supervisor.prepare_restart());
        }
        assert!(!supervisor.prepare_restart());
        assert!(supervisor.recovery);
    }

    #[test]
    fn stale_generation_replaces_only_stopped_record() {
        let mut supervisor = LiveSupervisor::new();
        let old = handle(ServiceId::Flow);
        let new = ServiceHandle::new(old.index(), old.generation() + 1).unwrap();
        assert!(supervisor.register(old, 1));
        assert!(!supervisor.ensure(new));
        assert!(supervisor.prepare_restart());
        assert!(supervisor.register(new, 2));
        assert_eq!(supervisor.state(new), ServiceState::Running);
        let extra = ServiceHandle::new(12, 1).unwrap();
        assert!(supervisor.register(extra, 2));
        supervisor.retain_slots(&[new]);
        assert_eq!(supervisor.state(extra), ServiceState::Stopped);
    }
}
