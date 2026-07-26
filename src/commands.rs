use crate::{
    capabilities::{CapabilityKind, CapabilityManager},
    session,
};
use logos_terminal::terminal::Submission;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Result {
    Recovery,
    Denied,
    Unknown,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Descriptor {
    pub name: &'static [u8],
    pub summary: &'static [u8],
    pub required_capability: CapabilityKind,
}

const DESCRIPTORS: [Descriptor; 1] = [Descriptor {
    name: b"recovery",
    summary: b"switch to the recovery console",
    required_capability: CapabilityKind::Recovery,
}];

pub fn descriptors() -> &'static [Descriptor] {
    &DESCRIPTORS
}

pub fn invoke(
    submission: Submission,
    session: &session::Context,
    capabilities: &CapabilityManager,
) -> Result {
    let Some(descriptor) =
        descriptors().iter().find(|descriptor| descriptor.name == submission.as_bytes())
    else {
        return Result::Unknown;
    };
    if session.allows(capabilities, descriptor.required_capability) {
        Result::Recovery
    } else {
        Result::Denied
    }
}

pub fn self_check() -> bool {
    let mut capabilities = CapabilityManager::new();
    let Some(recovery) = capabilities.grant(CapabilityKind::Recovery) else {
        return false;
    };
    let Some(session) =
        session::Context::new(session::Id(1), session::Principal::LOCAL, &[recovery])
    else {
        return false;
    };
    let Some(denied_session) =
        session::Context::new(session::Id(2), session::Principal::LOCAL, &[])
    else {
        return false;
    };
    let Some(submission) = Submission::from_bytes(b"recovery") else {
        return false;
    };
    invoke(submission, &denied_session, &capabilities) == Result::Denied
        && invoke(submission, &session, &capabilities) == Result::Recovery
        && descriptors().len() == 1
        && descriptors()[0] == DESCRIPTORS[0]
}
