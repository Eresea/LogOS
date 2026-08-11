#![no_main]
#![no_std]

use core::arch::asm;
use uefi::{entry, prelude::*};

const DEBUG_PORT: u16 = 0xe9;

fn debug_line(message: &[u8]) {
    for &byte in message {
        unsafe { asm!("out dx, al", in("dx") DEBUG_PORT, in("al") byte) };
    }
    debug_line_raw(b"\r\n");
}

fn debug_line_raw(message: &[u8]) {
    for &byte in message {
        unsafe { asm!("out dx, al", in("dx") DEBUG_PORT, in("al") byte) };
    }
}

#[entry]
fn main() -> Status {
    debug_line(b"LogOS vNext: booted");
    loop {
        core::hint::spin_loop();
    }
}
