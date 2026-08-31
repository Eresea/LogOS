use core::{
    arch::asm,
    sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering},
};

use uefi::{
    boot,
    mem::memory_map::MemoryMap as UefiMemoryMap,
    mem::memory_map::MemoryType,
    prelude::*,
    proto::{
        console::gop::{GraphicsOutput, PixelFormat as UefiPixelFormat},
        pi::mp::MpServices,
    },
};

use crate::{
    MAX_CPUS, SCHEDULER,
    boot_resources::{BootResources, FramebufferInfo, MemoryDescriptor, MemoryMap, PixelFormat},
    process::ProcessHandle,
    service_loader::ServiceImageBundle,
};

mod virtio_device;
mod virtio_net;

pub(crate) fn flush_storage_device() -> Result<(), logos_abi::StorageStatus> {
    virtio_device::flush_storage_device().map_err(storage_error_status)
}

pub(crate) fn prepare_storage_power_control() -> Result<(), logos_abi::StorageStatus> {
    virtio_device::prepare_power_control().map_err(storage_error_status)
}

pub(crate) fn storage_block_count() -> Result<u64, logos_abi::StorageStatus> {
    virtio_device::storage_block_count().map_err(storage_error_status)
}

pub(crate) fn device_list_response(
    request: logos_abi::DeviceRequest,
    generation: u16,
    service_epoch: u64,
) -> logos_abi::DeviceResponse {
    match storage_block_count() {
        Ok(block_count) => {
            let record = logos_abi::DeviceRecord::disk(0, block_count, b"disk0");
            match record {
                Some(record) => logos_abi::DeviceResponse::new(
                    request,
                    logos_abi::DeviceStatus::Ok,
                    generation,
                    service_epoch,
                )
                .with_record(record),
                None => logos_abi::DeviceResponse::new(
                    request,
                    logos_abi::DeviceStatus::Invalid,
                    generation,
                    service_epoch,
                ),
            }
        }
        Err(_) => logos_abi::DeviceResponse::new(
            request,
            logos_abi::DeviceStatus::Io,
            generation,
            service_epoch,
        ),
    }
}

pub(crate) fn transfer_storage_block(
    request: logos_abi::StorageRequest,
    data_address: usize,
) -> Result<(), logos_abi::StorageStatus> {
    virtio_device::transfer_storage_block(request, data_address).map_err(storage_error_status)
}

fn storage_error_status(error: virtio_device::DeviceError) -> logos_abi::StorageStatus {
    match error {
        virtio_device::DeviceError::Busy | virtio_device::DeviceError::QueueFull => {
            logos_abi::StorageStatus::Full
        }
        virtio_device::DeviceError::OutOfBounds => logos_abi::StorageStatus::OutOfBounds,
        virtio_device::DeviceError::ReadOnly => logos_abi::StorageStatus::ReadOnly,
        virtio_device::DeviceError::StaleCompletion => logos_abi::StorageStatus::Stale,
        virtio_device::DeviceError::Unsupported => logos_abi::StorageStatus::Unsupported,
        _ => logos_abi::StorageStatus::Io,
    }
}

pub(crate) fn handle_storage_interrupt() {
    virtio_device::handle_storage_interrupt();
}

pub(crate) fn reset_network_device() {
    virtio_net::reset();
}

pub(crate) fn handle_network_interrupt() {
    virtio_net::handle_interrupt();
    crate::runtime_events::signal_hardware_event(
        crate::runtime_events::HardwareEventSource::Network,
    );
}

pub(crate) fn submit_network_frame(source: usize, length: usize) -> bool {
    virtio_net::submit_frame(source, length)
}

pub(crate) fn take_network_frame(destination: usize) -> Option<usize> {
    virtio_net::take_frame(destination)
}

pub(crate) fn network_mac() -> Option<[u8; 6]> {
    virtio_net::mac()
}

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
const PIC_MASTER_COMMAND: u16 = 0x20;
const PIC_MASTER_DATA: u16 = 0x21;
const PIC_SLAVE_COMMAND: u16 = 0xa0;
const PIC_SLAVE_DATA: u16 = 0xa1;
const PIC_EOI: u8 = 0x20;
const KEYBOARD_STATUS_PORT: u16 = 0x64;
const KEYBOARD_COMMAND_PORT: u16 = 0x64;
const KEYBOARD_READ_CONFIG: u8 = 0x20;
const KEYBOARD_WRITE_CONFIG: u8 = 0x60;
const KEYBOARD_INPUT_FULL: u8 = 1 << 1;
const KEYBOARD_OUTPUT_FULL: u8 = 1;
const KEYBOARD_CLOCK_DISABLED: u8 = 1 << 4;
const POINTER_CLOCK_DISABLED: u8 = 1 << 5;
const KEYBOARD_TRANSLATION_ENABLED: u8 = 1 << 6;
const KEYBOARD_AUX_OUTPUT: u8 = 1 << 5;
const KEYBOARD_DATA_PORT: u16 = 0x60;
const KEYBOARD_SET_SCANCODE: u8 = 0xf0;
const KEYBOARD_SCANCODE_SET_2: u8 = 0x02;
const KEYBOARD_ACK: u8 = 0xfa;
const POINTER_COMMAND: u8 = 0xd4;
const POINTER_ENABLE_AUX: u8 = 0xa8;
const POINTER_ENABLE_STREAM: u8 = 0xf4;
const POINTER_ACK: u8 = 0xfa;
const ACPI_SHUTDOWN_PORT: u16 = 0x604;
const RESET_CONTROL_PORT: u16 = 0xcf9;
const TIMER_VECTOR: u8 = 32;
const KEYBOARD_VECTOR: u8 = 33;
const POINTER_VECTOR: u8 = 44;
const SWITCH_VECTOR: u8 = 49;
const RESCHEDULE_VECTOR: u8 = 50;
pub(crate) const STORAGE_VECTOR: u8 = 0x52;
pub(crate) const NETWORK_VECTOR: u8 = 0x53;
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
static SERVICE_RUNTIME_LOCK: AtomicBool = AtomicBool::new(false);
static SERVICE_RUNTIME_RESTARTING: AtomicBool = AtomicBool::new(false);
static SERVICE_RUNTIME_READY: AtomicBool = AtomicBool::new(false);
static KERNEL_CR3: AtomicUsize = AtomicUsize::new(0);
static KEYBOARD_RING: AtomicUsize = AtomicUsize::new(0);
static KEYBOARD_IRQ_ENABLED: AtomicBool = AtomicBool::new(false);
static KEYBOARD_IRQ_IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);
static POINTER_RING: AtomicUsize = AtomicUsize::new(0);
static POINTER_IRQ_AVAILABLE: AtomicBool = AtomicBool::new(false);
static POINTER_IRQ_ENABLED: AtomicBool = AtomicBool::new(false);
static POINTER_IRQ_IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);

