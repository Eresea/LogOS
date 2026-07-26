use crate::capabilities::{Capability, CapabilityKind, CapabilityManager};

const CAPABILITIES: usize = 3;

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Id(pub u32);

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Principal(pub u32);

impl Principal {
    pub const LOCAL: Self = Self(0);
}

pub struct Context {
    id: Id,
    principal: Principal,
    capabilities: [Option<Capability>; CAPABILITIES],
    length: usize,
}

impl Context {
    pub fn new(id: Id, principal: Principal, capabilities: &[Capability]) -> Option<Self> {
        if capabilities.len() > CAPABILITIES {
            return None;
        }
        let mut context =
            Self { id, principal, capabilities: [None; CAPABILITIES], length: capabilities.len() };
        for (slot, capability) in context.capabilities.iter_mut().zip(capabilities) {
            *slot = Some(*capability);
        }
        Some(context)
    }

    pub const fn id(&self) -> Id {
        self.id
    }

    pub const fn principal(&self) -> Principal {
        self.principal
    }

    pub fn allows(&self, manager: &CapabilityManager, kind: CapabilityKind) -> bool {
        self.capabilities[..self.length]
            .iter()
            .flatten()
            .any(|capability| manager.allows(*capability, kind))
    }

    pub fn self_check() -> bool {
        let mut manager = CapabilityManager::new();
        let Some(debug) = manager.grant(CapabilityKind::Debug) else {
            return false;
        };
        let Some(recovery) = manager.grant(CapabilityKind::Recovery) else {
            return false;
        };
        let Some(context) = Self::new(Id(1), Principal::LOCAL, &[recovery]) else {
            return false;
        };
        context.id() == Id(1)
            && context.principal() == Principal::LOCAL
            && context.allows(&manager, CapabilityKind::Recovery)
            && !context.allows(&manager, CapabilityKind::Debug)
            && manager.revoke(recovery)
            && !context.allows(&manager, CapabilityKind::Recovery)
            && Self::new(Id(2), Principal::LOCAL, &[debug, debug, debug, debug]).is_none()
    }
}
