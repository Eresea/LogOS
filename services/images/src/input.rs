#![no_std]
#![no_main]

mod common;

use logos_abi::{
    INPUT_KEYBOARD_RING_BASE, InputIpc, InputMessage, KeyboardByteRing, SERVICE_IPC_BASE,
    SharedSendError,
};

static mut PENDING: [Option<InputMessage>; 2] = [None, None];

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    let keyboard = unsafe { &*(INPUT_KEYBOARD_RING_BASE as *const KeyboardByteRing) };
    let output = unsafe { &*(SERVICE_IPC_BASE as *const InputIpc) };
    let pending = unsafe { &mut *core::ptr::addr_of_mut!(PENDING) };
    let mut decoder = logos_input::InputDecoder::new();
    let mut heartbeat_ticks = 0u16;
    loop {
        common::heartbeat_tick(&mut heartbeat_ticks, logos_abi::ServiceId::Input);
        if let Some(message) = pending[0] {
            let identity = output.endpoint().identity();
            match output.send(identity, message) {
                Ok(notification) => {
                    common::notify_edge(common::ipc_read_event(0), notification);
                    pending[0] = pending[1];
                    pending[1] = None;
                }
                Err(SharedSendError::Full) => {
                    common::wait(common::ipc_write_event(0), logos_abi::ServiceId::Input);
                    continue;
                }
                Err(SharedSendError::Stale | SharedSendError::Disconnected) => {
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