pub(crate) struct ServiceRuntimeGuard {
    held: bool,
    interrupts_enabled: bool,
}

impl ServiceRuntimeGuard {
    fn acquire() -> Self {
        let interrupts_enabled = interrupts_enabled();
        disable_interrupts();
        acquire_service_runtime_lock();
        Self { held: true, interrupts_enabled }
    }

    pub(crate) fn pause(&mut self) {
        if self.held {
            SERVICE_RUNTIME_LOCK.store(false, Ordering::Release);
            self.held = false;
            if self.interrupts_enabled {
                enable_interrupts();
            }
        }
    }

    pub(crate) fn resume(&mut self) {
        if !self.held {
            self.interrupts_enabled = interrupts_enabled();
            disable_interrupts();
            acquire_service_runtime_lock();
            self.held = true;
        }
    }
}

impl Drop for ServiceRuntimeGuard {
    fn drop(&mut self) {
        if self.held {
            SERVICE_RUNTIME_LOCK.store(false, Ordering::Release);
            if self.interrupts_enabled {
                enable_interrupts();
            }
        }
    }
}

fn acquire_service_runtime_lock() {
    while SERVICE_RUNTIME_LOCK
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        core::hint::spin_loop();
    }
}

fn interrupts_enabled() -> bool {
    let flags: usize;
    unsafe {
        core::arch::asm!("pushfq", "pop {}", out(reg) flags, options(nomem, preserves_flags));
    }
    flags & (1 << 9) != 0
}

fn disable_interrupts() {
    unsafe { core::arch::asm!("cli", options(nomem, nostack, preserves_flags)) };
}

fn enable_interrupts() {
    unsafe { core::arch::asm!("sti", options(nomem, nostack, preserves_flags)) };
}

pub(crate) fn service_runtime_restarting() -> bool {
    SERVICE_RUNTIME_RESTARTING.load(Ordering::Acquire)
}

pub(crate) fn service_runtime_ready() -> bool {
    SERVICE_RUNTIME_READY.load(Ordering::Acquire)
}

pub(crate) fn begin_service_runtime_transition() {
    SERVICE_RUNTIME_READY.store(false, Ordering::Release);
}

pub(crate) fn finish_service_runtime_transition() {
    SERVICE_RUNTIME_READY.store(true, Ordering::Release);
}

pub(crate) fn begin_service_restart() {
    SERVICE_RUNTIME_RESTARTING.store(true, Ordering::Release);
}

pub(crate) fn end_service_restart() {
    SERVICE_RUNTIME_RESTARTING.store(false, Ordering::Release);
}

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

const SCHEDULER_STACK_GUARD: u8 = 0xa5;

impl<const N: usize> CpuStack<N> {
    const fn new() -> Self {
        Self([0; N])
    }

    const fn with_scheduler_guard() -> Self {
        let mut bytes = [0; N];
        let mut index = 0;
        while index < crate::scheduler::SCHEDULER_STACK_GUARD_BYTES {
            bytes[index] = SCHEDULER_STACK_GUARD;
            index += 1;
        }
        Self(bytes)
    }

    fn scheduler_guard_intact(&self) -> bool {
        self.0[..crate::scheduler::SCHEDULER_STACK_GUARD_BYTES]
            .iter()
            .all(|byte| *byte == SCHEDULER_STACK_GUARD)
    }
}

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
            scheduler_stack: CpuStack::with_scheduler_guard(),
            idle_stack: CpuStack::new(),
            user_entry_stack: CpuStack::new(),
        }
    }

    fn initialize(&mut self, index: usize) {
        self.self_ptr = self as *mut CpuLocal as u64;
        self.cpu_index = index as u64;
        self.scheduler_stack_top = self.scheduler_stack.0.as_ptr_range().end as u64;
        self.idle_stack_top = self.idle_stack.0.as_ptr_range().end as u64;
        self.user_entry_stack_top = self.user_entry_stack.0.as_ptr_range().end as u64;
    }

    fn scheduler_stack_guard_intact(&self) -> bool {
        self.scheduler_stack.scheduler_guard_intact()
    }
}

static mut CPU_LOCALS: [CpuLocal; MAX_CPUS] = [const { CpuLocal::new() }; MAX_CPUS];

pub fn boot() -> Status {
    let stack_top = unsafe { CPU_LOCALS[0].scheduler_stack.0.as_ptr_range().end as u64 };
    unsafe {
        asm!(
            "mov rsp, {stack}",
            "and rsp, -16",
            "jmp {entry}",
            stack = in(reg) stack_top,
            entry = sym boot_impl,
            options(noreturn),
        );
    }
}

