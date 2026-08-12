use core::{
    arch::{asm, global_asm},
    sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering},
};

use uefi::{boot, prelude::*, proto::pi::mp::MpServices};

use crate::{MAX_CPUS, SCHEDULER};

const DEBUG_PORT: u16 = 0xe9;
const APIC_BASE_MSR: u32 = 0x1b;
const APIC_ID: usize = 0x20;
const APIC_EOI: usize = 0xb0;
const APIC_SVR: usize = 0xf0;
const APIC_LVT_TIMER: usize = 0x320;
const APIC_TIMER_INITIAL: usize = 0x380;
const APIC_TIMER_CURRENT: usize = 0x390;
const APIC_TIMER_DIVIDE: usize = 0x3e0;
const APIC_ICR_LOW: usize = 0x300;
const APIC_ICR_HIGH: usize = 0x310;
const TIMER_VECTOR: u8 = 32;
const SWITCH_VECTOR: u8 = 49;
const ACTION_YIELD: u64 = 1;
const ACTION_BLOCK: u64 = 2;
const ACTION_COMPLETE: u64 = 3;
const ACTION_TIMED_BLOCK: u64 = 4;
const FX_STATE_SIZE: usize = 512;
const FX_CONTEXT_POINTER: usize = FX_STATE_SIZE;
const GPR_WORDS: usize = 15;
const VECTOR_OFFSET: usize = GPR_WORDS * 8;
const IDT_ENTRIES: usize = 256;

// Core tasks are ring-0 only: the hardware return frame carries RIP, CS, and
// RFLAGS, while the flat kernel SS is implicit. The saved RSP is the canonical
// frame pointer itself; no user-mode stack-segment frame is synthesized.

static APIC: AtomicUsize = AtomicUsize::new(0);
static TSC_PER_US: AtomicU64 = AtomicU64::new(1);
static HALTED: AtomicBool = AtomicBool::new(false);
static TIMER_TICKS: AtomicU64 = AtomicU64::new(0);
static CPU_COUNT: AtomicUsize = AtomicUsize::new(1);
static APIC_TIMER_COUNT: AtomicU32 = AtomicU32::new(10_000_000);
static APIC_IDS: [AtomicU32; MAX_CPUS] = [const { AtomicU32::new(0) }; MAX_CPUS];
static TRAMPOLINE_PAGE: AtomicUsize = AtomicUsize::new(0);

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct IdtEntry {
    offset_low: u16,
    selector: u16,
    ist: u8,
    attributes: u8,
    offset_middle: u16,
    offset_high: u32,
    reserved: u32,
}

impl IdtEntry {
    const MISSING: Self = Self {
        offset_low: 0,
        selector: 0,
        ist: 0,
        attributes: 0,
        offset_middle: 0,
        offset_high: 0,
        reserved: 0,
    };

    fn new(handler: usize, selector: u16, attributes: u8) -> Self {
        Self {
            offset_low: handler as u16,
            selector,
            ist: 0,
            attributes,
            offset_middle: (handler >> 16) as u16,
            offset_high: (handler >> 32) as u32,
            reserved: 0,
        }
    }
}

#[repr(C, packed)]
struct IdtPointer {
    limit: u16,
    base: u64,
}

#[repr(C, packed)]
struct GdtPointer {
    limit: u16,
    base: u64,
}

static GDT: [u64; 3] = [0, 0x00af_9a00_0000_ffff, 0x00af_9200_0000_ffff];
static mut CPU_GDTS: [[u64; 3]; MAX_CPUS] = [[0; 3]; MAX_CPUS];
static mut CPU_IDTS: [[IdtEntry; IDT_ENTRIES]; MAX_CPUS] =
    [[IdtEntry::MISSING; IDT_ENTRIES]; MAX_CPUS];

#[repr(C, align(16))]
struct CpuStack<const N: usize>([u8; N]);

#[repr(C)]
struct CpuLocal {
    // Keep the GS self-pointer at offset zero; assembly also uses the
    // following fixed offsets for the scheduler and idle stack tops.
    self_ptr: u64,
    scheduler_stack_top: u64,
    idle_stack_top: u64,
    cpu_index: u64,
    pending_action: AtomicU64,
    idle_context: AtomicUsize,
    online: AtomicBool,
    scheduler_cursor: AtomicUsize,
    current_slot: AtomicUsize,
    current_generation: AtomicU64,
    tick_count: AtomicU64,
    switch_count: AtomicU64,
    scheduler_stack: CpuStack<{ crate::SCHEDULER_STACK_SIZE }>,
    idle_stack: CpuStack<{ crate::IDLE_STACK_SIZE }>,
}

