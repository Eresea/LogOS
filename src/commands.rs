use crate::{
    capabilities::{CapabilityKind, CapabilityManager},
    session,
};
use logos_terminal::{input::Layout, terminal::Submission};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Recovery,
    Reboot,
    PowerOff,
    Clear,
    Layout(Layout),
    Tasks,
    Services,
    Drivers,
    Trace,
    Inspect(Submission),
    Restart,
    Cancel,
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
const LAYOUT_ARGUMENT: [Argument; 1] =
    [Argument { name: b"layout", kind: ArgumentKind::Text, required: true }];

const DESCRIPTORS: [Descriptor; 16] = [
    Descriptor {
        name: b"health",
        summary: b"show machine health",
        arguments: &NO_ARGUMENTS,
        required_capability: None,
    },
    Descriptor {
        name: b"clear",
        summary: b"clear terminal output",
        arguments: &NO_ARGUMENTS,
        required_capability: None,
    },
    Descriptor {
        name: b"layout",
        summary: b"set keyboard layout: qwerty or azerty",
        arguments: &LAYOUT_ARGUMENT,
        required_capability: None,
    },
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
    Descriptor {
        name: b"reboot",
        summary: b"restart the machine",
        arguments: &NO_ARGUMENTS,
        required_capability: Some(CapabilityKind::Recovery),
    },
    Descriptor {
        name: b"poweroff",
        summary: b"turn off the machine",
        arguments: &NO_ARGUMENTS,
        required_capability: Some(CapabilityKind::Recovery),
    },
    Descriptor {
        name: b"tasks",
        summary: b"list tasks",
        arguments: &NO_ARGUMENTS,
        required_capability: None,
    },
    Descriptor {
        name: b"services",
        summary: b"list services",
        arguments: &NO_ARGUMENTS,
        required_capability: None,
    },
    Descriptor {
        name: b"drivers",
        summary: b"list drivers",
        arguments: &NO_ARGUMENTS,
        required_capability: None,
    },
    Descriptor {
        name: b"trace",
        summary: b"show latest trace",
        arguments: &NO_ARGUMENTS,
        required_capability: None,
    },
    Descriptor {
        name: b"inspect",
        summary: b"inspect a resource",
        arguments: &TEXT_ARGUMENT,
        required_capability: None,
    },
    Descriptor {
        name: b"restart",
        summary: b"restart a service",
        arguments: &TEXT_ARGUMENT,
        required_capability: Some(CapabilityKind::Service),
    },
    Descriptor {
        name: b"cancel",
        summary: b"cancel a service request",
        arguments: &TEXT_ARGUMENT,
        required_capability: Some(CapabilityKind::Service),
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
    } else if descriptor.name == b"reboot" {
        Outcome::Reboot
    } else if descriptor.name == b"poweroff" {
        Outcome::PowerOff
    } else if descriptor.name == b"tasks" {
        Outcome::Tasks
    } else if descriptor.name == b"services" {
        Outcome::Services
    } else if descriptor.name == b"drivers" {
        Outcome::Drivers
    } else if descriptor.name == b"trace" {
        Outcome::Trace
    } else if descriptor.name == b"inspect" && !argument.is_empty() {
        Submission::from_bytes(argument)
            .map_or(Outcome::Error(Error::UnknownCommand), Outcome::Inspect)
    } else if descriptor.name == b"restart" && argument == b"virtio-balloon" {
        Outcome::Restart
    } else if descriptor.name == b"cancel" && argument == b"virtio-balloon" {
        Outcome::Cancel
    } else if descriptor.name == b"health" {
        Submission::from_bytes(b"healthy")
            .map_or(Outcome::Error(Error::UnknownCommand), Outcome::Text)
    } else if descriptor.name == b"clear" {
        Outcome::Clear
    } else if descriptor.name == b"layout" && argument == b"qwerty" {
        Outcome::Layout(Layout::Qwerty)
    } else if descriptor.name == b"layout" && argument == b"azerty" {
        Outcome::Layout(Layout::Azerty)
    } else if descriptor.name == b"echo" && !argument.is_empty() {
        Submission::from_bytes(argument)
            .map_or(Outcome::Error(Error::UnknownCommand), Outcome::Text)
    } else if descriptor.name == b"echo" {
        input.map_or(Outcome::Error(Error::UnknownCommand), Outcome::Text)
    } else if descriptor.name == b"commands" {
        Submission::from_bytes(b"16 commands")
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
    let Some(service) = capabilities.grant(CapabilityKind::Service) else {
        return false;
    };
    let Some(session) =
        session::Context::new(session::Id(1), session::Principal::LOCAL, &[recovery, service])
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
    let Some(reboot) = Submission::from_bytes(b"reboot") else {
        return false;
    };
    let Some(poweroff) = Submission::from_bytes(b"poweroff") else {
        return false;
    };
    let Some(health) = Submission::from_bytes(b"health") else {
        return false;
    };
    let Some(clear) = Submission::from_bytes(b"clear") else {
        return false;
    };
    let Some(qwerty) = Submission::from_bytes(b"layout qwerty") else {
        return false;
    };
    let Some(azerty) = Submission::from_bytes(b"layout azerty") else {
        return false;
    };
    let Some(tasks) = Submission::from_bytes(b"tasks") else {
        return false;
    };
    let Some(services) = Submission::from_bytes(b"services") else {
        return false;
    };
    let Some(drivers) = Submission::from_bytes(b"drivers") else {
        return false;
    };
    let Some(trace) = Submission::from_bytes(b"trace") else {
        return false;
    };
    let Some(inspect) = Submission::from_bytes(b"inspect service:/virtio-balloon") else {
        return false;
    };
    let Some(restart) = Submission::from_bytes(b"restart virtio-balloon") else {
        return false;
    };
    let Some(cancel) = Submission::from_bytes(b"cancel virtio-balloon") else {
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
            .is_text(b"16 commands")
        && invoke(reboot, &session, &capabilities, Invocation::new(2), 1) == Outcome::Reboot
        && invoke(poweroff, &session, &capabilities, Invocation::new(2), 1) == Outcome::PowerOff
        && invoke(health, &denied_session, &capabilities, Invocation::new(2), 1).is_text(b"healthy")
        && invoke(clear, &denied_session, &capabilities, Invocation::new(2), 1) == Outcome::Clear
        && invoke(qwerty, &denied_session, &capabilities, Invocation::new(2), 1)
            == Outcome::Layout(Layout::Qwerty)
        && invoke(azerty, &denied_session, &capabilities, Invocation::new(2), 1)
            == Outcome::Layout(Layout::Azerty)
        && invoke(tasks, &denied_session, &capabilities, Invocation::new(2), 1) == Outcome::Tasks
        && invoke(services, &denied_session, &capabilities, Invocation::new(2), 1)
            == Outcome::Services
        && invoke(drivers, &denied_session, &capabilities, Invocation::new(2), 1)
            == Outcome::Drivers
        && invoke(trace, &denied_session, &capabilities, Invocation::new(2), 1) == Outcome::Trace
        && matches!(
            invoke(inspect, &denied_session, &capabilities, Invocation::new(2), 1),
            Outcome::Inspect(_)
        )
        && invoke(restart, &session, &capabilities, Invocation::new(2), 1) == Outcome::Restart
        && invoke(cancel, &session, &capabilities, Invocation::new(2), 1) == Outcome::Cancel
        && descriptors().len() == 16
        && descriptors()[3].arguments.is_empty()
        && text_argument.kind == ArgumentKind::Text
        && text_argument.required
}