fn boot_impl() -> ! {
    debug_line(b"LogOS vNext: UEFI entered");
    let cpu_count = discover_cpus();
    measure_tsc();
    stage_trampoline();
    install_cpu(0);
    let framebuffer = capture_gop();
    let network_config = crate::network_config::load_from_esp();
    if network_config.is_enabled() && !virtio_net::prepare_dma() {
        debug_line(b"LogOS vNext: network DMA unavailable");
    }
    #[cfg(feature = "qemu-proof")]
    crate::proof::configure_network(network_config.is_enabled());
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
    let frame_metadata = reserve_frame_metadata();
    let memory_map = unsafe { boot::exit_boot_services(None) };
    publish_boot_resources(memory_map, framebuffer, frame_metadata);
    unsafe {
        let _runtime_guard = ServiceRuntimeGuard::acquire();
        SERVICE_IMAGES = Some(service_images);
        (*core::ptr::addr_of_mut!(SERVICE_RUNTIME)).configure_network(network_config);
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
                crate::service_runtime::ServiceRuntimeError::Ipc(_) => {
                    fatal(b"LogOS vNext: service IPC pages")
                }
                crate::service_runtime::ServiceRuntimeError::IpcPrivateMapping(_) => {
                    fatal(b"LogOS vNext: service IPC private mapping")
                }
                crate::service_runtime::ServiceRuntimeError::IpcPrivateProcess(_) => {
                    fatal(b"LogOS vNext: service IPC private process")
                }
                crate::service_runtime::ServiceRuntimeError::Framebuffer(_) => {
                    fatal(b"LogOS vNext: framebuffer mapping")
                }
                crate::service_runtime::ServiceRuntimeError::FramebufferProcess(_) => {
                    fatal(b"LogOS vNext: framebuffer process mapping")
                }
                crate::service_runtime::ServiceRuntimeError::FramebufferConfig(_) => {
                    fatal(b"LogOS vNext: framebuffer config mapping")
                }
                crate::service_runtime::ServiceRuntimeError::FramebufferConfigProcess(_) => {
                    fatal(b"LogOS vNext: framebuffer config process mapping")
                }
                crate::service_runtime::ServiceRuntimeError::FramebufferPresent(_) => {
                    fatal(b"LogOS vNext: framebuffer present mapping")
                }
                crate::service_runtime::ServiceRuntimeError::FramebufferPresentProcess(_) => {
                    fatal(b"LogOS vNext: framebuffer present process mapping")
                }
                crate::service_runtime::ServiceRuntimeError::Keyboard(_) => {
                    fatal(b"LogOS vNext: keyboard mapping")
                }
                crate::service_runtime::ServiceRuntimeError::KeyboardProcess(_) => {
                    fatal(b"LogOS vNext: keyboard process mapping")
                }
                crate::service_runtime::ServiceRuntimeError::Pointer(_) => {
                    fatal(b"LogOS vNext: pointer mapping")
                }
                crate::service_runtime::ServiceRuntimeError::PointerProcess(_) => {
                    fatal(b"LogOS vNext: pointer process mapping")
                }
                crate::service_runtime::ServiceRuntimeError::TaskCapacity => {
                    fatal(b"LogOS vNext: service task capacity")
                }
                crate::service_runtime::ServiceRuntimeError::TaskAddressSpace => {
                    fatal(b"LogOS vNext: service task address space")
                }
                crate::service_runtime::ServiceRuntimeError::TaskLaunch => {
                    fatal(b"LogOS vNext: service task launch")
                }
                crate::service_runtime::ServiceRuntimeError::TaskStop => {
                    fatal(b"LogOS vNext: service task stop")
                }
                crate::service_runtime::ServiceRuntimeError::RestartLimit => {
                    fatal(b"LogOS vNext: service restart limit")
                }
                crate::service_runtime::ServiceRuntimeError::StaleGeneration => {
                    fatal(b"LogOS vNext: service stale generation")
                }
            },
        );
        let runtime = &mut *core::ptr::addr_of_mut!(SERVICE_RUNTIME);
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
    if virtio_device::initialize_storage_device() {
        proof_line(b"LogOS vNext: VirtIO block ready");
    } else {
        debug_line(b"LogOS vNext: VirtIO block unavailable");
    }
    if virtio_net::initialize(network_config) {
        proof_line(b"LogOS vNext: VirtIO net ready");
    } else if network_config.is_enabled() {
        debug_line(b"LogOS vNext: VirtIO net unavailable");
    } else {
        proof_line(b"LogOS vNext: network disabled");
    }
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
        if width < logos_abi::MIN_FRAMEBUFFER_WIDTH || height < logos_abi::MIN_FRAMEBUFFER_HEIGHT {
            continue;
        }
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
        // The terminal has a fixed 80x25 grid; avoid opening a needlessly
        // large QEMU window when a smaller valid GOP mode is available.
        if selected.as_ref().is_none_or(|current| candidate.0 < current.0) {
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

fn reserve_frame_metadata() -> crate::boot_resources::FrameMetadataReservation {
    let available_pages = boot::memory_map(MemoryType::LOADER_DATA)
        .unwrap_or_else(|_| fatal(b"LogOS vNext: metadata memory map"))
        .entries()
        .filter(|descriptor| descriptor.ty == MemoryType::CONVENTIONAL)
        .map(|descriptor| descriptor.page_count)
        .try_fold(0u64, u64::checked_add)
        .unwrap_or_else(|| fatal(b"LogOS vNext: metadata size"));
    let pages = crate::memory::frame_metadata_pages_for_frames(available_pages)
        .and_then(|pages| usize::try_from(pages).ok())
        .filter(|pages| *pages != 0)
        .unwrap_or_else(|| fatal(b"LogOS vNext: metadata size"));
    let base = boot::allocate_pages(boot::AllocateType::AnyPages, MemoryType::LOADER_DATA, pages)
        .unwrap_or_else(|_| fatal(b"LogOS vNext: metadata allocation"));
    crate::boot_resources::FrameMetadataReservation::new(base.as_ptr() as u64, pages as u64)
        .unwrap_or_else(|| fatal(b"LogOS vNext: metadata reservation"))
}

fn publish_boot_resources(
    memory_map: impl UefiMemoryMap,
    framebuffer: FramebufferInfo,
    frame_metadata: crate::boot_resources::FrameMetadataReservation,
) {
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
    resources.publish_frame_metadata(frame_metadata);
    unsafe {
        BOOT_RESOURCES = Some(resources);
    }
    proof_line(b"LogOS vNext: boot resources ready");
}

#[allow(dead_code)]
pub(crate) fn boot_resources() -> Option<&'static BootResources> {
    // SAFETY: boot resources are published once before runtime starts and
    // remain immutable for the rest of the kernel lifetime.
    unsafe { (*core::ptr::addr_of!(BOOT_RESOURCES)).as_ref() }
}

pub(crate) fn framebuffer_present_snapshot()
-> Option<(u32, bool, [logos_abi::GuiRect; logos_abi::MAX_DISPLAY_PRESENT_RECTS])> {
    let _runtime_guard = ServiceRuntimeGuard::acquire();
    unsafe {
        (*core::ptr::addr_of!(SERVICE_RUNTIME)).framebuffer_present_frame().map(|frame| {
            (&*(frame.raw() as usize as *const logos_abi::FramebufferPresentState)).snapshot()
        })
    }
}

pub(crate) fn framebuffer_cursor_snapshot() -> Option<(u32, bool, i16, i16, bool)> {
    let _runtime_guard = ServiceRuntimeGuard::acquire();
    unsafe {
        (*core::ptr::addr_of!(SERVICE_RUNTIME)).framebuffer_present_frame().map(|frame| {
            let state = &*(frame.raw() as usize as *const logos_abi::FramebufferPresentState);
            state.cursor_snapshot()
        })
    }
}