impl CpuLocal {
    const fn new() -> Self {
        Self {
            self_ptr: 0,
            scheduler_stack_top: 0,
            idle_stack_top: 0,
            cpu_index: 0,
            pending_action: AtomicU64::new(0),
            idle_context: AtomicUsize::new(0),
            online: AtomicBool::new(false),
            scheduler_cursor: AtomicUsize::new(0),
            current_slot: AtomicUsize::new(usize::MAX),
            current_generation: AtomicU64::new(0),
            tick_count: AtomicU64::new(0),
            switch_count: AtomicU64::new(0),
            scheduler_stack: CpuStack([0; crate::SCHEDULER_STACK_SIZE]),
            idle_stack: CpuStack([0; crate::IDLE_STACK_SIZE]),
        }
    }

    fn initialize(&mut self, index: usize) {
        self.self_ptr = self as *mut CpuLocal as u64;
        self.cpu_index = index as u64;
        self.scheduler_stack_top = self.scheduler_stack.0.as_ptr_range().end as u64;
        self.idle_stack_top = self.idle_stack.0.as_ptr_range().end as u64;
    }
}

static mut CPU_LOCALS: [CpuLocal; MAX_CPUS] = [const { CpuLocal::new() }; MAX_CPUS];

pub fn boot() -> Status {
    debug_line(b"LogOS vNext: UEFI entered");
    let cpu_count = discover_cpus();
    measure_tsc();
    stage_trampoline();
    install_cpu(0);
    let _memory_map = unsafe { boot::exit_boot_services(None) };
    initialize_post_uefi(cpu_count);
    handoff_to_runtime();
    debug_line(b"LogOS vNext: core ready");
    enter_scheduler(0)
}

fn handoff_to_runtime() {
    if SCHEDULER.spawn(crate::runtime_entry).is_err() {
        fatal(b"LogOS vNext: runtime handoff");
    }
}

fn discover_cpus() -> usize {
    let bsp = unsafe { core::arch::x86_64::__cpuid(1).ebx >> 24 };
    APIC_IDS[0].store(bsp, Ordering::Release);
    let Ok(handle) = boot::get_handle_for_protocol::<MpServices>() else {
        return 1;
    };
    let Ok(mp) = boot::open_protocol_exclusive::<MpServices>(handle) else {
        return 1;
    };
    let Ok(count) = mp.get_number_of_processors() else {
        return 1;
    };
    if count.total == 0 || count.enabled == 0 || count.enabled > count.total {
        fatal(b"LogOS vNext: CPU topology");
    }
    if count.enabled > MAX_CPUS {
        fatal(b"LogOS vNext: CPU capacity");
    }
    let mut enabled = 1;
    for index in 0..count.total {
        let info =
            mp.get_processor_info(index).unwrap_or_else(|_| fatal(b"LogOS vNext: CPU topology"));
        if info.is_enabled() && info.is_healthy() && !info.is_bsp() {
            if enabled == MAX_CPUS {
                fatal(b"LogOS vNext: CPU capacity");
            }
            if info.processor_id > u64::from(u8::MAX) {
                fatal(b"LogOS vNext: x2APIC unsupported");
            }
            let apic_id = info.processor_id as u32;
            for prior in APIC_IDS.iter().take(enabled) {
                if prior.load(Ordering::Acquire) == apic_id {
                    fatal(b"LogOS vNext: CPU topology");
                }
            }
            APIC_IDS[enabled].store(apic_id, Ordering::Release);
            enabled += 1;
        }
    }
    CPU_COUNT.store(enabled, Ordering::Release);
    enabled
}

fn measure_tsc() {
    let before = rdtsc();
    boot::stall(10_000);
    let elapsed = rdtsc().saturating_sub(before);
    TSC_PER_US.store((elapsed / 10_000).max(1), Ordering::Release);
}

fn wait_tsc_us(microseconds: u64) {
    let start = rdtsc();
    let ticks = TSC_PER_US.load(Ordering::Acquire).saturating_mul(microseconds);
    while rdtsc().wrapping_sub(start) < ticks {
        core::hint::spin_loop();
    }
}

