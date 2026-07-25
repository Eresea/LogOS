use crate::{
    capabilities::{Capability, CapabilityKind, CapabilityManager},
    terminal::Submission,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Result {
    Recovery,
    Denied,
    Unknown,
}

pub fn invoke(
    submission: Submission,
    capabilities: &CapabilityManager,
    capability: Capability,
) -> Result {
    if submission.as_bytes() != b"recovery" {
        return Result::Unknown;
    }
    if capabilities.allows(capability, CapabilityKind::Recovery) {
        Result::Recovery
    } else {
        Result::Denied
    }
}

pub fn self_check() -> bool {
    let mut capabilities = CapabilityManager::new();
    let Some(debug) = capabilities.grant(CapabilityKind::Debug) else {
        return false;
    };
    let Some(recovery) = capabilities.grant(CapabilityKind::Recovery) else {
        return false;
    };
    let submission = Submission::new(
        [b'r', b'e', b'c', b'o', b'v', b'e', b'r', b'y', 0, 0, 0, 0, 0, 0, 0, 0],
        8,
    );
    invoke(submission, &capabilities, debug) == Result::Denied
        && invoke(submission, &capabilities, recovery) == Result::Recovery
}
