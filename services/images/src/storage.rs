#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", no_main)]
#![cfg_attr(not(target_os = "none"), allow(dead_code, unused_imports, unused_variables))]

mod common;

/// The storage image is admitted as a fixed service endpoint. Block requests
/// remain kernel-mediated; this bounded lifecycle image keeps the service
/// alive until that endpoint is exposed to user mode.
#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    let mut heartbeat_ticks = 0u16;
    loop {
        common::heartbeat_tick(&mut heartbeat_ticks, logos_abi::ServiceId::Storage);
        common::wait(0, logos_abi::ServiceId::Storage);
    }
}

#[cfg(target_os = "none")]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo<'_>) -> ! {
    common::idle()
}

#[cfg(not(target_os = "none"))]
fn main() {}