fn stage_trampoline() {
    let allocation = boot::allocate_pages(
        boot::AllocateType::Address(0x0000_8000),
        boot::MemoryType::LOADER_DATA,
        1,
    )
    .unwrap_or_else(|_| fatal(b"LogOS vNext: trampoline allocation"));
    let page = allocation.as_ptr() as usize;
    if page != 0x8000 || current_cr3() > u32::MAX as usize {
        fatal(b"LogOS vNext: trampoline address");
    }
    let start = ap_trampoline_start as *const u8;
    let end = ap_trampoline_end as *const u8;
    let length = unsafe { end.offset_from(start) as usize };
    if length > 0x800 {
        fatal(b"LogOS vNext: trampoline size");
    }
    unsafe {
        core::ptr::copy_nonoverlapping(start, page as *mut u8, length);
        let cr3_patch = ap_cr3_patch as *const u8 as usize - start as usize;
        core::ptr::write_unaligned((page + cr3_patch - 4) as *mut u32, current_cr3() as u32);
        let mailbox = (page + 0x800) as *mut u8;
        core::ptr::write_bytes(mailbox, 0, 0x100);
        core::ptr::write_unaligned((page + 0x800) as *mut u64, current_cr3() as u64);
        core::ptr::write_unaligned((page + 0x830) as *mut u16, 31);
        core::ptr::write_unaligned((page + 0x832) as *mut u32, (page + 0x840) as u32);
        let gdt = [0, 0x00cf_9a00_0000_ffff, 0x00af_9a00_0000_ffff, 0x00cf_9200_0000_ffff];
        core::ptr::copy_nonoverlapping(gdt.as_ptr(), (page + 0x840) as *mut u64, gdt.len());
    }
    TRAMPOLINE_PAGE.store(page, Ordering::Release);
}

fn start_aps(cpu_count: usize) {
    if cpu_count <= 1 {
        return;
    }
    let page = TRAMPOLINE_PAGE.load(Ordering::Acquire);
    let vector = (page >> 12) as u8;
    for cpu in 1..cpu_count {
        unsafe { CPU_LOCALS[cpu].initialize(cpu) };
        let mailbox = page + 0x800;
        unsafe {
            core::ptr::write_unaligned(mailbox as *mut u64, current_cr3() as u64);
            core::ptr::write_unaligned((mailbox + 8) as *mut u64, ap_entry as *const () as u64);
            core::ptr::write_unaligned((mailbox + 16) as *mut u64, CPU_LOCALS[cpu].idle_stack_top);
            core::ptr::write_unaligned(
                (mailbox + 24) as *mut u64,
                &CPU_LOCALS[cpu] as *const CpuLocal as u64,
            );
            core::ptr::write_unaligned((mailbox + 32) as *mut u64, cpu as u64);
        }
        let apic_id = APIC_IDS[cpu].load(Ordering::Acquire);
        send_ipi(apic_id, 0x4500);
        wait_tsc_us(10_000);
        send_ipi(apic_id, 0x0500);
        wait_tsc_us(200);
        send_ipi(apic_id, 0x4600 | u32::from(vector));
        wait_tsc_us(200);
        send_ipi(apic_id, 0x4600 | u32::from(vector));
        let start = rdtsc();
        while !unsafe { CPU_LOCALS[cpu].online.load(Ordering::Acquire) } {
            if rdtsc().wrapping_sub(start) > TSC_PER_US.load(Ordering::Acquire) * 500_000 {
                fatal(b"LogOS vNext: AP startup");
            }
            core::hint::spin_loop();
        }
    }
}

fn send_ipi(apic_id: u32, command: u32) {
    unsafe {
        let start = rdtsc();
        while read_apic(APIC_ICR_LOW) & 0x1000 != 0 {
            if rdtsc().wrapping_sub(start) > TSC_PER_US.load(Ordering::Acquire) * 10_000 {
                fatal(b"LogOS vNext: APIC IPI");
            }
            core::hint::spin_loop();
        }
        write_apic(APIC_ICR_HIGH, apic_id << 24);
        write_apic(APIC_ICR_LOW, command);
    }
}

fn install_cpu(index: usize) {
    unsafe {
        CPU_LOCALS[index].initialize(index);
        write_gs(&CPU_LOCALS[index]);
    }
}

