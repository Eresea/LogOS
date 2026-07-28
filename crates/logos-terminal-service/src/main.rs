#![no_main]
#![no_std]

use core::arch::asm;
use logos_core::native_service::{
    ACKNOWLEDGED, COMPLETE, Context, Header, PRESENT_PIXEL, READ_INPUT, READY,
};
use uefi::{Status, prelude::*};

#[used]
#[unsafe(link_section = ".logos")]
static HEADER: Header = Header::new(*b"terminal\0\0\0\0\0\0\0\0", logos_service_entry);

#[unsafe(no_mangle)]
extern "C" fn logos_service_entry(context: *mut Context) -> ! {
    unsafe {
        (*context).operation = READY;
        asm!("int 0x80");
        while (*context).status == ACKNOWLEDGED {
            (*context).operation = READ_INPUT;
            asm!("int 0x80");
            if (*context).input == 0x1b {
                (*context).operation = COMPLETE;
                asm!("int 0x80");
            }
            if (*context).input == u32::from(b'k') {
                (*context).operation = PRESENT_PIXEL;
                (*context).color = 0x0000_ff00;
                asm!("int 0x80");
            }
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
