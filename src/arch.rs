use core::{
    arch::asm,
    sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering},
};

use uefi::{
    boot,
    mem::memory_map::MemoryMap as UefiMemoryMap,
    prelude::*,
    proto::{
        console::gop::{GraphicsOutput, PixelFormat as UefiPixelFormat},
        pi::mp::MpServices,
    },
};

use crate::{
    MAX_CPUS, SCHEDULER,
    boot_resources::{BootResources, FramebufferInfo, MemoryDescriptor, MemoryMap, PixelFormat},
    service_loader::ServiceImageBundle,
};

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
const USER_ENTRY_STACK_SIZE: usize = 16 * 1024;
const KERNEL_CODE_SELECTOR: u16 = 0x08;
const KERNEL_DATA_SELECTOR: u16 = 0x10;
pub(crate) const USER_CODE_SELECTOR: u16 = 0x1b;
pub(crate) const USER_DATA_SELECTOR: u16 = 0x23;
const TSS_SELECTOR: u16 = 0x28;

// Core tasks normally run in ring 0. A bounded proof task may enter ring 3;
// its active scheduler stack is also its TSS ring-transition stack so the
// saved interrupt frame remains private to that task.

static APIC: AtomicUsize = AtomicUsize::new(0);
static TSC_PER_US: AtomicU64 = AtomicU64::new(1);
static HALTED: AtomicBool = AtomicBool::new(false);
static TIMER_TICKS: AtomicU64 = AtomicU64::new(0);
static CPU_COUNT: AtomicUsize = AtomicUsize::new(1);
static APIC_TIMER_COUNT: AtomicU32 = AtomicU32::new(10_000_000);
static APIC_IDS: [AtomicU32; MAX_CPUS] = [const { AtomicU32::new(0) }; MAX_CPUS];
static TRAMPOLINE_PAGE: AtomicUsize = AtomicUsize::new(0);
static mut BOOT_RESOURCES: Option<BootResources> = None;
static mut SERVICE_IMAGES: Option<ServiceImageBundle> = None;
#[cfg(target_os = "uefi")]
static mut SERVICE_RUNTIME: crate::service_runtime::ServiceRuntime =
    crate::service_runtime::ServiceRuntime::new();
static KERNEL_CR3: AtomicUsize = AtomicUsize::new(0);

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

static GDT: [u64; 5] =
    [0, 0x00af_9a00_0000_ffff, 0x00cf_9200_0000_ffff, 0x00af_fa00_0000_ffff, 0x00cf_f200_0000_ffff];
static mut CPU_GDTS: [[u64; 7]; MAX_CPUS] = [[0; 7]; MAX_CPUS];
static mut CPU_IDTS: [[IdtEntry; IDT_ENTRIES]; MAX_CPUS] =
    [[IdtEntry::MISSING; IDT_ENTRIES]; MAX_CPUS];

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct TaskStateSegment {
    reserved: u32,
    rsp: [u64; 3],
    reserved_2: u64,
    ist: [u64; 7],
    reserved_3: u64,
    reserved_4: u16,
    iomap_base: u16,
}

impl TaskStateSegment {
    const fn new() -> Self {
        Self {
            reserved: 0,
            rsp: [0; 3],
            reserved_2: 0,
            ist: [0; 7],
            reserved_3: 0,
            reserved_4: 0,
            iomap_base: core::mem::size_of::<Self>() as u16,
        }
    }
}

static mut CPU_TSS: [TaskStateSegment; MAX_CPUS] = [const { TaskStateSegment::new() }; MAX_CPUS];

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
    user_entry_stack_top: u64,
    scheduler_stack: CpuStack<{ crate::SCHEDULER_STACK_SIZE }>,
    idle_stack: CpuStack<{ crate::IDLE_STACK_SIZE }>,
    user_entry_stack: CpuStack<USER_ENTRY_STACK_SIZE>,
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
            user_entry_stack_top: 0,
            scheduler_stack: CpuStack([0; crate::SCHEDULER_STACK_SIZE]),
            idle_stack: CpuStack([0; crate::IDLE_STACK_SIZE]),
            user_entry_stack: CpuStack([0; USER_ENTRY_STACK_SIZE]),
        }
    }

    fn initialize(&mut self, index: usize) {
        self.self_ptr = self as *mut CpuLocal as u64;
        self.cpu_index = index as u64;
        self.scheduler_stack_top = self.scheduler_stack.0.as_ptr_range().end as u64;
        self.idle_stack_top = self.idle_stack.0.as_ptr_range().end as u64;
        self.user_entry_stack_top = self.user_entry_stack.0.as_ptr_range().end as u64;
    }
}

static mut CPU_LOCALS: [CpuLocal; MAX_CPUS] = [const { CpuLocal::new() }; MAX_CPUS];

