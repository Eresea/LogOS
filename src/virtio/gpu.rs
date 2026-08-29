use core::{
    arch::asm,
    mem::MaybeUninit,
    ptr::{read_volatile, write_volatile},
    sync::atomic::{AtomicBool, Ordering, fence},
};

use super::frame_queue::{FrameLease, FrameQueue, FrameQueueError};

use logos_storage::{
    PCI_CONFIG_BYTES, PciAddress, VIRTIO_F_VERSION_1, VIRTIO_GPU_FORMAT_B8G8R8A8_UNORM,
    VIRTIO_GPU_FORMAT_B8G8R8X8_UNORM, VIRTIO_GPU_MAX_BACKING_BYTES, VIRTIO_GPU_MAX_COMMAND_BYTES,
    VirtioGpuCommand, VirtioGpuRect, VirtioPciDevice, response_is_ok,
};

const PCI_CONFIG_ADDRESS: u16 = 0xcf8;
const PCI_CONFIG_DATA: u16 = 0xcfc;
const QUEUE_SIZE: usize = 8;
const VIRTQ_DESC_F_NEXT: u16 = 1;
const VIRTQ_DESC_F_WRITE: u16 = 2;
const STATUS_ACKNOWLEDGE: u8 = 1;
const STATUS_DRIVER: u8 = 2;
const STATUS_DRIVER_OK: u8 = 4;
const STATUS_FEATURES_OK: u8 = 8;
const COMMON_DEVICE_FEATURE_SELECT: usize = 0x00;
const COMMON_DEVICE_FEATURE: usize = 0x04;
const COMMON_DRIVER_FEATURE_SELECT: usize = 0x08;
const COMMON_DRIVER_FEATURE: usize = 0x0c;
const COMMON_DEVICE_STATUS: usize = 0x14;
const COMMON_QUEUE_SELECT: usize = 0x16;
const COMMON_QUEUE_SIZE: usize = 0x18;
const COMMON_QUEUE_ENABLE: usize = 0x1c;
const COMMON_QUEUE_NOTIFY_OFF: usize = 0x1e;
const COMMON_QUEUE_DESC: usize = 0x20;
const COMMON_QUEUE_DRIVER: usize = 0x28;
const COMMON_QUEUE_DEVICE: usize = 0x30;
const COMPLETION_SPIN_LIMIT: usize = 1_000_000;
const RESOURCE_ID: u32 = 1;
const SECONDARY_RESOURCE_ID: u32 = 3;
const FRAME_SLOT_COUNT: usize = 2;
const CURSOR_RESOURCE_ID: u32 = 2;
const SCANOUT_ID: u32 = 0;
const CURSOR_BACKING_BYTES: u32 = 4096;
const CURSOR_MASK: [u32; 24] = [
    0x00001, 0x00003, 0x00007, 0x0000f, 0x0001f, 0x0003f, 0x0007f, 0x000ff, 0x001ff, 0x003ff,
    0x007ff, 0x00fff, 0x01fff, 0x03fff, 0x07fff, 0x0ffff, 0x0ffff, 0x0fff8, 0x0fff8, 0x01ff0,
    0x01ff0, 0x03fe0, 0x03fe0, 0x07fc0,
];

#[repr(C, align(4096))]
struct QueueMemory {
    descriptors: [Descriptor; QUEUE_SIZE * 2],
    available_flags: u16,
    available_index: u16,
    available_ring: [u16; QUEUE_SIZE],
    available_used_event: u16,
    available_padding: u16,
    used_flags: u16,
    used_index: u16,
    used_ring: [UsedElement; QUEUE_SIZE],
    used_available_event: u16,
    request: [u8; VIRTIO_GPU_MAX_COMMAND_BYTES],
    response: [u8; 64],
}

impl QueueMemory {
    const EMPTY_DESCRIPTOR: Descriptor = Descriptor { address: 0, length: 0, flags: 0, next: 0 };
    const EMPTY_USED: UsedElement = UsedElement { id: 0, length: 0 };

