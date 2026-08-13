//! Fixed dependency barrier for service startup.

use crate::service_images::SERVICE_IMAGES;
use logos_abi::ServiceId;

const SERVICE_COUNT: usize = SERVICE_IMAGES.len();

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum StartupState {
    Empty,
    ImageReady,
    AddressSpaceReady,
    ProcessReady,
    LaunchReady,
    Started,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StartupError {
    InvalidTransition,
    Dependency,
}

pub struct ServiceStartup {
    states: [StartupState; SERVICE_COUNT],
}

impl ServiceStartup {
    pub const fn new() -> Self {
        Self { states: [StartupState::Empty; SERVICE_COUNT] }
    }

    pub const fn state(&self, service: ServiceId) -> StartupState {
        self.states[service.index()]
    }

    pub fn mark_image(&mut self, service: ServiceId) -> Result<(), StartupError> {
        self.advance(service, StartupState::ImageReady)
    }

    pub fn mark_address_space(&mut self, service: ServiceId) -> Result<(), StartupError> {
        self.advance(service, StartupState::AddressSpaceReady)
    }

    pub fn mark_process(&mut self, service: ServiceId) -> Result<(), StartupError> {
        self.advance(service, StartupState::ProcessReady)
    }

    pub fn mark_launch_ready(&mut self, service: ServiceId) -> Result<(), StartupError> {
        self.advance(service, StartupState::LaunchReady)
    }

    pub fn start(&mut self, service: ServiceId) -> Result<(), StartupError> {
        if self.state(service) != StartupState::LaunchReady {
            return Err(StartupError::InvalidTransition);
        }
        if !dependencies_started(service, &self.states) {
            return Err(StartupError::Dependency);
        }
        self.states[service.index()] = StartupState::Started;
        Ok(())
    }

    pub fn all_launch_ready(&self) -> bool {
        let mut index = 0;
        while index < SERVICE_COUNT {
            if self.states[index] != StartupState::LaunchReady {
                return false;
            }
            index += 1;
        }
        true
    }

    fn advance(&mut self, service: ServiceId, next: StartupState) -> Result<(), StartupError> {
        let current = self.state(service) as u8;
        if next as u8 != current.saturating_add(1) {
            return Err(StartupError::InvalidTransition);
        }
        self.states[service.index()] = next;
        Ok(())
    }
}

impl Default for ServiceStartup {
    fn default() -> Self {
        Self::new()
    }
}

fn dependencies_started(service: ServiceId, states: &[StartupState; SERVICE_COUNT]) -> bool {
    match service {
        ServiceId::Input | ServiceId::Display => true,
        ServiceId::Terminal => {
            states[0] == StartupState::Started && states[1] == StartupState::Started
        }
        ServiceId::Session => states[2] == StartupState::Started,
        ServiceId::Commands => states[3] == StartupState::Started,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transitions_are_bounded_and_dependency_ordered() {
        let mut startup = ServiceStartup::new();
        startup.mark_image(ServiceId::Terminal).unwrap();
        startup.mark_address_space(ServiceId::Terminal).unwrap();
        startup.mark_process(ServiceId::Terminal).unwrap();
        startup.mark_launch_ready(ServiceId::Terminal).unwrap();
        assert_eq!(startup.start(ServiceId::Terminal), Err(StartupError::Dependency));
        assert_eq!(startup.start(ServiceId::Input), Err(StartupError::InvalidTransition));
        assert!(!startup.all_launch_ready());
    }

    #[test]
    fn graph_starts_in_dependency_order() {
        let mut startup = ServiceStartup::new();
        for service in [
            ServiceId::Input,
            ServiceId::Display,
            ServiceId::Terminal,
            ServiceId::Session,
            ServiceId::Commands,
        ] {
            startup.mark_image(service).unwrap();
            startup.mark_address_space(service).unwrap();
            startup.mark_process(service).unwrap();
            startup.mark_launch_ready(service).unwrap();
        }
        assert!(startup.all_launch_ready());
        startup.start(ServiceId::Input).unwrap();
        startup.start(ServiceId::Display).unwrap();
        startup.start(ServiceId::Terminal).unwrap();
        startup.start(ServiceId::Session).unwrap();
        startup.start(ServiceId::Commands).unwrap();
        assert_eq!(startup.state(ServiceId::Commands), StartupState::Started);
    }
}
