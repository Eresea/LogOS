use core::{
    arch::asm,
    mem::size_of,
    ptr,
    sync::atomic::{AtomicBool, Ordering},
};

use crate::memory::{Page, PhysicalMemory};

pub const KERNEL_CODE: u16 = 0x08;
pub const USER_DATA: u16 = 0x1b;
pub const USER_CODE: u16 = 0x23;
const TSS_SELECTOR: u16 = 0x28;

#[repr(C, packed)]
struct Tss {
    reserved: u32,
    rsp: [u64; 3],
    reserved2: u64,
    ist: [u64; 7],
    reserved3: u64,
    reserved4: u16,
    iomap_base: u16,
}

#[repr(C, packed)]
struct DescriptorTablePointer {
    limit: u16,
    base: u64,
}

// ponytail: bootstrap CPU only; add per-CPU GDT/TSS when multicore support begins.
static mut GDT: [u64; 7] = [0; 7];
static mut TSS: Tss = Tss {
    reserved: 0,
    rsp: [0; 3],
    reserved2: 0,
    ist: [0; 7],
    reserved3: 0,
    reserved4: 0,
    iomap_base: size_of::<Tss>() as u16,
};
static USER_RETURNED: AtomicBool = AtomicBool::new(false);
#[unsafe(no_mangle)]
static mut USER_RETURN_RSP: u64 = 0;
#[unsafe(no_mangle)]
static mut USER_RETURN_CR3: u64 = 0;
#[unsafe(no_mangle)]
static mut USER_RETURN_FLAGS: u64 = 0;

pub struct Privilege {
    stack: Page,
}

impl Privilege {
    pub fn install(physical: &mut PhysicalMemory) -> Option<Self> {
        let stack = physical.allocate_owned()?;
        let stack_top = stack.address().checked_add(4096)?;
        unsafe {
            let tss = ptr::addr_of_mut!(TSS);
            (*tss).rsp[0] = stack_top;
            (*tss).iomap_base = size_of::<Tss>() as u16;
            let gdt = ptr::addr_of_mut!(GDT);
            *gdt = [
                0,
                0x00af_9a00_0000_ffff,
                0x00af_9200_0000_ffff,
                0x00af_f200_0000_ffff,
                0x00af_fa00_0000_ffff,
                tss_low(tss as u64),
                (tss as u64) >> 32,
            ];
            let pointer = DescriptorTablePointer {
                limit: (size_of::<[u64; 7]>() - 1) as u16,
                base: gdt as u64,
            };
            asm!("lgdt [{}]", in(reg) &pointer);
            reload_segments();
            asm!("ltr ax", in("ax") TSS_SELECTOR);
        }
        Some(Self { stack })
    }

    pub fn self_check(&self) -> bool {
        let code: u16;
        let task: u16;
        unsafe {
            asm!("mov {0:x}, cs", out(reg) code);
            asm!("str {0:x}", out(reg) task);
        }
        self.stack.address() != 0
            && code == KERNEL_CODE
            && task == TSS_SELECTOR
            && USER_DATA == 0x1b
            && USER_CODE == 0x23
    }

    pub fn run_entry(&self, space: &mut crate::address_space::AddressSpace, entry: u64) -> bool {
        if !space.map_kernel_stack(self.stack.address()) {
            return false;
        }
        unsafe { set_tss_stack(space.kernel_stack_top()) };
        USER_RETURNED.store(false, Ordering::Release);
        unsafe { enter_user(space.cr3(), entry, space.stack_top()) };
        unsafe { set_tss_stack(self.stack.address() + 4096) };
        USER_RETURNED.load(Ordering::Acquire)
    }
}

#[unsafe(no_mangle)]
extern "C" fn user_gate_returned() {
    USER_RETURNED.store(true, Ordering::Release);
}

fn tss_low(base: u64) -> u64 {
    let limit = (size_of::<Tss>() - 1) as u64;
    (limit & 0xffff)
        | ((base & 0x00ff_ffff) << 16)
        | (0x89 << 40)
        | (limit & 0x000f_0000) << 32
        | (base & 0xff00_0000) << 32
}

unsafe fn set_tss_stack(stack_top: u64) {
    unsafe { (*ptr::addr_of_mut!(TSS)).rsp[0] = stack_top };
}

unsafe extern "C" {
    fn reload_segments();
    fn enter_user(cr3: u64, entry: u64, stack: u64);
}

core::arch::global_asm!(
    ".global reload_segments",
    "reload_segments:",
    "mov ax, 0x10",
    "mov ds, ax",
    "mov es, ax",
    "mov ss, ax",
    "push 0x08",
    "lea rax, [rip + 1f]",
    "push rax",
    "retfq",
    "1:",
    "ret",
    ".global enter_user",
    "enter_user:",
    "mov [rip + USER_RETURN_RSP], rsp",
    "mov rax, cr3",
    "mov [rip + USER_RETURN_CR3], rax",
    "pushfq",
    "pop qword ptr [rip + USER_RETURN_FLAGS]",
    "cli",
    "mov rax, rcx",
    "mov cr3, rax",
    "push 0x1b",
    "push r8",
    "push 0x202",
    "push 0x23",
    "push rdx",
    "iretq",
    ".global user_gate",
    "user_gate:",
    "sub rsp, 40",
    "call user_gate_returned",
    "add rsp, 40",
    "mov rax, [rip + USER_RETURN_CR3]",
    "mov cr3, rax",
    "mov rsp, [rip + USER_RETURN_RSP]",
    "push qword ptr [rip + USER_RETURN_FLAGS]",
    "popfq",
    "ret",
);