fn current_cr3() -> usize {
    let value: usize;
    unsafe { asm!("mov {}, cr3", out(reg) value, options(nomem, nostack, preserves_flags)) };
    value
}

#[allow(clippy::needless_range_loop)]
fn initialize_post_uefi(cpu_count: usize) {
    install_gdt(0);
    install_idt(0);
    configure_sse();
    enable_local_apic();
    calibrate_timer();
    start_aps(cpu_count);
    configure_timer();
    unsafe { CPU_LOCALS[0].online.store(true, Ordering::Release) };
    SCHEDULER.online_cpu(0);
    let mut ready = 0;
    while ready < cpu_count {
        ready = 0;
        for cpu in 0..cpu_count {
            if unsafe { CPU_LOCALS[cpu].online.load(Ordering::Acquire) } {
                ready += 1;
            }
        }
        if ready != cpu_count {
            wait_tsc_us(100);
        }
    }
    #[cfg(feature = "qemu-proof")]
    crate::proof::initialize(cpu_count);
}

fn configure_timer() {
    unsafe {
        write_apic(APIC_LVT_TIMER, u32::from(TIMER_VECTOR) | 0x20000);
        write_apic(APIC_TIMER_DIVIDE, 0x3);
        write_apic(APIC_TIMER_INITIAL, APIC_TIMER_COUNT.load(Ordering::Acquire));
    }
}

fn calibrate_timer() {
    unsafe {
        // Count in one-shot mode while IF is still clear; masking the LVT
        // stops the timer on some virtual APIC implementations.
        asm!("cli");
        write_apic(APIC_LVT_TIMER, u32::from(TIMER_VECTOR));
        write_apic(APIC_TIMER_DIVIDE, 0x3);
        write_apic(APIC_TIMER_INITIAL, u32::MAX);
    }
    let before = u32::MAX;
    wait_tsc_us(1_000);
    let after = unsafe { read_apic(APIC_TIMER_CURRENT) };
    let elapsed = before.wrapping_sub(after);
    if elapsed == 0 {
        fatal(b"LogOS vNext: APIC timer calibration");
    }
    let period = (u64::from(elapsed).saturating_mul(10_000) / 1_000).clamp(1, u64::from(u32::MAX));
    APIC_TIMER_COUNT.store(period as u32, Ordering::Release);
}

fn enter_scheduler(_cpu: usize) -> ! {
    unsafe { asm!("mov rsp, gs:16", "sti", "2:", "hlt", "jmp 2b", options(noreturn)) }
}

pub fn fatal(message: &[u8]) -> ! {
    if !HALTED.swap(true, Ordering::AcqRel) {
        debug_line(b"LogOS vNext: FATAL");
        debug_line(message);
    }
    unsafe {
        asm!("cli");
        loop {
            asm!("hlt");
        }
    }
}

#[cfg_attr(not(feature = "qemu-proof"), allow(dead_code))]
pub(crate) fn proof_line(message: &[u8]) {
    debug_line(message);
}

fn debug_line(message: &[u8]) {
    for &byte in message {
        unsafe { asm!("out dx, al", in("dx") DEBUG_PORT, in("al") byte) };
    }
    unsafe { asm!("out dx, al", in("dx") DEBUG_PORT, in("al") b'\r') };
    unsafe { asm!("out dx, al", in("dx") DEBUG_PORT, in("al") b'\n') };
}

fn install_gdt(cpu: usize) {
    unsafe { CPU_GDTS[cpu].copy_from_slice(&GDT) };
    let pointer = GdtPointer {
        limit: (core::mem::size_of::<[u64; 3]>() - 1) as u16,
        base: unsafe { CPU_GDTS[cpu].as_ptr() as u64 },
    };
    unsafe {
        asm!("lgdt [{}]", in(reg) &pointer);
        asm!(
            "push 0x08",
            "lea rax, [rip + 2f]",
            "push rax",
            "retfq",
            "2:",
            "mov ax, 0x10",
            "mov ds, ax",
            "mov es, ax",
            "mov ss, ax",
        );
    }
}

fn install_idt(cpu: usize) {
    unsafe {
        let idt = &mut CPU_IDTS[cpu];
        idt.fill(IdtEntry::new(default_interrupt as *const () as usize, 0x08, 0x8e));
        idt[TIMER_VECTOR as usize] =
            IdtEntry::new(context_timer_interrupt as *const () as usize, 0x08, 0x8e);
        idt[SWITCH_VECTOR as usize] =
            IdtEntry::new(context_switch_interrupt as *const () as usize, 0x08, 0x8e);
    }
    load_idt(cpu);
}

