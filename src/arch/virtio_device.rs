#![allow(dead_code)]

use core::{
    arch::asm,
    mem::MaybeUninit,
    ptr::{read_volatile, write_volatile},
    sync::atomic::{AtomicBool, Ordering, fence},
};

use logos_storage::{
    BlockRequestId, PciError, VirtioBlkChain, VirtioBlkHeader, VirtioPciDevice, negotiate_features,
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
const COMMON_QUEUE_DESC: usize = 0x20;
const COMMON_QUEUE_DRIVER: usize = 0x28;
const COMMON_QUEUE_DEVICE: usize = 0x30;
const ISR_CAP_OFFSET: usize = 0;

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

#[repr(C, align(4096))]
struct QueueMemory {
    descriptors: [Descriptor; QUEUE_SIZE * 3],
    available_flags: u16,
    available_index: u16,
    available_ring: [u16; QUEUE_SIZE],
    available_used_event: u16,
    used_flags: u16,
    used_index: u16,
    used_ring: [UsedElement; QUEUE_SIZE],
    used_available_event: u16,
    headers: [VirtioBlkHeader; QUEUE_SIZE],
    statuses: [u8; QUEUE_SIZE],
}

// One fixed, page-aligned Core-owned queue arena. The future frame allocator
// will replace this singleton when multiple block devices are supported.
#[unsafe(link_section = ".dma")]
static mut QUEUE_MEMORY: QueueMemory = QueueMemory::new();
static mut DEVICE: MaybeUninit<VirtioBlockDevice> = MaybeUninit::uninit();
static DEVICE_READY: AtomicBool = AtomicBool::new(false);

impl QueueMemory {
    const EMPTY_DESCRIPTOR: Descriptor = Descriptor { address: 0, length: 0, flags: 0, next: 0 };
    const EMPTY_USED: UsedElement = UsedElement { id: 0, length: 0 };
    const fn new() -> Self {
        Self {
            descriptors: [Self::EMPTY_DESCRIPTOR; QUEUE_SIZE * 3],
            available_flags: 0,
            available_index: 0,
            available_ring: [0; QUEUE_SIZE],
            available_used_event: 0,
            used_flags: 0,
            used_index: 0,
            used_ring: [Self::EMPTY_USED; QUEUE_SIZE],
            used_available_event: 0,
            headers: [VirtioBlkHeader { request_type: 0, reserved: 0, sector: 0 }; QUEUE_SIZE],
            statuses: [0xff; QUEUE_SIZE],
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeviceError {
    NotFound,
    Pci(PciError),
    MissingBar,
    InvalidBar,
    FeatureNegotiation,
    DeviceRejectedFeatures,
    QueueUnavailable,
    QueueFull,
    StaleCompletion,
    InvalidCompletion,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeviceCompletion {
    pub request_id: BlockRequestId,
    pub status: u8,
    pub bytes_written: u32,
}

#[derive(Clone, Copy)]
struct MmioRegion {
    address: u64,
    length: u32,
}

impl MmioRegion {
    fn new(address: u64, length: u32) -> Result<Self, DeviceError> {
        if address == 0 || address % 4 != 0 || length < 4 {
            return Err(DeviceError::InvalidBar);
        }
        Ok(Self { address, length })
    }

    fn ptr<T>(&self, offset: usize) -> Result<*mut T, DeviceError> {
        let size = core::mem::size_of::<T>();
        if offset.checked_add(size).is_none_or(|end| end > self.length as usize) {
            return Err(DeviceError::InvalidBar);
        }
        Ok((self.address as usize + offset) as *mut T)
    }

    unsafe fn read_u8(&self, offset: usize) -> Result<u8, DeviceError> {
        Ok(unsafe { read_volatile(self.ptr(offset)?) })
    }

    unsafe fn read_u16(&self, offset: usize) -> Result<u16, DeviceError> {
        Ok(unsafe { read_volatile(self.ptr(offset)?) })
    }

    unsafe fn read_u32(&self, offset: usize) -> Result<u32, DeviceError> {
        Ok(unsafe { read_volatile(self.ptr(offset)?) })
    }

    unsafe fn write_u8(&self, offset: usize, value: u8) -> Result<(), DeviceError> {
        unsafe { write_volatile(self.ptr(offset)?, value) };
        Ok(())
    }

    unsafe fn write_u16(&self, offset: usize, value: u16) -> Result<(), DeviceError> {
        unsafe { write_volatile(self.ptr(offset)?, value) };
        Ok(())
    }

    unsafe fn write_u32(&self, offset: usize, value: u32) -> Result<(), DeviceError> {
        unsafe { write_volatile(self.ptr(offset)?, value) };
        Ok(())
    }

    unsafe fn write_u64(&self, offset: usize, value: u64) -> Result<(), DeviceError> {
        unsafe { write_volatile(self.ptr(offset)?, value) };
        Ok(())
    }
}

pub struct VirtioBlockDevice {
    common: MmioRegion,
    notify: MmioRegion,
    isr: MmioRegion,
    notify_multiplier: u32,
    queue: &'static mut QueueMemory,
    requests: [Option<BlockRequestId>; QUEUE_SIZE],
    used_index: u16,
}

pub(crate) fn initialize_storage_device() -> bool {
    if DEVICE_READY.load(Ordering::Acquire) {
        return true;
    }
    let Ok(device) = VirtioBlockDevice::initialize(true) else {
        return false;
    };
    unsafe { core::ptr::addr_of_mut!(DEVICE).write(MaybeUninit::new(device)) };
    DEVICE_READY.store(true, Ordering::Release);
    true
}

impl VirtioBlockDevice {
    pub fn initialize(writable: bool) -> Result<Self, DeviceError> {
        let probe = PciConfig::find().ok_or(DeviceError::NotFound)?;
        let common = region_for(&probe, probe.capabilities.common)?;
        let notify = region_for(&probe, probe.capabilities.notify)?;
        let isr = region_for(&probe, probe.capabilities.isr)?;
        let queue = unsafe {
            let queue = &mut *core::ptr::addr_of_mut!(QUEUE_MEMORY);
            *queue = QueueMemory::new();
            queue
        };
        let mut device = Self {
            common,
            notify,
            isr,
            notify_multiplier: probe.capabilities.notify_multiplier,
            queue,
            requests: [None; QUEUE_SIZE],
            used_index: 0,
        };
        device.reset()?;
        unsafe { device.common.write_u8(COMMON_DEVICE_STATUS, STATUS_ACKNOWLEDGE)? };
        unsafe {
            device.common.write_u8(COMMON_DEVICE_STATUS, STATUS_ACKNOWLEDGE | STATUS_DRIVER)?
        };
        let features = device.device_features()?;
        let negotiated =
            negotiate_features(features, writable).map_err(|_| DeviceError::FeatureNegotiation)?;
        device.driver_features(negotiated.raw())?;
        unsafe {
            device.common.write_u8(
                COMMON_DEVICE_STATUS,
                STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK,
            )?
        };
        let status = unsafe { device.common.read_u8(COMMON_DEVICE_STATUS)? };
        if status & STATUS_FEATURES_OK == 0 {
            return Err(DeviceError::DeviceRejectedFeatures);
        }
        device.configure_queue()?;
        unsafe {
            device.common.write_u8(
                COMMON_DEVICE_STATUS,
                STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK | STATUS_DRIVER_OK,
            )?
        };
        Ok(device)
    }

    fn reset(&mut self) -> Result<(), DeviceError> {
        unsafe { self.common.write_u8(COMMON_DEVICE_STATUS, 0) }
    }

    fn device_features(&self) -> Result<u64, DeviceError> {
        unsafe {
            self.common.write_u32(COMMON_DEVICE_FEATURE_SELECT, 0)?;
            let low = self.common.read_u32(COMMON_DEVICE_FEATURE)? as u64;
            self.common.write_u32(COMMON_DEVICE_FEATURE_SELECT, 1)?;
            let high = self.common.read_u32(COMMON_DEVICE_FEATURE)? as u64;
            Ok(low | high << 32)
        }
    }

    fn driver_features(&self, features: u64) -> Result<(), DeviceError> {
        unsafe {
            self.common.write_u32(COMMON_DRIVER_FEATURE_SELECT, 0)?;
            self.common.write_u32(COMMON_DRIVER_FEATURE, features as u32)?;
            self.common.write_u32(COMMON_DRIVER_FEATURE_SELECT, 1)?;
            self.common.write_u32(COMMON_DRIVER_FEATURE, (features >> 32) as u32)
        }
    }

    fn configure_queue(&mut self) -> Result<(), DeviceError> {
        unsafe {
            self.common.write_u16(COMMON_QUEUE_SELECT, 0)?;
            if self.common.read_u16(COMMON_QUEUE_SIZE)? < QUEUE_SIZE as u16 {
                return Err(DeviceError::QueueUnavailable);
            }
            let descriptors = self.queue.descriptors.as_ptr() as u64;
            let driver = &self.queue.available_flags as *const u16 as u64;
            let device = &self.queue.used_flags as *const u16 as u64;
            self.common.write_u64(COMMON_QUEUE_DESC, descriptors)?;
            self.common.write_u64(COMMON_QUEUE_DRIVER, driver)?;
            self.common.write_u64(COMMON_QUEUE_DEVICE, device)?;
            self.common.write_u16(COMMON_QUEUE_ENABLE, 1)
        }
    }

    pub fn submit(&mut self, chain: VirtioBlkChain, data_address: u64) -> Result<(), DeviceError> {
        let used = unsafe { read_volatile(&self.queue.available_index) };
        let slot = (used as usize) % QUEUE_SIZE;
        if self.requests[slot].is_some() {
            return Err(DeviceError::QueueFull);
        }
        let first = slot * 3;
        self.queue.headers[slot] = chain.header;
        self.queue.statuses[slot] = 0xff;
        let header_address = &self.queue.headers[slot] as *const VirtioBlkHeader as u64;
        let status_address = &self.queue.statuses[slot] as *const u8 as u64;
        self.queue.descriptors[first] = Descriptor {
            address: header_address,
            length: core::mem::size_of::<VirtioBlkHeader>() as u32,
            flags: VIRTQ_DESC_F_NEXT,
            next: (first + 1) as u16,
        };
        let mut last = first + 1;
        if let Some(data) = chain.data {
            let flags =
                if data.device_writable { VIRTQ_DESC_F_WRITE } else { 0 } | VIRTQ_DESC_F_NEXT;
            self.queue.descriptors[last] = Descriptor {
                address: data_address,
                length: data.length,
                flags,
                next: (first + 2) as u16,
            };
            last += 1;
        }
        self.queue.descriptors[last] =
            Descriptor { address: status_address, length: 1, flags: VIRTQ_DESC_F_WRITE, next: 0 };
        self.queue.available_ring[slot] = first as u16;
        fence(Ordering::Release);
        self.queue.available_index = used.wrapping_add(1);
        let notify_offset = unsafe { self.common.read_u16(0x1e)? } as u64;
        let notify_address = self.notify.address + notify_offset * self.notify_multiplier as u64;
        unsafe { write_volatile(notify_address as *mut u16, 0) };
        self.requests[slot] = Some(chain.request_id);
        Ok(())
    }

    pub fn poll_completion(&mut self) -> Result<Option<DeviceCompletion>, DeviceError> {
        let used = unsafe { read_volatile(&self.queue.used_index) };
        if used == self.used_index {
            return Ok(None);
        }
        let slot = (self.used_index as usize) % QUEUE_SIZE;
        let element = unsafe { read_volatile(&self.queue.used_ring[slot]) };
        self.used_index = self.used_index.wrapping_add(1);
        let request_id = self
            .requests
            .get_mut(element.id as usize)
            .and_then(Option::take)
            .ok_or(DeviceError::StaleCompletion)?;
        let status = self.queue.statuses[element.id as usize];
        Ok(Some(DeviceCompletion { request_id, status, bytes_written: element.length }))
    }

    pub fn interrupt_status(&self) -> Result<u8, DeviceError> {
        unsafe { self.isr.read_u8(ISR_CAP_OFFSET) }
    }

    pub fn reset_device(&mut self) -> Result<(), DeviceError> {
        self.reset()?;
        self.requests.fill(None);
        self.used_index = 0;
        Ok(())
    }
}

fn region_for(
    probe: &PciProbe,
    capability: logos_storage::VirtioPciCapability,
) -> Result<MmioRegion, DeviceError> {
    let base = probe.bars[capability.bar as usize];
    if base == 0 {
        return Err(DeviceError::MissingBar);
    }
    MmioRegion::new(base + capability.offset as u64, capability.length)
}

struct PciConfig;

impl PciConfig {
    fn find() -> Option<PciProbe> {
        for bus in 0..=u8::MAX {
            for device in 0..32u8 {
                for function in 0..8u8 {
                    let Some(address) = logos_storage::PciAddress::new(bus, device, function)
                    else {
                        continue;
                    };
                    let config = Self::read_config(address);
                    if u16::from_le_bytes([config[0], config[1]]) == 0xffff {
                        continue;
                    }
                    if let Ok(parsed) = VirtioPciDevice::from_config(address, &config) {
                        return Some(PciProbe {
                            capabilities: parsed.capabilities,
                            bars: Self::bars(&config),
                        });
                    }
                }
            }
        }
        None
    }

    fn read_config(address: logos_storage::PciAddress) -> [u8; logos_storage::PCI_CONFIG_BYTES] {
        let mut config = [0; logos_storage::PCI_CONFIG_BYTES];
        for offset in (0..logos_storage::PCI_CONFIG_BYTES).step_by(4) {
            let value = unsafe { config_read(address, offset as u8) };
            config[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        config
    }

    fn bars(config: &[u8; logos_storage::PCI_CONFIG_BYTES]) -> [u64; 6] {
        let mut bars = [0; 6];
        for (index, bar) in bars.iter_mut().enumerate() {
            let offset = 0x10 + index * 4;
            let low = u32::from_le_bytes(config[offset..offset + 4].try_into().unwrap());
            if low & 1 != 0 {
                continue;
            }
            *bar = (low as u64) & !0xf;
            if low & 0x6 == 0x4 && index < 5 {
                let high = u32::from_le_bytes(config[offset + 4..offset + 8].try_into().unwrap());
                *bar |= (high as u64) << 32;
            }
        }
        bars
    }
}

struct PciProbe {
    capabilities: logos_storage::VirtioPciCapabilities,
    bars: [u64; 6],
}

unsafe fn config_read(address: logos_storage::PciAddress, offset: u8) -> u32 {
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

unsafe fn outl(port: u16, value: u32) {
    unsafe {
        asm!("out dx, eax", in("dx") port, in("eax") value, options(nomem, nostack, preserves_flags));
    }
}

unsafe fn inl(port: u16) -> u32 {
    let value: u32;
    unsafe {
        asm!("in eax, dx", in("dx") port, out("eax") value, options(nomem, nostack, preserves_flags));
    }
    value
}
