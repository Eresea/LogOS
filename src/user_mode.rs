use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use crate::arch::{USER_CODE_SELECTOR, USER_DATA_SELECTOR};
use crate::process::{
    AddressSpaceRoot, ElfLoadPlan, MappingFlags, ProcessHandle, ProcessTable, VirtualMapping,
};
use crate::syscall::{Syscall, SyscallFrame};
use crate::{SCHEDULER, TaskHandle};

const PAGE_SIZE: usize = 4096;
const ADDRESS_MASK: usize = 0x000f_ffff_ffff_f000;
const PRESENT: u64 = 1;
const WRITABLE: u64 = 1 << 1;
const USER: u64 = 1 << 2;
const USER_CODE_VA: usize = 0x0000_0080_0000_0000;
const USER_STACK_VA: usize = USER_CODE_VA + PAGE_SIZE;
const SWITCH_VECTOR: u8 = 49;
const PROOF_IMAGE_LEN: usize = 0x89;
const SYSCALL_YIELD: usize = logos_abi::YIELD_SYSCALL;

#[repr(C, align(4096))]
struct PageTable([u64; 512]);

#[repr(C, align(4096))]
struct UserPage([u8; PAGE_SIZE]);

static mut USER_PML4: PageTable = PageTable([0; 512]);
static mut USER_PDPT: PageTable = PageTable([0; 512]);
static mut USER_PD: PageTable = PageTable([0; 512]);
static mut USER_PT: PageTable = PageTable([0; 512]);
static mut USER_CODE_PAGE: UserPage = UserPage(proof_code_page());
static mut USER_STACK_PAGE: UserPage = UserPage([0; PAGE_SIZE]);
static mut USER_IMAGE: [u8; PROOF_IMAGE_LEN] = [0; PROOF_IMAGE_LEN];
static mut USER_PROCESS_TABLE: ProcessTable = ProcessTable::new();
static mut USER_PROCESS: Option<ProcessHandle> = None;

static USER_CR3: AtomicUsize = AtomicUsize::new(0);
static KERNEL_CR3: AtomicUsize = AtomicUsize::new(0);
static USER_TASK_RAW: AtomicU64 = AtomicU64::new(0);
static USER_SYSCALLS: AtomicU64 = AtomicU64::new(0);
static USER_FAULTED: AtomicBool = AtomicBool::new(false);
static USER_FAULT_VECTOR: AtomicUsize = AtomicUsize::new(0);

const fn proof_code_page() -> [u8; PAGE_SIZE] {
    let mut page = [0x90; PAGE_SIZE];
    page[0] = 0xcd;
    page[1] = SWITCH_VECTOR;
    // CPL3 cannot disable interrupts; CLI raises #GP and exercises its error-code frame.
    page[2] = 0xfa;
    page
}

pub(crate) fn spawn_proof() {
    let kernel_cr3 = crate::arch::current_cr3() & ADDRESS_MASK;
    KERNEL_CR3.store(kernel_cr3, Ordering::Release);
    build_address_space(kernel_cr3);
    register_proof_process();
    crate::arch_proof_line(b"LogOS vNext: user space ready");
    let handle = SCHEDULER
        .spawn_with_address_space(user_task_entry, USER_CR3.load(Ordering::Acquire))
        .unwrap_or_else(|_| crate::arch_fatal(b"LogOS vNext: user task capacity"));
    USER_TASK_RAW.store(handle.raw(), Ordering::Release);
}

pub(crate) fn initialize_kernel_cr3(root: usize) {
    KERNEL_CR3.store(root & ADDRESS_MASK, Ordering::Release);
}