fn load_idt(cpu: usize) {
    let pointer = IdtPointer {
        limit: (core::mem::size_of::<[IdtEntry; IDT_ENTRIES]>() - 1) as u16,
        base: unsafe { CPU_IDTS[cpu].as_ptr() as u64 },
    };
    unsafe { asm!("lidt [{}]", in(reg) &pointer) };
}

fn configure_sse() {
    unsafe {
        let mut cr0: u64;
        let mut cr4: u64;
        asm!("mov {}, cr0", out(reg) cr0);
        asm!("mov {}, cr4", out(reg) cr4);
        cr0 = (cr0 & !(1 << 2)) | (1 << 1) | (1 << 5);
        cr4 |= (1 << 9) | (1 << 10);
        asm!("mov cr0, {}", in(reg) cr0);
        asm!("mov cr4, {}", in(reg) cr4);
        asm!("fninit");
    }
}

fn enable_local_apic() {
    let apic_msr = unsafe { rdmsr(APIC_BASE_MSR) };
    if apic_msr & (1 << 10) != 0 {
        fatal(b"LogOS vNext: x2APIC unsupported");
    }
    let base = apic_msr as usize & !0xfff;
    if base == 0 {
        fatal(b"LogOS vNext: no local APIC");
    }
    unsafe {
        let value = apic_msr | (1 << 11);
        wrmsr(APIC_BASE_MSR, value);
        APIC.store(base, Ordering::Release);
        write_apic(APIC_SVR, read_apic(APIC_SVR) | 0x100 | 0xff);
    }
}

fn write_gs(local: &CpuLocal) {
    unsafe { wrmsr(0xc000_0101, local as *const CpuLocal as u64) };
}

#[unsafe(no_mangle)]
#[allow(clippy::needless_range_loop)]
extern "C" fn ap_entry(_cpu: usize) -> ! {
    let apic_id = unsafe { read_apic(APIC_ID) >> 24 };
    let mut cpu = usize::MAX;
    for index in 1..CPU_COUNT.load(Ordering::Acquire) {
        if APIC_IDS[index].load(Ordering::Acquire) == apic_id {
            cpu = index;
            break;
        }
    }
    if cpu == 0 || cpu >= CPU_COUNT.load(Ordering::Acquire) {
        fatal(b"LogOS vNext: AP index");
    }
    install_cpu(cpu);
    install_gdt(cpu);
    install_idt(cpu);
    configure_sse();
    enable_local_apic();
    calibrate_timer();
    configure_timer();
    SCHEDULER.online_cpu(cpu);
    unsafe { CPU_LOCALS[cpu].online.store(true, Ordering::Release) };
    enter_scheduler(cpu)
}

fn rdtsc() -> u64 {
    let low: u32;
    let high: u32;
    unsafe {
        asm!("rdtsc", out("eax") low, out("edx") high, options(nomem, nostack, preserves_flags))
    };
    (u64::from(high) << 32) | u64::from(low)
}

unsafe fn write_apic(offset: usize, value: u32) {
    let base = APIC.load(Ordering::Acquire);
    unsafe { core::ptr::write_volatile((base + offset) as *mut u32, value) };
}

unsafe fn read_apic(offset: usize) -> u32 {
    let base = APIC.load(Ordering::Acquire);
    unsafe { core::ptr::read_volatile((base + offset) as *const u32) }
}

unsafe fn wrmsr(msr: u32, value: u64) {
    unsafe {
        asm!("wrmsr", in("ecx") msr, in("eax") value as u32, in("edx") (value >> 32) as u32);
    }
}

unsafe fn rdmsr(msr: u32) -> u64 {
    let low: u32;
    let high: u32;
    unsafe { asm!("rdmsr", in("ecx") msr, out("eax") low, out("edx") high) };
    (u64::from(high) << 32) | u64::from(low)
}

