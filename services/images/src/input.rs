#![no_std]
#![no_main]

mod common;

use logos_abi::{INPUT_KEYBOARD_RING_BASE, InputMessage, IpcStatus, KeyboardByteRing};

static mut PENDING: [Option<InputMessage>; 2] = [None, None];
const OUTPUT_CAPABILITY: usize =
    common::capability_slot(logos_abi::ServiceId::Input, 0, logos_abi::IpcRights::Send);

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
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
                    common::wait(common::ipc_write_event(0), logos_abi::ServiceId::Input);
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

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo<'_>) -> ! {
    common::idle()
}