pub(crate) fn set_hardware_cursor(active: bool) {
    let _runtime_guard = ServiceRuntimeGuard::acquire();
    unsafe {
        if let Some(frame) = (*core::ptr::addr_of!(SERVICE_RUNTIME)).framebuffer_present_frame() {
            (&*(frame.raw() as usize as *const logos_abi::FramebufferPresentState))
                .set_hardware_cursor(active);
        }
    }
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

pub(super) fn notify_reschedule_cpus(source_cpu: usize) {
    let cpu_count = CPU_COUNT.load(Ordering::Acquire);
    let mut fallback = None;
    // The global slot table makes one prompt sufficient; prefer an idle target
    // and avoid an IPI fan-out when several wakeups arrive together.
    for (cpu, apic_id) in APIC_IDS.iter().enumerate().take(cpu_count) {
        if cpu == source_cpu || !SCHEDULER.cpu_online(cpu) {
            continue;
        }
        fallback.get_or_insert((cpu, apic_id.load(Ordering::Acquire)));
        if SCHEDULER.current_task(cpu).is_some() {
            continue;
        }
        send_ipi(apic_id.load(Ordering::Acquire), u32::from(RESCHEDULE_VECTOR));
        return;
    }
    if let Some((_, apic_id)) = fallback {
        send_ipi(apic_id, u32::from(RESCHEDULE_VECTOR));
    } else if cpu_count == 1 && SCHEDULER.current_task(source_cpu).is_some() {
        // With one CPU there is no remote target to prompt after an IPC wake.
        // A self-IPI lets the newly runnable receiver preempt the sender.
        send_ipi(APIC_IDS[source_cpu].load(Ordering::Acquire), u32::from(RESCHEDULE_VECTOR));
    }
}

fn install_cpu(index: usize) {
    unsafe {
        CPU_LOCALS[index].initialize(index);
        wrmsr(0xc000_0103, index as u64);
        write_gs(&CPU_LOCALS[index]);
    }
}

pub(crate) fn current_cr3() -> usize {
    let value: usize;
    unsafe { asm!("mov {}, cr3", out(reg) value, options(nomem, nostack, preserves_flags)) };
    value
}

pub(crate) fn reserve_kernel_frames(pool: &mut crate::frame_pool::FramePool) {
    reserve_storage_frames(
        pool,
        core::ptr::addr_of!(crate::SCHEDULER) as usize,
        core::mem::size_of::<crate::Scheduler>(),
    );
    reserve_storage_frames(
        pool,
        core::ptr::addr_of!(CPU_LOCALS) as usize,
        core::mem::size_of::<[CpuLocal; MAX_CPUS]>(),
    );
    reserve_storage_frames(
        pool,
        core::ptr::addr_of!(SERVICE_RUNTIME) as usize,
        core::mem::size_of::<crate::service_runtime::ServiceRuntime>(),
    );
    reserve_storage_frames(
        pool,
        core::ptr::addr_of!(CPU_GDTS) as usize,
        core::mem::size_of::<[[u64; 7]; MAX_CPUS]>(),
    );
    reserve_storage_frames(
        pool,
        core::ptr::addr_of!(CPU_IDTS) as usize,
        core::mem::size_of::<[[IdtEntry; IDT_ENTRIES]; MAX_CPUS]>(),
    );
    reserve_storage_frames(
        pool,
        core::ptr::addr_of!(CPU_TSS) as usize,
        core::mem::size_of::<[TaskStateSegment; MAX_CPUS]>(),
    );
    reserve_storage_frames(
        pool,
        core::ptr::addr_of!(crate::memory::KERNEL_GLOBAL_ALLOCATOR) as usize,
        core::mem::size_of::<crate::memory::KernelGlobalAllocator<'static>>(),
    );
    virtio_device::reserve_frames(pool);
    virtio_net::reserve_frames(pool);
    crate::virtio::gpu::reserve_frames(pool);
    #[cfg(feature = "qemu-proof")]
    {
        crate::user_mode::reserve_frames(pool);
        crate::proof::reserve_frames(pool);
    }
}

pub(crate) fn reserve_storage_frames(
    pool: &mut crate::frame_pool::FramePool,
    address: usize,
    bytes: usize,
) {
    let start = address & !0xfff;
    let Some(end) = address.checked_add(bytes).and_then(|end| end.checked_add(0xfff)) else {
        return;
    };
    let root = current_cr3() as u64 & 0x000f_ffff_ffff_f000;
    for virtual_address in (start..end & !0xfff).step_by(0x1000) {
        if let Some(frame) = translate_kernel_page(root, virtual_address as u64) {
            pool.reserve(crate::frame_pool::FrameAddress::from_raw(frame & !0xfff));
        }
    }
}

fn translate_kernel_page(root: u64, virtual_address: u64) -> Option<u64> {
    const PRESENT: u64 = 1;
    const HUGE: u64 = 1 << 7;
    const ADDRESS_MASK: u64 = 0x000f_ffff_ffff_f000;
    let pml4 = root;
    let pml4_entry = unsafe {
        core::ptr::read_volatile((pml4 + ((virtual_address >> 39) & 0xff8)) as *const u64)
    };
    if pml4_entry & PRESENT == 0 {
        return None;
    }
    let pdpt = pml4_entry & ADDRESS_MASK;
    let pdpt_entry = unsafe {
        core::ptr::read_volatile((pdpt + ((virtual_address >> 30) & 0xff8)) as *const u64)
    };
    if pdpt_entry & PRESENT == 0 {
        return None;
    }
    if pdpt_entry & HUGE != 0 {
        return Some((pdpt_entry & 0x000f_ffff_c000_0000) | (virtual_address & 0x3fff_ffff));
    }
    let pd = pdpt_entry & ADDRESS_MASK;
    let pd_entry =
        unsafe { core::ptr::read_volatile((pd + ((virtual_address >> 21) & 0xff8)) as *const u64) };
    if pd_entry & PRESENT == 0 {
        return None;
    }
    if pd_entry & HUGE != 0 {
        return Some((pd_entry & 0x000f_ffff_ffe0_0000) | (virtual_address & 0x1f_ffff));
    }
    let pt = pd_entry & ADDRESS_MASK;
    let pt_entry =
        unsafe { core::ptr::read_volatile((pt + ((virtual_address >> 12) & 0xff8)) as *const u64) };
    (pt_entry & PRESENT != 0).then_some((pt_entry & ADDRESS_MASK) | (virtual_address & 0xfff))
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
    let root = if root == 0 {
        let kernel = KERNEL_CR3.load(Ordering::Acquire);
        if kernel == 0 || kernel & 0xfff != 0 {
            proof_line(b"LogOS vNext: invalid kernel CR3");
        }
        kernel
    } else {
        if root & 0xfff != 0 {
            proof_line(b"LogOS vNext: invalid task CR3");
        }
        root
    };
    switch_cr3(root);
}

pub(crate) fn restart_critical_section<R>(operation: impl FnOnce() -> R) -> R {
    unsafe { asm!("cli", options(nomem, nostack, preserves_flags)) };
    let result = operation();
    unsafe { asm!("sti", options(nomem, nostack, preserves_flags)) };
    result
}

pub(crate) fn restart_critical_section_held<R>(operation: impl FnOnce() -> R) -> R {
    unsafe { asm!("cli", options(nomem, nostack, preserves_flags)) };
    operation()
}

pub(crate) fn start_services() {
    // Tasks are published before this function returns. Keep interrupts off
    // until READY is visible so a just-published service cannot enter through
    // an intentionally closed runtime boundary.
    restart_critical_section(|| unsafe {
        reset_events();
        let runtime = &mut *core::ptr::addr_of_mut!(SERVICE_RUNTIME);
        runtime.start_tasks().unwrap_or_else(|error| match error {
            crate::service_runtime::ServiceRuntimeError::TaskCapacity => {
                fatal(b"LogOS vNext: service task capacity")
            }
            crate::service_runtime::ServiceRuntimeError::TaskAddressSpace => {
                fatal(b"LogOS vNext: service task address space")
            }
            crate::service_runtime::ServiceRuntimeError::TaskLaunch => {
                fatal(b"LogOS vNext: service task launch")
            }
            crate::service_runtime::ServiceRuntimeError::TaskStop => {
                fatal(b"LogOS vNext: service task stop")
            }
            _ => fatal(b"LogOS vNext: service task startup"),
        });
        let ring =
            runtime.keyboard_ring_address().unwrap_or_else(|| fatal(b"LogOS vNext: keyboard ring"));
        publish_keyboard_ring(ring);
        let pointer_ring =
            runtime.pointer_ring_address().unwrap_or_else(|| fatal(b"LogOS vNext: pointer ring"));
        publish_pointer_ring(pointer_ring);
        enable_keyboard_irq();
        enable_pointer_irq();
        finish_service_runtime_transition();
    });
    proof_line(b"LogOS vNext: service tasks started");
}

pub(crate) fn publish_keyboard_ring(address: usize) {
    KEYBOARD_RING.store(address, Ordering::Release);
}

pub(crate) fn publish_pointer_ring(address: usize) {
    POINTER_RING.store(address, Ordering::Release);
}

pub(crate) fn disable_keyboard_irq() {
    KEYBOARD_IRQ_ENABLED.store(false, Ordering::Release);
    unsafe {
        let mask = in_port(PIC_MASTER_DATA);
        out_port(PIC_MASTER_DATA, mask | (1 << 1));
    }
    while KEYBOARD_IRQ_IN_FLIGHT.load(Ordering::Acquire) != 0 {
        core::hint::spin_loop();
    }
    KEYBOARD_RING.store(0, Ordering::Release);
}

pub(crate) fn disable_pointer_irq() {
    POINTER_IRQ_ENABLED.store(false, Ordering::Release);
    unsafe {
        let mask = in_port(PIC_SLAVE_DATA);
        out_port(PIC_SLAVE_DATA, mask | (1 << 4));
    }
    while POINTER_IRQ_IN_FLIGHT.load(Ordering::Acquire) != 0 {
        core::hint::spin_loop();
    }
    POINTER_RING.store(0, Ordering::Release);
}

pub(crate) fn supervise_services() -> bool {
    if !service_runtime_ready() {
        return false;
    }
    let mut runtime_guard = ServiceRuntimeGuard::acquire();
    unsafe {
        let Some(images) = (*core::ptr::addr_of!(SERVICE_IMAGES)).as_ref() else {
            fatal(b"LogOS vNext: service image state");
        };
        (&mut *core::ptr::addr_of_mut!(SERVICE_RUNTIME))
            .supervise(images, current_ticks(), &mut runtime_guard)
            .unwrap_or_else(|_| fatal(b"LogOS vNext: service restart"))
    }
}

pub(crate) fn record_service_heartbeat(
    service: logos_abi::ServiceHandle,
    process: crate::process::ProcessHandle,
    now: u64,
) -> bool {
    if !service_runtime_ready() && !service_runtime_restarting() {
        return true;
    }
    let _runtime_guard = ServiceRuntimeGuard::acquire();
    unsafe {
        let runtime = &mut *core::ptr::addr_of_mut!(SERVICE_RUNTIME);
        if service_runtime_restarting() {
            runtime.owns_service_process_handle(service, process)
        } else {
            runtime.record_heartbeat_handle(service, process, now)
        }
    }
}

pub(crate) fn ipc_send(
    process: crate::process::ProcessHandle,
    capability_raw: u64,
    length: usize,
) -> crate::service_ipc::IpcOutcome {
    if !service_runtime_ready() {
        return crate::service_ipc::IpcOutcome {
            status: logos_abi::IpcStatus::Unauthorized,
            notified: false,
        };
    }
    let _runtime_guard = ServiceRuntimeGuard::acquire();
    if service_runtime_restarting() {
        return crate::service_ipc::IpcOutcome {
            status: logos_abi::IpcStatus::Disconnected,
            notified: false,
        };
    }
    unsafe {
        (&mut *core::ptr::addr_of_mut!(SERVICE_RUNTIME)).ipc_send(process, capability_raw, length)
    }
}

pub(crate) fn ipc_receive(
    process: crate::process::ProcessHandle,
    capability_raw: u64,
) -> crate::service_ipc::IpcOutcome {
    if !service_runtime_ready() {
        return crate::service_ipc::IpcOutcome {
            status: logos_abi::IpcStatus::Unauthorized,
            notified: false,
        };
    }
    let _runtime_guard = ServiceRuntimeGuard::acquire();
    if service_runtime_restarting() {
        return crate::service_ipc::IpcOutcome {
            status: logos_abi::IpcStatus::Disconnected,
            notified: false,
        };
    }
    unsafe { (&mut *core::ptr::addr_of_mut!(SERVICE_RUNTIME)).ipc_receive(process, capability_raw) }
}

pub(crate) fn grow_service_heap(
    process: crate::process::ProcessHandle,
    capability_raw: u64,
    pages: usize,
) -> logos_abi::IpcStatus {
    if !service_runtime_ready() {
        return logos_abi::IpcStatus::Unauthorized;
    }
    let _runtime_guard = ServiceRuntimeGuard::acquire();
    if service_runtime_restarting() {
        return logos_abi::IpcStatus::Disconnected;
    }
    unsafe {
        (&mut *core::ptr::addr_of_mut!(SERVICE_RUNTIME)).grow_service_heap(
            process,
            capability_raw,
            pages,
        )
    }
}

pub(crate) fn shrink_service_heap(
    process: crate::process::ProcessHandle,
    capability_raw: u64,
    pages: usize,
) -> logos_abi::IpcStatus {
    if !service_runtime_ready() {
        return logos_abi::IpcStatus::Unauthorized;
    }
    let _runtime_guard = ServiceRuntimeGuard::acquire();
    if service_runtime_restarting() {
        return logos_abi::IpcStatus::Disconnected;
    }
    unsafe {
        (&mut *core::ptr::addr_of_mut!(SERVICE_RUNTIME)).shrink_service_heap(
            process,
            capability_raw,
            pages,
        )
    }
}

pub(crate) fn directory_call(
    process: crate::process::ProcessHandle,
    capability_raw: u64,
    length: usize,
) -> logos_abi::DirectoryStatus {
    if !service_runtime_ready() {
        return logos_abi::DirectoryStatus::Unauthorized;
    }
    let _runtime_guard = ServiceRuntimeGuard::acquire();
    if service_runtime_restarting() {
        return logos_abi::DirectoryStatus::Stale;
    }
    unsafe {
        (&mut *core::ptr::addr_of_mut!(SERVICE_RUNTIME)).directory_call(
            process,
            capability_raw,
            length,
        )
    }
}

pub(crate) fn event_call(
    process: crate::process::ProcessHandle,
    length: usize,
) -> logos_abi::EventStatus {
    if !service_runtime_ready() {
        return logos_abi::EventStatus::Unauthorized;
    }
    let _runtime_guard = ServiceRuntimeGuard::acquire();
    if service_runtime_restarting() {
        return logos_abi::EventStatus::Stale;
    }
    unsafe { (&mut *core::ptr::addr_of_mut!(SERVICE_RUNTIME)).event_call(process, length) }
}

pub(crate) fn manager_call(
    process: crate::process::ProcessHandle,
    capability_raw: u64,
    length: usize,
) -> logos_abi::IpcStatus {
    if !service_runtime_ready() {
        return logos_abi::IpcStatus::Unauthorized;
    }
    let _runtime_guard = ServiceRuntimeGuard::acquire();
    if service_runtime_restarting() {
        return logos_abi::IpcStatus::Disconnected;
    }
    unsafe {
        (&mut *core::ptr::addr_of_mut!(SERVICE_RUNTIME)).manager_call(
            process,
            capability_raw,
            length,
        )
    }
}

#[cfg(feature = "qemu-proof")]
pub(crate) fn manager_proof(
    request: logos_abi::ManagerRequest,
) -> Option<logos_abi::ManagerResponse> {
    let _runtime_guard = ServiceRuntimeGuard::acquire();
    unsafe { (&mut *core::ptr::addr_of_mut!(SERVICE_RUNTIME)).manager_proof(request) }
}

#[cfg(feature = "qemu-proof")]
pub(crate) fn event_proof() -> bool {
    let _runtime_guard = ServiceRuntimeGuard::acquire();
    unsafe { (&mut *core::ptr::addr_of_mut!(SERVICE_RUNTIME)).event_proof() }
}

#[cfg(feature = "qemu-proof")]
pub(crate) fn dynamic_ipc_proof() -> bool {
    let _runtime_guard = ServiceRuntimeGuard::acquire();
    unsafe { (&mut *core::ptr::addr_of_mut!(SERVICE_RUNTIME)).dynamic_ipc_proof() }
}

#[cfg(feature = "qemu-proof")]
pub(crate) fn allocator_proof() -> bool {
    let _runtime_guard = ServiceRuntimeGuard::acquire();
    unsafe { (&mut *core::ptr::addr_of_mut!(SERVICE_RUNTIME)).allocator_proof() }
}

#[cfg(any(feature = "qemu-proof", feature = "input-debug"))]
pub(crate) fn service_debug_line(process: crate::process::ProcessHandle, length: usize) -> bool {
    let _runtime_guard = ServiceRuntimeGuard::acquire();
    unsafe { (&*core::ptr::addr_of!(SERVICE_RUNTIME)).service_debug_line(process, length) }
}

#[cfg(feature = "qemu-proof")]
pub(crate) fn manager_restart_ready(service: logos_abi::ServiceId) -> bool {
    let _runtime_guard = ServiceRuntimeGuard::acquire();
    unsafe { (&*core::ptr::addr_of!(SERVICE_RUNTIME)).manager_restart_ready(service) }
}

// Package activation remains an internal Core hook until package-manager policy exists.
#[allow(dead_code)]
pub(crate) fn activate_service_package(
    service: logos_abi::ServiceId,
) -> Result<(), crate::service_runtime::ServiceRuntimeError> {
    let mut runtime_guard = ServiceRuntimeGuard::acquire();
    unsafe {
        (&mut *core::ptr::addr_of_mut!(SERVICE_RUNTIME))
            .activate_package(service, &mut runtime_guard)
    }
}

#[cfg(feature = "package-proof")]
pub(crate) fn restart_service_graph_for_proof() -> bool {
    let mut runtime_guard = ServiceRuntimeGuard::acquire();
    unsafe {
        (&mut *core::ptr::addr_of_mut!(SERVICE_RUNTIME))
            .restart_for_package_proof(&mut runtime_guard)
            .is_ok()
    }
}

#[cfg(feature = "package-proof")]
pub(crate) fn package_frame_accounting_valid() -> bool {
    let _runtime_guard = ServiceRuntimeGuard::acquire();
    unsafe { (&*core::ptr::addr_of!(SERVICE_RUNTIME)).package_frame_accounting_valid() }
}

#[cfg(feature = "qemu-proof")]
pub(crate) fn hostile_ipc_layout_valid() -> bool {
    let _runtime_guard = ServiceRuntimeGuard::acquire();
    unsafe { (&*core::ptr::addr_of!(SERVICE_RUNTIME)).hostile_ipc_layout_valid() }
}

#[cfg(feature = "qemu-proof")]
pub(crate) fn suppress_service_heartbeat(service: logos_abi::ServiceId) {
    let _runtime_guard = ServiceRuntimeGuard::acquire();
    unsafe { (&*core::ptr::addr_of!(SERVICE_RUNTIME)).suppress_heartbeat(service) }
}

pub(crate) fn fault_service_process(process: crate::process::ProcessHandle, vector: u8) -> bool {
    let _runtime_guard = ServiceRuntimeGuard::acquire();
    unsafe {
        (&mut *core::ptr::addr_of_mut!(SERVICE_RUNTIME)).fault_process(process, vector).is_ok()
    }
}

pub(crate) fn fault_service_page(
    process: crate::process::ProcessHandle,
    fault_address: usize,
) -> crate::service_runtime::ServiceFaultOutcome {
    let _runtime_guard = ServiceRuntimeGuard::acquire();
    unsafe {
        (&mut *core::ptr::addr_of_mut!(SERVICE_RUNTIME)).handle_page_fault(process, fault_address)
    }
}

pub(crate) fn page_fault_address() -> usize {
    let mut address = 0;
    unsafe {
        asm!("mov {address}, cr2", address = out(reg) address, options(nostack, preserves_flags));
    }
    address
}

pub(crate) fn exit_process(process: crate::process::ProcessHandle, code: u8) -> bool {
    let _runtime_guard = ServiceRuntimeGuard::acquire();
    unsafe { (&mut *core::ptr::addr_of_mut!(SERVICE_RUNTIME)).exit_process(process, code).is_ok() }
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
    configure_pic();
    if !configure_keyboard() {
        fatal(b"LogOS vNext: keyboard controller");
    }
    #[cfg(any(feature = "qemu-proof", feature = "input-debug"))]
    proof_line(b"LogOS vNext: PS/2 keyboard set 2");
    POINTER_IRQ_AVAILABLE.store(configure_pointer(), Ordering::Release);
    calibrate_timer();
    crate::user_mode::initialize_kernel_cr3(current_cr3());
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

pub(crate) fn power_control(process: ProcessHandle, action: usize) -> bool {
    #[cfg(target_os = "uefi")]
    {
        if !service_runtime_ready() {
            return false;
        }
        let _runtime_guard = ServiceRuntimeGuard::acquire();
        let authorized = unsafe {
            (&*core::ptr::addr_of!(SERVICE_RUNTIME))
                .launch(logos_abi::ServiceId::Flow)
                .is_some_and(|(current, _)| current == process)
        };
        if !authorized {
            return false;
        }
        if matches!(action, logos_abi::POWER_SHUTDOWN | logos_abi::POWER_REBOOT)
            && prepare_storage_power_control().is_err()
        {
            debug_line(b"LogOS vNext: power flush failed");
            return false;
        }
        match action {
            logos_abi::POWER_SHUTDOWN => shutdown_qemu(),
            logos_abi::POWER_REBOOT => reboot_qemu(),
            _ => false,
        }
    }
    #[cfg(not(target_os = "uefi"))]
    {
        let _ = (process, action);
        false
    }
}

#[cfg(target_os = "uefi")]
fn shutdown_qemu() -> ! {
    debug_line(b"LogOS vNext: shutdown requested");
    unsafe { out_word_port(ACPI_SHUTDOWN_PORT, 0x2000) };
    fatal(b"LogOS vNext: shutdown returned")
}

#[cfg(target_os = "uefi")]
fn reboot_qemu() -> ! {
    debug_line(b"LogOS vNext: reboot requested");
    unsafe { out_port(RESET_CONTROL_PORT, 0x02) };
    unsafe { io_wait() };
    unsafe { out_port(RESET_CONTROL_PORT, 0x06) };
    fatal(b"LogOS vNext: reboot returned")
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
        idt[KEYBOARD_VECTOR as usize] =
            IdtEntry::new(keyboard_interrupt as *const () as usize, KERNEL_CODE_SELECTOR, 0x8e);
        idt[POINTER_VECTOR as usize] =
            IdtEntry::new(pointer_interrupt as *const () as usize, KERNEL_CODE_SELECTOR, 0x8e);
        idt[STORAGE_VECTOR as usize] =
            IdtEntry::new(storage_interrupt as *const () as usize, KERNEL_CODE_SELECTOR, 0x8e);
        idt[NETWORK_VECTOR as usize] =
            IdtEntry::new(network_interrupt as *const () as usize, KERNEL_CODE_SELECTOR, 0x8e);
        idt[SWITCH_VECTOR as usize] = IdtEntry::new(
            context_switch_interrupt as *const () as usize,
            KERNEL_CODE_SELECTOR,
            0xee,
        );
        idt[RESCHEDULE_VECTOR as usize] = IdtEntry::new(
            context_reschedule_interrupt as *const () as usize,
            KERNEL_CODE_SELECTOR,
            0x8e,
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

fn configure_pic() {
    unsafe {
        out_port(PIC_MASTER_COMMAND, 0x11);
        io_wait();
        out_port(PIC_SLAVE_COMMAND, 0x11);
        io_wait();
        out_port(PIC_MASTER_DATA, 0x20);
        io_wait();
        out_port(PIC_SLAVE_DATA, 0x28);
        io_wait();
        out_port(PIC_MASTER_DATA, 0x04);
        io_wait();
        out_port(PIC_SLAVE_DATA, 0x02);
        io_wait();
        out_port(PIC_MASTER_DATA, 0x01);
        io_wait();
        out_port(PIC_SLAVE_DATA, 0x01);
        io_wait();
        out_port(PIC_MASTER_DATA, 0xff);
        out_port(PIC_SLAVE_DATA, 0xff);
    }
}

fn configure_keyboard() -> bool {
    unsafe {
        // InputDecoder consumes Set-2 bytes; establish that device contract
        // after UEFI handoff instead of relying on firmware state.
        let mut ready = false;
        for _ in 0..1_000_000 {
            if in_port(KEYBOARD_STATUS_PORT) & KEYBOARD_INPUT_FULL == 0 {
                out_port(KEYBOARD_COMMAND_PORT, KEYBOARD_READ_CONFIG);
                ready = true;
                break;
            }
        }
        if !ready {
            return false;
        }
        if in_port(KEYBOARD_STATUS_PORT) & KEYBOARD_OUTPUT_FULL == 0 {
            return false;
        }
        let config = (in_port(KEYBOARD_DATA_PORT)
            & !(KEYBOARD_CLOCK_DISABLED | POINTER_CLOCK_DISABLED | KEYBOARD_TRANSLATION_ENABLED))
            | 0x03;
        ready = false;
        for _ in 0..1_000_000 {
            if in_port(KEYBOARD_STATUS_PORT) & KEYBOARD_INPUT_FULL == 0 {
                out_port(KEYBOARD_COMMAND_PORT, KEYBOARD_WRITE_CONFIG);
                ready = true;
                break;
            }
        }
        if !ready {
            return false;
        }
        for _ in 0..1_000_000 {
            if in_port(KEYBOARD_STATUS_PORT) & KEYBOARD_INPUT_FULL == 0 {
                out_port(KEYBOARD_DATA_PORT, config);
                ready = true;
                break;
            }
        }
        if !ready {
            return false;
        }
        ready = false;
        for _ in 0..1_000_000 {
            if in_port(KEYBOARD_STATUS_PORT) & KEYBOARD_INPUT_FULL == 0 {
                out_port(KEYBOARD_DATA_PORT, KEYBOARD_SET_SCANCODE);
                ready = true;
                break;
            }
        }
        if !ready {
            return false;
        }
        if !wait_keyboard_ack() {
            return false;
        }
        ready = false;
        for _ in 0..1_000_000 {
            if in_port(KEYBOARD_STATUS_PORT) & KEYBOARD_INPUT_FULL == 0 {
                out_port(KEYBOARD_DATA_PORT, KEYBOARD_SCANCODE_SET_2);
                ready = true;
                break;
            }
        }
        if !ready {
            return false;
        }
        return wait_keyboard_ack();
    }
}

unsafe fn wait_keyboard_ack() -> bool {
    for _ in 0..1_000_000 {
        let status = unsafe { in_port(KEYBOARD_STATUS_PORT) };
        if status & KEYBOARD_OUTPUT_FULL != 0 && status & KEYBOARD_AUX_OUTPUT == 0 {
            return unsafe { in_port(KEYBOARD_DATA_PORT) } == KEYBOARD_ACK;
        }
    }
    false
}

fn configure_pointer() -> bool {
    unsafe {
        let mut ready = false;
        for _ in 0..1_000_000 {
            if in_port(KEYBOARD_STATUS_PORT) & KEYBOARD_INPUT_FULL == 0 {
                out_port(KEYBOARD_COMMAND_PORT, POINTER_ENABLE_AUX);
                ready = true;
                break;
            }
        }
        if !ready {
            return false;
        }
        ready = false;
        for _ in 0..1_000_000 {
            if in_port(KEYBOARD_STATUS_PORT) & KEYBOARD_INPUT_FULL == 0 {
                out_port(KEYBOARD_COMMAND_PORT, POINTER_COMMAND);
                ready = true;
                break;
            }
        }
        if !ready {
            return false;
        }
        ready = false;
        for _ in 0..1_000_000 {
            if in_port(KEYBOARD_STATUS_PORT) & KEYBOARD_INPUT_FULL == 0 {
                out_port(KEYBOARD_DATA_PORT, POINTER_ENABLE_STREAM);
                ready = true;
                break;
            }
        }
        if !ready {
            return false;
        }
        for _ in 0..1_000_000 {
            if in_port(KEYBOARD_STATUS_PORT) & KEYBOARD_OUTPUT_FULL != 0 {
                return in_port(KEYBOARD_DATA_PORT) == POINTER_ACK;
            }
        }
    }
    false
}

pub(crate) fn enable_keyboard_irq() {
    KEYBOARD_IRQ_ENABLED.store(true, Ordering::Release);
    unsafe {
        let mask = in_port(PIC_MASTER_DATA);
        out_port(PIC_MASTER_DATA, mask & !(1 << 1));
    }
}

pub(crate) fn enable_pointer_irq() {
    if !POINTER_IRQ_AVAILABLE.load(Ordering::Acquire) {
        return;
    }
    POINTER_IRQ_ENABLED.store(true, Ordering::Release);
    unsafe {
        let slave_mask = in_port(PIC_SLAVE_DATA);
        out_port(PIC_SLAVE_DATA, slave_mask & !(1 << 4));
        let master_mask = in_port(PIC_MASTER_DATA);
        out_port(PIC_MASTER_DATA, master_mask & !(1 << 2));
    }
}

pub(super) fn handle_keyboard_interrupt() {
    KEYBOARD_IRQ_IN_FLIGHT.fetch_add(1, Ordering::AcqRel);
    let byte = unsafe { in_port(KEYBOARD_DATA_PORT) };
    if KEYBOARD_IRQ_ENABLED.load(Ordering::Acquire) {
        let ring = KEYBOARD_RING.load(Ordering::Acquire);
        if ring != 0 {
            // The frame is identity-mapped in the kernel root and is mapped into
            // Input separately by the service runtime.
            if let Ok(notification) =
                unsafe { (&*(ring as *const logos_abi::KeyboardByteRing)).push(byte) }
            {
                if notification == logos_abi::Notify::Notified {
                    crate::runtime_events::signal_hardware_event(
                        crate::runtime_events::HardwareEventSource::Keyboard,
                    );
                    #[cfg(any(feature = "qemu-proof", feature = "input-debug"))]
                    proof_line(b"LogOS vNext: keyboard event wake");
                }
            }
        }
    }
    KEYBOARD_IRQ_IN_FLIGHT.fetch_sub(1, Ordering::Release);
    unsafe {
        out_port(PIC_MASTER_COMMAND, PIC_EOI);
    }
}

pub(super) fn handle_pointer_interrupt() {
    POINTER_IRQ_IN_FLIGHT.fetch_add(1, Ordering::AcqRel);
    let byte = unsafe { in_port(KEYBOARD_DATA_PORT) };
    if POINTER_IRQ_ENABLED.load(Ordering::Acquire) {
        let ring = POINTER_RING.load(Ordering::Acquire);
        if ring != 0 {
            if let Ok(notification) =
                unsafe { (&*(ring as *const logos_abi::PointerByteRing)).push(byte) }
            {
                if notification == logos_abi::Notify::Notified {
                    crate::runtime_events::signal_hardware_event(
                        crate::runtime_events::HardwareEventSource::Pointer,
                    );
                    #[cfg(feature = "qemu-proof")]
                    proof_line(b"LogOS vNext: pointer event wake");
                }
            }
        }
    }
    POINTER_IRQ_IN_FLIGHT.fetch_sub(1, Ordering::Release);
    unsafe {
        out_port(PIC_SLAVE_COMMAND, PIC_EOI);
        out_port(PIC_MASTER_COMMAND, PIC_EOI);
    }
}

unsafe fn io_wait() {
    unsafe { out_port(0x80, 0) };
}

unsafe fn out_port(port: u16, value: u8) {
    unsafe {
        asm!("out dx, al", in("dx") port, in("al") value, options(nomem, nostack, preserves_flags))
    };
}

unsafe fn out_word_port(port: u16, value: u16) {
    unsafe {
        asm!("out dx, ax", in("dx") port, in("ax") value, options(nomem, nostack, preserves_flags))
    };
}

unsafe fn in_port(port: u16) -> u8 {
    let value: u8;
    unsafe {
        asm!("in al, dx", in("dx") port, out("al") value, options(nomem, nostack, preserves_flags))
    };
    value
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
    fn keyboard_interrupt();
    fn pointer_interrupt();
    fn storage_interrupt();
    fn network_interrupt();
    fn context_switch_interrupt();
    fn context_reschedule_interrupt();
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

pub(crate) fn prepare_service_event_set_wait(
    task: crate::TaskHandle,
    event_set: logos_abi::EventSetHandle,
    deadline: u64,
) -> Option<bool> {
    let should_block = SCHEDULER.wait_for_event_object(task, event_set.raw(), deadline)?;
    if should_block {
        let cpu = current_cpu();
        unsafe {
            CPU_LOCALS[cpu].pending_action.store(
                if deadline == u64::MAX { ACTION_BLOCK } else { ACTION_TIMED_BLOCK },
                Ordering::Release,
            );
        }
    }
    Some(should_block)
}

pub(crate) fn signal_event_set(event_set: logos_abi::EventSetHandle) -> usize {
    let previous_wakes = SCHEDULER.event_wakes();
    let woken = SCHEDULER.signal_event_object(event_set.raw());
    if SCHEDULER.event_wakes() != previous_wakes {
        #[cfg(feature = "qemu-proof")]
        crate::proof::event_wake_ipi_sent();
        notify_reschedule_cpus(current_cpu());
    }
    woken
}

#[cfg_attr(not(feature = "qemu-proof"), allow(dead_code))]
pub(crate) fn wake_task(handle: crate::TaskHandle) -> bool {
    let was_blocked = SCHEDULER.state(handle) == Some(crate::TaskState::Blocked);
    let woken = SCHEDULER.wake(handle);
    if woken && was_blocked {
        notify_reschedule_cpus(current_cpu());
    }
    woken
}

pub(crate) fn reset_events() {
    SCHEDULER.reset_events();
}

#[cfg_attr(not(feature = "qemu-proof"), allow(dead_code))]
pub(crate) fn current_cpu() -> usize {
    context::current_cpu()
}

pub(crate) fn current_ticks() -> u64 {
    context::current_ticks()
}
