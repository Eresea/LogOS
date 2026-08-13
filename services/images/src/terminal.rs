#![no_std]
#![no_main]

mod common;

use logos_abi::{InputIpc, IpcBytes, MessageKind, RenderIpc, SERVICE_IPC_BASE, StreamIpc};

const PAGE_BYTES: usize = logos_abi::IPC_PAGE_BYTES;
const INPUT_TO_TERMINAL: usize = SERVICE_IPC_BASE;
const TERMINAL_TO_DISPLAY: usize = SERVICE_IPC_BASE + PAGE_BYTES;
const TERMINAL_TO_SESSION: usize = SERVICE_IPC_BASE + PAGE_BYTES * 2;
const SESSION_TO_TERMINAL: usize = SERVICE_IPC_BASE + PAGE_BYTES * 3;

static mut TERMINAL: logos_terminal::TerminalService = logos_terminal::TerminalService::new();

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    let terminal = unsafe { &mut *core::ptr::addr_of_mut!(TERMINAL) };
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
        while let Ok(event) = input.receive(input_identity) {
            progressed = true;
            if let Some(message) = terminal.input(&event) {
                if let Some(bytes) = IpcBytes::from_bytes(
                    MessageKind::SessionInput,
                    message.as_bytes().unwrap_or_default(),
                ) {
                    let _ = session_input.send(session_input_identity, bytes);
                }
            }
        }
        while let Ok(message) = session_output.receive(session_output_identity) {
            progressed = true;
            if let Some(bytes) = message.as_bytes() {
                terminal.session_output_bytes(bytes);
            }
        }
        while let Some(render) = terminal.next_render() {
            progressed = true;
            let _ = display.send(display_identity, render);
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
