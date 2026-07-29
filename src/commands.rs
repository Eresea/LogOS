//! Everything a `Call` needs on the kernel side: capability gating and the
//! handful of actions that actually require kernel state (ACPI, scheduler,
//! service registry, IPC channel). Command *parsing* lives entirely in
//! `logos_terminal::command` now — this module never sees raw command text.
use crate::{
    capabilities::{CapabilityKind, CapabilityManager},
    session,
};
use logos_terminal::{command::Call, terminal::Submission};

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
fn required_capability(name: &[u8]) -> Result<Option<CapabilityKind>, ()> {
    match name {
        b"recovery" | b"reboot" | b"poweroff" => Ok(Some(CapabilityKind::Recovery)),
        b"ping" | b"restart" | b"cancel" => Ok(Some(CapabilityKind::Service)),
        b"tasks" | b"services" | b"drivers" | b"trace" | b"inspect" => Ok(None),
        _ => Err(()),
    }
}

pub fn dispatch(
    call: Call,
    session: &session::Context,
    capabilities: &CapabilityManager,
    invocation: Invocation,
    now: u64,
) -> Outcome {
    if let Some(error) = invocation.error(now) {
        return Outcome::Error(error);
    }
    let Ok(required) = required_capability(call.name) else {
        return Outcome::Error(Error::UnknownCommand);
    };
    if required.is_some_and(|kind| !session.allows(capabilities, kind)) {
        return Outcome::Error(Error::Denied);
    }
    match call.name {
        b"recovery" => Outcome::Recovery,
        b"reboot" => Outcome::Reboot,
        b"poweroff" => Outcome::PowerOff,
        b"ping" => Outcome::Ping,
        b"tasks" => Outcome::Tasks,
        b"services" => Outcome::Services,
        b"drivers" => Outcome::Drivers,
        b"trace" => Outcome::Trace,
        b"inspect" => call.argument.map_or(Outcome::Error(Error::UnknownCommand), Outcome::Inspect),
        b"restart" => call.argument.map_or(Outcome::Error(Error::UnknownCommand), Outcome::Restart),
        b"cancel" => call.argument.map_or(Outcome::Error(Error::UnknownCommand), Outcome::Cancel),
        _ => Outcome::Error(Error::UnknownCommand),
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

    let recovery_call = Call { name: b"recovery", argument: None };
    let reboot_call = Call { name: b"reboot", argument: None };
    let poweroff_call = Call { name: b"poweroff", argument: None };
    let ping_call = Call { name: b"ping", argument: None };
    let tasks_call = Call { name: b"tasks", argument: None };
    let services_call = Call { name: b"services", argument: None };
    let drivers_call = Call { name: b"drivers", argument: None };
    let trace_call = Call { name: b"trace", argument: None };
    let inspect_call = Call { name: b"inspect", argument: Some(target) };
    let restart_call = Call { name: b"restart", argument: Some(target) };
    let cancel_call = Call { name: b"cancel", argument: Some(target) };
    let bogus_call = Call { name: b"not-a-real-call", argument: None };

    dispatch(recovery_call, &denied_session, &capabilities, Invocation::new(2), 1)
        == Outcome::Error(Error::Denied)
        && dispatch(recovery_call, &session, &capabilities, Invocation::new(2), 1)
            == Outcome::Recovery
        && dispatch(bogus_call, &session, &capabilities, Invocation::new(2), 1)
            == Outcome::Error(Error::UnknownCommand)
        && dispatch(recovery_call, &session, &capabilities, Invocation::cancelled(2), 1)
            == Outcome::Error(Error::Cancelled)
        && dispatch(recovery_call, &session, &capabilities, Invocation::new(1), 1)
            == Outcome::Error(Error::TimedOut)
        && dispatch(reboot_call, &session, &capabilities, Invocation::new(2), 1) == Outcome::Reboot
        && dispatch(poweroff_call, &session, &capabilities, Invocation::new(2), 1)
            == Outcome::PowerOff
        && dispatch(ping_call, &session, &capabilities, Invocation::new(2), 1) == Outcome::Ping
        && dispatch(tasks_call, &denied_session, &capabilities, Invocation::new(2), 1)
            == Outcome::Tasks
        && dispatch(services_call, &denied_session, &capabilities, Invocation::new(2), 1)
            == Outcome::Services
        && dispatch(drivers_call, &denied_session, &capabilities, Invocation::new(2), 1)
            == Outcome::Drivers
        && dispatch(trace_call, &denied_session, &capabilities, Invocation::new(2), 1)
            == Outcome::Trace
        && matches!(
            dispatch(inspect_call, &denied_session, &capabilities, Invocation::new(2), 1),
            Outcome::Inspect(value) if value.as_bytes() == b"virtio-balloon"
        )
        && matches!(
            dispatch(restart_call, &session, &capabilities, Invocation::new(2), 1),
            Outcome::Restart(value) if value.as_bytes() == b"virtio-balloon"
        )
        && matches!(
            dispatch(cancel_call, &session, &capabilities, Invocation::new(2), 1),
            Outcome::Cancel(value) if value.as_bytes() == b"virtio-balloon"
        )
        && dispatch(restart_call, &denied_session, &capabilities, Invocation::new(2), 1)
            == Outcome::Error(Error::Denied)
}
