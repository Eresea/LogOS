#![no_main]
#![no_std]

use logos_abi::{Effect, EffectResult, SessionRequest, Syscall};
use logos_service_rt::{Context, Header};

#[used]
#[unsafe(link_section = ".logos")]
static HEADER: Header = Header::new(*b"sessions\0\0\0\0\0\0\0\0", logos_service_entry);

#[unsafe(no_mangle)]
extern "C" fn logos_service_entry(context: *mut logos_service_rt::RawContext) -> ! {
    unsafe { logos_service_rt::entry(context, run) }
}

fn run(context: &mut Context) -> ! {
    if !context.ready() {
        spin();
    }
    while context.acknowledged() {
        if !context.wait_for_input() {
            spin();
        }
        let Some(request) = context.session_request() else { continue };
        if context.input() != 1 || !request.valid() {
            continue;
        }
        let Some(result) = context.session_effect(dispatch(request.syscall)) else {
            spin();
        };
        let reply = format(&request, result);
        if !context.session_reply(reply) {
            spin();
        }
    }
    spin()
}

fn dispatch(syscall: Syscall) -> Effect {
    match syscall {
        Syscall::Recovery => Effect::EnterRecovery,
        Syscall::Reboot => Effect::ResetMachine,
        Syscall::PowerOff => Effect::PowerOffMachine,
        Syscall::Ping => Effect::PingService,
        Syscall::Tasks => Effect::ReadTasks,
        Syscall::Services => Effect::ReadServices,
        Syscall::Drivers => Effect::ReadDrivers,
        Syscall::Trace => Effect::ReadTrace,
        Syscall::Inspect => Effect::InspectResource,
        Syscall::Restart => Effect::RestartService,
        Syscall::Cancel => Effect::CancelService,
        Syscall::SetInputLayout => Effect::SetInputLayout,
    }
}

fn format(request: &SessionRequest, result: EffectResult) -> &[u8] {
    match result {
        EffectResult::Completed => match request.syscall {
            Syscall::Reboot => b"reboot requested",
            Syscall::PowerOff => b"poweroff requested",
            _ => b"ok",
        },
        EffectResult::Recovery => b"recovery requested",
        EffectResult::Unavailable => match request.syscall {
            Syscall::Reboot => b"reboot unavailable",
            Syscall::PowerOff => b"poweroff unavailable",
            Syscall::Ping => b"ping unavailable",
            Syscall::Restart | Syscall::Cancel => b"unknown or unavailable service",
            _ => b"unavailable",
        },
        EffectResult::Pong => b"pong",
        EffectResult::TasksActive => b"scheduler active",
        EffectResult::ServiceRunning => b"platform service running",
        EffectResult::ServiceOverdue => b"platform service overdue",
        EffectResult::DriverBound => b"platform driver bound",
        EffectResult::TraceNone => b"TRACE NONE\n",
        EffectResult::TraceBoot => b"TRACE BOOT\n",
        EffectResult::TraceTaskBlocked => b"TRACE TASK BLOCKED\n",
        EffectResult::TraceTaskWoken => b"TRACE TASK WOKEN\n",
        EffectResult::TraceVirtioSubmit => b"TRACE VIRTIO SUBMIT\n",
        EffectResult::TraceVirtioComplete => b"TRACE VIRTIO COMPLETE\n",
        EffectResult::TraceDriverBound => b"TRACE DRIVER BOUND\n",
        EffectResult::TraceDriverQuiesced => b"TRACE DRIVER QUIESCED\n",
        EffectResult::TraceDriverRecovered => b"TRACE DRIVER RECOVERED\n",
        EffectResult::TraceDriverFailed => b"TRACE DRIVER FAILED\n",
        EffectResult::TraceFault => b"TRACE FAULT\n",
        EffectResult::TraceSelfCheck => b"TRACE SELF CHECK\n",
        EffectResult::Inspected => &request.argument[..request.length],
        EffectResult::RestartScheduled => b"restart scheduled",
        EffectResult::CancelRequested => b"cancel requested",
        EffectResult::LayoutQwerty => b"layout qwerty",
        EffectResult::LayoutAzerty => b"layout azerty",
        EffectResult::Denied => b"permission denied",
        EffectResult::Unknown => b"unknown command",
    }
}

fn spin() -> ! {
    loop {
        core::hint::spin_loop();
    }
}