pub fn boot() -> Status {
    debug_line(b"LogOS vNext: UEFI entered");
    let cpu_count = discover_cpus();
    measure_tsc();
    stage_trampoline();
    install_cpu(0);
    let framebuffer = capture_gop();
    let service_images = match crate::service_loader::load_from_esp() {
        Ok(images) => images,
        Err(crate::service_loader::UefiImageError::Firmware(_)) => {
            fatal(b"LogOS vNext: service filesystem")
        }
        Err(crate::service_loader::UefiImageError::Path) => fatal(b"LogOS vNext: service path"),
        Err(crate::service_loader::UefiImageError::NotRegularFile) => {
            fatal(b"LogOS vNext: service file type")
        }
        Err(crate::service_loader::UefiImageError::Service(error)) => match error {
            crate::service_loader::ServiceLoadError::Empty => fatal(b"LogOS vNext: service empty"),
            crate::service_loader::ServiceLoadError::TooLarge => {
                fatal(b"LogOS vNext: service size")
            }
            crate::service_loader::ServiceLoadError::InvalidElf(_) => {
                fatal(b"LogOS vNext: service ELF")
            }
            crate::service_loader::ServiceLoadError::InvalidAddress
            | crate::service_loader::ServiceLoadError::Duplicate => {
                fatal(b"LogOS vNext: service record")
            }
        },
    };
    proof_line(b"LogOS vNext: service images ready");
    let memory_map = unsafe { boot::exit_boot_services(None) };
    publish_boot_resources(memory_map, framebuffer);
    unsafe {
        SERVICE_IMAGES = Some(service_images);
        let images = (*core::ptr::addr_of!(SERVICE_IMAGES))
            .as_ref()
            .unwrap_or_else(|| fatal(b"LogOS vNext: service image state"));
        (*core::ptr::addr_of_mut!(SERVICE_RUNTIME)).start(images).unwrap_or_else(
            |error| match error {
                crate::service_runtime::ServiceRuntimeError::Resources => {
                    fatal(b"LogOS vNext: service resources")
                }
                crate::service_runtime::ServiceRuntimeError::Image => {
                    fatal(b"LogOS vNext: service image state")
                }
                crate::service_runtime::ServiceRuntimeError::Load(_) => {
                    fatal(b"LogOS vNext: service image pages")
                }
                crate::service_runtime::ServiceRuntimeError::Populate(_) => {
                    fatal(b"LogOS vNext: service page population")
                }
                crate::service_runtime::ServiceRuntimeError::PageTableRoot(
                    crate::page_table::PageTableError::Capacity,
                ) => fatal(b"LogOS vNext: service table capacity"),
                crate::service_runtime::ServiceRuntimeError::PageTableRoot(
                    crate::page_table::PageTableError::Exhausted,
                ) => fatal(b"LogOS vNext: service table frames"),
                crate::service_runtime::ServiceRuntimeError::PageTableRoot(
                    crate::page_table::PageTableError::Memory,
                ) => fatal(b"LogOS vNext: service table memory"),
                crate::service_runtime::ServiceRuntimeError::PageTableRoot(
                    crate::page_table::PageTableError::InvalidMapping,
                ) => fatal(b"LogOS vNext: service table mapping"),
                crate::service_runtime::ServiceRuntimeError::PageTableRoot(
                    crate::page_table::PageTableError::InvalidVirtualAddress,
                ) => fatal(b"LogOS vNext: service table VA"),
                crate::service_runtime::ServiceRuntimeError::PageTableRoot(
                    crate::page_table::PageTableError::InvalidFrame,
                ) => fatal(b"LogOS vNext: service table frame"),
                crate::service_runtime::ServiceRuntimeError::PageTableRoot(
                    crate::page_table::PageTableError::InvalidFlags,
                ) => fatal(b"LogOS vNext: service table flags"),
                crate::service_runtime::ServiceRuntimeError::PageTableRoot(
                    crate::page_table::PageTableError::Conflict,
                ) => fatal(b"LogOS vNext: service table conflict"),
                crate::service_runtime::ServiceRuntimeError::PageTableMap(_) => {
                    fatal(b"LogOS vNext: service table map")
                }
                crate::service_runtime::ServiceRuntimeError::Process(_) => {
                    fatal(b"LogOS vNext: service process")
                }
                crate::service_runtime::ServiceRuntimeError::Startup(_) => {
                    fatal(b"LogOS vNext: service startup")
                }
            },
        );
        let runtime = &*core::ptr::addr_of!(SERVICE_RUNTIME);
        for spec in crate::service_images::SERVICE_IMAGES {
            if runtime.image(spec.service()).is_none()
                || runtime.root(spec.service()).is_none()
                || runtime.launch(spec.service()).is_none()
            {
                fatal(b"LogOS vNext: service root state");
            }
        }
        if !runtime.all_launch_ready() {
            fatal(b"LogOS vNext: service launch barrier");
        }
    }
    proof_line(b"LogOS vNext: service address spaces ready");
    initialize_post_uefi(cpu_count);
    handoff_to_runtime();
    debug_line(b"LogOS vNext: core ready");
    enter_scheduler(0)
}

