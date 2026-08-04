//! Ring-0 bootstrap entry point.
//!
//! The long-lived coordination loop lives in `platform::runtime`; this module
//! keeps the boot-facing symbol stable while the kernel remains responsible for
//! the privileged entry boundary.

use crate::arch::acpi;
use crate::boot;
use crate::platform::{identity, payload, root_key, runtime, time};
use uefi::mem::memory_map::MemoryMap;

#[allow(clippy::too_many_arguments)]
pub(crate) fn main(
    boot_info: boot::Info,
    memory_map: impl MemoryMap,
    acpi: Option<acpi::Tables>,
    machine: identity::Machine,
    secret_root: Option<root_key::RootKey>,
    remote_bootstrap: Option<logos_remote::Bootstrap>,
    wall_clock: time::WallClock,
    payload: Option<payload::Payloads>,
) -> ! {
    runtime::run(
        boot_info,
        memory_map,
        acpi,
        machine,
        secret_root,
        remote_bootstrap,
        wall_clock,
        payload,
    )
}
