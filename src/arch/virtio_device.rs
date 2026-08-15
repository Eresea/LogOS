#![allow(dead_code)]

use core::{
    arch::asm,
    mem::MaybeUninit,
    ptr::{copy_nonoverlapping, read_volatile, write_volatile},
    sync::atomic::{AtomicBool, Ordering, fence},
};

#[cfg(feature = "storage-proof")]
use core::sync::atomic::AtomicU8;

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
const COMMON_MSIX_CONFIG: usize = 0x10;
const COMMON_DEVICE_STATUS: usize = 0x14;
const COMMON_QUEUE_SELECT: usize = 0x16;
const COMMON_QUEUE_SIZE: usize = 0x18;
const COMMON_QUEUE_MSIX_VECTOR: usize = 0x1a;
const COMMON_QUEUE_ENABLE: usize = 0x1c;
const COMMON_QUEUE_DESC: usize = 0x20;
const COMMON_QUEUE_DRIVER: usize = 0x28;
const COMMON_QUEUE_DEVICE: usize = 0x30;
const ISR_CAP_OFFSET: usize = 0;
const PCI_CAP_MSIX: u8 = 0x11;
const MSIX_VECTOR_INDEX: u16 = 0;
const MSIX_TABLE_ENTRY_BYTES: u32 = 16;

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
    available_padding: u16,
    used_flags: u16,
    used_index: u16,
    used_ring: [UsedElement; QUEUE_SIZE],
    used_available_event: u16,
    headers: [VirtioBlkHeader; QUEUE_SIZE],
    statuses: [u8; QUEUE_SIZE],
}

#[repr(C, align(4096))]
struct DmaMemory {
    queue: QueueMemory,
    data: [u8; logos_storage::BLOCK_BYTES],
}

// One fixed, page-aligned Core-owned DMA arena. The future frame allocator will
// replace this singleton when multiple block devices are supported.
#[unsafe(link_section = ".dma")]
static mut DMA_MEMORY: DmaMemory = DmaMemory::new();
static mut DEVICE: MaybeUninit<VirtioBlockDevice> = MaybeUninit::uninit();
static DEVICE_READY: AtomicBool = AtomicBool::new(false);
static DEVICE_BUSY: AtomicBool = AtomicBool::new(false);
#[cfg(feature = "storage-proof")]
static STORAGE_WRITE_COMPLETE: AtomicBool = AtomicBool::new(false);
#[cfg(feature = "storage-proof")]
static STORAGE_VALID_MEDIA: AtomicBool = AtomicBool::new(false);
#[cfg(feature = "storage-proof")]
static STORAGE_INTERRUPT_COMPLETION: AtomicBool = AtomicBool::new(false);
#[cfg(feature = "storage-proof")]
static STORAGE_PROOF_STATE: AtomicU8 = AtomicU8::new(0);

#[cfg(feature = "storage-proof")]
const SUPERBLOCK_MAGIC: &[u8; 8] = b"LOGOSFS\0";

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
            available_padding: 0,
            used_flags: 0,
            used_index: 0,
            used_ring: [Self::EMPTY_USED; QUEUE_SIZE],
            used_available_event: 0,
            headers: [VirtioBlkHeader { request_type: 0, reserved: 0, sector: 0 }; QUEUE_SIZE],
            statuses: [0xff; QUEUE_SIZE],
        }
    }
}