fn capture_gop() -> FramebufferInfo {
    let handle = boot::get_handle_for_protocol::<GraphicsOutput>()
        .unwrap_or_else(|_| fatal(b"LogOS vNext: GOP unavailable"));
    let mut gop = boot::open_protocol_exclusive::<GraphicsOutput>(handle)
        .unwrap_or_else(|_| fatal(b"LogOS vNext: GOP open"));
    let mut selected: Option<(usize, uefi::proto::console::gop::Mode, PixelFormat)> = None;
    for mode in gop.modes() {
        let info = mode.info();
        let (width, height) = info.resolution();
        let format = match info.pixel_format() {
            UefiPixelFormat::Rgb => PixelFormat::Rgb8,
            UefiPixelFormat::Bgr => PixelFormat::Bgr8,
            UefiPixelFormat::Bitmask | UefiPixelFormat::BltOnly => continue,
        };
        let Some(bytes) = (height as u64)
            .checked_mul(info.stride() as u64)
            .and_then(|pixels| pixels.checked_mul(4))
        else {
            continue;
        };
        if bytes > logos_abi::MAX_FRAMEBUFFER_BYTES as u64 {
            continue;
        }
        let candidate = (width.saturating_mul(height), mode, format);
        if selected.as_ref().is_none_or(|current| candidate.0 > current.0) {
            selected = Some(candidate);
        }
    }
    let Some((_, mode, format)) = selected else { fatal(b"LogOS vNext: GOP mode capacity") };
    gop.set_mode(&mode).unwrap_or_else(|_| fatal(b"LogOS vNext: GOP mode"));
    let info = gop.current_mode_info();
    let (width, height) = info.resolution();
    let mut framebuffer = gop.frame_buffer();
    FramebufferInfo::new(
        framebuffer.as_mut_ptr() as u64,
        framebuffer.size() as u64,
        width as u32,
        height as u32,
        info.stride() as u32,
        format,
    )
    .unwrap_or_else(|| fatal(b"LogOS vNext: GOP metadata"))
}

fn publish_boot_resources(memory_map: impl UefiMemoryMap, framebuffer: FramebufferInfo) {
    let mut copied = MemoryMap::new();
    for descriptor in memory_map.entries() {
        let entry = MemoryDescriptor::new(
            descriptor.phys_start,
            descriptor.page_count,
            descriptor.ty == uefi::mem::memory_map::MemoryType::CONVENTIONAL,
        )
        .unwrap_or_else(|| fatal(b"LogOS vNext: memory map"));
        copied.push(entry).unwrap_or_else(|_| fatal(b"LogOS vNext: memory map capacity"));
    }
    let mut resources = BootResources::new(copied, crate::boot_resources::KeyboardResource::PS2);
    resources.publish_framebuffer(framebuffer);
    unsafe {
        BOOT_RESOURCES = Some(resources);
    }
    proof_line(b"LogOS vNext: boot resources ready");
}

#[allow(dead_code)]
pub(crate) fn boot_resources() -> Option<BootResources> {
    unsafe { BOOT_RESOURCES }
}

#[allow(dead_code)]
pub(crate) fn service_images() -> Option<&'static ServiceImageBundle> {
    unsafe { (*core::ptr::addr_of!(SERVICE_IMAGES)).as_ref() }
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

pub(crate) fn current_cr3() -> usize {
    let value: usize;
    unsafe { asm!("mov {}, cr3", out(reg) value, options(nomem, nostack, preserves_flags)) };
    value
}

#[allow(dead_code)]
pub(crate) fn switch_cr3(root: usize) {
    if root == 0 || root & 0xfff != 0 {
        fatal(b"LogOS vNext: invalid CR3");
    }
    unsafe {
        asm!("mov cr3, {root}", root = in(reg) root, options(nostack, preserves_flags));
    }
}

pub(crate) fn prepare_task_address_space(root: usize) {
    let root = if root == 0 { KERNEL_CR3.load(Ordering::Acquire) } else { root };
    switch_cr3(root);
}

#[allow(dead_code)]
pub(crate) fn enter_user_launch(launch: crate::process::UserLaunch) -> ! {
    unsafe {
        asm!(
            "push {user_data}",
            "push {user_stack}",
            "pushfq",
            "push {user_code}",
            "push {user_entry}",
            "iretq",
            user_data = const USER_DATA_SELECTOR,
            user_stack = in(reg) launch.stack_top() - 8,
            user_code = const USER_CODE_SELECTOR,
            user_entry = in(reg) launch.entry(),
            options(noreturn),
        );
    }
}