#[unsafe(no_mangle)]
extern "C" fn schedule_from_interrupt(fx_context: usize, cpu: usize, vector: usize) -> usize {
    if HALTED.load(Ordering::Acquire) {
        fatal(b"LogOS vNext: halted");
    }
    if vector == usize::from(TIMER_VECTOR) {
        if cpu == 0 {
            let now = TIMER_TICKS.fetch_add(1, Ordering::AcqRel) + 1;
            SCHEDULER.wake_due(now);
        }
        SCHEDULER.record_tick(cpu);
        local_tick(cpu);
        #[cfg(feature = "qemu-proof")]
        crate::proof::observe(cpu);
    }
    let local = unsafe { &*core::ptr::addr_of_mut!(CPU_LOCALS).cast::<CpuLocal>().add(cpu) };
    unsafe { write_apic(APIC_EOI, 0) };
    if let Some(current) = SCHEDULER.current_task(cpu) {
        if !SCHEDULER.save_context(current, fx_context) {
            fatal(b"LogOS vNext: context save");
        }
        let outcome = if vector == usize::from(TIMER_VECTOR) {
            crate::FinishState::Runnable
        } else {
            match local.pending_action.swap(0, Ordering::AcqRel) {
                ACTION_BLOCK => crate::FinishState::Blocked,
                ACTION_COMPLETE => crate::FinishState::Completed,
                ACTION_TIMED_BLOCK => crate::FinishState::TimedBlocked,
                _ => crate::FinishState::Runnable,
            }
        };
        if !SCHEDULER.finish(current, outcome) {
            fatal(b"LogOS vNext: context publish");
        }
    } else {
        local.idle_context.store(fx_context, Ordering::Release);
    }
    SCHEDULER.clear_current(cpu);
    unsafe {
        let local = &*core::ptr::addr_of!(CPU_LOCALS).cast::<CpuLocal>().add(cpu);
        local.current_slot.store(usize::MAX, Ordering::Release);
        local.current_generation.store(0, Ordering::Release);
    }
    let Some(next) = SCHEDULER.claim_next(cpu) else {
        return local.idle_context.load(Ordering::Acquire);
    };
    local.current_slot.store(next.slot(), Ordering::Release);
    local.current_generation.store(next.generation(), Ordering::Release);
    local.scheduler_cursor.store(SCHEDULER.cursor(cpu).unwrap_or(0), Ordering::Release);
    local.switch_count.store(SCHEDULER.switches(cpu).unwrap_or(0), Ordering::Release);
    if SCHEDULER.saved_context(next).is_none() {
        initialize_task_context(next);
    }
    SCHEDULER.saved_context(next).unwrap_or_else(|| fatal(b"LogOS vNext: no context"))
}

fn local_tick(cpu: usize) {
    let ticks = SCHEDULER.ticks(cpu).unwrap_or(0);
    unsafe {
        (*core::ptr::addr_of_mut!(CPU_LOCALS).cast::<CpuLocal>().add(cpu))
            .tick_count
            .store(ticks, Ordering::Release)
    };
}

fn initialize_task_context(handle: crate::TaskHandle) {
    let Some(top) = SCHEDULER.task_stack_top(handle) else {
        fatal(b"LogOS vNext: task stack");
    };
    let fx = (top.saturating_sub(FX_STATE_SIZE + 8)) & !15;
    let gpr = fx.saturating_sub(GPR_WORDS * 8 + 8 + 24);
    unsafe {
        core::ptr::write_bytes(gpr as *mut u8, 0, GPR_WORDS * 8 + 8 + 24);
        core::ptr::write_bytes(fx as *mut u8, 0, FX_STATE_SIZE + 8);
        core::ptr::write_unaligned((fx as *mut u8).add(0) as *mut u16, 0x037f);
        core::ptr::write_unaligned((fx as *mut u8).add(24) as *mut u32, 0x1f80);
        core::ptr::write_unaligned((fx + FX_CONTEXT_POINTER) as *mut usize, gpr);
        core::ptr::write_unaligned((gpr + VECTOR_OFFSET) as *mut usize, SWITCH_VECTOR as usize);
        core::ptr::write_unaligned(
            (gpr + VECTOR_OFFSET + 8) as *mut usize,
            task_bootstrap as *const () as usize,
        );
        core::ptr::write_unaligned((gpr + VECTOR_OFFSET + 16) as *mut usize, 0x08);
        core::ptr::write_unaligned((gpr + VECTOR_OFFSET + 24) as *mut usize, 0x202);
        // The save area is in push order (r15 first, rax last on restore).
        core::ptr::write_unaligned(gpr as *mut usize, top);
    }
    if !SCHEDULER.set_initial_context(handle, fx) {
        fatal(b"LogOS vNext: initial context");
    }
}

