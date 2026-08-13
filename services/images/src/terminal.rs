#![no_std]
#![no_main]

mod common;

use logos_abi::{
    InputIpc, IpcBytes, MessageKind, RenderIpc, SERVICE_IPC_BASE, SharedSendError, StreamIpc,
};

const PAGE_BYTES: usize = logos_abi::IPC_PAGE_BYTES;
const INPUT_TO_TERMINAL: usize = SERVICE_IPC_BASE;
const TERMINAL_TO_DISPLAY: usize = SERVICE_IPC_BASE + PAGE_BYTES;
const TERMINAL_TO_SESSION: usize = SERVICE_IPC_BASE + PAGE_BYTES * 2;
const SESSION_TO_TERMINAL: usize = SERVICE_IPC_BASE + PAGE_BYTES * 3;

static mut TERMINAL: logos_terminal::TerminalService = logos_terminal::TerminalService::new();
static mut PENDING_RENDER: Option<logos_abi::RenderMessage> = None;
static mut PENDING_SESSION_INPUT: Option<IpcBytes> = None;

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    let terminal = unsafe { &mut *core::ptr::addr_of_mut!(TERMINAL) };
    let pending_render = unsafe { &mut *core::ptr::addr_of_mut!(PENDING_RENDER) };
    let pending_session_input = unsafe { &mut *core::ptr::addr_of_mut!(PENDING_SESSION_INPUT) };
    let input = unsafe { &*(INPUT_TO_TERMINAL as *const InputIpc) };
    let display = unsafe { &*(TERMINAL_TO_DISPLAY as *const RenderIpc) };
    let session_input = unsafe { &*(TERMINAL_TO_SESSION as *const StreamIpc) };
    let session_output = unsafe { &*(SESSION_TO_TERMINAL as *const StreamIpc) };
    let mut heartbeat_ticks = 0u16;
    loop {
        heartbeat_ticks = heartbeat_ticks.wrapping_add(1);
        if heartbeat_ticks == 1024 {
            heartbeat_ticks = 0;
            common::heartbeat(logos_abi::ServiceId::Terminal);
        }
        let input_identity = input.endpoint().identity();
        let display_identity = display.endpoint().identity();
        let session_input_identity = session_input.endpoint().identity();
        let session_output_identity = session_output.endpoint().identity();
        let mut progressed = false;
        if let Some(message) = *pending_session_input {
            match session_input.send(session_input_identity, message) {
                Ok(_) => *pending_session_input = None,
                Err(SharedSendError::Full) => {}
                Err(SharedSendError::Stale | SharedSendError::Disconnected) => {
                    *pending_session_input = None
                }
            }
            progressed = true;
        }
        if pending_session_input.is_none() {
            while let Ok(event) = input.receive(input_identity) {
                progressed = true;
                if let Some(message) = terminal.input(&event) {
                    if let Some(bytes) = IpcBytes::from_bytes(
                        MessageKind::SessionInput,
                        message.as_bytes().unwrap_or_default(),
                    ) {
                        match session_input.send(session_input_identity, bytes) {
                            Ok(_) => {}
                            Err(SharedSendError::Full) => {
                                *pending_session_input = Some(bytes);
                                break;
                            }
                            Err(SharedSendError::Stale | SharedSendError::Disconnected) => {}
                        }
                    }
                }
            }
        }
        while let Ok(message) = session_output.receive(session_output_identity) {
            progressed = true;
            if let Some(bytes) = message.as_bytes() {
                terminal.session_output_bytes(bytes);
            }
        }
        if pending_render.is_none() {
            *pending_render = terminal.next_render();
        }
        if let Some(render) = *pending_render {
            match display.send(display_identity, render) {
                Ok(_) => *pending_render = None,
                Err(SharedSendError::Full) => {}
                Err(SharedSendError::Stale | SharedSendError::Disconnected) => {
                    *pending_render = None
                }
            }
            progressed = true;
        }
        if !progressed {
            core::hint::spin_loop();
        }
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo<'_>) -> ! {
    common::idle()
}