#[allow(clippy::needless_range_loop)]
fn initialize_post_uefi(cpu_count: usize) {
    KERNEL_CR3.store(current_cr3(), Ordering::Release);
    install_gdt(0);
    install_idt(0);
    configure_sse();
    enable_local_apic();
    calibrate_timer();
    #[cfg(feature = "qemu-proof")]
    crate::user_mode::initialize_kernel_cr3(current_cr3());
    #[cfg(feature = "qemu-proof")]
    crate::proof::terminal_integration();
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
    #[cfg(feature = "qemu-proof")]
    crate::user_mode::spawn_proof();
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
    unsafe {
        CPU_GDTS[cpu][..GDT.len()].copy_from_slice(&GDT);
        write_tss_rsp0(cpu, CPU_LOCALS[cpu].user_entry_stack_top);
        let base = core::ptr::addr_of!(CPU_TSS[cpu]) as u64;
        let limit = (core::mem::size_of::<TaskStateSegment>() - 1) as u64;
        let low = (limit & 0xffff)
            | ((base & 0x00ff_ffff) << 16)
            | (0x89 << 40)
            | ((limit & 0xf0000) << 32)
            | ((base & 0xff00_0000) << 32);
        CPU_GDTS[cpu][5] = low;
        CPU_GDTS[cpu][6] = base >> 32;
    }
    let pointer = GdtPointer {
        limit: (core::mem::size_of::<[u64; 7]>() - 1) as u16,
        base: unsafe { CPU_GDTS[cpu].as_ptr() as u64 },
    };
    unsafe {
        asm!("lgdt [{}]", in(reg) &pointer);
        asm!(
            "push {kernel_code}",
            "lea rax, [rip + 2f]",
            "push rax",
            "retfq",
            "2:",
            "mov ax, {kernel_data}",
            "mov ds, ax",
            "mov es, ax",
            "mov ss, ax",
            kernel_code = const KERNEL_CODE_SELECTOR,
            kernel_data = const KERNEL_DATA_SELECTOR,
        );
        asm!("mov ax, {tss}", "ltr ax", tss = const TSS_SELECTOR);
    }
}

pub(crate) fn set_task_kernel_stack(cpu: usize, stack_top: usize) {
    unsafe {
        write_tss_rsp0(cpu, stack_top as u64);
    }
}

unsafe fn write_tss_rsp0(cpu: usize, stack_top: u64) {
    unsafe {
        let tss = core::ptr::addr_of_mut!(CPU_TSS[cpu]).cast::<u8>();
        core::ptr::write_unaligned(tss.add(4).cast::<u64>(), stack_top);
    }
}

fn install_idt(cpu: usize) {
    unsafe {
        let idt = &mut CPU_IDTS[cpu];
        idt.fill(IdtEntry::new(
            default_interrupt as *const () as usize,
            KERNEL_CODE_SELECTOR,
            0x8e,
        ));
        idt[TIMER_VECTOR as usize] = IdtEntry::new(
            context_timer_interrupt as *const () as usize,
            KERNEL_CODE_SELECTOR,
            0x8e,
        );
        idt[SWITCH_VECTOR as usize] = IdtEntry::new(
            context_switch_interrupt as *const () as usize,
            KERNEL_CODE_SELECTOR,
            0xee,
        );
        idt[6] =
            IdtEntry::new(user_fault_no_error as *const () as usize, KERNEL_CODE_SELECTOR, 0x8e);
        idt[13] =
            IdtEntry::new(user_gp_fault_error as *const () as usize, KERNEL_CODE_SELECTOR, 0x8e);
        idt[14] =
            IdtEntry::new(user_pf_fault_error as *const () as usize, KERNEL_CODE_SELECTOR, 0x8e);
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

unsafe extern "C" {
    fn default_interrupt();
    fn context_timer_interrupt();
    fn context_switch_interrupt();
    fn user_fault_no_error();
    fn user_gp_fault_error();
    fn user_pf_fault_error();
    fn task_bootstrap();
    fn ap_trampoline_start();
    fn ap_trampoline_end();
    fn ap_cr3_patch();
}

mod context;

pub fn yield_current() {
    context::yield_current();
}

pub fn block_current() {
    context::block_current();
}

pub fn sleep_current_for(ticks: u64) {
    context::sleep_current_for(ticks);
}

#[cfg_attr(not(feature = "qemu-proof"), allow(dead_code))]
pub(crate) fn current_cpu() -> usize {
    context::current_cpu()
}

pub(crate) fn current_ticks() -> u64 {
    context::current_ticks()
}
