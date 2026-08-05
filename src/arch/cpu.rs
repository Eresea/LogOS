use core::{
    arch::asm,
    mem::size_of,
    ptr,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};

use super::writable::Writable;
use crate::mm::memory::{Page, PhysicalMemory};

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

static GDT: Writable<[u64; 7]> = Writable::new([0; 7]);
static TSS: Writable<Tss> = Writable::new(Tss {
    reserved: 0,
    rsp: [0; 3],
    reserved2: 0,
    ist: [0; 7],
    reserved3: 0,
    reserved4: 0,
    iomap_base: size_of::<Tss>() as u16,
});
static USER_RETURNED: AtomicBool = AtomicBool::new(false);
static USER_CONTEXT: AtomicU64 = AtomicU64::new(0);
static USER_BLOCKED: AtomicBool = AtomicBool::new(false);
static USER_COMMAND: AtomicBool = AtomicBool::new(false);
static USER_DISPLAY: AtomicBool = AtomicBool::new(false);
static USER_PANICKED: AtomicBool = AtomicBool::new(false);
static USER_FAULTED: AtomicBool = AtomicBool::new(false);
static USER_FAULT_VECTOR: AtomicU64 = AtomicU64::new(0);
static USER_FAULT_ERROR: AtomicU64 = AtomicU64::new(0);
static USER_FAULT_RIP: AtomicU64 = AtomicU64::new(0);
static USER_FAULT_CR2: AtomicU64 = AtomicU64::new(0);
const USER_FRAME_WORDS: usize = 20;
#[unsafe(no_mangle)]
static USER_FRAME: Writable<[u64; USER_FRAME_WORDS]> = Writable::new([0; USER_FRAME_WORDS]);
#[unsafe(no_mangle)]
static USER_RETURN_RSP: Writable<u64> = Writable::new(0);
#[unsafe(no_mangle)]
static USER_RETURN_CR3: Writable<u64> = Writable::new(0);
#[unsafe(no_mangle)]
static USER_RETURN_FLAGS: Writable<u64> = Writable::new(0);

pub struct Privilege {
    stack: Page,
}

pub struct GateState {
    blocked: bool,
    command: bool,
    display: bool,
    frame: [u64; USER_FRAME_WORDS],
}