impl DmaMemory {
    const fn new() -> Self {
        Self { queue: QueueMemory::new(), data: [0; logos_storage::BLOCK_BYTES] }
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
    OutOfBounds,
    ReadOnly,
    Busy,
    StaleCompletion,
    InvalidCompletion,
    Timeout,
    Io,
    Unsupported,
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

    unsafe fn read_u64(&self, offset: usize) -> Result<u64, DeviceError> {
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
    device: MmioRegion,
    notify_multiplier: u32,
    queue: &'static mut QueueMemory,
    requests: [Option<BlockRequestId>; QUEUE_SIZE],
    used_index: u16,
    next_request_generation: u64,
    negotiated_features: u64,
    pci_address: logos_storage::PciAddress,
    bars: [u64; 6],
    msix: Option<MsixCapability>,
    interrupt_completion: Option<DeviceCompletion>,
    interrupt_error: Option<DeviceError>,
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

pub(crate) fn flush_storage_device() -> Result<(), DeviceError> {
    with_device_mut(VirtioBlockDevice::flush)
}

pub(crate) fn storage_block_count() -> Result<u64, DeviceError> {
    with_device_mut(|device| device.capacity_blocks())
}

pub(crate) fn transfer_storage_block(
    request: logos_abi::StorageRequest,
    data_address: usize,
) -> Result<(), DeviceError> {
    with_device_mut(|device| device.transfer(request, data_address))
}

pub(crate) fn handle_storage_interrupt() {
    if !DEVICE_READY.load(Ordering::Acquire)
        || DEVICE_BUSY.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).is_err()
    {
        return;
    }
    unsafe {
        (&mut *core::ptr::addr_of_mut!(DEVICE).cast::<VirtioBlockDevice>()).handle_interrupt();
    }
    DEVICE_BUSY.store(false, Ordering::Release);
}

fn with_device_mut<T>(
    operation: impl FnOnce(&mut VirtioBlockDevice) -> Result<T, DeviceError>,
) -> Result<T, DeviceError> {
    if !DEVICE_READY.load(Ordering::Acquire) {
        return Err(DeviceError::NotFound);
    }
    if DEVICE_BUSY.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).is_err() {
        return Err(DeviceError::Busy);
    }
    let result =
        unsafe { operation(&mut *core::ptr::addr_of_mut!(DEVICE).cast::<VirtioBlockDevice>()) };
    DEVICE_BUSY.store(false, Ordering::Release);
    result
}

impl VirtioBlockDevice {
    pub fn initialize(writable: bool) -> Result<Self, DeviceError> {
        let probe = PciConfig::find().ok_or(DeviceError::NotFound)?;
        let common = region_for(&probe, probe.capabilities.common)?;
        let notify = region_for(&probe, probe.capabilities.notify)?;
        let isr = region_for(&probe, probe.capabilities.isr)?;
        let device_config = region_for(&probe, probe.capabilities.device)?;
        let queue = unsafe {
            let dma = &mut *core::ptr::addr_of_mut!(DMA_MEMORY);
            *dma = DmaMemory::new();
            &mut dma.queue
        };
        let mut device = Self {
            common,
            notify,
            isr,
            device: device_config,
            notify_multiplier: probe.capabilities.notify_multiplier,
            queue,
            requests: [None; QUEUE_SIZE],
            used_index: 0,
            next_request_generation: 1,
            negotiated_features: 0,
            pci_address: probe.address,
            bars: probe.bars,
            msix: probe.msix,
            interrupt_completion: None,
            interrupt_error: None,
        };
        device.reset()?;
        unsafe { device.common.write_u8(COMMON_DEVICE_STATUS, STATUS_ACKNOWLEDGE)? };
        unsafe {
            device.common.write_u8(COMMON_DEVICE_STATUS, STATUS_ACKNOWLEDGE | STATUS_DRIVER)?
        };
        let features = device.device_features()?;
        let negotiated =
            negotiate_features(features, writable).map_err(|_| DeviceError::FeatureNegotiation)?;
        device.negotiated_features = negotiated.raw();
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
        device.configure_interrupts()?;
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

    fn capacity_blocks(&self) -> Result<u64, DeviceError> {
        let sectors = unsafe { self.device.read_u64(0)? };
        let blocks = sectors / logos_storage::SECTORS_PER_LOGOS_BLOCK;
        if blocks == 0 { Err(DeviceError::InvalidCompletion) } else { Ok(blocks) }
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
            self.common.write_u16(
                COMMON_QUEUE_MSIX_VECTOR,
                if self.msix.is_some() { MSIX_VECTOR_INDEX } else { u16::MAX },
            )?;
            self.common.write_u16(COMMON_QUEUE_ENABLE, 1)
        }
    }

    fn configure_interrupts(&mut self) -> Result<(), DeviceError> {
        let Some(msix) = self.msix else {
            return Ok(());
        };
        let table_bar = *self.bars.get(msix.table_bar as usize).ok_or(DeviceError::MissingBar)?;
        let table_address =
            table_bar.checked_add(u64::from(msix.table_offset)).ok_or(DeviceError::InvalidBar)?;
        let table = MmioRegion::new(table_address, MSIX_TABLE_ENTRY_BYTES)?;
        let apic_id = super::APIC_IDS[0].load(Ordering::Acquire);
        let message_address = 0xfee0_0000u64 | (u64::from(apic_id) << 12);
        unsafe {
            table.write_u32(12, 1)?;
            table.write_u32(0, message_address as u32)?;
            table.write_u32(4, (message_address >> 32) as u32)?;
            table.write_u32(8, u32::from(super::STORAGE_VECTOR))?;
            self.common.write_u16(COMMON_MSIX_CONFIG, MSIX_VECTOR_INDEX)?;
            self.set_msix_enabled(msix.cap_offset)?;
            table.write_u32(12, 0)?;
        }
        Ok(())
    }

