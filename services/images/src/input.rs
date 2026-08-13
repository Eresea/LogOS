#![no_std]
#![no_main]

mod common;

use logos_abi::{INPUT_KEYBOARD_RING_BASE, InputIpc, KeyboardByteRing, SERVICE_IPC_BASE};

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    let keyboard = unsafe { &*(INPUT_KEYBOARD_RING_BASE as *const KeyboardByteRing) };
    let output = unsafe { &*(SERVICE_IPC_BASE as *const InputIpc) };
    let mut decoder = logos_input::InputDecoder::new();
    loop {
        let Some(byte) = keyboard.pop() else {
            core::hint::spin_loop();
            continue;
        };
        let Some(event) = decoder.feed(byte) else {
            continue;
        };
        let identity = output.endpoint().identity();
        let _ = output.send(identity, event.key);
        if let Some(text) = event.text {
            let _ = output.send(identity, text);
        }
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo<'_>) -> ! {
    common::idle()
}