    const fn new() -> Self {
        Self {
            descriptors: [Self::EMPTY_DESCRIPTOR; QUEUE_SIZE * 2],
            available_flags: 0,
            available_index: 0,
            available_ring: [0; QUEUE_SIZE],
            available_used_event: 0,
            available_padding: 0,
            used_flags: 0,
            used_index: 0,
            used_ring: [Self::EMPTY_USED; QUEUE_SIZE],
            used_available_event: 0,
            request: [0; VIRTIO_GPU_MAX_COMMAND_BYTES],
            response: [0; 64],
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Descriptor {
    address: u64,
    length: u32,
    flags: u16,
    next: u16,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct UsedElement {
    id: u32,
    length: u32,
}

const _: () = assert!(core::mem::align_of::<QueueMemory>() == 4096);
const _: () = assert!(core::mem::offset_of!(QueueMemory, used_flags) % 4 == 0);

#[unsafe(link_section = ".dma")]
static mut QUEUE_MEMORY: QueueMemory = QueueMemory::new();
#[unsafe(link_section = ".dma")]
static mut CURSOR_QUEUE_MEMORY: QueueMemory = QueueMemory::new();
#[repr(C, align(4096))]
struct CursorMemory([u8; CURSOR_BACKING_BYTES as usize]);

#[unsafe(link_section = ".dma")]
static mut CURSOR_MEMORY: CursorMemory = CursorMemory([0; CURSOR_BACKING_BYTES as usize]);
#[repr(C, align(4096))]
struct FrameMemory([u8; VIRTIO_GPU_MAX_BACKING_BYTES as usize]);

#[unsafe(link_section = ".dma")]
static mut FRAME_MEMORY: [FrameMemory; FRAME_SLOT_COUNT] = [
    FrameMemory([0; VIRTIO_GPU_MAX_BACKING_BYTES as usize]),
    FrameMemory([0; VIRTIO_GPU_MAX_BACKING_BYTES as usize]),
];
static mut DEVICE: MaybeUninit<VirtioGpuDevice> = MaybeUninit::uninit();
static DEVICE_READY: AtomicBool = AtomicBool::new(false);
static DEVICE_ATTEMPTED: AtomicBool = AtomicBool::new(false);
static DEVICE_BUSY: AtomicBool = AtomicBool::new(false);
#[cfg(feature = "qemu-proof")]
static GPU_PROOF_IDLE_SUPPRESSED: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GpuError {
    NotFound,
    InvalidFramebuffer,
    MissingBar,
    InvalidBar,
    FeatureNegotiation,
    DeviceRejectedFeatures,
    QueueUnavailable,
    Timeout,
    Device(u32),
    Busy,
}

#[cfg(feature = "qemu-proof")]
fn gpu_error_proof_line(error: GpuError) {
    let marker = match error {
        GpuError::NotFound => b"LogOS vNext: VirtIO GPU init not found".as_slice(),
        GpuError::InvalidFramebuffer => b"LogOS vNext: VirtIO GPU init framebuffer".as_slice(),
        GpuError::MissingBar | GpuError::InvalidBar => {
            b"LogOS vNext: VirtIO GPU init BAR".as_slice()
        }
        GpuError::FeatureNegotiation | GpuError::DeviceRejectedFeatures => {
            b"LogOS vNext: VirtIO GPU init features".as_slice()
        }
        GpuError::QueueUnavailable => b"LogOS vNext: VirtIO GPU init queue".as_slice(),
        GpuError::Timeout => b"LogOS vNext: VirtIO GPU init timeout".as_slice(),
        GpuError::Device(_) => b"LogOS vNext: VirtIO GPU init device".as_slice(),
        GpuError::Busy => b"LogOS vNext: VirtIO GPU init busy".as_slice(),
    };
    crate::arch_proof_line(marker);
}

#[derive(Clone, Copy)]
struct MmioRegion {
    address: u64,
    length: u32,
}

impl MmioRegion {
    fn new(address: u64, length: u32) -> Result<Self, GpuError> {
        if address == 0 || address % 4 != 0 || length < 4 {
            return Err(GpuError::InvalidBar);
        }
        Ok(Self { address, length })
    }

    fn ptr<T>(&self, offset: usize) -> Result<*mut T, GpuError> {
        let size = core::mem::size_of::<T>();
        if offset.checked_add(size).is_none_or(|end| end > self.length as usize) {
            return Err(GpuError::InvalidBar);
        }
        let address = self.address.checked_add(offset as u64).ok_or(GpuError::InvalidBar)?;
        Ok(address as *mut T)
    }

    unsafe fn read_u8(&self, offset: usize) -> Result<u8, GpuError> {
        Ok(unsafe { read_volatile(self.ptr(offset)?) })
    }

    unsafe fn read_u16(&self, offset: usize) -> Result<u16, GpuError> {
        Ok(unsafe { read_volatile(self.ptr(offset)?) })
    }

    unsafe fn read_u32(&self, offset: usize) -> Result<u32, GpuError> {
        Ok(unsafe { read_volatile(self.ptr(offset)?) })
    }

    unsafe fn write_u8(&self, offset: usize, value: u8) -> Result<(), GpuError> {
        unsafe { write_volatile(self.ptr(offset)?, value) };
        Ok(())
    }

    unsafe fn write_u16(&self, offset: usize, value: u16) -> Result<(), GpuError> {
        unsafe { write_volatile(self.ptr(offset)?, value) };
        Ok(())
    }

    unsafe fn write_u32(&self, offset: usize, value: u32) -> Result<(), GpuError> {
        unsafe { write_volatile(self.ptr(offset)?, value) };
        Ok(())
    }

    unsafe fn write_u64(&self, offset: usize, value: u64) -> Result<(), GpuError> {
        unsafe { write_volatile(self.ptr(offset)?, value) };
        Ok(())
    }
}

struct VirtioGpuDevice {
    common: MmioRegion,
    notify: MmioRegion,
    notify_multiplier: u32,
    queue: &'static mut QueueMemory,
    cursor_queue: &'static mut QueueMemory,
    negotiated_features: u64,
    framebuffer: VirtioGpuRect,
    transfer: VirtioGpuRect,
    next_fence: u64,
    frame_queue: FrameQueue<FRAME_SLOT_COUNT>,
    pending_frame: Option<PendingFrame>,
    frame_bytes: u32,
    frame_stride: usize,
    last_present_sequence: Option<u32>,
    last_cursor_sequence: Option<u32>,
    cursor_initialized: bool,
}

#[derive(Clone, Copy)]
enum FramePhase {
    Transfer,
    Flush,
    Scanout,
}

#[derive(Clone, Copy)]
struct PendingFrame {
    lease: FrameLease,
    phase: FramePhase,
    completion: u16,
    rects: [FrameRect; logos_abi::MAX_DISPLAY_PRESENT_RECTS],
    rect_count: usize,
    rect_index: usize,
}

#[derive(Clone, Copy)]
struct FrameRect {
    transfer: VirtioGpuRect,
    flush: VirtioGpuRect,
}

fn frame_resource_id(slot: usize) -> u32 {
    match slot {
        0 => RESOURCE_ID,
        _ => SECONDARY_RESOURCE_ID,
    }
}

pub(crate) fn reserve_frames(pool: &mut crate::frame_pool::FramePool) {
    crate::arch::reserve_storage_frames(
        pool,
        core::ptr::addr_of!(QUEUE_MEMORY) as usize,
        core::mem::size_of::<QueueMemory>(),
    );
    crate::arch::reserve_storage_frames(
        pool,
        core::ptr::addr_of!(CURSOR_QUEUE_MEMORY) as usize,
        core::mem::size_of::<QueueMemory>(),
    );
    crate::arch::reserve_storage_frames(
        pool,
        core::ptr::addr_of!(DEVICE) as usize,
        core::mem::size_of::<MaybeUninit<VirtioGpuDevice>>(),
    );
    crate::arch::reserve_storage_frames(
        pool,
        core::ptr::addr_of!(CURSOR_MEMORY) as usize,
        CURSOR_BACKING_BYTES as usize,
    );
    crate::arch::reserve_storage_frames(
        pool,
        core::ptr::addr_of!(FRAME_MEMORY) as usize,
        core::mem::size_of::<[FrameMemory; FRAME_SLOT_COUNT]>(),
    );
}

pub(crate) fn present() -> bool {
    let present_state = crate::arch::framebuffer_present_snapshot();
    let cursor_state = crate::arch::framebuffer_cursor_snapshot();
    if !DEVICE_READY.load(Ordering::Acquire) {
        if DEVICE_ATTEMPTED.swap(true, Ordering::AcqRel) {
            return false;
        }
        let Some(resources) = crate::arch::boot_resources() else { return false };
        let Some(framebuffer) = resources.framebuffer() else { return false };
        let device = match VirtioGpuDevice::initialize(framebuffer, present_state, cursor_state) {
            Ok(device) => device,
            Err(error) => {
                #[cfg(feature = "qemu-proof")]
                gpu_error_proof_line(error);
                #[cfg(not(feature = "qemu-proof"))]
                let _ = error;
                return false;
            }
        };
        unsafe { core::ptr::addr_of_mut!(DEVICE).write(MaybeUninit::new(device)) };
        DEVICE_READY.store(true, Ordering::Release);
        crate::arch::set_hardware_cursor(true);
        #[cfg(feature = "qemu-proof")]
        crate::arch_proof_line(b"LogOS vNext: VirtIO GPU scanout ready");
    }
    let result = with_device_mut(|device| {
        let framebuffer = crate::arch::boot_resources()
            .and_then(|resources| resources.framebuffer())
            .ok_or(GpuError::InvalidFramebuffer)?;
        device.present(present_state, cursor_state, framebuffer)
    });
    if result.is_err() {
        crate::arch::set_hardware_cursor(false);
    }
    result.is_ok()
}

fn with_device_mut<T>(
    operation: impl FnOnce(&mut VirtioGpuDevice) -> Result<T, GpuError>,
) -> Result<T, GpuError> {
    if !DEVICE_READY.load(Ordering::Acquire) {
        return Err(GpuError::NotFound);
    }
    if DEVICE_BUSY.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).is_err() {
        return Err(GpuError::Busy);
    }
    let result =
        unsafe { operation(&mut *core::ptr::addr_of_mut!(DEVICE).cast::<VirtioGpuDevice>()) };
    DEVICE_BUSY.store(false, Ordering::Release);
    result
}

fn cursor_mask_has(row: i32, column: i32) -> bool {
    (0..24).contains(&row)
        && (0..24).contains(&column)
        && CURSOR_MASK[row as usize] & (1 << column) != 0
}

fn initialize_cursor_memory() {
    let memory = unsafe { &mut (*core::ptr::addr_of_mut!(CURSOR_MEMORY)).0 };
    memory.fill(0);
    for row in 0..24i32 {
        for column in 0..24i32 {
            let mut outline = false;
            for offset_y in -1..=1 {
                for offset_x in -1..=1 {
                    outline |= cursor_mask_has(row + offset_y, column + offset_x);
                }
            }
            if !outline {
                continue;
            }
            let offset = (row as usize * 24 + column as usize) * 4;
            let pixel = if cursor_mask_has(row, column) {
                [0xff, 0xff, 0xff, 0xff]
            } else {
                [0x20, 0x18, 0x10, 0xff]
            };
            memory[offset..offset + 4].copy_from_slice(&pixel);
        }
    }
}

impl VirtioGpuDevice {
    fn initialize(
        framebuffer: crate::boot_resources::FramebufferInfo,
        present_state: Option<(
            u32,
            bool,
            [logos_abi::GuiRect; logos_abi::MAX_DISPLAY_PRESENT_RECTS],
        )>,
        cursor_state: Option<(u32, bool, i16, i16)>,
    ) -> Result<Self, GpuError> {
        if framebuffer.format() != crate::boot_resources::PixelFormat::Bgr8 {
            return Err(GpuError::InvalidFramebuffer);
        }
        let width = framebuffer.width();
        let height = framebuffer.height();
        let stride = framebuffer.stride();
        let bytes = u64::from(stride)
            .checked_mul(u64::from(height))
            .and_then(|pixels| pixels.checked_mul(4))
            .filter(|bytes| *bytes <= VIRTIO_GPU_MAX_BACKING_BYTES)
            .and_then(|bytes| u32::try_from(bytes).ok())
            .ok_or(GpuError::InvalidFramebuffer)?;
        let framebuffer_rect =
            VirtioGpuRect::new(0, 0, width, height).ok_or(GpuError::InvalidFramebuffer)?;
        let transfer =
            VirtioGpuRect::new(0, 0, stride, height).ok_or(GpuError::InvalidFramebuffer)?;
        let probe = PciConfig::find().ok_or(GpuError::NotFound)?;
        PciConfig::enable_device(probe.address);
        let common = region_for(&probe, probe.capabilities.common)?;
        let notify = region_for(&probe, probe.capabilities.notify)?;
        let queue = unsafe {
            let queue = &mut *core::ptr::addr_of_mut!(QUEUE_MEMORY);
            *queue = QueueMemory::new();
            queue
        };
        let cursor_queue = unsafe {
            let queue = &mut *core::ptr::addr_of_mut!(CURSOR_QUEUE_MEMORY);
            *queue = QueueMemory::new();
            queue
        };
        let mut device = Self {
            common,
            notify,
            notify_multiplier: probe.capabilities.notify_multiplier,
            queue,
            cursor_queue,
            negotiated_features: VIRTIO_F_VERSION_1,
            framebuffer: framebuffer_rect,
            transfer,
            next_fence: 1,
            frame_queue: FrameQueue::new(),
            pending_frame: None,
            frame_bytes: bytes,
            frame_stride: stride as usize * 4,
            last_present_sequence: None,
            last_cursor_sequence: None,
            cursor_initialized: false,
        };
        device.reset()?;
        unsafe {
            device.common.write_u8(COMMON_DEVICE_STATUS, STATUS_ACKNOWLEDGE)?;
            device.common.write_u8(COMMON_DEVICE_STATUS, STATUS_ACKNOWLEDGE | STATUS_DRIVER)?;
        }
        let features = device.device_features()?;
        if features & VIRTIO_F_VERSION_1 == 0 {
            return Err(GpuError::FeatureNegotiation);
        }
        device.driver_features(device.negotiated_features)?;
        unsafe {
            device.common.write_u8(
                COMMON_DEVICE_STATUS,
                STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK,
            )?;
            if device.common.read_u8(COMMON_DEVICE_STATUS)? & STATUS_FEATURES_OK == 0 {
                return Err(GpuError::DeviceRejectedFeatures);
            }
        }
        device.configure_queue(0)?;
        device.configure_queue(1)?;
        unsafe {
            device.common.write_u8(
                COMMON_DEVICE_STATUS,
                STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK | STATUS_DRIVER_OK,
            )?;
        }
        device.command(VirtioGpuCommand::ResourceCreate2d {
            resource_id: RESOURCE_ID,
            format: VIRTIO_GPU_FORMAT_B8G8R8X8_UNORM,
            width: stride,
            height,
        })?;
        device.command(VirtioGpuCommand::ResourceAttachBacking {
            resource_id: RESOURCE_ID,
            address: unsafe { core::ptr::addr_of!(FRAME_MEMORY[0]) as u64 },
            length: bytes,
        })?;
        device.command(VirtioGpuCommand::ResourceCreate2d {
            resource_id: SECONDARY_RESOURCE_ID,
            format: VIRTIO_GPU_FORMAT_B8G8R8X8_UNORM,
            width: stride,
            height,
        })?;
        device.command(VirtioGpuCommand::ResourceAttachBacking {
            resource_id: SECONDARY_RESOURCE_ID,
            address: unsafe { core::ptr::addr_of!(FRAME_MEMORY[1]) as u64 },
            length: bytes,
        })?;
        let _ = cursor_state.ok_or(GpuError::InvalidFramebuffer)?;
        #[cfg(feature = "qemu-proof")]
        crate::arch_proof_line(b"LogOS vNext: VirtIO GPU cursor memory");
        initialize_cursor_memory();
        device.command(VirtioGpuCommand::ResourceCreate2d {
            resource_id: CURSOR_RESOURCE_ID,
            format: VIRTIO_GPU_FORMAT_B8G8R8A8_UNORM,
            width: logos_abi::FRAMEBUFFER_CURSOR_WIDTH as u32,
            height: logos_abi::FRAMEBUFFER_CURSOR_HEIGHT as u32,
        })?;
        #[cfg(feature = "qemu-proof")]
        crate::arch_proof_line(b"LogOS vNext: VirtIO GPU cursor resource");
        device.command(VirtioGpuCommand::ResourceAttachBacking {
            resource_id: CURSOR_RESOURCE_ID,
            address: core::ptr::addr_of!(CURSOR_MEMORY) as u64,
            length: CURSOR_BACKING_BYTES,
        })?;
        #[cfg(feature = "qemu-proof")]
        crate::arch_proof_line(b"LogOS vNext: VirtIO GPU cursor backing");
        device.command(VirtioGpuCommand::SetScanout {
            scanout_id: SCANOUT_ID,
            resource_id: RESOURCE_ID,
            rect: framebuffer_rect,
        })?;
        #[cfg(feature = "qemu-proof")]
        crate::arch_proof_line(b"LogOS vNext: VirtIO GPU scanout");
        device.copy_frame(framebuffer, 0, true, &[])?;
        device.transfer_rect(device.transfer, device.framebuffer, RESOURCE_ID)?;
        device.frame_queue.present_initial(0, present_state.map(|state| state.0).unwrap_or(0));
        device.last_present_sequence = present_state.map(|state| state.0);
        device.present(present_state, cursor_state, framebuffer)?;
        Ok(device)
    }

    fn reset(&mut self) -> Result<(), GpuError> {
        unsafe { self.common.write_u8(COMMON_DEVICE_STATUS, 0)? };
        for _ in 0..1024 {
            if unsafe { self.common.read_u8(COMMON_DEVICE_STATUS)? } == 0 {
                return Ok(());
            }
            core::hint::spin_loop();
        }
        Err(GpuError::Timeout)
    }

    fn device_features(&self) -> Result<u64, GpuError> {
        unsafe {
            self.common.write_u32(COMMON_DEVICE_FEATURE_SELECT, 0)?;
            let low = self.common.read_u32(COMMON_DEVICE_FEATURE)? as u64;
            self.common.write_u32(COMMON_DEVICE_FEATURE_SELECT, 1)?;
            let high = self.common.read_u32(COMMON_DEVICE_FEATURE)? as u64;
            Ok(low | high << 32)
        }
    }

    fn driver_features(&self, features: u64) -> Result<(), GpuError> {
        unsafe {
            self.common.write_u32(COMMON_DRIVER_FEATURE_SELECT, 0)?;
            self.common.write_u32(COMMON_DRIVER_FEATURE, features as u32)?;
            self.common.write_u32(COMMON_DRIVER_FEATURE_SELECT, 1)?;
            self.common.write_u32(COMMON_DRIVER_FEATURE, (features >> 32) as u32)
        }
    }

    fn configure_queue(&mut self, index: u16) -> Result<(), GpuError> {
        let queue = if index == 0 {
            self.queue as *mut QueueMemory
        } else {
            self.cursor_queue as *mut QueueMemory
        };
        unsafe {
            self.common.write_u16(COMMON_QUEUE_SELECT, index)?;
            if self.common.read_u16(COMMON_QUEUE_SIZE)? < QUEUE_SIZE as u16 {
                return Err(GpuError::QueueUnavailable);
            }
            self.common.write_u16(COMMON_QUEUE_SIZE, QUEUE_SIZE as u16)?;
            let queue = &mut *queue;
            self.common.write_u64(COMMON_QUEUE_DESC, queue.descriptors.as_ptr() as u64)?;
            self.common.write_u64(
                COMMON_QUEUE_DRIVER,
                core::ptr::addr_of!(queue.available_flags) as u64,
            )?;
            self.common
                .write_u64(COMMON_QUEUE_DEVICE, core::ptr::addr_of!(queue.used_flags) as u64)?;
            self.common.write_u16(COMMON_QUEUE_ENABLE, 1)
        }
    }

    fn command(&mut self, command: VirtioGpuCommand) -> Result<(), GpuError> {
        let completion = Self::submit_command(
            self.common,
            self.notify,
            self.notify_multiplier,
            self.queue,
            0,
            &mut self.next_fence,
            command,
        )?;
        Self::wait_command(self.queue, completion)
    }

    fn cursor_command(&mut self, command: VirtioGpuCommand) -> Result<(), GpuError> {
        let completion = Self::submit_command(
            self.common,
            self.notify,
            self.notify_multiplier,
            self.cursor_queue,
            1,
            &mut self.next_fence,
            command,
        )?;
        Self::wait_command(self.cursor_queue, completion)
    }

    fn submit_command(
        common: MmioRegion,
        notify: MmioRegion,
        notify_multiplier: u32,
        queue: &mut QueueMemory,
        queue_index: u16,
        next_fence: &mut u64,
        command: VirtioGpuCommand,
    ) -> Result<u16, GpuError> {
        let fence_id = *next_fence;
        *next_fence = next_fence.checked_add(1).ok_or(GpuError::Timeout)?;
        let length = command
            .encode(fence_id, &mut queue.request)
            .map_err(|_| GpuError::InvalidFramebuffer)?;
        queue.response.fill(0xff);
        queue.descriptors[0] = Descriptor {
            address: queue.request.as_ptr() as u64,
            length: length as u32,
            flags: VIRTQ_DESC_F_NEXT,
            next: 1,
        };
        queue.descriptors[1] = Descriptor {
            address: queue.response.as_mut_ptr() as u64,
            length: queue.response.len() as u32,
            flags: VIRTQ_DESC_F_WRITE,
            next: 0,
        };
        let available = unsafe { read_volatile(&queue.available_index) };
        queue.available_ring[usize::from(available) % QUEUE_SIZE] = 0;
        fence(Ordering::Release);
        unsafe { write_volatile(&mut queue.available_index, available.wrapping_add(1)) };
        let notify_offset = u64::from(unsafe {
            common.write_u16(COMMON_QUEUE_SELECT, queue_index)?;
            common.read_u16(COMMON_QUEUE_NOTIFY_OFF)?
        });
        let notify_delta = notify_offset
            .checked_mul(u64::from(notify_multiplier))
            .and_then(|offset| usize::try_from(offset).ok())
            .ok_or(GpuError::InvalidBar)?;
        unsafe { notify.write_u16(notify_delta, queue_index)? };
        Ok(available.wrapping_add(1))
    }

    fn wait_command(queue: &mut QueueMemory, completion: u16) -> Result<(), GpuError> {
        for _ in 0..COMPLETION_SPIN_LIMIT {
            if Self::poll_command(queue, completion)? {
                return Ok(());
            }
            core::hint::spin_loop();
        }
        Err(GpuError::Timeout)
    }

    fn poll_command(queue: &mut QueueMemory, completion: u16) -> Result<bool, GpuError> {
        let used = unsafe { read_volatile(&queue.used_index) };
        if used != completion {
            return Ok(false);
        }
        let element = queue.used_ring[usize::from(completion.wrapping_sub(1)) % QUEUE_SIZE];
        if element.id != 0 {
            return Err(GpuError::Timeout);
        }
        let response_type = u32::from_le_bytes(queue.response[..4].try_into().unwrap());
        if !response_is_ok(response_type) {
            return Err(GpuError::Device(response_type));
        }
        Ok(true)
    }

    fn present(
        &mut self,
        present_state: Option<(
            u32,
            bool,
            [logos_abi::GuiRect; logos_abi::MAX_DISPLAY_PRESENT_RECTS],
        )>,
        cursor_state: Option<(u32, bool, i16, i16)>,
        framebuffer: crate::boot_resources::FramebufferInfo,
    ) -> Result<(), GpuError> {
        self.poll_pending_frame()?;
        let present_sequence = present_state.map(|state| state.0);
        let frame_changed = self.last_present_sequence != present_sequence;
        if !frame_changed
            && self.pending_frame.is_none()
            && cursor_state.is_none_or(|state| self.last_cursor_sequence == Some(state.0))
        {
            #[cfg(feature = "qemu-proof")]
            if !GPU_PROOF_IDLE_SUPPRESSED.swap(true, Ordering::AcqRel) {
                crate::arch_proof_line(b"LogOS vNext: VirtIO GPU idle present suppressed");
            }
            return Ok(());
        }
        if frame_changed && self.pending_frame.is_none() {
            if let Some(sequence) = present_sequence {
                self.start_frame(framebuffer, sequence, present_state)?;
            }
        }
        if let Some((sequence, visible, x, y)) = cursor_state {
            if self.last_cursor_sequence != Some(sequence) {
                #[cfg(feature = "qemu-proof")]
                let cursor_update = visible && !self.cursor_initialized;
                let command = if visible {
                    if self.cursor_initialized {
                        VirtioGpuCommand::MoveCursor { x, y, scanout_id: SCANOUT_ID }
                    } else {
                        self.cursor_initialized = true;
                        VirtioGpuCommand::UpdateCursor {
                            x,
                            y,
                            scanout_id: SCANOUT_ID,
                            resource_id: CURSOR_RESOURCE_ID,
                            hot_x: 0,
                            hot_y: 0,
                        }
                    }
                } else if self.cursor_initialized {
                    self.cursor_initialized = false;
                    VirtioGpuCommand::UpdateCursor {
                        x: 0,
                        y: 0,
                        scanout_id: SCANOUT_ID,
                        resource_id: 0,
                        hot_x: 0,
                        hot_y: 0,
                    }
                } else {
                    self.last_cursor_sequence = Some(sequence);
                    return Ok(());
                };
                self.cursor_command(command)?;
                self.last_cursor_sequence = Some(sequence);
                #[cfg(feature = "qemu-proof")]
                if cursor_update {
                    crate::arch_proof_line(b"LogOS vNext: VirtIO GPU cursor ready");
                } else if visible {
                    crate::arch_proof_line(b"LogOS vNext: VirtIO GPU cursor moved");
                }
            }
        }
        Ok(())
    }

    fn start_frame(
        &mut self,
        framebuffer: crate::boot_resources::FramebufferInfo,
        sequence: u32,
        present_state: Option<(
            u32,
            bool,
            [logos_abi::GuiRect; logos_abi::MAX_DISPLAY_PRESENT_RECTS],
        )>,
    ) -> Result<(), GpuError> {
        let full = present_state
            .is_none_or(|(_, full, rects)| full || rects.iter().all(|rect| rect.is_empty()))
            || self.last_present_sequence.is_none();
        let mut rects = [FrameRect { transfer: self.transfer, flush: self.framebuffer };
            logos_abi::MAX_DISPLAY_PRESENT_RECTS];
        let rect_count = if full {
            1
        } else if let Some((_, _, damage)) = present_state {
            let mut count = 0;
            for rect in damage.iter().copied().filter(|rect| !rect.is_empty()) {
                let rect = self.present_rect(rect)?;
                rects[count] = FrameRect { transfer: rect, flush: rect };
                count += 1;
            }
            count
        } else {
            0
        };
        if rect_count == 0 {
            return Ok(());
        }
        let lease = self.frame_queue.acquire(sequence).map_err(|error| match error {
            FrameQueueError::Full => GpuError::Busy,
            FrameQueueError::InvalidLease | FrameQueueError::StaleLease => GpuError::Timeout,
        })?;
        self.copy_frame(framebuffer, lease.slot, full, &rects[..rect_count])?;
        let completion = self.submit_graphics(VirtioGpuCommand::TransferToHost2d {
            resource_id: frame_resource_id(lease.slot),
            rect: rects[0].transfer,
        })?;
        self.frame_queue.submit(lease).map_err(|_| GpuError::Timeout)?;
        self.pending_frame = Some(PendingFrame {
            lease,
            phase: FramePhase::Transfer,
            completion,
            rects,
            rect_count,
            rect_index: 0,
        });
        Ok(())
    }

    fn poll_pending_frame(&mut self) -> Result<(), GpuError> {
        let Some(mut pending) = self.pending_frame else { return Ok(()) };
        if !Self::poll_command(self.queue, pending.completion)? {
            return Ok(());
        }
        match pending.phase {
            FramePhase::Transfer => {
                pending.phase = FramePhase::Flush;
                pending.completion = self.submit_graphics(VirtioGpuCommand::ResourceFlush {
                    resource_id: frame_resource_id(pending.lease.slot),
                    rect: pending.rects[pending.rect_index].flush,
                })?;
                self.pending_frame = Some(pending);
            }
            FramePhase::Flush if pending.rect_index + 1 < pending.rect_count => {
                pending.rect_index += 1;
                pending.phase = FramePhase::Transfer;
                pending.completion = self.submit_graphics(VirtioGpuCommand::TransferToHost2d {
                    resource_id: frame_resource_id(pending.lease.slot),
                    rect: pending.rects[pending.rect_index].transfer,
                })?;
                self.pending_frame = Some(pending);
            }
            FramePhase::Flush => {
                pending.phase = FramePhase::Scanout;
                pending.completion = self.submit_graphics(VirtioGpuCommand::SetScanout {
                    scanout_id: SCANOUT_ID,
                    resource_id: frame_resource_id(pending.lease.slot),
                    rect: self.framebuffer,
                })?;
                self.pending_frame = Some(pending);
            }
            FramePhase::Scanout => {
                self.frame_queue.complete(pending.lease).map_err(|_| GpuError::Timeout)?;
                self.last_present_sequence = Some(pending.lease.sequence);
                self.pending_frame = None;
                #[cfg(feature = "qemu-proof")]
                crate::arch_proof_line(b"LogOS vNext: VirtIO GPU frame present");
            }
        }
        Ok(())
    }

    fn copy_frame(
        &self,
        framebuffer: crate::boot_resources::FramebufferInfo,
        slot: usize,
        full: bool,
        rects: &[FrameRect],
    ) -> Result<(), GpuError> {
        let source = unsafe {
            core::slice::from_raw_parts(framebuffer.base() as *const u8, self.frame_bytes as usize)
        };
        let destination = unsafe {
            let frame = &mut *core::ptr::addr_of_mut!(FRAME_MEMORY[slot]);
            &mut frame.0[..self.frame_bytes as usize]
        };
        if full {
            destination.copy_from_slice(source);
            return Ok(());
        }
        for rect in rects {
            for row in 0..rect.transfer.height as usize {
                let y = rect.transfer.y as usize + row;
                let x = rect.transfer.x as usize * 4;
                let width = rect.transfer.width as usize * 4;
                let offset =
                    y.checked_mul(self.frame_stride).and_then(|offset| offset.checked_add(x));
                let Some(offset) = offset else { return Err(GpuError::InvalidFramebuffer) };
                let end = offset.checked_add(width).ok_or(GpuError::InvalidFramebuffer)?;
                if end > source.len() || end > destination.len() {
                    return Err(GpuError::InvalidFramebuffer);
                }
                destination[offset..end].copy_from_slice(&source[offset..end]);
            }
        }
        Ok(())
    }

    fn submit_graphics(&mut self, command: VirtioGpuCommand) -> Result<u16, GpuError> {
        Self::submit_command(
            self.common,
            self.notify,
            self.notify_multiplier,
            self.queue,
            0,
            &mut self.next_fence,
            command,
        )
    }

    fn transfer_rect(
        &mut self,
        transfer: VirtioGpuRect,
        flush: VirtioGpuRect,
        resource_id: u32,
    ) -> Result<(), GpuError> {
        self.command(VirtioGpuCommand::TransferToHost2d { resource_id, rect: transfer })?;
        self.command(VirtioGpuCommand::ResourceFlush { resource_id, rect: flush })
    }

    fn present_rect(&self, rect: logos_abi::GuiRect) -> Result<VirtioGpuRect, GpuError> {
        if rect.x < 0 || rect.y < 0 {
            return Err(GpuError::InvalidFramebuffer);
        }
        let rect = VirtioGpuRect::new(rect.x as u32, rect.y as u32, rect.width, rect.height)
            .ok_or(GpuError::InvalidFramebuffer)?;
        if rect.x + rect.width > self.framebuffer.width
            || rect.y + rect.height > self.framebuffer.height
        {
            return Err(GpuError::InvalidFramebuffer);
        }
        Ok(rect)
    }
}

#[derive(Clone, Copy)]
struct PciProbe {
    address: PciAddress,
    capabilities: logos_storage::VirtioPciCapabilities,
    bars: [PciBar; 6],
}

#[derive(Clone, Copy)]
struct PciBar {
    base: u64,
    length: u64,
}

impl PciBar {
    const EMPTY: Self = Self { base: 0, length: 0 };
}

fn region_for(
    probe: &PciProbe,
    capability: logos_storage::VirtioPciCapability,
) -> Result<MmioRegion, GpuError> {
    let bar = *probe.bars.get(capability.bar as usize).ok_or(GpuError::MissingBar)?;
    if bar.base == 0
        || u64::from(capability.offset)
            .checked_add(u64::from(capability.length))
            .is_none_or(|end| end > bar.length)
    {
        return Err(GpuError::MissingBar);
    }
    let address = bar.base.checked_add(u64::from(capability.offset)).ok_or(GpuError::InvalidBar)?;
    MmioRegion::new(address, capability.length)
}

struct PciConfig;

impl PciConfig {
    fn find() -> Option<PciProbe> {
        for bus in 0..=u8::MAX {
            for device in 0..32u8 {
                for function in 0..8u8 {
                    let address = PciAddress::new(bus, device, function)?;
                    let config = Self::read_config(address);
                    let vendor = u16::from_le_bytes([config[0], config[1]]);
                    if vendor == 0xffff {
                        continue;
                    }
                    if VirtioPciDevice::from_config_for_device(
                        address,
                        &config,
                        logos_storage::VIRTIO_GPU_MODERN_DEVICE_ID,
                    )
                    .is_ok()
                    {
                        return Some(PciProbe {
                            address,
                            capabilities: VirtioPciDevice::from_config_for_device(
                                address,
                                &config,
                                logos_storage::VIRTIO_GPU_MODERN_DEVICE_ID,
                            )
                            .ok()?
                            .capabilities,
                            bars: Self::bars(address, &config),
                        });
                    }
                }
            }
        }
        None
    }

    fn read_config(address: PciAddress) -> [u8; PCI_CONFIG_BYTES] {
        let mut config = [0; PCI_CONFIG_BYTES];
        for offset in (0..PCI_CONFIG_BYTES).step_by(4) {
            let value = unsafe { config_read(address, offset as u8) };
            config[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        config
    }

    fn bars(address: PciAddress, config: &[u8; PCI_CONFIG_BYTES]) -> [PciBar; 6] {
        let mut bars = [PciBar::EMPTY; 6];
        let mut index = 0;
        while index < bars.len() {
            let offset = 0x10 + index * 4;
            let low = u32::from_le_bytes(config[offset..offset + 4].try_into().unwrap());
            if low & 1 != 0 {
                index += 1;
                continue;
            }
            let is_64 = low & 0x6 == 0x4 && index < 5;
            let high = if is_64 {
                u32::from_le_bytes(config[offset + 4..offset + 8].try_into().unwrap())
            } else {
                0
            };
            unsafe { config_write(address, offset as u8, u32::MAX) };
            if is_64 {
                unsafe { config_write(address, (offset + 4) as u8, u32::MAX) };
            }
            let mask_low = unsafe { config_read(address, offset as u8) };
            let mask_high =
                if is_64 { unsafe { config_read(address, (offset + 4) as u8) } } else { 0 };
            unsafe { config_write(address, offset as u8, low) };
            if is_64 {
                unsafe { config_write(address, (offset + 4) as u8, high) };
            }
            let base = if is_64 {
                (u64::from(high) << 32) | u64::from(low & !0xf)
            } else {
                u64::from(low & !0xf)
            };
            let mask = if is_64 {
                (u64::from(mask_high) << 32) | u64::from(mask_low & !0xf)
            } else {
                u64::from(mask_low & !0xf)
            };
            let length = (!mask).wrapping_add(1);
            if base != 0 && length != 0 {
                bars[index] = PciBar { base, length };
            }
            index += if is_64 { 2 } else { 1 };
        }
        bars
    }

    fn enable_device(address: PciAddress) {
        let command = unsafe { config_read_u16(address, 0x04) };
        unsafe { config_write_u16(address, 0x04, command | 0x0006) };
    }
}

unsafe fn config_read(address: PciAddress, offset: u8) -> u32 {
    let value = 0x8000_0000u32
        | (u32::from(address.bus()) << 16)
        | (u32::from(address.device()) << 11)
        | (u32::from(address.function()) << 8)
        | u32::from(offset & 0xfc);
    unsafe {
        outl(PCI_CONFIG_ADDRESS, value);
        inl(PCI_CONFIG_DATA)
    }
}

unsafe fn config_read_u16(address: PciAddress, offset: u8) -> u16 {
    let value = unsafe { config_read(address, offset & 0xfc) };
    ((value >> (u32::from(offset & 2) * 8)) & 0xffff) as u16
}

unsafe fn config_write_u16(address: PciAddress, offset: u8, value: u16) {
    let aligned = offset & 0xfc;
    let current = unsafe { config_read(address, aligned) };
    let shift = u32::from(offset & 2) * 8;
    let mask = 0xffffu32 << shift;
    unsafe { config_write(address, aligned, (current & !mask) | (u32::from(value) << shift)) };
}

unsafe fn config_write(address: PciAddress, offset: u8, value: u32) {
    let address_value = 0x8000_0000u32
        | (u32::from(address.bus()) << 16)
        | (u32::from(address.device()) << 11)
        | (u32::from(address.function()) << 8)
        | u32::from(offset & 0xfc);
    unsafe {
        outl(PCI_CONFIG_ADDRESS, address_value);
        outl(PCI_CONFIG_DATA, value);
    }
}

unsafe fn outl(port: u16, value: u32) {
    unsafe {
        asm!("out dx, eax", in("dx") port, in("eax") value, options(nomem, nostack, preserves_flags))
    };
}

unsafe fn inl(port: u16) -> u32 {
    let value: u32;
    unsafe {
        asm!("in eax, dx", in("dx") port, out("eax") value, options(nomem, nostack, preserves_flags))
    };
    value
}
