use core::{
    arch::asm,
    mem::MaybeUninit,
    ptr::{read_volatile, write_volatile},
    sync::atomic::{AtomicBool, Ordering, fence},
};

use logos_storage::{
    PCI_CONFIG_BYTES, PciAddress, VIRTIO_F_VERSION_1, VIRTIO_GPU_FORMAT_B8G8R8X8_UNORM,
    VIRTIO_GPU_MAX_BACKING_BYTES, VIRTIO_GPU_MAX_COMMAND_BYTES, VirtioGpuCommand, VirtioGpuRect,
    VirtioPciDevice, response_is_ok,
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
const SCANOUT_ID: u32 = 0;

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
    negotiated_features: u64,
    framebuffer: VirtioGpuRect,
    transfer: VirtioGpuRect,
    next_fence: u64,
    last_present_sequence: Option<u32>,
}

pub(crate) fn reserve_frames(pool: &mut crate::frame_pool::FramePool) {
    crate::arch::reserve_storage_frames(
        pool,
        core::ptr::addr_of!(QUEUE_MEMORY) as usize,
        core::mem::size_of::<QueueMemory>(),
    );
    crate::arch::reserve_storage_frames(
        pool,
        core::ptr::addr_of!(DEVICE) as usize,
        core::mem::size_of::<MaybeUninit<VirtioGpuDevice>>(),
    );
}