#[unsafe(no_mangle)]
extern "C" fn task_trampoline() -> ! {
    let cpu = unsafe { (*(read_gs() as *const CpuLocal)).cpu_index as usize };
    if let Some(handle) = SCHEDULER.current_task(cpu) {
        if let Some(entry) = SCHEDULER.entry(handle) {
            entry();
        }
    }
    request_switch(ACTION_COMPLETE);
    loop {
        core::hint::spin_loop();
    }
}

pub fn yield_current() {
    request_switch(ACTION_YIELD)
}

pub fn block_current() {
    request_switch(ACTION_BLOCK)
}

pub fn sleep_current_for(ticks: u64) {
    let cpu = current_cpu();
    let Some(handle) = SCHEDULER.current_task(cpu) else {
        fatal(b"LogOS vNext: sleep without task");
    };
    let now = TIMER_TICKS.load(Ordering::Acquire);
    let deadline = now.saturating_add(ticks.max(1)).min(u64::MAX - 1);
    unsafe {
        let local = &*(read_gs() as *const CpuLocal);
        asm!("cli");
        if !SCHEDULER.arm_deadline(handle, deadline) {
            fatal(b"LogOS vNext: sleep deadline");
        }
        local.pending_action.store(ACTION_TIMED_BLOCK, Ordering::Release);
        asm!("sti; int 49");
    }
}

fn request_switch(action: u64) {
    unsafe {
        let local = &*(read_gs() as *const CpuLocal);
        // Keep the action publication and software interrupt adjacent. STI's
        // one-instruction interrupt shadow prevents a timer from observing a
        // half-published voluntary transition, while the pushed flags retain
        // interrupts enabled for the resumed task.
        asm!("cli");
        local.pending_action.store(action, Ordering::Release);
        asm!("sti; int 49");
    }
}

fn read_gs() -> usize {
    let value: usize;
    unsafe { asm!("mov {}, gs:0", out(reg) value, options(nomem, nostack, preserves_flags)) };
    value
}

#[cfg_attr(not(feature = "qemu-proof"), allow(dead_code))]
pub(crate) fn current_cpu() -> usize {
    unsafe { (*(read_gs() as *const CpuLocal)).cpu_index as usize }
}

pub(crate) fn current_ticks() -> u64 {
    TIMER_TICKS.load(Ordering::Acquire)
}

unsafe extern "C" {
    fn default_interrupt();
    fn context_timer_interrupt();
    fn context_switch_interrupt();
    fn task_bootstrap();
    fn ap_trampoline_start();
    fn ap_trampoline_end();
    fn ap_cr3_patch();
}

global_asm!(
    ".section .text.ap_trampoline,\"ax\"",
    ".code16",
    ".global ap_trampoline_start",
    "ap_trampoline_start:",
    "cli",
    "xor ax, ax",
    "mov ds, ax",
    "mov esp, 0x7000",
    "mov ebp, 0x8000",
    "mov eax, 0",
    ".global ap_cr3_patch",
    "ap_cr3_patch:",
    "mov cr3, eax",
    "lgdt [ebp + 0x830]",
    "mov eax, cr4",
    "or eax, 0x20",
    "mov cr4, eax",
    "mov ecx, 0xc0000080",
    "rdmsr",
    "or eax, 0x100",
    "wrmsr",
    "mov eax, cr0",
    "or eax, 0x80000001",
    "mov cr0, eax",
    ".byte 0x66, 0xea",
    ".long 0x8000 + (ap_trampoline_protected - ap_trampoline_start)",
    ".word 0x08",
    ".code32",
    "ap_trampoline_protected:",
    "mov ax, 0x18",
    "mov ds, ax",
    "mov es, ax",
    "mov ss, ax",
    ".byte 0xea",
    ".long 0x8000 + (ap_trampoline_long - ap_trampoline_start)",
    ".word 0x10",
    ".code64",
    "ap_trampoline_long:",
    "mov rsp, [rbp + 0x810]",
    "mov rax, [rbp + 0x818]",
    "mov rdx, rax",
    "shr rdx, 32",
    "mov ecx, 0xc0000101",
    "wrmsr",
    "mov edi, [rbp + 0x820]",
    "mov rax, [rbp + 0x808]",
    "jmp rax",
    ".global ap_trampoline_end",
    "ap_trampoline_end:",
    ".section .text",
);

