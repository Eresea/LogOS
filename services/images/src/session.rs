#![no_std]
#![no_main]

mod common;

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    let _session = logos_session::SessionService::new();
    common::idle()
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo<'_>) -> ! {
    common::idle()
}