pub(crate) fn present() -> bool {
    let present_state = crate::arch::framebuffer_present_snapshot();
    if !DEVICE_READY.load(Ordering::Acquire) {
        if DEVICE_ATTEMPTED.swap(true, Ordering::AcqRel) {
            return false;
        }
        let Some(resources) = crate::arch::boot_resources() else { return false };
        let Some(framebuffer) = resources.framebuffer() else { return false };
        let Ok(device) = VirtioGpuDevice::initialize(framebuffer, present_state) else {
            return false;
        };
        unsafe { core::ptr::addr_of_mut!(DEVICE).write(MaybeUninit::new(device)) };
        DEVICE_READY.store(true, Ordering::Release);
        #[cfg(feature = "qemu-proof")]
        crate::arch_proof_line(b"LogOS vNext: VirtIO GPU scanout ready");
    }
    with_device_mut(|device| device.present(present_state)).is_ok()
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

impl VirtioGpuDevice {
    fn initialize(
        framebuffer: crate::boot_resources::FramebufferInfo,
        present_state: Option<(
            u32,
            bool,
            [logos_abi::GuiRect; logos_abi::MAX_DISPLAY_PRESENT_RECTS],
        )>,
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
        let mut device = Self {
            common,
            notify,
            notify_multiplier: probe.capabilities.notify_multiplier,
            queue,
            negotiated_features: VIRTIO_F_VERSION_1,
            framebuffer: framebuffer_rect,
            transfer,
            next_fence: 1,
            last_present_sequence: None,
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
        device.configure_queue()?;
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
            address: framebuffer.base(),
            length: bytes,
        })?;
        device.command(VirtioGpuCommand::SetScanout {
            scanout_id: SCANOUT_ID,
            resource_id: RESOURCE_ID,
            rect: framebuffer_rect,
        })?;
        device.present(present_state)?;
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

    fn configure_queue(&mut self) -> Result<(), GpuError> {
        unsafe {
            self.common.write_u16(COMMON_QUEUE_SELECT, 0)?;
            if self.common.read_u16(COMMON_QUEUE_SIZE)? < QUEUE_SIZE as u16 {
                return Err(GpuError::QueueUnavailable);
            }
            self.common.write_u16(COMMON_QUEUE_SIZE, QUEUE_SIZE as u16)?;
            self.common.write_u64(COMMON_QUEUE_DESC, self.queue.descriptors.as_ptr() as u64)?;
            self.common.write_u64(
                COMMON_QUEUE_DRIVER,
                core::ptr::addr_of!(self.queue.available_flags) as u64,
            )?;
            self.common.write_u64(
                COMMON_QUEUE_DEVICE,
                core::ptr::addr_of!(self.queue.used_flags) as u64,
            )?;
            self.common.write_u16(COMMON_QUEUE_ENABLE, 1)
        }
    }

    fn command(&mut self, command: VirtioGpuCommand) -> Result<(), GpuError> {
        let fence_id = self.next_fence;
        self.next_fence = self.next_fence.checked_add(1).ok_or(GpuError::Timeout)?;
        let length = command
            .encode(fence_id, &mut self.queue.request)
            .map_err(|_| GpuError::InvalidFramebuffer)?;
        self.queue.response.fill(0xff);
        self.queue.descriptors[0] = Descriptor {
            address: self.queue.request.as_ptr() as u64,
            length: length as u32,
            flags: VIRTQ_DESC_F_NEXT,
            next: 1,
        };
        self.queue.descriptors[1] = Descriptor {
            address: self.queue.response.as_mut_ptr() as u64,
            length: self.queue.response.len() as u32,
            flags: VIRTQ_DESC_F_WRITE,
            next: 0,
        };
        let available = unsafe { read_volatile(&self.queue.available_index) };
        self.queue.available_ring[usize::from(available) % QUEUE_SIZE] = 0;
        fence(Ordering::Release);
        unsafe { write_volatile(&mut self.queue.available_index, available.wrapping_add(1)) };
        let notify_offset = u64::from(unsafe { self.common.read_u16(COMMON_QUEUE_NOTIFY_OFF)? });
        let notify_delta = notify_offset
            .checked_mul(u64::from(self.notify_multiplier))
            .and_then(|offset| usize::try_from(offset).ok())
            .ok_or(GpuError::InvalidBar)?;
        unsafe { self.notify.write_u16(notify_delta, 0)? };
        for _ in 0..COMPLETION_SPIN_LIMIT {
            let used = unsafe { read_volatile(&self.queue.used_index) };
            if used != available.wrapping_add(1) {
                core::hint::spin_loop();
                continue;
            }
            let element = self.queue.used_ring[usize::from(available) % QUEUE_SIZE];
            if element.id != 0 {
                return Err(GpuError::Timeout);
            }
            let response_type = u32::from_le_bytes(self.queue.response[..4].try_into().unwrap());
            if !response_is_ok(response_type) {
                return Err(GpuError::Device(response_type));
            }
            return Ok(());
        }
        Err(GpuError::Timeout)
    }

    fn present(
        &mut self,
        present_state: Option<(
            u32,
            bool,
            [logos_abi::GuiRect; logos_abi::MAX_DISPLAY_PRESENT_RECTS],
        )>,
    ) -> Result<(), GpuError> {
        let present_sequence = present_state.map(|state| state.0);
        if self.last_present_sequence == present_sequence {
            #[cfg(feature = "qemu-proof")]
            if !GPU_PROOF_IDLE_SUPPRESSED.swap(true, Ordering::AcqRel) {
                crate::arch_proof_line(b"LogOS vNext: VirtIO GPU idle present suppressed");
            }
            return Ok(());
        }
        let full = present_state
            .is_none_or(|(_, full, rects)| full || rects.iter().all(|rect| rect.is_empty()))
            || self.last_present_sequence.is_none();
        if full {
            self.transfer_rect(self.transfer, self.framebuffer)?;
        } else if let Some((_, _, rects)) = present_state {
            for rect in rects.iter().copied().filter(|rect| !rect.is_empty()) {
                let rect = self.present_rect(rect)?;
                self.transfer_rect(rect, rect)?;
            }
        }
        self.last_present_sequence = present_sequence;
        Ok(())
    }

    fn transfer_rect(
        &mut self,
        transfer: VirtioGpuRect,
        flush: VirtioGpuRect,
    ) -> Result<(), GpuError> {
        self.command(VirtioGpuCommand::TransferToHost2d {
            resource_id: RESOURCE_ID,
            rect: transfer,
        })?;
        self.command(VirtioGpuCommand::ResourceFlush { resource_id: RESOURCE_ID, rect: flush })
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
