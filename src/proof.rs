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
static RUNTIME_MAILBOX_BUSY: AtomicBool = AtomicBool::new(false);
static COMPLETION_HANDLE: AtomicU64 = AtomicU64::new(0);
static COMPLETION_RECLAIMED: AtomicBool = AtomicBool::new(false);
static COMPLETION_STALE_REJECTED: AtomicBool = AtomicBool::new(false);
static COMPLETION_SLOT_REUSED: AtomicBool = AtomicBool::new(false);
static REPLACEMENT_RAN: AtomicBool = AtomicBool::new(false);
static HEALTH_RESTARTED: AtomicBool = AtomicBool::new(false);
static HEALTH_LATE_COMPLETION_REJECTED: AtomicBool = AtomicBool::new(false);
static HEALTH_RETRY_COMPLETED: AtomicBool = AtomicBool::new(false);
static PASSED: AtomicBool = AtomicBool::new(false);
static REPORTED: AtomicBool = AtomicBool::new(false);
static CPU_COUNT: AtomicUsize = AtomicUsize::new(1);
static LIVE_SERVICE_RESTARTED: AtomicBool = AtomicBool::new(false);
static MANAGER_RESTART_COMPLETED: AtomicBool = AtomicBool::new(false);
static MANAGER_SYSCALL_SUCCEEDED: AtomicBool = AtomicBool::new(false);
static BACKPRESSURE_HANDLE: AtomicU64 = AtomicU64::new(0);
static BACKPRESSURE_FULL: AtomicBool = AtomicBool::new(false);
static BACKPRESSURE_BLOCKED: AtomicBool = AtomicBool::new(false);
static BACKPRESSURE_WAKE: AtomicBool = AtomicBool::new(false);
static BACKPRESSURE_RESUMED: AtomicBool = AtomicBool::new(false);
static RING3_CPU_MASK: AtomicUsize = AtomicUsize::new(0);
static RING3_AP_REPORTED: AtomicBool = AtomicBool::new(false);
static RING3_CR3_VALID: AtomicBool = AtomicBool::new(false);
static HOSTILE_IPC_PROCESS_REJECTED: AtomicBool = AtomicBool::new(false);
static HOSTILE_IPC_SYSCALL_REJECTED: AtomicBool = AtomicBool::new(false);
static RESCHEDULE_IPIS: AtomicU64 = AtomicU64::new(0);
static EVENT_WAKE_IPI_SENT: AtomicBool = AtomicBool::new(false);
static EVENT_WAKE_IPI_RECEIVED: AtomicBool = AtomicBool::new(false);
static PROBE_RING: logos_abi::RenderIpc =
    logos_abi::RenderIpc::new(logos_abi::EndpointHeader::new(1, 1));

unsafe extern "C" {
    fn proof_task_a();
    fn proof_task_b();
}

pub fn initialize(cpu_count: usize) {
    CPU_COUNT.store(cpu_count, Ordering::Release);
    verify_hostile_ipc_boundary();
    let a: TaskEntry = unsafe { core::mem::transmute(proof_task_a as *const () as usize) };
    let b: TaskEntry = unsafe { core::mem::transmute(proof_task_b as *const () as usize) };
    let block = SCHEDULER.spawn(block_task).expect("proof task capacity");
    BLOCK_HANDLE.store(block.raw(), Ordering::Release);
    SCHEDULER.spawn(a).expect("proof task capacity");
    SCHEDULER.spawn(b).expect("proof task capacity");
    SCHEDULER.spawn(wake_task).expect("proof task capacity");
    let backpressure = SCHEDULER.spawn(backpressure_sender_task).expect("proof task capacity");
    BACKPRESSURE_HANDLE.store(backpressure.raw(), Ordering::Release);
    let completion = SCHEDULER.spawn(completion_task).expect("proof task capacity");
    COMPLETION_HANDLE.store(completion.raw(), Ordering::Release);
    SCHEDULER.spawn(reclaimer_task).expect("proof task capacity");
}

