#![no_main]
#![no_std]

#[cfg(test)]
extern crate std;

mod arch;
mod boot;
mod console;
mod debug;
mod drivers;
mod ipc;
mod kernel;
mod mm;
mod platform;
mod sched;
#[cfg(feature = "test-hooks")]
mod test_hooks;
