use crate::{
    capabilities::{CapabilityKind, CapabilityManager},
    session,
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
    session: &session::Context,
    capabilities: &CapabilityManager,
) -> Result {
    if submission.as_bytes() != b"recovery" {
        return Result::Unknown;
    }
    if session.allows(capabilities, CapabilityKind::Recovery) {
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
}
