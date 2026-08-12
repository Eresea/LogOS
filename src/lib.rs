#![no_std]

#[cfg(test)]
extern crate std;

mod scheduler;
pub use scheduler::{
    FinishState, IDLE_STACK_SIZE, MAX_CPUS, MAX_TASKS, SCHEDULER, SCHEDULER_STACK_SIZE, Scheduler,
    SpawnError, TASK_STACK_SIZE, TaskEntry, TaskHandle, TaskState,
};
pub mod runtime;
pub mod service_lifecycle;

#[cfg(all(feature = "qemu-proof", target_os = "uefi"))]
mod proof;

/// The single production handoff seam. Future Runtime code replaces this
/// entry without changing UEFI or scheduler ownership.
#[cfg(target_os = "uefi")]
pub(crate) fn runtime_entry() {
    #[cfg(feature = "qemu-proof")]
    proof::handoff_started();

    let mut runtime = runtime::Runtime::new();
    let timed = runtime.submit().unwrap_or_else(|_| arch_fatal(b"LogOS vNext: runtime capacity"));
    let deadline = current_ticks().saturating_add(1);
    if !runtime.wait(timed, deadline) {
        arch_fatal(b"LogOS vNext: runtime wait");
    }
    sleep_current_for(3);
    if !runtime.timeout(timed, current_ticks()) {
        arch_fatal(b"LogOS vNext: runtime timeout");
    }
    #[cfg(feature = "qemu-proof")]
    proof::runtime_timed_out();
    if !runtime.reclaim(timed) {
        arch_fatal(b"LogOS vNext: runtime reclaim");
    }

    let completed = runtime.submit().unwrap_or_else(|_| arch_fatal(b"LogOS vNext: runtime reuse"));
    if !runtime.wait(completed, current_ticks().saturating_add(3)) {
        arch_fatal(b"LogOS vNext: runtime wait");
    }
    sleep_current_for(3);
    if !runtime.complete(completed) {
        arch_fatal(b"LogOS vNext: runtime complete");
    }
    #[cfg(feature = "qemu-proof")]
    proof::runtime_completed();
    if !runtime.reclaim(completed) {
        arch_fatal(b"LogOS vNext: runtime reclaim");
    }

    let replacement =
        runtime.submit().unwrap_or_else(|_| arch_fatal(b"LogOS vNext: runtime reuse"));
    if replacement.slot() != timed.slot() || replacement.generation() == timed.generation() {
        arch_fatal(b"LogOS vNext: runtime generation");
    }
    if !runtime.cancel(replacement) || !runtime.reclaim(replacement) {
        arch_fatal(b"LogOS vNext: runtime cancel");
    }
    #[cfg(feature = "qemu-proof")]
    proof::runtime_slot_reused();

    loop {
        sleep_current_for(3);
        #[cfg(feature = "qemu-proof")]
        proof::runtime_wait_resumed();
    }
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
fn current_ticks() -> u64 {
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
