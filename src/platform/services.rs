use crate::drivers::supervisor::Protocol;
use logos_core::capabilities::{Capability, CapabilityKind, CapabilityManager};

const SERVICES: usize = 5;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Service {
    VirtioBalloon,
    VirtioBlock,
    Storage,
    Network,
    Gateway,
}

impl Service {
    pub const fn protocol(self) -> Protocol {
        match self {
            Self::VirtioBalloon => Protocol { abi: 1, version: 0 },
            Self::VirtioBlock | Self::Storage | Self::Network | Self::Gateway => {
                Protocol { abi: 1, version: 0 }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ServiceHandle(u8);

impl ServiceHandle {
    pub(crate) const fn self_check() -> Self {
        Self(0)
    }

    pub const fn principal(self) -> crate::platform::session::Principal {
        crate::platform::session::Principal::service(self.0 as u32)
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
