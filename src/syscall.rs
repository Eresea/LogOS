//! Host-tested control-plane syscall authorization.

use logos_abi::{
    Capability, CapabilityKind, MAX_CAPABILITIES, ServiceId, SyscallKind, SyscallRequest,
    SyscallResponse, SyscallStatus,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ServiceContext {
    pub service: ServiceId,
    pub generation: u16,
    pub epoch: u64,
    capabilities: [Option<Capability>; MAX_CAPABILITIES],
}

impl ServiceContext {
    pub const fn new(service: ServiceId, generation: u16, epoch: u64) -> Self {
        Self { service, generation, epoch, capabilities: [None; MAX_CAPABILITIES] }
    }

    pub fn grant(&mut self, capability: Capability) -> Result<(), SyscallStatus> {
        if capability.service != self.service || capability.generation != self.generation {
            return Err(SyscallStatus::InvalidCapability);
        }
        let Some(slot) = self.capabilities.iter_mut().find(|slot| slot.is_none()) else {
            return Err(SyscallStatus::Exhausted);
        };
        *slot = Some(capability);
        Ok(())
    }

    pub fn revoke_all(&mut self) {
        self.capabilities.fill(None);
    }

    fn has(&self, capability: Capability) -> bool {
        self.capabilities.iter().flatten().any(|current| *current == capability)
    }
}

pub fn authorize(context: ServiceContext, request: SyscallRequest) -> SyscallResponse {
    if request.capability.service != context.service
        || request.capability.generation != context.generation
    {
        return SyscallResponse::new(SyscallStatus::InvalidCapability, 0);
    }

    let required = match request.kind {
        SyscallKind::Yield | SyscallKind::Wait | SyscallKind::Exit | SyscallKind::Heartbeat => None,
        SyscallKind::IpcCreate | SyscallKind::IpcMap | SyscallKind::IpcSignal => {
            Some(CapabilityKind::IpcEndpoint)
        }
        SyscallKind::ProcessStart | SyscallKind::ProcessReap => {
            Some(CapabilityKind::ProcessControl)
        }
        SyscallKind::CapabilityMap => Some(request.capability.kind),
    };

    if let Some(kind) = required {
        if request.capability.kind != kind || !context.has(request.capability) {
            return SyscallResponse::new(SyscallStatus::InvalidCapability, 0);
        }
    }
    SyscallResponse::new(SyscallStatus::Ok, 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn endpoint_capability(generation: u16) -> Capability {
        Capability::new(CapabilityKind::IpcEndpoint, ServiceId::Terminal, generation, 0, 1)
    }

    #[test]
    fn control_plane_requires_matching_generation_and_capability() {
        let capability = endpoint_capability(4);
        let mut context = ServiceContext::new(ServiceId::Terminal, 4, 7);
        context.grant(capability).unwrap();
        let request = SyscallRequest::new(SyscallKind::IpcSignal, capability);
        assert_eq!(authorize(context, request).status, SyscallStatus::Ok);

        let stale = Capability::new(CapabilityKind::IpcEndpoint, ServiceId::Terminal, 3, 0, 1);
        let request = SyscallRequest::new(SyscallKind::IpcSignal, stale);
        assert_eq!(authorize(context, request).status, SyscallStatus::InvalidCapability);
    }

    #[test]
    fn process_control_cannot_be_used_for_data_plane_ipc() {
        let capability =
            Capability::new(CapabilityKind::ProcessControl, ServiceId::Commands, 1, 0, 1);
        let mut context = ServiceContext::new(ServiceId::Commands, 1, 1);
        context.grant(capability).unwrap();
        let request = SyscallRequest::new(SyscallKind::IpcCreate, capability);
        assert_eq!(authorize(context, request).status, SyscallStatus::InvalidCapability);
    }

    #[test]
    fn lifecycle_calls_without_resources_need_no_capability() {
        let context = ServiceContext::new(ServiceId::Input, 1, 1);
        let empty = Capability::new(CapabilityKind::ServiceControl, ServiceId::Input, 1, 0, 0);
        for kind in [SyscallKind::Yield, SyscallKind::Wait, SyscallKind::Exit] {
            assert_eq!(
                authorize(context, SyscallRequest::new(kind, empty)).status,
                SyscallStatus::Ok
            );
        }
    }
}
