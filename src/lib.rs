#![no_std]

#[cfg(test)]
extern crate std;

pub mod display;
pub mod input;
pub mod ipc;
pub mod process;
mod scheduler;
pub mod session;
pub mod supervisor;
pub mod terminal;
pub mod terminal_abi;
pub mod terminal_stack;
pub use scheduler::{
    FinishState, IDLE_STACK_SIZE, MAX_CPUS, MAX_TASKS, SCHEDULER, SCHEDULER_STACK_SIZE, Scheduler,
    SpawnError, TASK_STACK_SIZE, TaskEntry, TaskHandle, TaskState,
};
pub mod health;
pub mod runtime;
pub mod service_lifecycle;

#[cfg(target_os = "uefi")]
mod runtime_entry;

#[cfg(target_os = "uefi")]
#[allow(dead_code)]
mod user_mode;

#[cfg(all(feature = "qemu-proof", target_os = "uefi"))]
mod proof;

/// The single production handoff seam. Future Runtime code replaces this
/// entry without changing UEFI or scheduler ownership.
#[cfg(target_os = "uefi")]
pub(crate) fn runtime_entry() {
    runtime_entry::run()
}

pub fn boot() -> uefi::prelude::Status {
    #[cfg(target_os = "uefi")]
    {
        arch::boot()
    }
    #[cfg(not(target_os = "uefi"))]
    {
        uefi::prelude::Status::SUCCESS
    }
}

#[cfg(target_os = "uefi")]
pub fn yield_current() {
    arch::yield_current()
}

#[cfg(target_os = "uefi")]
pub fn block_current() {
    arch::block_current()
}

#[cfg(target_os = "uefi")]
/// Block until the deadline or until another CPU calls `wake` for this task.
pub fn sleep_current_for(ticks: u64) {
    arch::sleep_current_for(ticks)
}

#[cfg(target_os = "uefi")]
#[cfg_attr(not(feature = "qemu-proof"), allow(dead_code))]
pub(crate) fn current_cpu() -> usize {
    arch::current_cpu()
}

#[cfg(target_os = "uefi")]
pub(crate) fn current_ticks() -> u64 {
    arch::current_ticks()
}

#[cfg(target_os = "uefi")]
#[cfg_attr(not(feature = "qemu-proof"), allow(dead_code))]
pub(crate) fn arch_proof_line(message: &[u8]) {
    arch::proof_line(message)
}

#[cfg(target_os = "uefi")]
#[cfg_attr(not(feature = "qemu-proof"), allow(dead_code))]
pub(crate) fn arch_fatal(message: &[u8]) -> ! {
    arch::fatal(message)
}

#[cfg(not(target_os = "uefi"))]
pub fn yield_current() {
    loop {
        core::hint::spin_loop();
    }
}

#[cfg(not(target_os = "uefi"))]
pub fn block_current() {
    loop {
        core::hint::spin_loop();
    }
}

#[cfg(target_os = "uefi")]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo<'_>) -> ! {
    arch::fatal(b"LogOS vNext: panic")
}

#[cfg(target_os = "uefi")]
mod arch;
