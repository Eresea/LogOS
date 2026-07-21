#![no_main]
#![no_std]

use core::arch::asm;
use uefi::prelude::*;

#[entry]
fn main() -> Status {
    for byte in b"LogOS: kernel entered\r\n" {
        unsafe { asm!("out dx, al", in("dx") 0xe9u16, in("al") *byte) };
    }
    Status::SUCCESS
}
