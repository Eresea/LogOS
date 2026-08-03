//! Command grammar for the terminal. This module owns parsing and decides,
//! per command, whether it can be answered entirely within the terminal task
//! (`Local`) or whether it needs a round trip to the kernel (`Call`). Only
//! `Call` ever needs to leave this crate: the kernel does not know about
//! `echo`, `help`, `commands`, `clear`, `layout`, or `health` at all.
use crate::{input::Layout, terminal::Submission};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    Local(Local),
    Call(Call),
    Error(Error),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Local {
    Text(Submission),
    Clear,
    Layout(Layout),
    CommandList,
}

/// A minimal, generic request bound for the kernel: a command name and an
/// optional argument. The kernel decides what capability it requires and
/// what it means; this crate does not know either.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Call {
    pub name: &'static [u8],
    pub argument: Option<Submission>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Error {
    UnknownCommand,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Descriptor {
    pub name: &'static [u8],
    pub summary: &'static [u8],
    pub takes_argument: bool,
    /// If true, this command cannot be resolved locally and must be sent to
    /// the kernel as a `Call`.
    pub remote: bool,
}

pub const COMMAND_LIST: [&[u8]; 8] = [
    b"health clear",
    b"layout recovery",
    b"echo help",
    b"commands reboot",
    b"ping poweroff",
    b"tasks services",
    b"drivers trace",
    b"inspect restart cancel remote-key enroll unenroll",
];

const DESCRIPTORS: [Descriptor; 20] = [
    Descriptor {
        name: b"health",
        summary: b"show machine health",
        takes_argument: false,
        remote: false,
    },
    Descriptor {
        name: b"clear",
        summary: b"clear terminal output",
        takes_argument: false,
        remote: false,
    },
    Descriptor {
        name: b"layout",
        summary: b"set keyboard layout: qwerty or azerty",
        takes_argument: true,
        remote: false,
    },
    Descriptor {
        name: b"recovery",
        summary: b"switch to the recovery console",
        takes_argument: false,
        remote: true,
    },
    Descriptor { name: b"echo", summary: b"return text", takes_argument: true, remote: false },
    Descriptor {
        name: b"help",
        summary: b"describe a command",
        takes_argument: true,
        remote: false,
    },
    Descriptor {
        name: b"commands",
        summary: b"list commands",
        takes_argument: false,
        remote: false,
    },
    Descriptor {
        name: b"reboot",
        summary: b"restart the machine",
        takes_argument: false,
        remote: true,
    },
    Descriptor {
        name: b"poweroff",
        summary: b"turn off the machine",
        takes_argument: false,
        remote: true,
    },
    Descriptor {
        name: b"ping",
        summary: b"ping the platform service and await pong",
        takes_argument: false,
        remote: true,
    },
    Descriptor { name: b"tasks", summary: b"list tasks", takes_argument: false, remote: true },
    Descriptor {
        name: b"services",
        summary: b"list services",
        takes_argument: false,
        remote: true,
    },
    Descriptor { name: b"drivers", summary: b"list drivers", takes_argument: false, remote: true },
    Descriptor {
        name: b"trace",
        summary: b"show latest trace",
        takes_argument: false,
        remote: true,
    },
    Descriptor {
        name: b"inspect",
        summary: b"inspect a resource",
        takes_argument: true,
        remote: true,
    },
    Descriptor {
        name: b"restart",
        summary: b"restart a service",
        takes_argument: true,
        remote: true,
    },
    Descriptor {
        name: b"cancel",
        summary: b"cancel a service request",
        takes_argument: true,
        remote: true,
    },
    Descriptor {
        name: b"remote-key",
        summary: b"show the machine public key",
        takes_argument: false,
        remote: true,
    },
    Descriptor {
        name: b"enroll",
        summary: b"enroll one X25519 client key",
        takes_argument: true,
        remote: true,
    },
    Descriptor {
        name: b"unenroll",
        summary: b"revoke the enrolled client",
        takes_argument: false,
        remote: true,
    },
];

pub fn descriptors() -> &'static [Descriptor] {
    &DESCRIPTORS
}

/// Resolve a single submission with no piped input.
pub fn resolve(submission: Submission) -> Resolution {
    resolve_stage(submission, None)
}

/// Resolve a `|`-separated pipeline. Only `Local::Text` stages can feed the
/// next stage; the first stage that is not `Local::Text` (a `Call`, an
/// `Error`, or a non-text `Local`) ends the pipeline and becomes its result.
pub fn pipeline(submission: Submission) -> Resolution {
    let mut input = None;
    for stage in submission.as_bytes().split(|byte| *byte == b'|') {
        let Some(stage) = Submission::from_bytes(stage.trim_ascii()) else {
            return Resolution::Error(Error::UnknownCommand);
        };
        match resolve_stage(stage, input) {
            Resolution::Local(Local::Text(value)) => input = Some(value),
            other => return other,
        }
    }
    input.map_or(Resolution::Error(Error::UnknownCommand), |value| {
        Resolution::Local(Local::Text(value))
    })
}