impl GateState {
    pub const fn new() -> Self {
        Self { blocked: false, command: false, display: false, frame: [0; USER_FRAME_WORDS] }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub enum EntryState {
    Returned,
    Input,
    Command,
    Display,
    Panic,
    Fault(FaultRecord),
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct FaultRecord {
    pub vector: u8,
    pub error: u64,
    pub rip: u64,
    pub cr2: u64,
}

impl Privilege {
    pub fn install(physical: &mut PhysicalMemory) -> Option<Self> {
        let stack = physical.allocate_owned()?;
        let stack_top = stack.address().checked_add(4096)?;
        unsafe {
            let tss = TSS.get();
            (*tss).rsp[0] = stack_top;
            (*tss).iomap_base = size_of::<Tss>() as u16;
            let gdt = GDT.get();
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

    pub fn run_entry(
        &self,
        space: &mut crate::mm::address_space::AddressSpace,
        entry: u64,
        context: u64,
        gate: &mut GateState,
    ) -> Option<EntryState> {
        if !space.map_kernel_stack(self.stack.address()) {
            return None;
        }
        unsafe { set_tss_stack(space.kernel_stack_top()) };
        self.restore_gate(gate);
        USER_RETURNED.store(false, Ordering::Release);
        USER_PANICKED.store(false, Ordering::Release);
        USER_FAULTED.store(false, Ordering::Release);
        USER_BLOCKED.store(false, Ordering::Release);
        USER_CONTEXT.store(context, Ordering::Release);
        unsafe { enter_user(space.cr3(), entry, space.stack_top(), context) };
        unsafe { set_tss_stack(self.stack.address() + 4096) };
        self.capture_gate(gate)
    }

    pub fn resume_entry(
        &self,
        space: &mut crate::mm::address_space::AddressSpace,
        context: u64,
        gate: &mut GateState,
    ) -> Option<EntryState> {
        self.restore_gate(gate);
        if !USER_BLOCKED.swap(false, Ordering::AcqRel)
            || !space.map_kernel_stack(self.stack.address())
        {
            return None;
        }
        unsafe { set_tss_stack(space.kernel_stack_top()) };
        USER_RETURNED.store(false, Ordering::Release);
        USER_PANICKED.store(false, Ordering::Release);
        USER_FAULTED.store(false, Ordering::Release);
        USER_CONTEXT.store(context, Ordering::Release);
        unsafe { resume_user(space.cr3(), space.kernel_stack_top()) };
        unsafe { set_tss_stack(self.stack.address() + 4096) };
        self.capture_gate(gate)
    }

    fn restore_gate(&self, gate: &GateState) {
        unsafe {
            ptr::copy_nonoverlapping(gate.frame.as_ptr(), USER_FRAME.get().cast(), USER_FRAME_WORDS)
        };
        USER_BLOCKED.store(gate.blocked, Ordering::Release);
        USER_COMMAND.store(gate.command, Ordering::Release);
        USER_DISPLAY.store(gate.display, Ordering::Release);
    }

    fn capture_gate(&self, gate: &mut GateState) -> Option<EntryState> {
        gate.blocked = USER_BLOCKED.load(Ordering::Acquire);
        gate.command = USER_COMMAND.load(Ordering::Acquire);
        gate.display = USER_DISPLAY.load(Ordering::Acquire);
        unsafe {
            ptr::copy_nonoverlapping(
                USER_FRAME.get().cast_const().cast(),
                gate.frame.as_mut_ptr(),
                USER_FRAME_WORDS,
            )
        };
        if USER_FAULTED.swap(false, Ordering::AcqRel) {
            USER_CONTEXT.store(0, Ordering::Release);
            Some(EntryState::Fault(FaultRecord {
                vector: USER_FAULT_VECTOR.load(Ordering::Acquire) as u8,
                error: USER_FAULT_ERROR.load(Ordering::Acquire),
                rip: USER_FAULT_RIP.load(Ordering::Acquire),
                cr2: USER_FAULT_CR2.load(Ordering::Acquire),
            }))
        } else if USER_PANICKED.swap(false, Ordering::AcqRel) {
            USER_CONTEXT.store(0, Ordering::Release);
            Some(EntryState::Panic)
        } else if gate.blocked {
            Some(if USER_DISPLAY.load(Ordering::Acquire) {
                EntryState::Display
            } else if USER_COMMAND.load(Ordering::Acquire) {
                EntryState::Command
            } else {
                EntryState::Input
            })
        } else {
            USER_CONTEXT.store(0, Ordering::Release);
            USER_RETURNED.load(Ordering::Acquire).then_some(EntryState::Returned)
        }
    }
}

#[unsafe(no_mangle)]
extern "C" fn user_fault(vector: u64, error: u64, rip: u64, cr2: u64) {
    USER_FAULT_VECTOR.store(vector, Ordering::Release);
    USER_FAULT_ERROR.store(error, Ordering::Release);
    USER_FAULT_RIP.store(rip, Ordering::Release);
    USER_FAULT_CR2.store(cr2, Ordering::Release);
    USER_FAULTED.store(true, Ordering::Release);
}

#[unsafe(no_mangle)]
extern "C" fn user_gate_returned() {
    USER_RETURNED.store(true, Ordering::Release);
}

#[unsafe(no_mangle)]
extern "C" fn user_gate_resume(frame: *const u64) -> u8 {
    let context = USER_CONTEXT.load(Ordering::Acquire);
    if context != 0 && unsafe { logos_abi::service::ControlPage::panicked_at(context) } {
        USER_PANICKED.store(true, Ordering::Release);
        return 0;
    }
    if context != 0 && unsafe { logos_core::native_service::ControlPage::acknowledge_at(context) } {
        return 1;
    }
    if context != 0
        && unsafe { logos_core::native_service::ControlPage::display_waiting_at(context) }
        && save_user_frame(frame, false, true)
    {
        return 2;
    }
    if context != 0
        && unsafe { logos_core::native_service::ControlPage::input_waiting_at(context) }
        && save_user_frame(frame, false, false)
    {
        return 2;
    }
    if context != 0
        && unsafe { logos_core::native_service::ControlPage::session_server_waiting_at(context) }
        && save_user_frame(frame, false, false)
    {
        return 2;
    }
    if context != 0
        && unsafe { logos_core::native_service::ControlPage::session_client_pending_at(context) }
        && save_user_frame(frame, true, false)
    {
        return 2;
    }
    if context != 0
        && unsafe { logos_core::native_service::ControlPage::store_client_pending_at(context) }
        && save_user_frame(frame, true, false)
    {
        return 2;
    }
    if context != 0
        && unsafe {
            logos_core::native_service::ControlPage::store_client_reply_pending_at(context)
        }
        && save_user_frame(frame, true, false)
    {
        return 2;
    }
    if context != 0
        && unsafe { logos_core::native_service::ControlPage::block_client_pending_at(context) }
        && save_user_frame(frame, true, false)
    {
        return 2;
    }
    if context != 0
        && unsafe {
            logos_core::native_service::ControlPage::session_server_reply_pending_at(context)
        }
        && save_user_frame(frame, true, false)
    {
        return 2;
    }
    if context != 0
        && unsafe { logos_core::native_service::ControlPage::effect_pending_at(context) }
        && save_user_frame(frame, true, false)
    {
        return 2;
    }
    if context != 0
        && (unsafe { logos_core::native_service::ControlPage::network_at(context) }.is_some()
            || unsafe {
                logos_core::native_service::ControlPage::network_reply_pending_at(context)
            }
            || unsafe {
                let page =
                    (context as *const logos_core::native_service::ControlPage).read_volatile();
                logos_core::native_service::NetworkDevicePage::active_for_core_at(
                    page.network_device_page,
                    page.generation,
                    page.generation,
                ) || logos_core::native_service::NetworkEventPage::active_for_core_at(
                    page.network_event_page,
                    page.generation,
                    page.generation,
                )
            })
        && save_user_frame(frame, true, false)
    {
        return 2;
    }
    if context != 0
        && unsafe { logos_core::native_service::ControlPage::remote_gate_at(context) }.is_some()
        && save_user_frame(frame, true, false)
    {
        return 2;
    }
    0
}

fn save_user_frame(frame: *const u64, command: bool, display: bool) -> bool {
    if USER_BLOCKED.swap(true, Ordering::AcqRel) {
        return false;
    }
    unsafe { ptr::copy_nonoverlapping(frame, USER_FRAME.get().cast(), USER_FRAME_WORDS) };
    USER_COMMAND.store(command, Ordering::Release);
    USER_DISPLAY.store(display, Ordering::Release);
    true
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
    unsafe { (*TSS.get()).rsp[0] = stack_top };
}

unsafe extern "C" {
    fn reload_segments();
    fn enter_user(cr3: u64, entry: u64, stack: u64, context: u64);
    fn resume_user(cr3: u64, stack: u64);
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
    "mov rcx, r9",
    "push 0x1b",
    "push r8",
    "push 0x202",
    "push 0x23",
    "push rdx",
    "iretq",
    ".global user_gate",
    "user_gate:",
    "push rax",
    "push rcx",
    "push rdx",
    "push rbx",
    "push rbp",
    "push rsi",
    "push rdi",
    "push r8",
    "push r9",
    "push r10",
    "push r11",
    "push r12",
    "push r13",
    "push r14",
    "push r15",
    "sub rsp, 48",
    "lea rcx, [rsp + 48]",
    "call user_gate_resume",
    "add rsp, 48",
    "cmp al, 1",
    "jne user_gate_not_resumed",
    "pop r15",
    "pop r14",
    "pop r13",
    "pop r12",
    "pop r11",
    "pop r10",
    "pop r9",
    "pop r8",
    "pop rdi",
    "pop rsi",
    "pop rbp",
    "pop rbx",
    "pop rdx",
    "pop rcx",
    "pop rax",
    "iretq",
    "user_gate_not_resumed:",
    "add rsp, 120",
    "user_gate_exit:",
    "sub rsp, 40",
    "call user_gate_returned",
    "add rsp, 40",
    ".global user_fault_exit",
    "user_fault_exit:",
    "mov rax, [rip + USER_RETURN_CR3]",
    "mov cr3, rax",
    "mov rsp, [rip + USER_RETURN_RSP]",
    "push qword ptr [rip + USER_RETURN_FLAGS]",
    "popfq",
    "ret",
    ".global resume_user",
    "resume_user:",
    "mov [rip + USER_RETURN_RSP], rsp",
    "mov rax, cr3",
    "mov [rip + USER_RETURN_CR3], rax",
    "pushfq",
    "pop qword ptr [rip + USER_RETURN_FLAGS]",
    "cli",
    "mov rax, rcx",
    "mov cr3, rax",
    "mov rsp, rdx",
    "sub rsp, 160",
    "lea rsi, [rip + USER_FRAME]",
    "mov rdi, rsp",
    "mov rcx, 20",
    "cld",
    "rep movsq",
    "pop r15",
    "pop r14",
    "pop r13",
    "pop r12",
    "pop r11",
    "pop r10",
    "pop r9",
    "pop r8",
    "pop rdi",
    "pop rsi",
    "pop rbp",
    "pop rbx",
    "pop rdx",
    "pop rcx",
    "pop rax",
    "iretq",
);
