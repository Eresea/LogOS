#![no_std]
#![no_main]

mod common;

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    let _decoder = logos_input::InputDecoder::new();
    common::idle()
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo<'_>) -> ! {
    common::idle()
}