fn resolve_stage(submission: Submission, input: Option<Submission>) -> Resolution {
    let bytes = submission.as_bytes();
    let (name, argument) = bytes
        .iter()
        .position(|byte| *byte == b' ')
        .map_or((bytes, &[][..]), |index| (&bytes[..index], &bytes[index + 1..]));
    let Some(descriptor) = descriptors().iter().find(|descriptor| descriptor.name == name) else {
        return Resolution::Error(Error::UnknownCommand);
    };

    if descriptor.remote {
        let argument = if argument.is_empty() { None } else { Submission::from_bytes(argument) };
        if descriptor.takes_argument && argument.is_none() {
            return Resolution::Error(Error::UnknownCommand);
        }
        return Resolution::Call(Call { name: descriptor.name, argument });
    }

    match descriptor.name {
        b"health" => Submission::from_bytes(b"healthy")
            .map_or(Resolution::Error(Error::UnknownCommand), |value| {
                Resolution::Local(Local::Text(value))
            }),
        b"clear" => Resolution::Local(Local::Clear),
        b"layout" if argument == b"qwerty" => Resolution::Local(Local::Layout(Layout::Qwerty)),
        b"layout" if argument == b"azerty" => Resolution::Local(Local::Layout(Layout::Azerty)),
        b"echo" if !argument.is_empty() => Submission::from_bytes(argument)
            .map_or(Resolution::Error(Error::UnknownCommand), |value| {
                Resolution::Local(Local::Text(value))
            }),
        b"echo" => input.map_or(Resolution::Error(Error::UnknownCommand), |value| {
            Resolution::Local(Local::Text(value))
        }),
        b"commands" => Resolution::Local(Local::CommandList),
        b"help" if !argument.is_empty() => descriptors()
            .iter()
            .find(|candidate| candidate.name == argument)
            .and_then(|candidate| Submission::from_bytes(candidate.summary))
            .map_or(Resolution::Error(Error::UnknownCommand), |value| {
                Resolution::Local(Local::Text(value))
            }),
        b"help" => Submission::from_bytes(b"use commands to list commands")
            .map_or(Resolution::Error(Error::UnknownCommand), |value| {
                Resolution::Local(Local::Text(value))
            }),
        _ => Resolution::Error(Error::UnknownCommand),
    }
}

pub fn self_check() -> bool {
    let Some(recovery) = Submission::from_bytes(b"recovery") else { return false };
    let Some(reboot) = Submission::from_bytes(b"reboot") else { return false };
    let Some(inspect) = Submission::from_bytes(b"inspect service:/virtio-balloon") else {
        return false;
    };
    let Some(restart) = Submission::from_bytes(b"restart virtio-balloon") else { return false };
    let Some(bare_restart) = Submission::from_bytes(b"restart") else { return false };
    let Some(unknown) = Submission::from_bytes(b"missing") else { return false };
    let Some(echo) = Submission::from_bytes(b"echo hello") else { return false };
    let Some(hello) = Submission::from_bytes(b"hello") else { return false };
    let Some(pipe) = Submission::from_bytes(b"echo hello | echo") else { return false };
    let Some(pipe_into_call) = Submission::from_bytes(b"echo hello | reboot") else {
        return false;
    };
    let Some(commands) = Submission::from_bytes(b"commands") else { return false };
    let Some(health) = Submission::from_bytes(b"health") else { return false };
    let Some(clear) = Submission::from_bytes(b"clear") else { return false };
    let Some(qwerty) = Submission::from_bytes(b"layout qwerty") else { return false };

    let recovery_is_call =
        matches!(resolve(recovery), Resolution::Call(Call { name: b"recovery", argument: None }));
    let reboot_is_call =
        matches!(resolve(reboot), Resolution::Call(Call { name: b"reboot", argument: None }));
    let inspect_is_call = matches!(
        resolve(inspect),
        Resolution::Call(Call { name: b"inspect", argument: Some(target) })
            if target.as_bytes() == b"service:/virtio-balloon"
    );
    let restart_is_call = matches!(
        resolve(restart),
        Resolution::Call(Call { name: b"restart", argument: Some(target) })
            if target.as_bytes() == b"virtio-balloon"
    );
    let bare_restart_errors =
        matches!(resolve(bare_restart), Resolution::Error(Error::UnknownCommand));
    let unknown_errors = matches!(resolve(unknown), Resolution::Error(Error::UnknownCommand));
    let echo_is_local = resolve(echo) == Resolution::Local(Local::Text(hello));
    let pipe_is_local = pipeline(pipe) == Resolution::Local(Local::Text(hello));
    let pipe_into_call_is_call =
        matches!(pipeline(pipe_into_call), Resolution::Call(Call { name: b"reboot", .. }));
    let commands_is_local = resolve(commands) == Resolution::Local(Local::CommandList);
    let health_is_local = matches!(
        resolve(health),
        Resolution::Local(Local::Text(value)) if value.as_bytes() == b"healthy"
    );
    let clear_is_local = resolve(clear) == Resolution::Local(Local::Clear);
    let layout_is_local = resolve(qwerty) == Resolution::Local(Local::Layout(Layout::Qwerty));

    recovery_is_call
        && reboot_is_call
        && inspect_is_call
        && restart_is_call
        && bare_restart_errors
        && unknown_errors
        && echo_is_local
        && pipe_is_local
        && pipe_into_call_is_call
        && commands_is_local
        && health_is_local
        && clear_is_local
        && layout_is_local
        && descriptors().len() == 20
        && COMMAND_LIST.len() == 8
}
