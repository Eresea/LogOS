#![no_main]
#![no_std]

use core::arch::asm;
use logos_core::native_service::Header;
use uefi::{Status, prelude::*};

#[used]
#[unsafe(link_section = ".logos")]
static HEADER: Header = Header::new(*b"terminal\0\0\0\0\0\0\0\0", logos_service_entry);

#[unsafe(no_mangle)]
extern "C" fn logos_service_entry() -> ! {
    unsafe { asm!("int 0x80") };
    loop {
        core::hint::spin_loop();
    }
}

#[entry]
fn main() -> Status {
    let _ = logos_terminal::terminal::Model::new();
    Status::SUCCESS
}