    unsafe fn set_msix_enabled(&self, capability: u8) -> Result<(), DeviceError> {
        let control = unsafe { config_read_u16(self.pci_address, capability.wrapping_add(2)) };
        let enabled = (control | (1 << 15)) & !(1 << 14);
        unsafe { config_write_u16(self.pci_address, capability.wrapping_add(2), enabled) };
        Ok(())
    }

    pub fn submit(&mut self, chain: VirtioBlkChain, data_address: u64) -> Result<(), DeviceError> {
        if chain.data.is_some() && data_address != dma_data_address() {
            return Err(DeviceError::InvalidCompletion);
        }
        if chain.data.is_none() && data_address != 0 {
            return Err(DeviceError::InvalidCompletion);
        }
        let available = unsafe { read_volatile(&self.queue.available_index) };
        if available != 0 && available as usize % QUEUE_SIZE == 0 {
            self.reset_device()?;
        }
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
        self.requests[slot] = Some(chain.request_id);
        fence(Ordering::Release);
        self.queue.available_index = used.wrapping_add(1);
        let notify_offset = unsafe { self.common.read_u16(0x1e)? } as u64;
        let notify_address = self.notify.address + notify_offset * self.notify_multiplier as u64;
        unsafe { write_volatile(notify_address as *mut u16, 0) };
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
        if element.id as usize >= QUEUE_SIZE * 3 {
            return Err(DeviceError::InvalidCompletion);
        }
        if element.id as usize % 3 != 0 {
            return Err(DeviceError::InvalidCompletion);
        }
        let queue_slot = element.id as usize / 3;
        let request_id = self
            .requests
            .get_mut(queue_slot)
            .and_then(Option::take)
            .ok_or(DeviceError::StaleCompletion)?;
        let status = self.queue.statuses[queue_slot];
        Ok(Some(DeviceCompletion { request_id, status, bytes_written: element.length }))
    }

    fn take_completion(&mut self) -> Result<Option<DeviceCompletion>, DeviceError> {
        if let Some(error) = self.interrupt_error.take() {
            return Err(error);
        }
        if self.interrupt_completion.is_some() {
            return Ok(self.interrupt_completion.take());
        }
        self.poll_completion()
    }

    fn handle_interrupt(&mut self) {
        let Ok(status) = self.interrupt_status() else {
            self.interrupt_error = Some(DeviceError::Io);
            return;
        };
        if status & 1 == 0 {
            return;
        }
        #[cfg(feature = "storage-proof")]
        STORAGE_INTERRUPT_COMPLETION.store(true, Ordering::Release);
        match self.poll_completion() {
            Ok(completion) => self.interrupt_completion = completion,
            Err(error) => self.interrupt_error = Some(error),
        }
    }

    fn flush(&mut self) -> Result<(), DeviceError> {
        let generation = self.next_request_generation;
        self.next_request_generation = self.next_request_generation.wrapping_add(1).max(1);
        let request_id = logos_storage::BlockRequestId::from_parts(0, generation)
            .ok_or(DeviceError::InvalidCompletion)?;
        self.submit(
            VirtioBlkChain {
                request_id,
                header: VirtioBlkHeader { request_type: 4, reserved: 0, sector: 0 },
                data: None,
                blocks: 0,
            },
            0,
        )?;
        for _ in 0..1_000_000 {
            if let Some(completion) = self.take_completion()? {
                if completion.request_id != request_id {
                    return Err(DeviceError::StaleCompletion);
                }
                return match completion.status {
                    0 => {
                        #[cfg(feature = "storage-proof")]
                        self.record_flush_success();
                        Ok(())
                    }
                    1 => Err(DeviceError::Io),
                    2 => Err(DeviceError::Unsupported),
                    _ => Err(DeviceError::InvalidCompletion),
                };
            }
            core::hint::spin_loop();
        }
        self.reset_device()?;
        Err(DeviceError::Timeout)
    }

