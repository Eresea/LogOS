use crate::platform::session::Principal;
use logos_core::capabilities::{Capability, CapabilityKind, CapabilityManager};
use logos_remote::{Bootstrap, Csprng, ENROLLMENT_BLOB_BYTES, Enrollment, TrustState};

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

pub struct RemoteState {
    trust: TrustState,
    storage_key: [u8; 32],
    rng: Csprng,
}

impl RemoteState {
    pub fn new(bootstrap: Bootstrap) -> Option<Self> {
        Some(Self {
            trust: TrustState::new(bootstrap.device_key).ok()?,
            storage_key: bootstrap.storage_key,
            rng: Csprng::from_seed(bootstrap.rng_seed),
        })
    }

    pub fn unavailable(bootstrap: Bootstrap) -> Self {
        Self {
            trust: TrustState::unavailable(bootstrap.device_key),
            storage_key: bootstrap.storage_key,
            rng: Csprng::from_seed(bootstrap.rng_seed),
        }
    }

    pub fn available(&self) -> bool {
        self.trust.available()
    }

    pub fn machine_public(&self) -> [u8; 32] {
        self.trust.machine_public()
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn enrollment(&self) -> Enrollment {
        self.trust.enrollment()
    }

    pub fn enroll(&mut self, client_key: [u8; 32]) -> Option<u64> {
        self.trust.enroll(client_key).ok()
    }

    pub fn unenroll(&mut self) -> Option<u64> {
        self.trust.unenroll().ok()
    }

    pub fn seal_enrollment(
        &self,
        nonce: &[u8; 24],
        output: &mut [u8; ENROLLMENT_BLOB_BYTES],
    ) -> bool {
        self.trust.seal_enrollment_blob(&self.storage_key, nonce, output).is_ok()
    }

    pub fn load_enrollment(bootstrap: Bootstrap, input: &mut [u8; ENROLLMENT_BLOB_BYTES]) -> Self {
        Self {
            trust: TrustState::open_enrollment_blob(
                bootstrap.device_key,
                &bootstrap.storage_key,
                input,
            ),
            storage_key: bootstrap.storage_key,
            rng: Csprng::from_seed(bootstrap.rng_seed),
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn storage_key(&self) -> [u8; 32] {
        self.storage_key
    }

    pub fn seal_enrollment_random(&mut self, output: &mut [u8; ENROLLMENT_BLOB_BYTES]) -> bool {
        let mut nonce = [0; 24];
        self.rng.fill(&mut nonce);
        self.seal_enrollment(&nonce, output)
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_state_round_trip_is_fail_closed() {
        let bootstrap = Bootstrap::from_root(&[7; 32], &[9; 32]).unwrap();
        let mut state = RemoteState::new(bootstrap).unwrap();
        let generation = state.enroll([8; 32]).unwrap();
        let mut blob = [0; ENROLLMENT_BLOB_BYTES];
        let nonce = logos_remote::protected_nonce(&state.storage_key(), generation);
        assert!(state.seal_enrollment(&nonce, &mut blob));
        let loaded = RemoteState::load_enrollment(bootstrap, &mut blob);
        assert!(loaded.available() && loaded.enrollment().generation == generation);
        blob[0] ^= 1;
        assert!(!RemoteState::load_enrollment(bootstrap, &mut blob).available());
    }
}
