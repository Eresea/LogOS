#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", no_main)]
#![cfg_attr(not(target_os = "none"), allow(dead_code, unused_imports, unused_variables))]

mod common;

use logos_abi::{InputMessage, IpcBytes, IpcStatus, KeyCode, KeyState, MessageKind};

const INPUT_CAPABILITY: common::CapabilitySpec = common::capability_contract(
    logos_abi::IPC_CONTRACT_INPUT,
    logos_abi::ServiceId::Input.index() as u32,
    core::mem::size_of::<InputMessage>(),
    logos_abi::IpcRights::Receive,
);
const DISPLAY_CAPABILITY: common::CapabilitySpec = common::capability_contract(
    logos_abi::IPC_CONTRACT_RENDER,
    logos_abi::ServiceId::Display.index() as u32,
    core::mem::size_of::<logos_abi::RenderMessage>(),
    logos_abi::IpcRights::Send,
);
const SESSION_INPUT_CAPABILITY: common::CapabilitySpec = common::capability_contract(
    logos_abi::IPC_CONTRACT_BYTES,
    logos_abi::ServiceId::Session.index() as u32,
    core::mem::size_of::<IpcBytes>(),
    logos_abi::IpcRights::Send,
);
const SESSION_OUTPUT_CAPABILITY: common::CapabilitySpec = common::capability_contract(
    logos_abi::IPC_CONTRACT_BYTES,
    logos_abi::ServiceId::Session.index() as u32,
    core::mem::size_of::<IpcBytes>(),
    logos_abi::IpcRights::Receive,
);

static mut TERMINAL: logos_terminal::TerminalService = logos_terminal::TerminalService::new();
static mut PENDING_RENDER: Option<logos_abi::RenderMessage> = None;
static mut PENDING_SESSION_INPUT: Option<IpcBytes> = None;

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    common::init_service_allocator();
    let terminal = unsafe { &mut *core::ptr::addr_of_mut!(TERMINAL) };
    let pending_render = unsafe { &mut *core::ptr::addr_of_mut!(PENDING_RENDER) };
    let pending_session_input = unsafe { &mut *core::ptr::addr_of_mut!(PENDING_SESSION_INPUT) };
    let input_capability = match common::capability_handle(INPUT_CAPABILITY) {
        Ok(capability) => capability,
        Err(_) => common::idle(),
    };
    let display_capability = match common::capability_handle(DISPLAY_CAPABILITY) {
        Ok(capability) => capability,
        Err(_) => common::idle(),
    };
    let session_input_capability = match common::capability_handle(SESSION_INPUT_CAPABILITY) {
        Ok(capability) => capability,
        Err(_) => common::idle(),
    };
    let session_output_capability = match common::capability_handle(SESSION_OUTPUT_CAPABILITY) {
        Ok(capability) => capability,
        Err(_) => common::idle(),
    };
    let mut heartbeat_ticks = 0u16;
    let mut render_more = false;
    loop {
        common::heartbeat_tick(&mut heartbeat_ticks);
        if let Some(message) = *pending_session_input {
            match common::ipc_send_handle(session_input_capability, &message) {
                IpcStatus::Ok => {
                    *pending_session_input = None;
                }
                IpcStatus::Full => {}
                IpcStatus::Stale
                | IpcStatus::Disconnected
                | IpcStatus::Unauthorized
                | IpcStatus::Malformed
                | IpcStatus::Empty => *pending_session_input = None,
            }
        }
        if pending_session_input.is_none() {
            let mut event = InputMessage::key(KeyCode::Unknown, KeyState::Released, 0);
            while common::ipc_receive_handle(input_capability, &mut event) == IpcStatus::Ok {
                if let Some(message) = terminal.input(&event) {
                    match common::ipc_send_handle(session_input_capability, &message) {
                        IpcStatus::Ok => {}
                        IpcStatus::Full => {
                            *pending_session_input = Some(message);
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
            if pending_session_input.is_none() {}
        }
        let mut message = IpcBytes::empty(MessageKind::SessionOutput);
        while common::ipc_receive_handle(session_output_capability, &mut message) == IpcStatus::Ok {
            if let Some(bytes) = message.as_bytes() {
                terminal.session_output_bytes(bytes);
            }
        }
        if pending_render.is_none() {
            *pending_render = terminal.next_render();
        }
        if let Some(render) = *pending_render {
            match common::ipc_send_handle(display_capability, &render) {
                IpcStatus::Ok => {
                    *pending_render = None;
                    render_more = render.flags & logos_abi::RENDER_FLAG_MORE != 0;
                }
                IpcStatus::Full => {}
                IpcStatus::Stale
                | IpcStatus::Disconnected
                | IpcStatus::Unauthorized
                | IpcStatus::Malformed
                | IpcStatus::Empty => {
                    *pending_render = None;
                    render_more = false;
                }
            }
        }
        if pending_render.is_none() && render_more {
            continue;
        }
        common::wait_on_capabilities(
            &[
                input_capability,
                session_input_capability,
                session_output_capability,
                display_capability,
            ],
            logos_abi::ServiceId::Terminal,
        );
    }
}

#[cfg(target_os = "none")]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo<'_>) -> ! {
    common::idle()
}

#[cfg(not(target_os = "none"))]
fn main() {}
