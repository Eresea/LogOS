//! Fixed dependency barrier for service startup.

use crate::service_images::SERVICE_IMAGES;
use logos_abi::ServiceId;

const SERVICE_COUNT: usize = SERVICE_IMAGES.len();
const ALL_SERVICES: u8 = (1 << SERVICE_COUNT) - 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StartupError {
    InvalidTransition,
    Dependency,
}

pub struct ServiceStartup {
    started: u8,
    launch_ready: bool,
}

impl ServiceStartup {
    pub const fn new() -> Self {
        Self { started: 0, launch_ready: false }
    }

    pub fn mark_launch_ready(&mut self) {
        self.launch_ready = true;
    }

    pub fn start(&mut self, service: ServiceId) -> Result<(), StartupError> {
        let bit = 1 << service.index();
        if !self.launch_ready || self.started & bit != 0 {
            return Err(StartupError::InvalidTransition);
        }
        if !dependencies_started(service, self.started) {
            return Err(StartupError::Dependency);
        }
        self.started |= bit;
        Ok(())
    }

    pub fn all_launch_ready(&self) -> bool {
        self.launch_ready
    }
}

impl Default for ServiceStartup {
    fn default() -> Self {
        Self::new()
    }
}

fn dependencies_started(service: ServiceId, started: u8) -> bool {
    match service {
        ServiceId::Input | ServiceId::Display => true,
        ServiceId::Terminal => {
            started & ((1 << ServiceId::Input.index()) | (1 << ServiceId::Display.index()))
                == ((1 << ServiceId::Input.index()) | (1 << ServiceId::Display.index()))
        }
        ServiceId::Session => started & (1 << ServiceId::Terminal.index()) != 0,
        ServiceId::Commands => started & (1 << ServiceId::Session.index()) != 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transitions_are_bounded_and_dependency_ordered() {
        let mut startup = ServiceStartup::new();
        assert_eq!(startup.start(ServiceId::Input), Err(StartupError::InvalidTransition));
        startup.mark_launch_ready();
        assert_eq!(startup.start(ServiceId::Terminal), Err(StartupError::Dependency));
        assert!(startup.all_launch_ready());
    }

    #[test]
    fn graph_starts_in_dependency_order() {
        let mut startup = ServiceStartup::new();
        startup.mark_launch_ready();
        assert!(startup.all_launch_ready());
        startup.start(ServiceId::Input).unwrap();
        startup.start(ServiceId::Display).unwrap();
        startup.start(ServiceId::Terminal).unwrap();
        startup.start(ServiceId::Session).unwrap();
        startup.start(ServiceId::Commands).unwrap();
        assert_eq!(startup.started, ALL_SERVICES);
    }
}
