use crate::{
    audit::{Effect, Log},
    session::Principal,
};
use logos_core::capabilities::{Capability, CapabilityKind, CapabilityManager};

const GRANTS: usize = 4;

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Id(u8);

#[derive(Clone, Copy)]
struct Grant {
    principal: Principal,
    kind: CapabilityKind,
    capability: Capability,
    expires: u64,
}

pub struct Store {
    grants: [Option<Grant>; GRANTS],
}

impl Store {
    pub const fn new() -> Self {
        Self { grants: [None; GRANTS] }
    }

    pub fn grant(
        &mut self,
        capabilities: &mut CapabilityManager,
        principal: Principal,
        kind: CapabilityKind,
        expires: u64,
        now: u64,
        audit: &mut Log,
    ) -> Option<Id> {
        if expires.wrapping_sub(now) == 0 || expires.wrapping_sub(now) >= 1 << 63 {
            return None;
        }
        let (index, slot) = self.grants.iter_mut().enumerate().find(|(_, slot)| slot.is_none())?;
        if !audit.can_record() {
            return None;
        }
        let capability = capabilities.grant(kind)?;
        *slot = Some(Grant { principal, kind, capability, expires });
        debug_assert!(audit.record(principal, Effect::ApprovalGrant));
        Some(Id(index as u8))
    }

    pub fn allows(
        &self,
        capabilities: &CapabilityManager,
        id: Id,
        principal: Principal,
        kind: CapabilityKind,
        now: u64,
    ) -> bool {
        self.grants.get(id.0 as usize).and_then(|grant| *grant).is_some_and(|grant| {
            grant.principal == principal
                && grant.kind == kind
                && now.wrapping_sub(grant.expires) >= 1 << 63
                && capabilities.allows(grant.capability, kind)
        })
    }

    pub fn revoke(
        &mut self,
        capabilities: &mut CapabilityManager,
        id: Id,
        audit: &mut Log,
    ) -> bool {
        if !audit.can_record() {
            return false;
        }
        let Some(slot) = self.grants.get_mut(id.0 as usize) else {
            return false;
        };
        let Some(grant) = slot.take() else {
            return false;
        };
        capabilities.revoke(grant.capability)
            && audit.record(grant.principal, Effect::ApprovalRevoke)
    }
}

pub fn self_check() -> bool {
    let mut capabilities = CapabilityManager::new();
    let mut audit = Log::new();
    let mut grants = Store::new();
    let principal = Principal::LOCAL;
    let Some(id) =
        grants.grant(&mut capabilities, principal, CapabilityKind::Recovery, 12, 10, &mut audit)
    else {
        return false;
    };
    grants.allows(&capabilities, id, principal, CapabilityKind::Recovery, 11)
        && !grants.allows(&capabilities, id, principal, CapabilityKind::Recovery, 12)
        && grants.revoke(&mut capabilities, id, &mut audit)
        && audit.latest().is_some_and(|event| event.effect == Effect::ApprovalRevoke)
        && {
            while audit.record(principal, Effect::ApprovalGrant) {}
            grants
                .grant(&mut capabilities, principal, CapabilityKind::Recovery, 20, 10, &mut audit)
                .is_none()
        }
}