global_asm!(
    ".global task_bootstrap",
    "task_bootstrap:",
    "mov rsp, r15",
    "and rsp, -16",
    "sub rsp, 40",
    "call task_trampoline",
    "1:",
    "hlt",
    "jmp 1b",
);

global_asm!(
    ".global default_interrupt",
    "default_interrupt:",
    "cli",
    "mov dx, 0xe9",
    "mov al, 'F'",
    "out dx, al",
    "mov al, 'A'",
    "out dx, al",
    "mov al, 'U'",
    "out dx, al",
    "mov al, 'L'",
    "out dx, al",
    "mov al, 'T'",
    "out dx, al",
    "mov al, 13",
    "out dx, al",
    "mov al, 10",
    "out dx, al",
    "1:",
    "hlt",
    "jmp 1b",
    ".global context_timer_interrupt",
    "context_timer_interrupt:",
    "push 32",
    "jmp context_common",
    ".global context_switch_interrupt",
    "context_switch_interrupt:",
    "push 49",
    "context_common:",
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
    "mov r11, rsp",
    "sub rsp, 520",
    "and rsp, -16",
    "fxsave64 [rsp]",
    "mov [rsp + 512], r11",
    "mov r10, rsp",
    "mov rsp, gs:8",
    "sub rsp, 40",
    "mov rcx, r10",
    "mov rdx, gs:24",
    "mov r8, [r11 + 120]",
    "call schedule_from_interrupt",
    "add rsp, 40",
    "mov rsp, rax",
    "fxrstor64 [rsp]",
    "mov rsp, [rsp + 512]",
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
    "add rsp, 8",
    "iretq",
);

#[cfg(feature = "qemu-proof")]
global_asm!(
    ".global proof_task_a",
    "proof_task_a:",
    "mov r12, 0x1111222233334444",
    "mov r13, 0x5555666677778888",
    "mov r14, 0x9999aaaabbbbcccc",
    "mov r15, 0xddddeeeeffff0000",
    "mov rax, 0x0123456789abcdef",
    "movq xmm6, rax",
    "1:",
    "stc",
    "mov ecx, 0x1000",
    "2:",
    "pause",
    "loop 2b",
    "pushfq",
    "pop rax",
    "test al, 1",
    "jz 9f",
    "mov rax, 0x1111222233334444",
    "cmp r12, rax",
    "jne 9f",
    "mov rax, 0x5555666677778888",
    "cmp r13, rax",
    "jne 9f",
    "mov rax, 0x9999aaaabbbbcccc",
    "cmp r14, rax",
    "jne 9f",
    "mov rax, 0xddddeeeeffff0000",
    "cmp r15, rax",
    "jne 9f",
    "movq rax, xmm6",
    "mov r11, 0x0123456789abcdef",
    "cmp rax, r11",
    "jne 9f",
    "call proof_a_progress",
    "jmp 1b",
    "9:",
    "call proof_fail",
    "hlt",
    "jmp 9b",
    ".global proof_task_b",
    "proof_task_b:",
    "mov r12, 0xaaaabbbbccccdddd",
    "mov r13, 0x1111222233334444",
    "mov r14, 0x5555666677778888",
    "mov r15, 0x9999aaaabbbbcccc",
    "mov rax, 0xfedcba9876543210",
    "movq xmm6, rax",
    "3:",
    "clc",
    "mov ecx, 0x1000",
    "4:",
    "pause",
    "loop 4b",
    "pushfq",
    "pop rax",
    "test al, 1",
    "jnz 10f",
    "mov rax, 0xaaaabbbbccccdddd",
    "cmp r12, rax",
    "jne 10f",
    "mov rax, 0x1111222233334444",
    "cmp r13, rax",
    "jne 10f",
    "mov rax, 0x5555666677778888",
    "cmp r14, rax",
    "jne 10f",
    "mov rax, 0x9999aaaabbbbcccc",
    "cmp r15, rax",
    "jne 10f",
    "movq rax, xmm6",
    "mov r11, 0xfedcba9876543210",
    "cmp rax, r11",
    "jne 10f",
    "call proof_b_progress",
    "jmp 3b",
    "10:",
    "call proof_fail",
    "hlt",
    "jmp 10b",
);
