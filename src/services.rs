use crate::capabilities::{Capability, CapabilityKind, CapabilityManager};
use crate::supervisor::Protocol;

const SERVICES: usize = 4;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Service {
    VirtioBalloon,
}

impl Service {
    pub const fn protocol(self) -> Protocol {
        match self {
            Self::VirtioBalloon => Protocol { abi: 1, version: 0 },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ServiceHandle(u8);

impl ServiceHandle {
    pub(crate) const fn self_check() -> Self {
        Self(0)
    }
}

pub struct Registry {
    services: [Option<Service>; SERVICES],
}

impl Registry {
    pub const fn new() -> Self {
        Self { services: [None; SERVICES] }
    }

    pub fn register(
        &mut self,
        capabilities: &CapabilityManager,
        capability: Capability,
        service: Service,
    ) -> Option<ServiceHandle> {
        if !capabilities.allows(capability, CapabilityKind::Service)
            || self.resolve(service).is_some()
        {
            return None;
        }
        for (index, slot) in self.services.iter_mut().enumerate() {
            if slot.is_none() {
                // ponytail: fixed registry; add dynamic lifecycle management with real services.
                *slot = Some(service);
                return Some(ServiceHandle(index as u8));
            }
        }
        None
    }

    pub fn resolve(&self, service: Service) -> Option<ServiceHandle> {
        self.services
            .iter()
            .position(|slot| *slot == Some(service))
            .map(|index| ServiceHandle(index as u8))
    }
}
