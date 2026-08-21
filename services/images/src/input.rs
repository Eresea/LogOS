#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", no_main)]
#![cfg_attr(not(target_os = "none"), allow(dead_code, unused_imports, unused_variables))]

mod common;

use logos_abi::{INPUT_KEYBOARD_RING_BASE, InputMessage, IpcStatus, KeyboardByteRing};

static mut PENDING: [Option<InputMessage>; 2] = [None, None];
const OUTPUT_CAPABILITY: usize = common::capability_slot(
    logos_abi::ServiceId::Input,
    logos_abi::IpcEndpointId::InputToTerminal,
    logos_abi::IpcRights::Send,
);

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    common::init_service_allocator();
    let keyboard = unsafe { &*(INPUT_KEYBOARD_RING_BASE as *const KeyboardByteRing) };
    let pending = unsafe { &mut *core::ptr::addr_of_mut!(PENDING) };
    let mut decoder = logos_input::InputDecoder::new();
    let mut heartbeat_ticks = 0u16;
    loop {
        common::heartbeat_tick(&mut heartbeat_ticks, logos_abi::ServiceId::Input);
        if let Some(message) = pending[0] {
            match common::ipc_send(OUTPUT_CAPABILITY, &message) {
                IpcStatus::Ok => {
                    pending[0] = pending[1];
                    pending[1] = None;
                }
                IpcStatus::Full => {
                    common::wait(
                        common::ipc_write_event(logos_abi::IpcEndpointId::InputToTerminal),
                        logos_abi::ServiceId::Input,
                    );
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
            common::wait(common::keyboard_read_event(), logos_abi::ServiceId::Input);
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
