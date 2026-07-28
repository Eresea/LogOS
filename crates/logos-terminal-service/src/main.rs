#![no_main]
#![no_std]

use core::arch::asm;
use logos_core::native_service::{
    ACKNOWLEDGED, COMPLETE, Context, Header, PRESENT_PIXEL, PRESENT_TEXT, READ_INPUT, READY,
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
            (&mut (*context).text)[..10].copy_from_slice(b"LOGOS RING");
            (*context).text_length = 10;
            (*context).color = 0x0000_ff00;
            (*context).operation = PRESENT_TEXT;
            asm!("int 0x80");
            (*context).operation = READ_INPUT;
            asm!("int 0x80");
            if (*context).input == 0x1b {
                (*context).operation = COMPLETE;
                asm!("int 0x80");
            }
            if (*context).input == u32::from(b'k') {
                (*context).operation = PRESENT_PIXEL;
                (*context).x = 100;
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
