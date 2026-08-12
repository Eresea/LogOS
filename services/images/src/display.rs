#![no_std]
#![no_main]

mod common;

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    common::idle()
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo<'_>) -> ! {
    common::idle()
}
