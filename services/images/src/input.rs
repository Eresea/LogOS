#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", no_main)]
#![cfg_attr(not(target_os = "none"), allow(dead_code, unused_imports, unused_variables))]

mod common;

use logos_abi::{
    INPUT_KEYBOARD_RING_BASE, IPC_CONTRACT_INPUT, InputMessage, IpcStatus, KeyboardByteRing,
};

static mut PENDING: [Option<InputMessage>; 2] = [None, None];

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    common::init_service_allocator();
    let output_capability = match common::discover_capability_contract_named(
        b"terminal",
        logos_abi::IpcRights::Send,
        IPC_CONTRACT_INPUT,
        core::mem::size_of::<InputMessage>(),
    ) {
        Ok(capability) => capability,
        Err(_) => common::idle(),
    };
    let keyboard = unsafe { &*(INPUT_KEYBOARD_RING_BASE as *const KeyboardByteRing) };
    let pending = unsafe { &mut *core::ptr::addr_of_mut!(PENDING) };
    let mut decoder = logos_input::InputDecoder::new();
    let mut heartbeat_ticks = 0u16;
    loop {
        common::heartbeat_tick(&mut heartbeat_ticks);
        if let Some(message) = pending[0] {
            match common::ipc_send_handle(output_capability, &message) {
                IpcStatus::Ok => {
                    pending[0] = pending[1];
                    pending[1] = None;
                }
                IpcStatus::Full => {
                    common::wait_on_capability(output_capability);
                    continue;
                }
                IpcStatus::Stale
                | IpcStatus::Disconnected
                | IpcStatus::Unauthorized
                | IpcStatus::Malformed
                | IpcStatus::Empty => {
                    pending[0] = None;
                    pending[1] = None;
                }
            }
            continue;
        }
        let Some(byte) = keyboard.pop() else {
            common::sleep_on_keyboard();
            continue;
        };
        let Some(event) = decoder.feed(byte) else {
            continue;
        };
        pending[0] = Some(event.terminal_message());
        pending[1] = None;
    }
}

#[cfg(target_os = "none")]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo<'_>) -> ! {
    common::idle()
}

#[cfg(not(target_os = "none"))]
fn main() {}
