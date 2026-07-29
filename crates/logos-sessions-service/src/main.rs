#![no_main]
#![no_std]

use core::arch::asm;
use logos_abi::{Effect, EffectResult, SessionRequest, Syscall};
use logos_core::native_service::{
    ACKNOWLEDGED, Context, Header, MAX_TEXT, READ_INPUT, READY, SESSION_EFFECT, SESSION_REPLY,
};
use uefi::{Status, prelude::*};

#[used]
#[unsafe(link_section = ".logos")]
static HEADER: Header = Header::new(*b"sessions\0\0\0\0\0\0\0\0", logos_service_entry);

#[unsafe(no_mangle)]
extern "C" fn logos_service_entry(context: *mut Context) -> ! {
    unsafe {
        (*context).operation = READY;
        asm!("int 0x80");
        while (*context).status == ACKNOWLEDGED {
            (*context).operation = READ_INPUT;
            asm!("int 0x80");
            let request = Syscall::from_wire((*context).x).map(|syscall| {
                SessionRequest::new(
                    syscall,
                    (*context).text,
                    usize::try_from((*context).text_length).unwrap_or(MAX_TEXT + 1),
                )
            });
            if (*context).input == 1 && request.is_some_and(SessionRequest::valid) {
                let request = request.unwrap();
                (*context).x = dispatch(request.syscall) as u32;
                (*context).operation = SESSION_EFFECT;
                asm!("int 0x80");
                let result = EffectResult::from_wire((*context).x).unwrap_or(EffectResult::Unknown);
                let reply = format(&request, result);
                (*context).text = [0; MAX_TEXT];
                (&mut (*context).text)[..reply.len()].copy_from_slice(reply);
                (*context).text_length = reply.len() as u32;
                (*context).operation = SESSION_REPLY;
                asm!("int 0x80");
            }
        }
    }
    loop {
        core::hint::spin_loop();
    }
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

#[entry]
fn main() -> Status {
    Status::SUCCESS
}
