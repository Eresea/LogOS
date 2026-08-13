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
        heartbeat_ticks = heartbeat_ticks.wrapping_add(1);
        if heartbeat_ticks == 1024 {
            heartbeat_ticks = 0;
            common::heartbeat(logos_abi::ServiceId::Input);
        }
        if let Some(message) = pending[0] {
            let identity = output.endpoint().identity();
            match output.send(identity, message) {
                Ok(_) => {
                    pending[0] = pending[1];
                    pending[1] = None;
                }
                Err(SharedSendError::Full) => {
                    core::hint::spin_loop();
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
            core::hint::spin_loop();
            continue;
        };
        let Some(event) = decoder.feed(byte) else {
            continue;
        };
        pending[0] = Some(event.key);
        pending[1] = event.text;
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo<'_>) -> ! {
    common::idle()
}