pub(crate) fn reserve_frames(pool: &mut crate::frame_pool::FramePool) {
    for (address, bytes) in [
        (core::ptr::addr_of!(USER_PML4) as usize, core::mem::size_of::<PageTable>()),
        (core::ptr::addr_of!(USER_PDPT) as usize, core::mem::size_of::<PageTable>()),
        (core::ptr::addr_of!(USER_PD) as usize, core::mem::size_of::<PageTable>()),
        (core::ptr::addr_of!(USER_PT) as usize, core::mem::size_of::<PageTable>()),
        (core::ptr::addr_of!(USER_CODE_PAGE) as usize, core::mem::size_of::<UserPage>()),
        (core::ptr::addr_of!(USER_STACK_PAGE) as usize, core::mem::size_of::<UserPage>()),
        (core::ptr::addr_of!(USER_IMAGE) as usize, core::mem::size_of::<[u8; PROOF_IMAGE_LEN]>()),
        (core::ptr::addr_of!(USER_PROCESS_TABLE) as usize, core::mem::size_of::<ProcessTable>()),
        (core::ptr::addr_of!(USER_PROCESS) as usize, core::mem::size_of::<Option<ProcessHandle>>()),
        (core::ptr::addr_of!(USER_CR3) as usize, core::mem::size_of::<AtomicUsize>()),
        (core::ptr::addr_of!(KERNEL_CR3) as usize, core::mem::size_of::<AtomicUsize>()),
        (core::ptr::addr_of!(USER_TASK_RAW) as usize, core::mem::size_of::<AtomicU64>()),
        (core::ptr::addr_of!(USER_SYSCALLS) as usize, core::mem::size_of::<AtomicU64>()),
        (core::ptr::addr_of!(USER_FAULTED) as usize, core::mem::size_of::<AtomicBool>()),
        (core::ptr::addr_of!(USER_FAULT_VECTOR) as usize, core::mem::size_of::<AtomicUsize>()),
    ] {
        crate::arch::reserve_storage_frames(pool, address, bytes);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FaultDisposition {
    Retry,
    Contained,
    Fatal(FatalFaultBranch),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FatalFaultBranch {
    UnsupportedVector,
    MissingServiceLaunch,
    ServicePageFault,
    ServiceProcessFault,
    ProofProcessFault,
}

pub(crate) fn faulted(handle: TaskHandle, vector: usize, fault_address: usize) -> FaultDisposition {
    if !matches!(vector, 6 | 13 | 14) {
        return FaultDisposition::Fatal(FatalFaultBranch::UnsupportedVector);
    }
    if handle.raw() != USER_TASK_RAW.load(Ordering::Acquire) {
        let Some(launch) = SCHEDULER.user_launch(handle) else {
            return FaultDisposition::Fatal(FatalFaultBranch::MissingServiceLaunch);
        };
        if vector == 14 {
            return match crate::arch::fault_service_page(launch.process(), fault_address) {
                crate::service_runtime::ServiceFaultOutcome::Retry => FaultDisposition::Retry,
                crate::service_runtime::ServiceFaultOutcome::Contained => {
                    FaultDisposition::Contained
                }
                crate::service_runtime::ServiceFaultOutcome::Fatal => {
                    FaultDisposition::Fatal(FatalFaultBranch::ServicePageFault)
                }
            };
        }
        return if crate::arch::fault_service_process(launch.process(), vector as u8) {
            FaultDisposition::Contained
        } else {
            FaultDisposition::Fatal(FatalFaultBranch::ServiceProcessFault)
        };
    }
    if mark_fault(vector) {
        FaultDisposition::Contained
    } else {
        FaultDisposition::Fatal(FatalFaultBranch::ProofProcessFault)
    }
}

#[cfg(feature = "qemu-proof")]
pub(crate) fn fault_service(handle: TaskHandle) -> Option<logos_abi::ServiceId> {
    SCHEDULER
        .user_launch(handle)
        .and_then(|launch| crate::arch::service_for_process(launch.process()))
}

pub(crate) fn syscall_faulted(handle: TaskHandle) -> bool {
    if handle.raw() == USER_TASK_RAW.load(Ordering::Acquire) {
        return mark_fault(SWITCH_VECTOR as usize);
    }
    let Some(launch) = SCHEDULER.user_launch(handle) else {
        return false;
    };
    crate::arch::fault_service_process(launch.process(), SWITCH_VECTOR)
}

fn mark_fault(vector: usize) -> bool {
    USER_FAULT_VECTOR.store(vector, Ordering::Release);
    let process = unsafe { *core::ptr::addr_of!(USER_PROCESS) };
    let Some(process) = process else {
        return false;
    };
    if unsafe { (*core::ptr::addr_of_mut!(USER_PROCESS_TABLE)).fault(process, vector as u8) }
        .is_err()
    {
        return false;
    }
    USER_FAULTED.store(true, Ordering::Release);
    crate::arch_proof_line(b"LogOS vNext: user exception contained");
    true
}

/// Syscalls that run kernel services and so need the kernel address space.
fn needs_kernel_space(syscall: Syscall) -> bool {
    matches!(
        syscall,
        Syscall::Event
            | Syscall::DebugLine
            | Syscall::HeapGrow
            | Syscall::HeapShrink
            | Syscall::IpcSend
            | Syscall::IpcReceive
            | Syscall::Directory
            | Syscall::Manager
    )
}

pub(crate) fn dispatch_syscall(handle: TaskHandle, fx_context: usize) -> bool {
    // SAFETY: the vector-49 stub passes the fxsave area built by `context_common`.
    let mut frame = unsafe { SyscallFrame::from_fx_context(fx_context) };
    // Unknown numbers fall through to the caller's contained-fault path.
    let Ok(syscall) = frame.number() else {
        return false;
    };
    match syscall {
        Syscall::Yield => {
            if USER_FAULTED.load(Ordering::Acquire)
                || handle.raw() != USER_TASK_RAW.load(Ordering::Acquire)
            {
                return false;
            }
            frame.set_return(0);
            USER_SYSCALLS.fetch_add(1, Ordering::Relaxed);
            return true;
        }
        Syscall::Heartbeat => return heartbeat(handle, &mut frame),
        // Without a debug console the number behaves like an unknown one.
        #[cfg(not(any(feature = "qemu-proof", feature = "input-debug")))]
        Syscall::DebugLine => return false,
        _ => {}
    }
    let Some(launch) = SCHEDULER.user_launch(handle) else {
        return false;
    };
    let process = launch.process();
    let kernel_space = needs_kernel_space(syscall);
    if kernel_space {
        prepare_kernel();
    }
    let value = match syscall {
        Syscall::Yield | Syscall::Heartbeat => unreachable!("handled before the common path"),
        Syscall::Event => crate::arch::event_call(process, frame.arg(1)) as usize,
        Syscall::CurrentTicks => crate::current_ticks() as usize,
        Syscall::WallTime => crate::arch::wall_time().pack() as usize,
        Syscall::SystemInfo => crate::arch::system_info().pack() as usize,
        #[cfg(any(feature = "qemu-proof", feature = "input-debug"))]
        Syscall::DebugLine => {
            if crate::arch::service_debug_line(process, frame.arg(0)) {
                0
            } else {
                1
            }
        }
        #[cfg(not(any(feature = "qemu-proof", feature = "input-debug")))]
        Syscall::DebugLine => unreachable!("rejected before the common path"),
        Syscall::HeapGrow => {
            crate::arch::grow_service_heap(process, frame.arg(0) as u64, frame.arg(1)) as usize
        }
        Syscall::HeapShrink => {
            crate::arch::shrink_service_heap(process, frame.arg(0) as u64, frame.arg(1)) as usize
        }
        Syscall::IpcSend => {
            let outcome = crate::arch::ipc_send(process, frame.arg(0) as u64, frame.arg(1));
            #[cfg(all(feature = "qemu-proof", target_os = "uefi"))]
            if outcome.status == logos_abi::IpcStatus::Unauthorized {
                crate::proof::hostile_ipc_syscall_rejected();
            }
            outcome.status as usize
        }
        Syscall::Directory => {
            crate::arch::directory_call(process, frame.arg(0) as u64, frame.arg(1)) as usize
        }
        Syscall::IpcReceive => {
            let outcome = crate::arch::ipc_receive(process, frame.arg(0) as u64);
            #[cfg(all(feature = "qemu-proof", target_os = "uefi"))]
            if outcome.status == logos_abi::IpcStatus::Unauthorized {
                crate::proof::hostile_ipc_syscall_rejected();
            }
            outcome.status as usize
        }
        Syscall::Manager => {
            let status = crate::arch::manager_call(process, frame.arg(0) as u64, frame.arg(1));
            #[cfg(all(feature = "qemu-proof", target_os = "uefi"))]
            if status == logos_abi::IpcStatus::Ok {
                crate::proof::manager_syscall_succeeded();
            }
            status as usize
        }
        Syscall::Power => {
            if crate::arch::power_control(process, frame.arg(0)) {
                0
            } else {
                1
            }
        }
        Syscall::Exit => {
            let code = frame.arg(0) as u8;
            if crate::arch::exit_process(process, code) {
                let _ = SCHEDULER.request_stop(handle);
                0
            } else {
                1
            }
        }
    };
    // The frame lives in kernel memory: write the result before leaving the
    // kernel address space.
    frame.set_return(value);
    USER_SYSCALLS.fetch_add(1, Ordering::Relaxed);
    if kernel_space {
        prepare_address_space(launch.address_space_root());
    }
    true
}

/// Heartbeat is not counted and rejects (contained fault) on a bad handle.
fn heartbeat(handle: TaskHandle, frame: &mut SyscallFrame) -> bool {
    let Some(launch) = SCHEDULER.user_launch(handle) else {
        return false;
    };
    let Some(service) = logos_abi::ServiceHandle::from_raw(frame.arg(0) as u64) else {
        return false;
    };
    if !crate::arch::record_service_heartbeat(service, launch.process(), crate::current_ticks()) {
        return false;
    }
    frame.set_return(0);
    true
}

pub(crate) fn is_user_task(handle: TaskHandle) -> bool {
    handle.raw() == USER_TASK_RAW.load(Ordering::Acquire) || SCHEDULER.user_launch(handle).is_some()
}

pub(crate) fn prepare_address_space(root: usize) {
    let root = if root == 0 { KERNEL_CR3.load(Ordering::Acquire) } else { root };
    switch_cr3(root);
}

pub(crate) fn prepare_kernel() {
    prepare_address_space(0);
}

fn switch_cr3(root: usize) {
    if root == 0 {
        crate::arch_fatal(b"LogOS vNext: missing CR3");
    }
    unsafe {
        core::arch::asm!("mov cr3, {root}", root = in(reg) root, options(nostack, preserves_flags));
    }
}

pub(crate) fn syscalls() -> u64 {
    USER_SYSCALLS.load(Ordering::Acquire)
}

pub(crate) fn fault_observed() -> bool {
    USER_FAULTED.load(Ordering::Acquire)
        && matches!(USER_FAULT_VECTOR.load(Ordering::Acquire), 6 | 13 | 14)
}

fn build_address_space(kernel_cr3: usize) {
    unsafe {
        // The current firmware page tables identity-map the kernel image and
        // these fixed tables; the user root preserves every kernel mapping.
        core::ptr::copy_nonoverlapping(
            kernel_cr3 as *const u64,
            core::ptr::addr_of_mut!(USER_PML4.0).cast::<u64>(),
            512,
        );
        USER_PML4.0[1] = table_address(core::ptr::addr_of!(USER_PDPT)) | PRESENT | WRITABLE | USER;
        USER_PDPT.0[0] = table_address(core::ptr::addr_of!(USER_PD)) | PRESENT | WRITABLE | USER;
        USER_PD.0[0] = table_address(core::ptr::addr_of!(USER_PT)) | PRESENT | WRITABLE | USER;
        USER_PT.0[0] = page_address(core::ptr::addr_of!(USER_CODE_PAGE)) | PRESENT | USER;
        USER_PT.0[1] =
            page_address(core::ptr::addr_of!(USER_STACK_PAGE)) | PRESENT | WRITABLE | USER;
    }
    USER_CR3.store(core::ptr::addr_of!(USER_PML4) as usize, Ordering::Release);
}

fn register_proof_process() {
    unsafe {
        build_proof_image();
        let image = core::slice::from_raw_parts(
            core::ptr::addr_of!(USER_IMAGE).cast::<u8>(),
            PROOF_IMAGE_LEN,
        );
        let plan = ElfLoadPlan::parse(image)
            .unwrap_or_else(|_| crate::arch_fatal(b"LogOS vNext: proof ELF"));
        let segment =
            plan.segment(0).unwrap_or_else(|| crate::arch_fatal(b"LogOS vNext: proof segment"));
        let process = (*core::ptr::addr_of_mut!(USER_PROCESS_TABLE))
            .start(image)
            .unwrap_or_else(|_| crate::arch_fatal(b"LogOS vNext: proof process"));
        let root = AddressSpaceRoot::new(core::ptr::addr_of!(USER_PML4) as usize)
            .unwrap_or_else(|| crate::arch_fatal(b"LogOS vNext: proof root"));
        let table = &mut *core::ptr::addr_of_mut!(USER_PROCESS_TABLE);
        table
            .bind_address_space_root(process, root)
            .unwrap_or_else(|_| crate::arch_fatal(b"LogOS vNext: proof root bind"));
        let code_page = core::ptr::addr_of_mut!(USER_CODE_PAGE).cast::<u8>();
        core::ptr::write_bytes(code_page, 0, PAGE_SIZE);
        let file = segment
            .file_bytes(image)
            .unwrap_or_else(|| crate::arch_fatal(b"LogOS vNext: proof bytes"));
        core::ptr::copy_nonoverlapping(file.as_ptr(), code_page, file.len());
        let code_mapping =
            VirtualMapping::new(USER_CODE_VA, code_page as usize, 1, segment.flags())
                .unwrap_or_else(|| crate::arch_fatal(b"LogOS vNext: proof code map"));
        table
            .map(process, code_mapping)
            .unwrap_or_else(|_| crate::arch_fatal(b"LogOS vNext: proof code map"));
        let stack_page = core::ptr::addr_of_mut!(USER_STACK_PAGE).cast::<u8>();
        let stack_mapping =
            VirtualMapping::new(USER_STACK_VA, stack_page as usize, 1, MappingFlags::DATA)
                .unwrap_or_else(|| crate::arch_fatal(b"LogOS vNext: proof stack map"));
        table
            .map(process, stack_mapping)
            .unwrap_or_else(|_| crate::arch_fatal(b"LogOS vNext: proof stack map"));
        *core::ptr::addr_of_mut!(USER_PROCESS) = Some(process);
    }
}

unsafe fn build_proof_image() {
    unsafe {
        let image = core::ptr::addr_of_mut!(USER_IMAGE).cast::<u8>();
        core::ptr::write_bytes(image, 0, PROOF_IMAGE_LEN);
        core::ptr::copy_nonoverlapping(b"\x7fELF".as_ptr(), image, 4);
        *image.add(4) = 2;
        *image.add(5) = 1;
        write_u16(image, 16, 2);
        write_u16(image, 18, 0x3e);
        write_u64(image, 24, USER_CODE_VA as u64);
        write_u64(image, 32, 64);
        write_u16(image, 54, 56);
        write_u16(image, 56, 1);
        write_u32(image, 64, 1);
        write_u32(image, 68, 5);
        write_u64(image, 72, 0x80);
        write_u64(image, 80, USER_CODE_VA as u64);
        write_u64(image, 88, 0);
        write_u64(image, 96, 9);
        write_u64(image, 104, 9);
        write_u64(image, 112, PAGE_SIZE as u64);
        *image.add(0x80) = 0xb8;
        *image.add(0x81) = SYSCALL_YIELD as u8;
        *image.add(0x82) = 0;
        *image.add(0x83) = 0;
        *image.add(0x84) = 0;
        *image.add(0x85) = 0xcd;
        *image.add(0x86) = SWITCH_VECTOR;
        *image.add(0x87) = 0x0f;
        *image.add(0x88) = 0x0b;
    }
}

unsafe fn write_u16(base: *mut u8, offset: usize, value: u16) {
    unsafe { core::ptr::copy_nonoverlapping(value.to_le_bytes().as_ptr(), base.add(offset), 2) };
}

unsafe fn write_u32(base: *mut u8, offset: usize, value: u32) {
    unsafe { core::ptr::copy_nonoverlapping(value.to_le_bytes().as_ptr(), base.add(offset), 4) };
}

unsafe fn write_u64(base: *mut u8, offset: usize, value: u64) {
    unsafe { core::ptr::copy_nonoverlapping(value.to_le_bytes().as_ptr(), base.add(offset), 8) };
}

fn table_address(table: *const PageTable) -> u64 {
    table as usize as u64 & ADDRESS_MASK as u64
}

fn page_address(page: *const UserPage) -> u64 {
    page as usize as u64 & ADDRESS_MASK as u64
}

fn user_task_entry() {
    let cr3 = USER_CR3.load(Ordering::Acquire);
    let user_stack = USER_STACK_VA + PAGE_SIZE - 8;
    let user_entry = USER_CODE_VA;
    crate::arch_proof_line(b"LogOS vNext: user ring3 entry");
    unsafe {
        core::arch::asm!(
            "mov cr3, {cr3}",
            "push {user_data}",
            "push {user_stack}",
            "pushfq",
            "push {user_code}",
            "push {user_entry}",
            "iretq",
            cr3 = in(reg) cr3,
            user_data = const USER_DATA_SELECTOR,
            user_stack = in(reg) user_stack,
            user_code = const USER_CODE_SELECTOR,
            user_entry = in(reg) user_entry,
            options(noreturn),
        );
    }
}
