#![no_main]
#![no_std]

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

pub(crate) use arch::{acpi, cpu, interrupts, pci};
pub(crate) use drivers::{device, keyboard, supervisor};
pub(crate) use ipc::approvals;
pub(crate) use mm::{address_space, memory};
pub(crate) use platform::{
    audit, balloon, entropy, health, identity, payload, pe, services, session, time, trace,
};
pub(crate) use sched::scheduler;
