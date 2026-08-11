#![no_main]
#![no_std]

use uefi::{entry, prelude::*};

#[entry]
fn main() -> Status {
    logos_vnext::boot()
}
