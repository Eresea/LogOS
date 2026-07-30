#![no_main]
#![no_std]

use core::arch::asm;
use logos_core::native_service::{ACKNOWLEDGED, Context, Header, READ_INPUT, READY};
use logos_service_rt as _;

#[used]
#[unsafe(link_section = ".logos")]
static HEADER: Header = Header::new(*b"storage\0\0\0\0\0\0\0\0\0", logos_service_entry);

#[unsafe(no_mangle)]
extern "C" fn logos_service_entry(context: *mut Context) -> ! {
    unsafe {
        (*context).operation = READY;
        asm!("int 0x80");
        while (*context).status == ACKNOWLEDGED {
            (*context).operation = READ_INPUT;
            asm!("int 0x80");
        }
    }
    loop {
        core::hint::spin_loop();
    }
}
