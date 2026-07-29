//! Everything a `Call` needs on the kernel side: capability gating and the
//! handful of actions that actually require kernel state (ACPI, scheduler,
//! service registry, IPC channel). Command *parsing* lives entirely in
//! `logos_terminal::command` now — this module never sees raw command text.
use crate::{
    capabilities::{CapabilityKind, CapabilityManager},
    session,
};
use logos_core::native_service::Syscall;
use logos_core::native_service::Syscall as Command;
use logos_terminal::terminal::Submission;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Recovery,
    Reboot,
    PowerOff,
    Ping,
    Tasks,
    Services,
    Drivers,
    Trace,
    Inspect(Submission),
    Restart(Submission),
    Cancel(Submission),
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

/// Which capability (if any) a remote call requires. Returning `Err(())`
/// means the name isn't a call the kernel recognizes at all -- this should
/// only happen if something upstream is corrupted or out of sync with
/// `logos_terminal::command`'s descriptor table.
fn required_capability(syscall: Syscall) -> Option<CapabilityKind> {
    match syscall {
        Syscall::Recovery | Syscall::Reboot | Syscall::PowerOff => Some(CapabilityKind::Recovery),
        Syscall::Ping | Syscall::Restart | Syscall::Cancel => Some(CapabilityKind::Service),
        _ => None,
    }
}

pub fn dispatch(
    syscall: Syscall,
    argument: Option<Submission>,
    session: &session::Context,
    capabilities: &CapabilityManager,
    invocation: Invocation,
    now: u64,
) -> Outcome {
    if let Some(error) = invocation.error(now) {
        return Outcome::Error(error);
    }
    let required = required_capability(syscall);
    if required.is_some_and(|kind| !session.allows(capabilities, kind)) {
        return Outcome::Error(Error::Denied);
    }
    match syscall {
        Command::Recovery => Outcome::Recovery,
        Command::Reboot => Outcome::Reboot,
        Command::PowerOff => Outcome::PowerOff,
        Command::Ping => Outcome::Ping,
        Command::Tasks => Outcome::Tasks,
        Command::Services => Outcome::Services,
        Command::Drivers => Outcome::Drivers,
        Command::Trace => Outcome::Trace,
        Command::Inspect => {
            argument.map_or(Outcome::Error(Error::UnknownCommand), Outcome::Inspect)
        }
        Command::Restart => {
            argument.map_or(Outcome::Error(Error::UnknownCommand), Outcome::Restart)
        }
        Command::Cancel => argument.map_or(Outcome::Error(Error::UnknownCommand), Outcome::Cancel),
        Command::SetInputLayout => Outcome::Error(Error::UnknownCommand),
    }
}

pub fn self_check() -> bool {
    let mut capabilities = CapabilityManager::new();
    let Some(recovery_capability) = capabilities.grant(CapabilityKind::Recovery) else {
        return false;
    };
    let Some(service_capability) = capabilities.grant(CapabilityKind::Service) else {
        return false;
    };
    let Some(session) = session::Context::new(
        session::Id(1),
        session::Principal::LOCAL,
        &[recovery_capability, service_capability],
    ) else {
        return false;
    };
    let Some(denied_session) =
        session::Context::new(session::Id(2), session::Principal::LOCAL, &[])
    else {
        return false;
    };
    let Some(target) = Submission::from_bytes(b"virtio-balloon") else {
        return false;
    };

    let recovery_call = Command::Recovery;
    let reboot_call = Command::Reboot;
    let poweroff_call = Command::PowerOff;
    let ping_call = Command::Ping;
    let tasks_call = Command::Tasks;
    let services_call = Command::Services;
    let drivers_call = Command::Drivers;
    let trace_call = Command::Trace;
    let inspect_call = Command::Inspect;
    let restart_call = Command::Restart;
    let cancel_call = Command::Cancel;

    dispatch(recovery_call, None, &denied_session, &capabilities, Invocation::new(2), 1)
        == Outcome::Error(Error::Denied)
        && dispatch(recovery_call, None, &session, &capabilities, Invocation::new(2), 1)
            == Outcome::Recovery
        && dispatch(recovery_call, None, &session, &capabilities, Invocation::cancelled(2), 1)
            == Outcome::Error(Error::Cancelled)
        && dispatch(recovery_call, None, &session, &capabilities, Invocation::new(1), 1)
            == Outcome::Error(Error::TimedOut)
        && dispatch(reboot_call, None, &session, &capabilities, Invocation::new(2), 1)
            == Outcome::Reboot
        && dispatch(poweroff_call, None, &session, &capabilities, Invocation::new(2), 1)
            == Outcome::PowerOff
        && dispatch(ping_call, None, &session, &capabilities, Invocation::new(2), 1)
            == Outcome::Ping
        && dispatch(tasks_call, None, &denied_session, &capabilities, Invocation::new(2), 1)
            == Outcome::Tasks
        && dispatch(services_call, None, &denied_session, &capabilities, Invocation::new(2), 1)
            == Outcome::Services
        && dispatch(drivers_call, None, &denied_session, &capabilities, Invocation::new(2), 1)
            == Outcome::Drivers
        && dispatch(trace_call, None, &denied_session, &capabilities, Invocation::new(2), 1)
            == Outcome::Trace
        && matches!(
            dispatch(inspect_call, Some(target), &denied_session, &capabilities, Invocation::new(2), 1),
            Outcome::Inspect(value) if value.as_bytes() == b"virtio-balloon"
        )
        && matches!(
            dispatch(restart_call, Some(target), &session, &capabilities, Invocation::new(2), 1),
            Outcome::Restart(value) if value.as_bytes() == b"virtio-balloon"
        )
        && matches!(
            dispatch(cancel_call, Some(target), &session, &capabilities, Invocation::new(2), 1),
            Outcome::Cancel(value) if value.as_bytes() == b"virtio-balloon"
        )
        && dispatch(
            restart_call,
            Some(target),
            &denied_session,
            &capabilities,
            Invocation::new(2),
            1,
        ) == Outcome::Error(Error::Denied)
}
