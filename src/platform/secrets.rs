use crate::platform::session::Principal;
use logos_core::capabilities::{Capability, CapabilityKind, CapabilityManager};

const SECRETS: usize = 4;
const BYTES: usize = 64;

#[derive(Clone, Copy)]
struct Secret {
    owner: Principal,
    bytes: [u8; BYTES],
}

pub struct Store {
    secrets: [Option<Secret>; SECRETS],
}

impl Store {
    pub const fn new() -> Self {
        Self { secrets: [None; SECRETS] }
    }

    pub fn put(
        &mut self,
        capabilities: &CapabilityManager,
        capability: Capability,
        owner: Principal,
        bytes: &[u8],
        audit: &mut crate::platform::audit::Log,
    ) -> bool {
        if bytes.is_empty()
            || bytes.len() > BYTES
            || !capabilities.allows(capability, CapabilityKind::Secret)
            || !audit.can_record()
        {
            return false;
        }
        let Some(slot) = self.secrets.iter_mut().find(|slot| slot.is_none()) else {
            return false;
        };
        let mut secret = Secret { owner, bytes: [0; BYTES] };
        secret.bytes[..bytes.len()].copy_from_slice(bytes);
        *slot = Some(secret);
        debug_assert!(audit.record(owner, crate::platform::audit::Effect::SecretWrite));
        true
    }

    pub fn has_secret(
        &self,
        capabilities: &CapabilityManager,
        capability: Capability,
        owner: Principal,
    ) -> bool {
        if !capabilities.allows(capability, CapabilityKind::Secret) {
            return false;
        }
        self.secrets.iter().flatten().any(|secret| secret.owner == owner)
    }
}

pub fn self_check() -> bool {
    let mut capabilities = CapabilityManager::new();
    let Some(secret) = capabilities.grant(CapabilityKind::Secret) else {
        return false;
    };
    let Some(service) = capabilities.grant(CapabilityKind::Service) else {
        return false;
    };
    let owner = Principal::service(1);
    let mut store = Store::new();
    let mut audit = crate::platform::audit::Log::new();
    store.put(&capabilities, secret, owner, b"secret", &mut audit)
        && store.has_secret(&capabilities, secret, owner)
        && !store.has_secret(&capabilities, secret, Principal::service(2))
        && !store.has_secret(&capabilities, service, owner)
        && audit.latest().is_some_and(|event| event.principal == owner)
        && {
            while audit.record(owner, crate::platform::audit::Effect::SecretWrite) {}
            !store.put(&capabilities, secret, Principal::service(2), b"blocked", &mut audit)
                && !store.has_secret(&capabilities, secret, Principal::service(2))
        }
}