pub(crate) fn reserve_frames(pool: &mut crate::frame_pool::FramePool) {
    for (address, bytes) in [
        (core::ptr::addr_of!(A_PROGRESS) as usize, core::mem::size_of::<AtomicU64>()),
        (core::ptr::addr_of!(B_PROGRESS) as usize, core::mem::size_of::<AtomicU64>()),
        (core::ptr::addr_of!(BLOCK_HANDLE) as usize, core::mem::size_of::<AtomicU64>()),
        (core::ptr::addr_of!(BLOCK_CPU) as usize, core::mem::size_of::<AtomicUsize>()),
        (core::ptr::addr_of!(WAKE_CPU) as usize, core::mem::size_of::<AtomicUsize>()),
        (core::ptr::addr_of!(BLOCK_STARTED) as usize, core::mem::size_of::<AtomicBool>()),
        (core::ptr::addr_of!(BLOCK_RESUMED) as usize, core::mem::size_of::<AtomicBool>()),
        (core::ptr::addr_of!(WAKE_DONE) as usize, core::mem::size_of::<AtomicBool>()),
        (core::ptr::addr_of!(HANDOFF_STARTED) as usize, core::mem::size_of::<AtomicBool>()),
        (core::ptr::addr_of!(BACKPRESSURE_HANDLE) as usize, core::mem::size_of::<AtomicU64>()),
        (core::ptr::addr_of!(BACKPRESSURE_FULL) as usize, core::mem::size_of::<AtomicBool>()),
        (core::ptr::addr_of!(BACKPRESSURE_BLOCKED) as usize, core::mem::size_of::<AtomicBool>()),
        (core::ptr::addr_of!(BACKPRESSURE_WAKE) as usize, core::mem::size_of::<AtomicBool>()),
        (core::ptr::addr_of!(BACKPRESSURE_RESUMED) as usize, core::mem::size_of::<AtomicBool>()),
        (core::ptr::addr_of!(RING3_CPU_MASK) as usize, core::mem::size_of::<AtomicUsize>()),
        (core::ptr::addr_of!(RING3_AP_REPORTED) as usize, core::mem::size_of::<AtomicBool>()),
        (core::ptr::addr_of!(RING3_CR3_VALID) as usize, core::mem::size_of::<AtomicBool>()),
        (
            core::ptr::addr_of!(HOSTILE_IPC_PROCESS_REJECTED) as usize,
            core::mem::size_of::<AtomicBool>(),
        ),
        (
            core::ptr::addr_of!(HOSTILE_IPC_SYSCALL_REJECTED) as usize,
            core::mem::size_of::<AtomicBool>(),
        ),
        (core::ptr::addr_of!(RESCHEDULE_IPIS) as usize, core::mem::size_of::<AtomicU64>()),
        (core::ptr::addr_of!(EVENT_WAKE_IPI_SENT) as usize, core::mem::size_of::<AtomicBool>()),
        (core::ptr::addr_of!(EVENT_WAKE_IPI_RECEIVED) as usize, core::mem::size_of::<AtomicBool>()),
        (
            core::ptr::addr_of!(MANAGER_SYSCALL_SUCCEEDED) as usize,
            core::mem::size_of::<AtomicBool>(),
        ),
        (core::ptr::addr_of!(PROBE_RING) as usize, core::mem::size_of::<logos_abi::RenderIpc>()),
    ] {
        crate::arch::reserve_storage_frames(pool, address, bytes);
    }
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

pub fn runtime_mailbox_busy() {
    RUNTIME_MAILBOX_BUSY.store(true, Ordering::Release);
}

pub fn health_restarted() {
    HEALTH_RESTARTED.store(true, Ordering::Release);
}

pub fn health_late_completion_rejected() {
    HEALTH_LATE_COMPLETION_REJECTED.store(true, Ordering::Release);
}

pub fn health_retry_completed() {
    HEALTH_RETRY_COMPLETED.store(true, Ordering::Release);
}

pub fn live_service_restarted() {
    LIVE_SERVICE_RESTARTED.store(true, Ordering::Release);
}

pub fn manager_restart_completed() {
    MANAGER_RESTART_COMPLETED.store(true, Ordering::Release);
}

pub fn manager_syscall_succeeded() {
    MANAGER_SYSCALL_SUCCEEDED.store(true, Ordering::Release);
}

pub fn observe_ring3_cpu(cpu: usize, expected_root: usize, actual_root: usize) {
    if cpu < MAX_CPUS {
        let previous = RING3_CPU_MASK.fetch_or(1usize << cpu, Ordering::AcqRel);
        if cpu != 0
            && previous & (1usize << cpu) == 0
            && !RING3_AP_REPORTED.swap(true, Ordering::AcqRel)
        {
            crate::arch_proof_line(b"LogOS vNext: ring3 AP execution");
        }
    }
    if expected_root != actual_root {
        crate::arch_fatal(b"LogOS vNext: ring3 CR3 mismatch");
    }
    RING3_CR3_VALID.store(true, Ordering::Release);
}

pub fn reschedule_ipi_received(_cpu: usize) {
    if RESCHEDULE_IPIS.fetch_add(1, Ordering::Relaxed) == 0 {
        crate::arch_proof_line(b"LogOS vNext: reschedule IPI received");
    }
    if EVENT_WAKE_IPI_SENT.load(Ordering::Acquire)
        && !EVENT_WAKE_IPI_RECEIVED.swap(true, Ordering::AcqRel)
    {
        crate::arch_proof_line(b"LogOS vNext: event wake reschedule IPI");
    }
}

pub fn event_wake_ipi_sent() {
    EVENT_WAKE_IPI_SENT.store(true, Ordering::Release);
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
    let ring3_cpus = RING3_CPU_MASK.load(Ordering::Acquire);
    let ring3_migrated = cpu_count == 1 || ring3_cpus & !1 != 0;
    let reschedule_ipi = cpu_count == 1 || RESCHEDULE_IPIS.load(Ordering::Acquire) != 0;
    let event_wake_ipi = cpu_count == 1 || EVENT_WAKE_IPI_RECEIVED.load(Ordering::Acquire);
    let write_event_mask =
        ((1u64 << logos_abi::IPC_ENDPOINT_COUNT) - 1) << logos_abi::IPC_WRITE_EVENT_BASE;
    let conditions_met = timers
        && progress
        && switches > 20
        && HANDOFF_STARTED.load(Ordering::Acquire)
        && RUNTIME_WAKE_CYCLES.load(Ordering::Acquire) >= 3
        && RUNTIME_TIMED_OUT.load(Ordering::Acquire)
        && RUNTIME_COMPLETED.load(Ordering::Acquire)
        && RUNTIME_SLOT_REUSED.load(Ordering::Acquire)
        && RUNTIME_MAILBOX_BUSY.load(Ordering::Acquire)
        && COMPLETION_RECLAIMED.load(Ordering::Acquire)
        && COMPLETION_STALE_REJECTED.load(Ordering::Acquire)
        && COMPLETION_SLOT_REUSED.load(Ordering::Acquire)
        && REPLACEMENT_RAN.load(Ordering::Acquire)
        && HEALTH_RESTARTED.load(Ordering::Acquire)
        && HEALTH_LATE_COMPLETION_REJECTED.load(Ordering::Acquire)
        && HEALTH_RETRY_COMPLETED.load(Ordering::Acquire)
        && MANAGER_RESTART_COMPLETED.load(Ordering::Acquire)
        && MANAGER_SYSCALL_SUCCEEDED.load(Ordering::Acquire)
        && LIVE_SERVICE_RESTARTED.load(Ordering::Acquire)
        && crate::user_mode::syscalls() > 0
        && crate::user_mode::blocked_waits() > 0
        && SCHEDULER.event_wakes() > 0
        && (SCHEDULER.event_signal_mask() & write_event_mask != 0)
        && BACKPRESSURE_FULL.load(Ordering::Acquire)
        && BACKPRESSURE_BLOCKED.load(Ordering::Acquire)
        && BACKPRESSURE_WAKE.load(Ordering::Acquire)
        && BACKPRESSURE_RESUMED.load(Ordering::Acquire)
        && crate::user_mode::fault_observed()
        && RING3_CR3_VALID.load(Ordering::Acquire)
        && HOSTILE_IPC_PROCESS_REJECTED.load(Ordering::Acquire)
        && HOSTILE_IPC_SYSCALL_REJECTED.load(Ordering::Acquire)
        && ring3_migrated
        && reschedule_ipi
        && event_wake_ipi
        && BLOCK_RESUMED.load(Ordering::Acquire)
        && WAKE_DONE.load(Ordering::Acquire)
        && wake_cpus_differ;
    if conditions_met {
        PASSED.store(true, Ordering::Release);
        if cpu == 0 && !REPORTED.swap(true, Ordering::AcqRel) {
            crate::arch_proof_line(b"LogOS vNext: QEMU proof PASS");
        }
    }
}

fn verify_hostile_ipc_boundary() {
    if !crate::arch::hostile_ipc_layout_valid() {
        crate::arch_fatal(b"LogOS vNext: hostile IPC layout");
    }
    let forged_process = crate::process::ProcessHandle::from_raw(0);
    let send = crate::arch::ipc_send(forged_process, 0, 0);
    let receive = crate::arch::ipc_receive(forged_process, 0);
    if send.status != logos_abi::IpcStatus::Unauthorized
        || receive.status != logos_abi::IpcStatus::Unauthorized
    {
        crate::arch_fatal(b"LogOS vNext: hostile IPC rejection");
    }
    HOSTILE_IPC_PROCESS_REJECTED.store(true, Ordering::Release);
}

pub(crate) fn verify_service_manager_boundary() {
    let list = crate::arch::manager_proof(logos_abi::ManagerRequest::new(
        logos_abi::ManagerOperation::List,
        1,
    ))
    .unwrap_or_else(|| crate::arch_fatal(b"LogOS vNext: manager list"));
    if list.status != logos_abi::ManagerStatus::Ok
        || list.record.slot != 0
        || &list.record.name[..usize::from(list.record.name_len)] != b"input"
    {
        crate::arch_fatal(b"LogOS vNext: manager list result");
    }
    let mut status = logos_abi::ManagerRequest::new(logos_abi::ManagerOperation::Status, 2);
    status.slot = 0;
    status.generation = 1;
    let status = crate::arch::manager_proof(status)
        .unwrap_or_else(|| crate::arch_fatal(b"LogOS vNext: manager status"));
    if status.status != logos_abi::ManagerStatus::Ok
        || status.record.state != logos_abi::ManagerState::Running
    {
        crate::arch_fatal(b"LogOS vNext: manager status result");
    }
    if crate::arch::manager_call(
        crate::process::ProcessHandle::from_raw(0),
        logos_abi::MANAGER_CAPABILITY_SLOT,
        0,
    ) != logos_abi::IpcStatus::Unauthorized
    {
        crate::arch_fatal(b"LogOS vNext: manager authorization");
    }
    crate::arch_proof_line(b"LogOS vNext: service manager ready");
}

pub fn hostile_ipc_syscall_rejected() {
    HOSTILE_IPC_SYSCALL_REJECTED.store(true, Ordering::Release);
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
                    if crate::arch::wake_task(handle) {
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

fn backpressure_sender_task() {
    let identity = logos_abi::MessageIdentity::new(1, 1);
    let message = logos_abi::RenderMessage::empty(logos_abi::MessageKind::RenderCells);
    let _ = PROBE_RING.send(identity, message);
    if !matches!(PROBE_RING.send(identity, message), Err(logos_abi::SharedSendError::Full)) {
        crate::arch_fatal(b"LogOS vNext: backpressure probe did not fill");
    }
    BACKPRESSURE_FULL.store(true, Ordering::Release);
    let handle = TaskHandle::from_raw(BACKPRESSURE_HANDLE.load(Ordering::Acquire));
    let event = logos_abi::ipc_write_event_mask(0);
    loop {
        match PROBE_RING.send(identity, message) {
            Ok(_) => {
                BACKPRESSURE_RESUMED.store(true, Ordering::Release);
                break;
            }
            Err(logos_abi::SharedSendError::Full) => {
                if SCHEDULER.wait_for_events(handle, event, u64::MAX) == Some(true) {
                    BACKPRESSURE_BLOCKED.store(true, Ordering::Release);
                    crate::block_current();
                }
            }
            Err(_) => crate::arch_fatal(b"LogOS vNext: backpressure probe IPC"),
        }
    }
    loop {
        crate::yield_current();
    }
}

fn try_backpressure_wake() {
    let identity = logos_abi::MessageIdentity::new(1, 1);
    let event = logos_abi::ipc_write_event_mask(0);
    let raw = BACKPRESSURE_HANDLE.load(Ordering::Acquire);
    if raw == 0
        || !BACKPRESSURE_BLOCKED.load(Ordering::Acquire)
        || SCHEDULER.state(TaskHandle::from_raw(raw)) != Some(TaskState::Blocked)
    {
        return;
    }
    let Ok((_, notification)) = PROBE_RING.receive_with_notify(identity) else {
        return;
    };
    if notification != logos_abi::Notify::Notified {
        crate::arch_fatal(b"LogOS vNext: backpressure probe edge");
    }
    crate::arch::signal_events(event);
    BACKPRESSURE_WAKE.store(true, Ordering::Release);
}

fn completion_task() {}

fn reclaimer_task() {
    loop {
        try_backpressure_wake();
        let raw = COMPLETION_HANDLE.load(Ordering::Acquire);
        if raw != 0 {
            let completed = TaskHandle::from_raw(raw);
            if SCHEDULER.state(completed) == Some(TaskState::Completed)
                && SCHEDULER.reclaim_completed(completed)
            {
                COMPLETION_RECLAIMED.store(true, Ordering::Release);
                let stale_rejected =
                    !crate::arch::wake_task(completed) && SCHEDULER.state(completed).is_none();
                COMPLETION_STALE_REJECTED.store(stale_rejected, Ordering::Release);
                let replacement = SCHEDULER.spawn(replacement_task).expect("proof task capacity");
                COMPLETION_SLOT_REUSED.store(
                    replacement.slot() == completed.slot()
                        && replacement.generation() != completed.generation(),
                    Ordering::Release,
                );
            }
        }
        crate::yield_current();
    }
}

fn replacement_task() {
    REPLACEMENT_RAN.store(true, Ordering::Release);
    loop {
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
