#![cfg_attr(target_os = "uefi", no_main)]
#![cfg_attr(target_os = "uefi", no_std)]

#[cfg(target_os = "uefi")]
use uefi::{entry, prelude::*};

#[cfg(target_os = "uefi")]
#[entry]
fn main() -> Status {
    logos_vnext::boot()
}

#[cfg(not(target_os = "uefi"))]
fn main() {}
