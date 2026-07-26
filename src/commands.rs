use crate::{
    capabilities::{CapabilityKind, CapabilityManager},
    session,
};
use logos_terminal::terminal::Submission;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Recovery,
    Error(Error),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Denied,
    UnknownCommand,
    Cancelled,
    TimedOut,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Invocation {
    deadline: u64,
    cancelled: bool,
}

impl Invocation {
    pub const fn new(deadline: u64) -> Self {
        Self { deadline, cancelled: false }
    }

    pub const fn cancelled(deadline: u64) -> Self {
        Self { deadline, cancelled: true }
    }

    fn error(self, now: u64) -> Option<Error> {
        if self.cancelled {
            Some(Error::Cancelled)
        } else if now.wrapping_sub(self.deadline) < 1 << 63 {
            Some(Error::TimedOut)
        } else {
            None
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Descriptor {
    pub name: &'static [u8],
    pub summary: &'static [u8],
    pub arguments: &'static [Argument],
    pub required_capability: CapabilityKind,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ArgumentKind {
    Text,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Argument {
    pub name: &'static [u8],
    pub kind: ArgumentKind,
    pub required: bool,
}

const NO_ARGUMENTS: [Argument; 0] = [];

const DESCRIPTORS: [Descriptor; 1] = [Descriptor {
    name: b"recovery",
    summary: b"switch to the recovery console",
    arguments: &NO_ARGUMENTS,
    required_capability: CapabilityKind::Recovery,
}];

pub fn descriptors() -> &'static [Descriptor] {
    &DESCRIPTORS
}

pub fn invoke(
    submission: Submission,
    session: &session::Context,
    capabilities: &CapabilityManager,
    invocation: Invocation,
    now: u64,
) -> Outcome {
    if let Some(error) = invocation.error(now) {
        return Outcome::Error(error);
    }
    let Some(descriptor) =
        descriptors().iter().find(|descriptor| descriptor.name == submission.as_bytes())
    else {
        return Outcome::Error(Error::UnknownCommand);
    };
    if session.allows(capabilities, descriptor.required_capability) {
        Outcome::Recovery
    } else {
        Outcome::Error(Error::Denied)
    }
}

pub fn self_check() -> bool {
    let text_argument = Argument { name: b"target", kind: ArgumentKind::Text, required: true };
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
    let Some(unknown) = Submission::from_bytes(b"missing") else {
        return false;
    };
    invoke(submission, &denied_session, &capabilities, Invocation::new(2), 1)
        == Outcome::Error(Error::Denied)
        && invoke(submission, &session, &capabilities, Invocation::new(2), 1) == Outcome::Recovery
        && invoke(unknown, &session, &capabilities, Invocation::new(2), 1)
            == Outcome::Error(Error::UnknownCommand)
        && invoke(submission, &session, &capabilities, Invocation::cancelled(2), 1)
            == Outcome::Error(Error::Cancelled)
        && invoke(submission, &session, &capabilities, Invocation::new(1), 1)
            == Outcome::Error(Error::TimedOut)
        && descriptors().len() == 1
        && descriptors()[0] == DESCRIPTORS[0]
        && descriptors()[0].arguments.is_empty()
        && text_argument.kind == ArgumentKind::Text
        && text_argument.required
}
