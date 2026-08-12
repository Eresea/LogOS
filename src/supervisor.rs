//! Fixed service graph and restart policy.

use crate::process::{
    Capabilities, ProcessError, ProcessHandle, ProcessKind, ProcessState, ProcessTable,
};

pub const MAX_SERVICES: usize = 5;
pub const HEARTBEAT_INTERVAL: u64 = 100;
pub const MISSED_HEARTBEATS: u8 = 3;
pub const MAX_RESTARTS: u8 = 3;
const SUPERVISOR_FAULT_VECTOR: u8 = u8::MAX;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServiceId {
    Input,
    Display,
    Terminal,
    Session,
    Commands,
}

impl ServiceId {
    const fn index(self) -> usize {
        match self {
            Self::Input => 0,
            Self::Display => 1,
            Self::Terminal => 2,
            Self::Session => 3,
            Self::Commands => 4,
        }
    }

    const fn kind(self) -> ProcessKind {
        match self {
            Self::Input => ProcessKind::Input,
            Self::Display => ProcessKind::Display,
            Self::Terminal => ProcessKind::Terminal,
            Self::Session => ProcessKind::Session,
            Self::Commands => ProcessKind::Command,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServiceState {
    Stopped,
    Running,
    Unhealthy,
    Recovery,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SupervisorError {
    InvalidService,
    Process(ProcessError),
    RestartLimit,
}

impl From<ProcessError> for SupervisorError {
    fn from(error: ProcessError) -> Self {
        Self::Process(error)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EndpointIdentity {
    pub generation: u16,
    pub service_epoch: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RestartEffects {
    pub service: ServiceId,
    pub identity: EndpointIdentity,
    pub full_redraw: bool,
    pub session_reconnect: bool,
}

#[derive(Clone, Copy)]
struct ServiceRecord {
    state: ServiceState,
    process: Option<ProcessHandle>,
    identity: EndpointIdentity,
    last_heartbeat: u64,
    missed_heartbeats: u8,
    restarts: u8,
}

impl ServiceRecord {
    const EMPTY: Self = Self {
        state: ServiceState::Stopped,
        process: None,
        identity: EndpointIdentity { generation: 1, service_epoch: 1 },
        last_heartbeat: 0,
        missed_heartbeats: 0,
        restarts: 0,
    };
}

pub struct ServiceSupervisor {
    processes: ProcessTable,
    services: [ServiceRecord; MAX_SERVICES],
    recovery: bool,
}

impl ServiceSupervisor {
    pub const fn new() -> Self {
        Self {
            processes: ProcessTable::new(),
            services: [ServiceRecord::EMPTY; MAX_SERVICES],
            recovery: false,
        }
    }

    pub const fn recovery(&self) -> bool {
        self.recovery
    }

    pub fn start(
        &mut self,
        service: ServiceId,
        image: &[u8],
        now: u64,
    ) -> Result<EndpointIdentity, SupervisorError> {
        let index = service.index();
        if self.services[index].state == ServiceState::Running {
            return Ok(self.services[index].identity);
        }
        let old_process = self.services[index].process;
        let replacing = old_process.is_some();
        if let Some(process) = old_process {
            self.reclaim_for_restart(process)?;
        }
        let capabilities = if service == ServiceId::Session {
            Capabilities::SESSION
        } else if service == ServiceId::Commands {
            Capabilities::COMMAND
        } else {
            Capabilities::SERVICE
        };
        let process = self.processes.start(image, service.kind(), capabilities)?;
        let record = &mut self.services[index];
        record.process = Some(process);
        record.state = ServiceState::Running;
        record.last_heartbeat = now;
        record.missed_heartbeats = 0;
        if replacing {
            record.identity.generation = record.identity.generation.wrapping_add(1).max(1);
            record.identity.service_epoch = record.identity.service_epoch.wrapping_add(1).max(1);
        }
        Ok(record.identity)
    }

    pub fn state(&self, service: ServiceId) -> ServiceState {
        self.services[service.index()].state
    }
    pub fn identity(&self, service: ServiceId) -> EndpointIdentity {
        self.services[service.index()].identity
    }
    pub fn process_state(&self, service: ServiceId) -> Option<ProcessState> {
        self.services[service.index()].process.and_then(|handle| self.processes.state(handle))
    }

    pub fn heartbeat(&mut self, service: ServiceId, now: u64, identity: EndpointIdentity) -> bool {
        let record = &mut self.services[service.index()];
        if record.state != ServiceState::Running || record.identity != identity {
            return false;
        }
        record.last_heartbeat = now;
        record.missed_heartbeats = 0;
        true
    }

    pub fn fault(&mut self, service: ServiceId, vector: u8) -> Result<(), SupervisorError> {
        let Some(handle) = self.services[service.index()].process else {
            return Err(SupervisorError::InvalidService);
        };
        self.processes.fault(handle, vector)?;
        self.services[service.index()].state = ServiceState::Unhealthy;
        Ok(())
    }

    pub fn tick(&mut self, now: u64) -> Option<RestartEffects> {
        for index in 0..MAX_SERVICES {
            let record = &mut self.services[index];
            if record.state != ServiceState::Running
                || now.saturating_sub(record.last_heartbeat) < HEARTBEAT_INTERVAL
            {
                continue;
            }
            let missed = (now.saturating_sub(record.last_heartbeat) / HEARTBEAT_INTERVAL)
                .min(u64::from(u8::MAX)) as u8;
            record.missed_heartbeats = record.missed_heartbeats.saturating_add(missed.max(1));
            record.last_heartbeat = now;
            if record.missed_heartbeats >= MISSED_HEARTBEATS {
                if let Some(handle) = record.process {
                    if self.processes.state(handle) == Some(ProcessState::Running) {
                        let _ = self.processes.fault(handle, SUPERVISOR_FAULT_VECTOR);
                    }
                }
                record.state = ServiceState::Unhealthy;
                return Some(RestartEffects {
                    service: service_at(index),
                    identity: record.identity,
                    full_redraw: index == ServiceId::Terminal.index(),
                    session_reconnect: index == ServiceId::Terminal.index()
                        || index == ServiceId::Session.index(),
                });
            }
        }
        None
    }

    pub fn restart(
        &mut self,
        service: ServiceId,
        image: &[u8],
        now: u64,
    ) -> Result<RestartEffects, SupervisorError> {
        let index = service.index();
        let old_process = self.services[index].process;
        if self.services[index].restarts >= MAX_RESTARTS {
            self.services[index].state = ServiceState::Recovery;
            self.recovery = true;
            return Err(SupervisorError::RestartLimit);
        }
        if let Some(handle) = old_process {
            self.reclaim_for_restart(handle)?;
        }
        let capabilities = if service == ServiceId::Session {
            Capabilities::SESSION
        } else if service == ServiceId::Commands {
            Capabilities::COMMAND
        } else {
            Capabilities::SERVICE
        };
        let process = self.processes.start(image, service.kind(), capabilities)?;
        let record = &mut self.services[index];
        record.process = Some(process);
        record.state = ServiceState::Running;
        record.identity.generation = record.identity.generation.wrapping_add(1).max(1);
        record.identity.service_epoch = record.identity.service_epoch.wrapping_add(1).max(1);
        record.last_heartbeat = now;
        record.missed_heartbeats = 0;
        record.restarts += 1;
        Ok(RestartEffects {
            service,
            identity: record.identity,
            full_redraw: service == ServiceId::Terminal,
            session_reconnect: matches!(service, ServiceId::Terminal | ServiceId::Session),
        })
    }

    pub fn process_table(&self) -> &ProcessTable {
        &self.processes
    }

    fn reclaim_for_restart(&mut self, handle: ProcessHandle) -> Result<(), SupervisorError> {
        match self.processes.state(handle) {
            Some(ProcessState::Running) => {
                self.processes.fault(handle, SUPERVISOR_FAULT_VECTOR)?;
                self.processes.reclaim(handle)?;
            }
            Some(ProcessState::Exited(_) | ProcessState::Faulted(_)) => {
                self.processes.reclaim(handle)?;
            }
            Some(ProcessState::Starting) => return Err(ProcessError::NotRunning.into()),
            Some(ProcessState::Vacant) | None => {}
        }
        Ok(())
    }
}

impl Default for ServiceSupervisor {
    fn default() -> Self {
        Self::new()
    }
}

const fn service_at(index: usize) -> ServiceId {
    match index {
        0 => ServiceId::Input,
        1 => ServiceId::Display,
        2 => ServiceId::Terminal,
        3 => ServiceId::Session,
        _ => ServiceId::Commands,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::ProcessError;

    fn image() -> [u8; 128] {
        let mut image = [0; 128];
        image[..4].copy_from_slice(b"\x7fELF");
        image[4] = 2;
        image[5] = 1;
        image[16..18].copy_from_slice(&2u16.to_le_bytes());
        image[18..20].copy_from_slice(&0x3eu16.to_le_bytes());
        image[24..32].copy_from_slice(&0x1000u64.to_le_bytes());
        image[32..40].copy_from_slice(&64u64.to_le_bytes());
        image[54..56].copy_from_slice(&56u16.to_le_bytes());
        image[56..58].copy_from_slice(&1u16.to_le_bytes());
        image[64..68].copy_from_slice(&1u32.to_le_bytes());
        image[68..72].copy_from_slice(&5u32.to_le_bytes());
        image[72..80].copy_from_slice(&0u64.to_le_bytes());
        image[80..88].copy_from_slice(&0x1000u64.to_le_bytes());
        image[96..104].copy_from_slice(&1u64.to_le_bytes());
        image[104..112].copy_from_slice(&0x1000u64.to_le_bytes());
        image
    }

    #[test]
    fn terminal_restart_changes_identity_and_requests_redraw() {
        let image = image();
        let mut supervisor = ServiceSupervisor::new();
        let old = supervisor.start(ServiceId::Terminal, &image, 0).unwrap();
        supervisor.fault(ServiceId::Terminal, 14).unwrap();
        let effects = supervisor.restart(ServiceId::Terminal, &image, 1).unwrap();
        assert_ne!(old, effects.identity);
        assert!(effects.full_redraw);
        assert!(effects.session_reconnect);
    }

    #[test]
    fn heartbeat_timeout_and_restart_limit_enter_recovery() {
        let image = image();
        let mut supervisor = ServiceSupervisor::new();
        supervisor.start(ServiceId::Input, &image, 0).unwrap();
        assert!(supervisor.tick(300).is_some());
        assert_eq!(
            supervisor.process_state(ServiceId::Input),
            Some(ProcessState::Faulted(SUPERVISOR_FAULT_VECTOR))
        );
        for _ in 0..MAX_RESTARTS {
            supervisor.restart(ServiceId::Input, &image, 1).unwrap();
            supervisor.fault(ServiceId::Input, 13).unwrap();
        }
        assert_eq!(
            supervisor.restart(ServiceId::Input, &image, 2),
            Err(SupervisorError::RestartLimit)
        );
        assert!(supervisor.recovery());
    }

    #[test]
    fn restart_reclaims_a_still_running_process_before_replacement() {
        let image = image();
        let mut supervisor = ServiceSupervisor::new();
        supervisor.start(ServiceId::Terminal, &image, 0).unwrap();
        let old = supervisor.services[ServiceId::Terminal.index()].process.unwrap();

        supervisor.restart(ServiceId::Terminal, &image, 1).unwrap();

        assert_eq!(supervisor.process_table().state(old), None);
        assert_eq!(supervisor.process_state(ServiceId::Terminal), Some(ProcessState::Running));
    }

    #[test]
    fn malformed_image_stays_outside_process_table() {
        let mut supervisor = ServiceSupervisor::new();
        assert_eq!(
            supervisor.start(ServiceId::Display, &[0; 64], 0),
            Err(SupervisorError::Process(ProcessError::InvalidImage))
        );
    }
}