    fn transfer(
        &mut self,
        request: logos_abi::StorageRequest,
        staging_address: usize,
    ) -> Result<(), DeviceError> {
        if request.blocks != 1 {
            return Err(DeviceError::InvalidCompletion);
        }
        if request.start_block >= self.capacity_blocks()? {
            return Err(DeviceError::OutOfBounds);
        }
        let request_type = match request.operation {
            logos_abi::StorageOperation::Read => 0,
            logos_abi::StorageOperation::Write => 1,
            _ => return Err(DeviceError::InvalidCompletion),
        };
        let sector = request
            .start_block
            .checked_mul(logos_storage::SECTORS_PER_LOGOS_BLOCK)
            .ok_or(DeviceError::InvalidCompletion)?;
        let generation = self.next_request_generation;
        self.next_request_generation = self.next_request_generation.wrapping_add(1).max(1);
        let request_id = logos_storage::BlockRequestId::from_parts(0, generation)
            .ok_or(DeviceError::InvalidCompletion)?;
        if staging_address == 0 || staging_address % logos_storage::BLOCK_BYTES != 0 {
            return Err(DeviceError::InvalidCompletion);
        }
        let buffer = logos_storage::BufferToken::new(dma_data_address())
            .ok_or(DeviceError::InvalidCompletion)?;
        if request.operation == logos_abi::StorageOperation::Write {
            unsafe {
                copy_nonoverlapping(
                    staging_address as *const u8,
                    core::ptr::addr_of_mut!(DMA_MEMORY.data).cast::<u8>(),
                    logos_storage::BLOCK_BYTES,
                );
            }
        }
        self.submit(
            VirtioBlkChain {
                request_id,
                header: VirtioBlkHeader { request_type, reserved: 0, sector },
                data: Some(logos_storage::VirtioDataDescriptor {
                    buffer,
                    length: logos_storage::BLOCK_BYTES as u32,
                    device_writable: request.operation == logos_abi::StorageOperation::Read,
                }),
                blocks: 1,
            },
            dma_data_address(),
        )?;
        for _ in 0..1_000_000 {
            if let Some(completion) = self.take_completion()? {
                if completion.request_id != request_id {
                    return Err(DeviceError::StaleCompletion);
                }
                let result = match completion.status {
                    0 => {
                        #[cfg(feature = "storage-proof")]
                        self.record_transfer_success(request, staging_address);
                        Ok(())
                    }
                    1 => Err(DeviceError::Io),
                    2 => Err(DeviceError::Unsupported),
                    _ => Err(DeviceError::InvalidCompletion),
                };
                if result.is_ok() && request.operation == logos_abi::StorageOperation::Read {
                    unsafe {
                        copy_nonoverlapping(
                            core::ptr::addr_of!(DMA_MEMORY.data).cast::<u8>(),
                            staging_address as *mut u8,
                            logos_storage::BLOCK_BYTES,
                        );
                    }
                }
                return result;
            }
            core::hint::spin_loop();
        }
        self.reset_device()?;
        Err(DeviceError::Timeout)
    }

    pub fn interrupt_status(&self) -> Result<u8, DeviceError> {
        unsafe { self.isr.read_u8(ISR_CAP_OFFSET) }
    }

    pub fn reset_device(&mut self) -> Result<(), DeviceError> {
        self.reset()?;
        *self.queue = QueueMemory::new();
        self.requests.fill(None);
        self.used_index = 0;
        self.interrupt_completion = None;
        self.interrupt_error = None;
        unsafe {
            self.common.write_u8(COMMON_DEVICE_STATUS, STATUS_ACKNOWLEDGE)?;
            self.common.write_u8(COMMON_DEVICE_STATUS, STATUS_ACKNOWLEDGE | STATUS_DRIVER)?;
        }
        self.driver_features(self.negotiated_features)?;
        unsafe {
            self.common.write_u8(
                COMMON_DEVICE_STATUS,
                STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK,
            )?;
            if self.common.read_u8(COMMON_DEVICE_STATUS)? & STATUS_FEATURES_OK == 0 {
                return Err(DeviceError::DeviceRejectedFeatures);
            }
        }
        self.configure_queue()?;
        if self.msix.is_some() {
            unsafe { self.common.write_u16(COMMON_MSIX_CONFIG, MSIX_VECTOR_INDEX)? };
        }
        unsafe {
            self.common.write_u8(
                COMMON_DEVICE_STATUS,
                STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK | STATUS_DRIVER_OK,
            )?;
        }
        Ok(())
    }

    #[cfg(feature = "storage-proof")]
    fn record_transfer_success(&self, request: logos_abi::StorageRequest, data_address: usize) {
        match request.operation {
            logos_abi::StorageOperation::Write => {
                STORAGE_WRITE_COMPLETE.store(true, Ordering::Release);
            }
            logos_abi::StorageOperation::Read if request.start_block < 2 => {
                let bytes = data_address as *const u8;
                let valid = (0..SUPERBLOCK_MAGIC.len()).all(
                    |index| unsafe { read_volatile(bytes.add(index)) } == SUPERBLOCK_MAGIC[index],
                );
                if valid {
                    STORAGE_VALID_MEDIA.store(true, Ordering::Release);
                }
            }
            _ => {}
        }
    }

