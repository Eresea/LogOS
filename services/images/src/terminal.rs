#![no_std]
#![no_main]

mod common;

use logos_abi::{InputMessage, IpcBytes, IpcStatus, KeyCode, KeyState, MessageKind};

const INPUT_CAPABILITY: usize = 0;
const DISPLAY_CAPABILITY: usize = 1;
const SESSION_INPUT_CAPABILITY: usize = 2;
const SESSION_OUTPUT_CAPABILITY: usize = 3;

static mut TERMINAL: logos_terminal::TerminalService = logos_terminal::TerminalService::new();
static mut PENDING_RENDER: Option<logos_abi::RenderMessage> = None;
static mut PENDING_SESSION_INPUT: Option<IpcBytes> = None;

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    let terminal = unsafe { &mut *core::ptr::addr_of_mut!(TERMINAL) };
    let pending_render = unsafe { &mut *core::ptr::addr_of_mut!(PENDING_RENDER) };
    let pending_session_input = unsafe { &mut *core::ptr::addr_of_mut!(PENDING_SESSION_INPUT) };
    let mut heartbeat_ticks = 0u16;
    loop {
        common::heartbeat_tick(&mut heartbeat_ticks, logos_abi::ServiceId::Terminal);
        let mut wait_mask = 0;
        if let Some(message) = *pending_session_input {
            match common::ipc_send(SESSION_INPUT_CAPABILITY, &message) {
                IpcStatus::Ok => {
                    *pending_session_input = None;
                }
                IpcStatus::Full => wait_mask |= common::ipc_write_event(2),
                IpcStatus::Stale
                | IpcStatus::Disconnected
                | IpcStatus::Unauthorized
                | IpcStatus::Malformed
                | IpcStatus::Empty => *pending_session_input = None,
            }
        }
        if pending_session_input.is_none() {
            let mut event = InputMessage::key(KeyCode::Unknown, KeyState::Released, 0);
            while common::ipc_receive(INPUT_CAPABILITY, &mut event) == IpcStatus::Ok {
                if let Some(message) = terminal.input(&event) {
                    match common::ipc_send(SESSION_INPUT_CAPABILITY, &message) {
                        IpcStatus::Ok => {}
                        IpcStatus::Full => {
                            *pending_session_input = Some(message);
                            wait_mask |= common::ipc_write_event(2);
                            break;
                        }
                        IpcStatus::Stale
                        | IpcStatus::Disconnected
                        | IpcStatus::Unauthorized
                        | IpcStatus::Malformed
                        | IpcStatus::Empty => {}
                    }
                }
            }
            if pending_session_input.is_none() {
                wait_mask |= common::ipc_read_event(0);
            }
        }
        let mut message = IpcBytes::empty(MessageKind::SessionOutput);
        while common::ipc_receive(SESSION_OUTPUT_CAPABILITY, &mut message) == IpcStatus::Ok {
            if let Some(bytes) = message.as_bytes() {
                terminal.session_output_bytes(bytes);
            }
        }
        wait_mask |= common::ipc_read_event(3);
        if pending_render.is_none() {
            *pending_render = terminal.next_render();
        }
        if let Some(render) = *pending_render {
            match common::ipc_send(DISPLAY_CAPABILITY, &render) {
                IpcStatus::Ok => {
                    *pending_render = None;
                }
                IpcStatus::Full => wait_mask |= common::ipc_write_event(1),
                IpcStatus::Stale
                | IpcStatus::Disconnected
                | IpcStatus::Unauthorized
                | IpcStatus::Malformed
                | IpcStatus::Empty => *pending_render = None,
            }
        }
        if wait_mask != 0 {
            common::wait(wait_mask, logos_abi::ServiceId::Terminal);
        }
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo<'_>) -> ! {
    common::idle()
}
