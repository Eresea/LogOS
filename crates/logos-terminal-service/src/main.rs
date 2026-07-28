#![no_main]
#![no_std]

use core::arch::asm;
use logos_core::native_service::{ACKNOWLEDGED, COMPLETE, Context, Header, READY, WAIT};
use uefi::{Status, prelude::*};

#[used]
#[unsafe(link_section = ".logos")]
static HEADER: Header = Header::new(*b"terminal\0\0\0\0\0\0\0\0", logos_service_entry);

#[unsafe(no_mangle)]
extern "C" fn logos_service_entry(context: *mut Context) -> ! {
    unsafe {
        (*context).operation = READY;
        asm!("int 0x80");
        if (*context).status == ACKNOWLEDGED {
            (*context).operation = WAIT;
            asm!("int 0x80");
            (*context).operation = COMPLETE;
            asm!("int 0x80");
        }
    }
    loop {
        core::hint::spin_loop();
    }
}

#[entry]
fn main() -> Status {
    let _ = logos_terminal::terminal::Model::new();
    Status::SUCCESS
}
