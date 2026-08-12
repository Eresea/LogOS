use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use crate::{MAX_CPUS, SCHEDULER, TaskEntry, TaskHandle, TaskState};

static A_PROGRESS: AtomicU64 = AtomicU64::new(0);
static B_PROGRESS: AtomicU64 = AtomicU64::new(0);
static BLOCK_HANDLE: AtomicU64 = AtomicU64::new(0);
static BLOCK_CPU: AtomicUsize = AtomicUsize::new(MAX_CPUS);
static WAKE_CPU: AtomicUsize = AtomicUsize::new(MAX_CPUS);
static BLOCK_STARTED: AtomicBool = AtomicBool::new(false);
static BLOCK_RESUMED: AtomicBool = AtomicBool::new(false);
static WAKE_DONE: AtomicBool = AtomicBool::new(false);
static HANDOFF_STARTED: AtomicBool = AtomicBool::new(false);
static RUNTIME_WAKE_CYCLES: AtomicU64 = AtomicU64::new(0);
static RUNTIME_TIMED_OUT: AtomicBool = AtomicBool::new(false);
static RUNTIME_COMPLETED: AtomicBool = AtomicBool::new(false);
static RUNTIME_SLOT_REUSED: AtomicBool = AtomicBool::new(false);
static PASSED: AtomicBool = AtomicBool::new(false);
static REPORTED: AtomicBool = AtomicBool::new(false);
static CPU_COUNT: AtomicUsize = AtomicUsize::new(1);

unsafe extern "C" {
    fn proof_task_a();
    fn proof_task_b();
}

pub fn initialize(cpu_count: usize) {
    CPU_COUNT.store(cpu_count, Ordering::Release);
    let a: TaskEntry = unsafe { core::mem::transmute(proof_task_a as *const () as usize) };
    let b: TaskEntry = unsafe { core::mem::transmute(proof_task_b as *const () as usize) };
    let block = SCHEDULER.spawn(block_task).expect("proof task capacity");
    BLOCK_HANDLE.store(block.raw(), Ordering::Release);
    SCHEDULER.spawn(a).expect("proof task capacity");
    SCHEDULER.spawn(b).expect("proof task capacity");
    SCHEDULER.spawn(wake_task).expect("proof task capacity");
}

pub fn handoff_started() {
    HANDOFF_STARTED.store(true, Ordering::Release);
}

pub fn runtime_wait_resumed() {
    RUNTIME_WAKE_CYCLES.fetch_add(1, Ordering::Relaxed);
}

pub fn runtime_timed_out() {
    RUNTIME_TIMED_OUT.store(true, Ordering::Release);
}

pub fn runtime_completed() {
    RUNTIME_COMPLETED.store(true, Ordering::Release);
}

pub fn runtime_slot_reused() {
    RUNTIME_SLOT_REUSED.store(true, Ordering::Release);
}

pub fn observe(cpu: usize) {
    if PASSED.load(Ordering::Acquire) {
        if cpu == 0 && !REPORTED.swap(true, Ordering::AcqRel) {
            crate::arch_proof_line(b"LogOS vNext: QEMU proof PASS");
        }
        return;
    }
    let cpu_count = CPU_COUNT.load(Ordering::Acquire);
    let timers = (0..cpu_count).all(|index| SCHEDULER.ticks(index).unwrap_or(0) >= 3);
    let progress = A_PROGRESS.load(Ordering::Acquire) > 4 && B_PROGRESS.load(Ordering::Acquire) > 4;
    let switches = (0..cpu_count).map(|index| SCHEDULER.switches(index).unwrap_or(0)).sum::<u64>();
    let wake_cpus_differ =
        cpu_count == 1 || BLOCK_CPU.load(Ordering::Acquire) != WAKE_CPU.load(Ordering::Acquire);
    if timers
        && progress
        && switches > 20
        && HANDOFF_STARTED.load(Ordering::Acquire)
        && RUNTIME_WAKE_CYCLES.load(Ordering::Acquire) >= 3
        && RUNTIME_TIMED_OUT.load(Ordering::Acquire)
        && RUNTIME_COMPLETED.load(Ordering::Acquire)
        && RUNTIME_SLOT_REUSED.load(Ordering::Acquire)
        && BLOCK_RESUMED.load(Ordering::Acquire)
        && WAKE_DONE.load(Ordering::Acquire)
        && wake_cpus_differ
    {
        PASSED.store(true, Ordering::Release);
        if cpu == 0 && !REPORTED.swap(true, Ordering::AcqRel) {
            crate::arch_proof_line(b"LogOS vNext: QEMU proof PASS");
        }
    }
}

fn block_task() {
    BLOCK_STARTED.store(true, Ordering::Release);
    BLOCK_CPU.store(crate::current_cpu(), Ordering::Release);
    crate::block_current();
    BLOCK_RESUMED.store(true, Ordering::Release);
    loop {
        crate::yield_current();
    }
}

fn wake_task() {
    loop {
        if BLOCK_STARTED.load(Ordering::Acquire) {
            let raw = BLOCK_HANDLE.load(Ordering::Acquire);
            let handle = TaskHandle::from_raw(raw);
            if SCHEDULER.state(handle) == Some(TaskState::Blocked) {
                let cpu = crate::current_cpu();
                let blocked = BLOCK_CPU.load(Ordering::Acquire);
                if CPU_COUNT.load(Ordering::Acquire) == 1 || cpu != blocked {
                    if SCHEDULER.wake(handle) {
                        WAKE_CPU.store(cpu, Ordering::Release);
                        WAKE_DONE.store(true, Ordering::Release);
                    }
                    loop {
                        crate::yield_current();
                    }
                }
            }
        }
        crate::yield_current();
    }
}

#[unsafe(no_mangle)]
extern "C" fn proof_a_progress() {
    A_PROGRESS.fetch_add(1, Ordering::Relaxed);
}

#[unsafe(no_mangle)]
extern "C" fn proof_b_progress() {
    B_PROGRESS.fetch_add(1, Ordering::Relaxed);
}

#[unsafe(no_mangle)]
extern "C" fn proof_fail() -> ! {
    crate::arch_fatal(b"LogOS vNext: QEMU proof FAIL")
}
