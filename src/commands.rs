use crate::{
    capabilities::{CapabilityKind, CapabilityManager},
    session,
};
use logos_terminal::terminal::Submission;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Recovery,
    Text(Submission),
    Error(Error),
}

impl Outcome {
    fn is_text(self, expected: &[u8]) -> bool {
        matches!(self, Self::Text(value) if value.as_bytes() == expected)
    }
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
    pub required_capability: Option<CapabilityKind>,
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
const TEXT_ARGUMENT: [Argument; 1] =
    [Argument { name: b"text", kind: ArgumentKind::Text, required: true }];

const DESCRIPTORS: [Descriptor; 4] = [
    Descriptor {
        name: b"recovery",
        summary: b"switch to the recovery console",
        arguments: &NO_ARGUMENTS,
        required_capability: Some(CapabilityKind::Recovery),
    },
    Descriptor {
        name: b"echo",
        summary: b"return text",
        arguments: &TEXT_ARGUMENT,
        required_capability: None,
    },
    Descriptor {
        name: b"help",
        summary: b"describe a command",
        arguments: &TEXT_ARGUMENT,
        required_capability: None,
    },
    Descriptor {
        name: b"commands",
        summary: b"list commands",
        arguments: &NO_ARGUMENTS,
        required_capability: None,
    },
];

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
    invoke_stage(submission, None, session, capabilities, invocation, now)
}

pub fn pipeline(
    submission: Submission,
    session: &session::Context,
    capabilities: &CapabilityManager,
    invocation: Invocation,
    now: u64,
) -> Outcome {
    let mut input = None;
    for stage in submission.as_bytes().split(|byte| *byte == b'|') {
        let Some(stage) = Submission::from_bytes(stage.trim_ascii()) else {
            return Outcome::Error(Error::UnknownCommand);
        };
        match invoke_stage(stage, input, session, capabilities, invocation, now) {
            Outcome::Text(value) => input = Some(value),
            outcome => return outcome,
        }
    }
    input.map_or(Outcome::Error(Error::UnknownCommand), Outcome::Text)
}

fn invoke_stage(
    submission: Submission,
    input: Option<Submission>,
    session: &session::Context,
    capabilities: &CapabilityManager,
    invocation: Invocation,
    now: u64,
) -> Outcome {
    if let Some(error) = invocation.error(now) {
        return Outcome::Error(error);
    }
    let bytes = submission.as_bytes();
    let (name, argument) = bytes
        .iter()
        .position(|byte| *byte == b' ')
        .map_or((bytes, &[][..]), |index| (&bytes[..index], &bytes[index + 1..]));
    let Some(descriptor) = descriptors().iter().find(|descriptor| descriptor.name == name) else {
        return Outcome::Error(Error::UnknownCommand);
    };
    if descriptor.required_capability.is_some_and(|kind| !session.allows(capabilities, kind)) {
        return Outcome::Error(Error::Denied);
    }
    if descriptor.name == b"recovery" {
        Outcome::Recovery
    } else if descriptor.name == b"echo" && !argument.is_empty() {
        Submission::from_bytes(argument)
            .map_or(Outcome::Error(Error::UnknownCommand), Outcome::Text)
    } else if descriptor.name == b"echo" {
        input.map_or(Outcome::Error(Error::UnknownCommand), Outcome::Text)
    } else if descriptor.name == b"commands" {
        Submission::from_bytes(b"recovery echo help commands")
            .map_or(Outcome::Error(Error::UnknownCommand), Outcome::Text)
    } else if descriptor.name == b"help" && !argument.is_empty() {
        descriptors()
            .iter()
            .find(|candidate| candidate.name == argument)
            .and_then(|candidate| Submission::from_bytes(candidate.summary))
            .map_or(Outcome::Error(Error::UnknownCommand), Outcome::Text)
    } else if descriptor.name == b"help" {
        Submission::from_bytes(b"use commands to list commands")
            .map_or(Outcome::Error(Error::UnknownCommand), Outcome::Text)
    } else {
        Outcome::Error(Error::UnknownCommand)
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
    let Some(echo) = Submission::from_bytes(b"echo hello") else {
        return false;
    };
    let Some(hello) = Submission::from_bytes(b"hello") else {
        return false;
    };
    let Some(pipe) = Submission::from_bytes(b"echo hello | echo") else {
        return false;
    };
    let Some(commands) = Submission::from_bytes(b"commands") else {
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
        && invoke(echo, &denied_session, &capabilities, Invocation::new(2), 1)
            == Outcome::Text(hello)
        && pipeline(pipe, &denied_session, &capabilities, Invocation::new(2), 1)
            == Outcome::Text(hello)
        && invoke(commands, &denied_session, &capabilities, Invocation::new(2), 1)
            .is_text(b"recovery echo help commands")
        && descriptors().len() == 4
        && descriptors()[0].arguments.is_empty()
        && text_argument.kind == ArgumentKind::Text
        && text_argument.required
}
