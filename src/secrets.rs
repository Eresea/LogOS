use crate::{
    capabilities::{Capability, CapabilityKind, CapabilityManager},
    session::Principal,
};

const SECRETS: usize = 4;
const BYTES: usize = 64;

#[derive(Clone, Copy)]
struct Secret {
    owner: Principal,
    bytes: [u8; BYTES],
    len: usize,
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
        audit: &mut crate::audit::Log,
    ) -> bool {
        if bytes.is_empty()
            || bytes.len() > BYTES
            || !capabilities.allows(capability, CapabilityKind::Secret)
        {
            return false;
        }
        let Some(slot) = self.secrets.iter_mut().find(|slot| slot.is_none()) else {
            return false;
        };
        let mut secret = Secret { owner, bytes: [0; BYTES], len: bytes.len() };
        secret.bytes[..bytes.len()].copy_from_slice(bytes);
        *slot = Some(secret);
        audit.record(crate::audit::Event {
            principal: owner,
            effect: crate::audit::Effect::SecretWrite,
        })
    }

    pub fn get(
        &self,
        capabilities: &CapabilityManager,
        capability: Capability,
        owner: Principal,
    ) -> Option<&[u8]> {
        capabilities.allows(capability, CapabilityKind::Secret).then(|| {
            self.secrets
                .iter()
                .flatten()
                .find(|secret| secret.owner == owner)
                .map(|secret| &secret.bytes[..secret.len])
        })?
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
    let mut audit = crate::audit::Log::new();
    store.put(&capabilities, secret, owner, b"secret", &mut audit)
        && store.get(&capabilities, secret, owner) == Some(b"secret" as &[u8])
        && store.get(&capabilities, secret, Principal::service(2)).is_none()
        && store.get(&capabilities, service, owner).is_none()
        && audit.latest().is_some_and(|event| event.principal == owner)
}