    #[cfg(feature = "storage-proof")]
    fn record_flush_success(&self) {
        if STORAGE_WRITE_COMPLETE.swap(false, Ordering::AcqRel)
            && STORAGE_INTERRUPT_COMPLETION.swap(false, Ordering::AcqRel)
            && STORAGE_PROOF_STATE
                .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
        {
            super::proof_line(b"LogOS vNext: storage proof PASS");
        } else if STORAGE_VALID_MEDIA.load(Ordering::Acquire)
            && STORAGE_INTERRUPT_COMPLETION.swap(false, Ordering::AcqRel)
            && STORAGE_PROOF_STATE
                .compare_exchange(0, 2, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
        {
            super::proof_line(b"LogOS vNext: storage recovery PASS");
        }
    }
}

fn dma_data_address() -> u64 {
    unsafe { core::ptr::addr_of!(DMA_MEMORY.data) as u64 }
}

fn region_for(
    probe: &PciProbe,
    capability: logos_storage::VirtioPciCapability,
) -> Result<MmioRegion, DeviceError> {
    let base = *probe.bars.get(capability.bar as usize).ok_or(DeviceError::MissingBar)?;
    if base == 0 {
        return Err(DeviceError::MissingBar);
    }
    let address = base.checked_add(capability.offset as u64).ok_or(DeviceError::InvalidBar)?;
    MmioRegion::new(address, capability.length)
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
                    let vendor = u16::from_le_bytes([config[0], config[1]]);
                    if vendor == 0xffff {
                        continue;
                    }
                    if let Ok(parsed) = VirtioPciDevice::from_config(address, &config) {
                        return Some(PciProbe {
                            address,
                            capabilities: parsed.capabilities,
                            bars: Self::bars(&config),
                            msix: Self::msix(&config),
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

    fn msix(config: &[u8; logos_storage::PCI_CONFIG_BYTES]) -> Option<MsixCapability> {
        let mut offset = config[0x34];
        let mut seen = [false; 256];
        while offset >= 0x40 && usize::from(offset) + 2 <= config.len() && !seen[offset as usize] {
            seen[offset as usize] = true;
            let id = config[offset as usize];
            let next = config[offset as usize + 1];
            if id == PCI_CAP_MSIX && usize::from(offset) + 12 <= config.len() {
                let control =
                    u16::from_le_bytes([config[offset as usize + 2], config[offset as usize + 3]]);
                let table = u32::from_le_bytes([
                    config[offset as usize + 4],
                    config[offset as usize + 5],
                    config[offset as usize + 6],
                    config[offset as usize + 7],
                ]);
                let table_entries = (control & 0x07ff).saturating_add(1);
                let table_bar = (table & 0x7) as u8;
                if table_bar < 6 && table_entries > MSIX_VECTOR_INDEX {
                    return Some(MsixCapability {
                        cap_offset: offset,
                        table_bar,
                        table_offset: table & !0x7,
                    });
                }
            }
            if next == 0 {
                break;
            }
            offset = next;
        }
        None
    }
}

struct PciProbe {
    address: logos_storage::PciAddress,
    capabilities: logos_storage::VirtioPciCapabilities,
    bars: [u64; 6],
    msix: Option<MsixCapability>,
}

#[derive(Clone, Copy)]
struct MsixCapability {
    cap_offset: u8,
    table_bar: u8,
    table_offset: u32,
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

unsafe fn config_read_u16(address: logos_storage::PciAddress, offset: u8) -> u16 {
    let value = unsafe { config_read(address, offset & 0xfc) };
    let shift = u32::from(offset & 2) * 8;
    ((value >> shift) & 0xffff) as u16
}

unsafe fn config_write_u16(address: logos_storage::PciAddress, offset: u8, value: u16) {
    let aligned = offset & 0xfc;
    let current = unsafe { config_read(address, aligned) };
    let shift = u32::from(offset & 2) * 8;
    let mask = 0xffffu32 << shift;
    let updated = (current & !mask) | (u32::from(value) << shift);
    unsafe { config_write(address, aligned, updated) };
}

unsafe fn config_write(address: logos_storage::PciAddress, offset: u8, value: u32) {
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
