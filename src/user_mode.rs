use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use crate::arch::{USER_CODE_SELECTOR, USER_DATA_SELECTOR};
use crate::{SCHEDULER, TaskHandle};

const PAGE_SIZE: usize = 4096;
const ADDRESS_MASK: usize = 0x000f_ffff_ffff_f000;
const PRESENT: u64 = 1;
const WRITABLE: u64 = 1 << 1;
const USER: u64 = 1 << 2;
const USER_CODE_VA: usize = 0x0000_0080_0000_0000;
const USER_STACK_VA: usize = USER_CODE_VA + PAGE_SIZE;
const SWITCH_VECTOR: u8 = 49;

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

static USER_CR3: AtomicUsize = AtomicUsize::new(0);
static USER_TASK_RAW: AtomicU64 = AtomicU64::new(0);
static USER_SYSCALLS: AtomicU64 = AtomicU64::new(0);
static USER_FAULTED: AtomicBool = AtomicBool::new(false);
static USER_FAULT_VECTOR: AtomicUsize = AtomicUsize::new(0);

const fn proof_code_page() -> [u8; PAGE_SIZE] {
    let mut page = [0x90; PAGE_SIZE];
    page[0] = 0xcd;
    page[1] = SWITCH_VECTOR;
    page[2] = 0x0f;
    page[3] = 0x0b;
    page
}

pub(crate) fn spawn_proof() {
    build_address_space(crate::arch::current_cr3() & ADDRESS_MASK);
    crate::arch_proof_line(b"LogOS vNext: user space ready");
    let handle = SCHEDULER
        .spawn(user_task_entry)
        .unwrap_or_else(|_| crate::arch_fatal(b"LogOS vNext: user task capacity"));
    USER_TASK_RAW.store(handle.raw(), Ordering::Release);
}

pub(crate) fn faulted(handle: TaskHandle, vector: usize) -> bool {
    if !matches!(vector, 6 | 13 | 14) || handle.raw() != USER_TASK_RAW.load(Ordering::Acquire) {
        return false;
    }
    USER_FAULT_VECTOR.store(vector, Ordering::Release);
    USER_FAULTED.store(true, Ordering::Release);
    crate::arch_proof_line(b"LogOS vNext: user exception contained");
    true
}

pub(crate) fn record_syscall(handle: TaskHandle) {
    if !USER_FAULTED.load(Ordering::Acquire)
        && handle.raw() == USER_TASK_RAW.load(Ordering::Acquire)
    {
        USER_SYSCALLS.fetch_add(1, Ordering::Relaxed);
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
