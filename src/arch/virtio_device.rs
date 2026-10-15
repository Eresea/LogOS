#![allow(dead_code)]

use core::{
    arch::asm,
    mem::MaybeUninit,
    ptr::{copy_nonoverlapping, read_volatile, write_volatile},
    sync::atomic::{AtomicBool, Ordering, fence},
};

#[cfg(feature = "storage-proof")]
use core::sync::atomic::AtomicU8;

use crate::virtio_queue::{
    Descriptor, QUEUE_SIZE, QueueMemory, RING_SIZE, UsedError, clamp_used_length, le_u32,
    negotiated_queue_size, notify_delta, pending_used, read_shared, validate_used_id, write_shared,
};
use logos_storage::{
    BlockRequestId, PciError, VIRTIO_BLK_TYPE_FLUSH, VirtioBlkChain, VirtioBlkHeader,
    VirtioPciDevice, negotiate_features,
};

const PCI_CONFIG_ADDRESS: u16 = 0xcf8;
const PCI_CONFIG_DATA: u16 = 0xcfc;
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
const COMMON_QUEUE_NOTIFY_OFF: usize = 0x1e;
const COMMON_QUEUE_DESC: usize = 0x20;
const COMMON_QUEUE_DRIVER: usize = 0x28;
const COMMON_QUEUE_DEVICE: usize = 0x30;
const ISR_CAP_OFFSET: usize = 0;
const PCI_CAP_MSIX: u8 = 0x11;
const MSIX_VECTOR_INDEX: u16 = 0;
const MSIX_TABLE_ENTRY_BYTES: u32 = 16;
const COMPLETION_TIMEOUT_TICKS: u64 = logos_abi::SERVICE_HEARTBEAT_INTERVAL_TICKS;
const COMPLETION_SPIN_LIMIT: usize = 1_000_000;

// One fixed, page-aligned Core-owned DMA arena. The future frame allocator will
// replace this singleton when multiple block devices are supported.
#[unsafe(link_section = ".dma")]
static mut QUEUE_MEMORY: QueueMemory = QueueMemory::new();
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
const SUPERBLOCK_MAGIC: &[u8; 8] = b"LOGOSCOW";

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
    GenerationExhausted,
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
    /// # Safety
    ///
    /// `address..address + length` must be a device MMIO window that stays mapped for the
    /// lifetime of the region; the volatile accessors below rely on it.
    unsafe fn new(address: u64, length: u32) -> Result<Self, DeviceError> {
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
        let address = (self.address as usize).checked_add(offset).ok_or(DeviceError::InvalidBar)?;
        Ok(address as *mut T)
    }

    fn read<T: Copy>(&self, offset: usize) -> Result<T, DeviceError> {
        let pointer = self.ptr::<T>(offset)?;
        // SAFETY: `ptr` proved `offset..offset + size_of::<T>()` lies inside the region, and
        // `MmioRegion::new` obliges its caller to keep that window mapped device memory.
        Ok(unsafe { read_volatile(pointer) })
    }

    fn write<T: Copy>(&self, offset: usize, value: T) -> Result<(), DeviceError> {
        let pointer = self.ptr::<T>(offset)?;
        // SAFETY: as for `read`; the write stays inside the mapped device window.
        unsafe { write_volatile(pointer, value) };
        Ok(())
    }

    fn read_u8(&self, offset: usize) -> Result<u8, DeviceError> {
        self.read(offset)
    }

    fn read_u16(&self, offset: usize) -> Result<u16, DeviceError> {
        self.read(offset)
    }

    fn read_u32(&self, offset: usize) -> Result<u32, DeviceError> {
        self.read(offset)
    }

    fn read_u64(&self, offset: usize) -> Result<u64, DeviceError> {
        self.read(offset)
    }

    fn write_u8(&self, offset: usize, value: u8) -> Result<(), DeviceError> {
        self.write(offset, value)
    }

    fn write_u16(&self, offset: usize, value: u16) -> Result<(), DeviceError> {
        self.write(offset, value)
    }

    fn write_u32(&self, offset: usize, value: u32) -> Result<(), DeviceError> {
        self.write(offset, value)
    }

    fn write_u64(&self, offset: usize, value: u64) -> Result<(), DeviceError> {
        self.write(offset, value)
    }
}

pub struct VirtioBlockDevice {
    common: MmioRegion,
    notify: MmioRegion,
    isr: MmioRegion,
    device: MmioRegion,
    notify_multiplier: u32,
    capacity_blocks: u64,
    queue: &'static mut QueueMemory,
    requests: [Option<BlockRequestId>; QUEUE_SIZE],
    /// Device-writable bytes posted per slot; bounds the length the device may report as used.
    writable_bytes: [u32; QUEUE_SIZE],
    used_index: u16,
    next_request_generation: u64,
    negotiated_features: u64,
    pci_address: logos_storage::PciAddress,
    bars: [PciBar; 6],
    msix: Option<MsixCapability>,
    interrupt_completion: Option<DeviceCompletion>,
    interrupt_error: Option<DeviceError>,
}

/// The single accessor for the block driver's `DEVICE` slot.
///
/// # Safety
///
/// `DEVICE_READY` must be set (the slot holds an initialized device) and the caller must hold
/// `DEVICE_BUSY`. That flag is the exclusivity lock: it is taken with a compare-exchange, so at
/// most one `&mut` produced here is live at a time, and it is released only after the reference
/// is dead.
unsafe fn device_mut() -> &'static mut VirtioBlockDevice {
    // SAFETY: the caller upholds the contract above, so the slot is initialized and unaliased.
    unsafe { &mut *core::ptr::addr_of_mut!(DEVICE).cast::<VirtioBlockDevice>() }
}

/// Stores a freshly initialized device into the `DEVICE` slot.
///
/// The slot is written only while `DEVICE_READY` is clear, i.e. before any `device_mut` caller
/// can run, and only from the single boot-time initialization path.
fn install_device(device: VirtioBlockDevice) {
    // SAFETY: `DEVICE_READY` is false here, so no `device_mut` reference exists; the write
    // replaces any stale device without dropping (the type has no drop glue).
    unsafe { core::ptr::addr_of_mut!(DEVICE).write(MaybeUninit::new(device)) };
}

/// The single accessor for the DMA queue arena `QUEUE_MEMORY`.
///
/// # Safety
///
/// Called only from `VirtioBlockDevice::initialize`. The returned reference is moved into the
/// one `VirtioBlockDevice` that owns the arena; no other code touches the static afterwards
/// except through that device or through `queue_memory_address`, which never dereferences it.
unsafe fn queue_memory() -> &'static mut QueueMemory {
    // SAFETY: the caller upholds the contract above, so this is the only live reference.
    unsafe { &mut *core::ptr::addr_of_mut!(QUEUE_MEMORY) }
}

/// Address of the DMA queue arena, for frame reservation. Does not create a reference.
fn queue_memory_address() -> usize {
    core::ptr::addr_of!(QUEUE_MEMORY) as usize
}

/// Marks the device failed after it violated the VirtIO contract. The device is reset (best
/// effort) so it stops using the DMA arena, and every later storage call reports `NotFound`.
fn mark_failed(device: &VirtioBlockDevice) {
    let _ = device.reset();
    DEVICE_READY.store(false, Ordering::Release);
}

pub(crate) fn initialize_storage_device() -> bool {
    if DEVICE_READY.load(Ordering::Acquire) {
        return true;
    }
    let Ok(device) = VirtioBlockDevice::initialize(true) else {
        return false;
    };
    install_device(device);
    DEVICE_READY.store(true, Ordering::Release);
    true
}

pub(crate) fn reserve_frames(pool: &mut crate::frame_pool::FramePool) {
    super::reserve_storage_frames(
        pool,
        queue_memory_address(),
        core::mem::size_of::<QueueMemory>(),
    );
}

pub(crate) fn flush_storage_device() -> Result<(), DeviceError> {
    with_device_mut(VirtioBlockDevice::flush)
}

pub(crate) fn prepare_power_control() -> Result<(), DeviceError> {
    with_device_mut(|device| {
        device.flush()?;
        // Device reset drains the backend before the platform reset is requested.
        device.reset()
    })
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
    // SAFETY: `DEVICE_READY` was observed set and `DEVICE_BUSY` was just acquired above.
    unsafe { device_mut() }.handle_interrupt();
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
    // SAFETY: `DEVICE_READY` was observed set and `DEVICE_BUSY` was just acquired above; the
    // reference does not outlive `operation`, and the flag is released only afterwards.
    let result = operation(unsafe { device_mut() });
    DEVICE_BUSY.store(false, Ordering::Release);
    result
}

impl VirtioBlockDevice {
    pub fn initialize(writable: bool) -> Result<Self, DeviceError> {
        let probe = PciConfig::find().ok_or(DeviceError::NotFound)?;
        PciConfig::enable_device(probe.address);
        let common = region_for(&probe, probe.capabilities.common)?;
        let notify = region_for(&probe, probe.capabilities.notify)?;
        let isr = region_for(&probe, probe.capabilities.isr)?;
        let device_config = region_for(&probe, probe.capabilities.device)?;
        // SAFETY: `initialize` is the only caller; `DEVICE_READY` is false, so no previous
        // device still uses the arena.
        let queue = unsafe { queue_memory() };
        *queue = QueueMemory::new();
        let mut device = Self {
            common,
            notify,
            isr,
            device: device_config,
            notify_multiplier: probe.capabilities.notify_multiplier,
            capacity_blocks: 0,
            queue,
            requests: [None; QUEUE_SIZE],
            writable_bytes: [0; QUEUE_SIZE],
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
        device.common.write_u8(COMMON_DEVICE_STATUS, STATUS_ACKNOWLEDGE)?;
        device.common.write_u8(COMMON_DEVICE_STATUS, STATUS_ACKNOWLEDGE | STATUS_DRIVER)?;
        let features = device.device_features()?;
        let negotiated =
            negotiate_features(features, writable).map_err(|_| DeviceError::FeatureNegotiation)?;
        device.negotiated_features = negotiated.raw();
        device.driver_features(negotiated.raw())?;
        device.common.write_u8(
            COMMON_DEVICE_STATUS,
            STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK,
        )?;
        let status = device.common.read_u8(COMMON_DEVICE_STATUS)?;
        if status & STATUS_FEATURES_OK == 0 {
            return Err(DeviceError::DeviceRejectedFeatures);
        }
        device.configure_queue()?;
        device.configure_interrupts()?;
        device.common.write_u8(
            COMMON_DEVICE_STATUS,
            STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK | STATUS_DRIVER_OK,
        )?;
        device.capacity_blocks = device.read_capacity_blocks()?;
        Ok(device)
    }

    fn reset(&self) -> Result<(), DeviceError> {
        self.common.write_u8(COMMON_DEVICE_STATUS, 0)?;
        for _ in 0..1024 {
            if self.common.read_u8(COMMON_DEVICE_STATUS)? == 0 {
                return Ok(());
            }
            core::hint::spin_loop();
        }
        Err(DeviceError::Timeout)
    }

    fn device_features(&self) -> Result<u64, DeviceError> {
        self.common.write_u32(COMMON_DEVICE_FEATURE_SELECT, 0)?;
        let low = self.common.read_u32(COMMON_DEVICE_FEATURE)? as u64;
        self.common.write_u32(COMMON_DEVICE_FEATURE_SELECT, 1)?;
        let high = self.common.read_u32(COMMON_DEVICE_FEATURE)? as u64;
        Ok(low | high << 32)
    }

    fn dma_data_address(&self) -> u64 {
        self.queue.data.bytes.as_ptr() as u64
    }

    fn capacity_blocks(&self) -> Result<u64, DeviceError> {
        if self.capacity_blocks == 0 {
            return Err(DeviceError::InvalidCompletion);
        }
        Ok(self.capacity_blocks)
    }

    fn read_capacity_blocks(&self) -> Result<u64, DeviceError> {
        let sectors = self.device.read_u64(0)?;
        let blocks = sectors / logos_storage::SECTORS_PER_LOGOS_BLOCK;
        if blocks == 0 { Err(DeviceError::InvalidCompletion) } else { Ok(blocks) }
    }

    fn driver_features(&self, features: u64) -> Result<(), DeviceError> {
        self.common.write_u32(COMMON_DRIVER_FEATURE_SELECT, 0)?;
        self.common.write_u32(COMMON_DRIVER_FEATURE, features as u32)?;
        self.common.write_u32(COMMON_DRIVER_FEATURE_SELECT, 1)?;
        self.common.write_u32(COMMON_DRIVER_FEATURE, (features >> 32) as u32)
    }

    fn configure_queue(&mut self) -> Result<(), DeviceError> {
        {
            self.common.write_u16(COMMON_QUEUE_SELECT, 0)?;
            // The device reports its maximum; the rings are laid out for exactly `RING_SIZE`
            // entries, so program that size and require the device to accept it. Leaving the
            // device default (often 256) would let it address past the static queue memory.
            let maximum = self.common.read_u16(COMMON_QUEUE_SIZE)?;
            let size =
                negotiated_queue_size(maximum, RING_SIZE).ok_or(DeviceError::QueueUnavailable)?;
            self.common.write_u16(COMMON_QUEUE_SIZE, size)?;
            if self.common.read_u16(COMMON_QUEUE_SIZE)? != size {
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
        if table_bar.base == 0
            || u64::from(msix.table_offset)
                .checked_add(u64::from(MSIX_TABLE_ENTRY_BYTES))
                .is_none_or(|end| end > table_bar.length)
        {
            return Err(DeviceError::InvalidBar);
        }
        let table_address = table_bar
            .base
            .checked_add(u64::from(msix.table_offset))
            .ok_or(DeviceError::InvalidBar)?;
        // SAFETY: the table entry window was checked above to lie inside the MSI-X table BAR.
        let table = unsafe { MmioRegion::new(table_address, MSIX_TABLE_ENTRY_BYTES)? };
        let apic_id = super::APIC_IDS[0].load(Ordering::Acquire);
        let message_address = 0xfee0_0000u64 | (u64::from(apic_id) << 12);
        table.write_u32(12, 1)?;
        table.write_u32(0, message_address as u32)?;
        table.write_u32(4, (message_address >> 32) as u32)?;
        table.write_u32(8, u32::from(super::STORAGE_VECTOR))?;
        self.common.write_u16(COMMON_MSIX_CONFIG, MSIX_VECTOR_INDEX)?;
        self.set_msix_enabled(msix.cap_offset)?;
        table.write_u32(12, 0)?;
        Ok(())
    }

    fn set_msix_enabled(&self, capability: u8) -> Result<(), DeviceError> {
        let control = config_read_u16(self.pci_address, capability.wrapping_add(2));
        let enabled = (control | (1 << 15)) & !(1 << 14);
        config_write_u16(self.pci_address, capability.wrapping_add(2), enabled);
        Ok(())
    }

    pub fn submit(&mut self, chain: VirtioBlkChain, data_address: u64) -> Result<(), DeviceError> {
        if chain.data.is_some() && data_address != self.dma_data_address() {
            return Err(DeviceError::InvalidCompletion);
        }
        if chain.data.is_none() && data_address != 0 {
            return Err(DeviceError::InvalidCompletion);
        }
        let available = read_shared(&self.queue.available_index);
        if available as usize % QUEUE_SIZE == QUEUE_SIZE - 1
            && chain.header.request_type != VIRTIO_BLK_TYPE_FLUSH
            && !self.requests.iter().any(Option::is_some)
        {
            self.flush()?;
        }
        let available = read_shared(&self.queue.available_index);
        let slot = (available as usize) % QUEUE_SIZE;
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
        // The device always writes the one-byte status; a device-writable data buffer adds more.
        let mut writable = 1u32;
        if let Some(data) = chain.data {
            if data.device_writable {
                writable = writable.saturating_add(data.length);
            }
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
        self.queue.available_ring[usize::from(available) % RING_SIZE] = first as u16;
        self.requests[slot] = Some(chain.request_id);
        self.writable_bytes[slot] = writable;
        fence(Ordering::Release);
        write_shared(&mut self.queue.available_index, available.wrapping_add(1));
        let notify_offset = self.common.read_u16(COMMON_QUEUE_NOTIFY_OFF)?;
        let delta = notify_delta(self.notify.length, notify_offset, self.notify_multiplier)
            .ok_or(DeviceError::InvalidBar)?;
        self.notify.write_u16(delta, 0)?;
        Ok(())
    }

    pub fn poll_completion(&mut self) -> Result<Option<DeviceCompletion>, DeviceError> {
        let used = read_shared(&self.queue.used_index);
        if used == self.used_index {
            return Ok(None);
        }
        fence(Ordering::Acquire);
        if pending_used(used, self.used_index, RING_SIZE).is_err() {
            return Err(self.device_violation(DeviceError::InvalidCompletion));
        }
        let slot = (self.used_index as usize) % RING_SIZE;
        let element = read_shared(&self.queue.used_ring[slot]);
        self.used_index = self.used_index.wrapping_add(1);
        let requests = &self.requests;
        let queue_slot =
            match validate_used_id(element.id, 3, QUEUE_SIZE, |slot| requests[slot].is_some()) {
                Ok(queue_slot) => queue_slot,
                Err(UsedError::NotInFlight) => {
                    return Err(self.device_violation(DeviceError::StaleCompletion));
                }
                Err(_) => return Err(self.device_violation(DeviceError::InvalidCompletion)),
            };
        let request_id = self.requests[queue_slot].take().ok_or(DeviceError::StaleCompletion)?;
        let status = read_shared(&self.queue.statuses[queue_slot]);
        let bytes_written = clamp_used_length(element.length, self.writable_bytes[queue_slot]);
        self.writable_bytes[queue_slot] = 0;
        Ok(Some(DeviceCompletion { request_id, status, bytes_written }))
    }

    /// Records that the device broke the used-ring contract: the device is reset so it stops
    /// touching the DMA arena and the driver reports itself unavailable from now on.
    fn device_violation(&self, error: DeviceError) -> DeviceError {
        mark_failed(self);
        error
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
        let request_id = self.next_request_id()?;
        self.submit(
            VirtioBlkChain {
                request_id,
                header: VirtioBlkHeader { request_type: 4, reserved: 0, sector: 0 },
                data: None,
                blocks: 0,
            },
            0,
        )?;
        let completion = self.wait_for_completion(request_id)?;
        let result = match completion.status {
            0 => {
                #[cfg(feature = "storage-proof")]
                self.record_flush_success();
                Ok(())
            }
            1 => Err(DeviceError::Io),
            2 => Err(DeviceError::Unsupported),
            _ => Err(DeviceError::InvalidCompletion),
        };
        if result.is_ok()
            && read_shared(&self.queue.available_index) as usize % QUEUE_SIZE == 0
            && !self.requests.iter().any(Option::is_some)
        {
            self.reset_device()?;
        }
        result
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
        let request_id = self.next_request_id()?;
        if staging_address == 0 || staging_address % logos_storage::BLOCK_BYTES != 0 {
            return Err(DeviceError::InvalidCompletion);
        }
        let dma_address = self.dma_data_address();
        let buffer =
            logos_storage::BufferToken::new(dma_address).ok_or(DeviceError::InvalidCompletion)?;
        if request.operation == logos_abi::StorageOperation::Write {
            // SAFETY: the caller (`arch` storage path) passes a validated, block-aligned Core
            // staging page of `BLOCK_BYTES`; the destination is the owned DMA block of the same
            // size, and the two regions are distinct allocations.
            unsafe {
                copy_nonoverlapping(
                    staging_address as *const u8,
                    self.queue.data.bytes.as_mut_ptr(),
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
            dma_address,
        )?;
        let completion = self.wait_for_completion(request_id)?;
        let result = match completion.status {
            0 => {
                #[cfg(feature = "storage-proof")]
                self.record_transfer_success(request, dma_address as usize);
                Ok(())
            }
            1 => Err(DeviceError::Io),
            2 => Err(DeviceError::Unsupported),
            _ => Err(DeviceError::InvalidCompletion),
        };
        if result.is_ok() && request.operation == logos_abi::StorageOperation::Read {
            // SAFETY: as above, in the other direction; the staging page is writable Core memory
            // of `BLOCK_BYTES` that does not overlap the DMA block.
            unsafe {
                copy_nonoverlapping(
                    self.queue.data.bytes.as_ptr(),
                    staging_address as *mut u8,
                    logos_storage::BLOCK_BYTES,
                );
            }
        }
        result
    }

    fn wait_for_completion(
        &mut self,
        request_id: BlockRequestId,
    ) -> Result<DeviceCompletion, DeviceError> {
        let deadline = super::current_ticks().saturating_add(COMPLETION_TIMEOUT_TICKS);
        for _ in 0..COMPLETION_SPIN_LIMIT {
            if let Some(completion) = self.take_completion()? {
                if completion.request_id != request_id {
                    return Err(DeviceError::StaleCompletion);
                }
                return Ok(completion);
            }
            if super::current_ticks() >= deadline {
                self.reset_device()?;
                return Err(DeviceError::Timeout);
            }
            core::hint::spin_loop();
        }
        self.reset_device()?;
        Err(DeviceError::Timeout)
    }

    pub fn interrupt_status(&self) -> Result<u8, DeviceError> {
        self.isr.read_u8(ISR_CAP_OFFSET)
    }

    fn next_request_id(&mut self) -> Result<BlockRequestId, DeviceError> {
        let generation = self.next_request_generation;
        self.next_request_generation =
            generation.checked_add(1).ok_or(DeviceError::GenerationExhausted)?;
        BlockRequestId::from_parts(0, generation).ok_or(DeviceError::InvalidCompletion)
    }

    pub fn reset_device(&mut self) -> Result<(), DeviceError> {
        self.reset()?;
        self.queue.reset_rings();
        self.requests.fill(None);
        self.writable_bytes.fill(0);
        self.used_index = 0;
        self.interrupt_completion = None;
        self.interrupt_error = None;
        self.common.write_u8(COMMON_DEVICE_STATUS, STATUS_ACKNOWLEDGE)?;
        self.common.write_u8(COMMON_DEVICE_STATUS, STATUS_ACKNOWLEDGE | STATUS_DRIVER)?;
        self.driver_features(self.negotiated_features)?;
        self.common.write_u8(
            COMMON_DEVICE_STATUS,
            STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK,
        )?;
        if self.common.read_u8(COMMON_DEVICE_STATUS)? & STATUS_FEATURES_OK == 0 {
            return Err(DeviceError::DeviceRejectedFeatures);
        }
        self.configure_queue()?;
        if self.msix.is_some() {
            self.common.write_u16(COMMON_MSIX_CONFIG, MSIX_VECTOR_INDEX)?;
        }
        self.common.write_u8(
            COMMON_DEVICE_STATUS,
            STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK | STATUS_DRIVER_OK,
        )?;
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
                let valid = (0..SUPERBLOCK_MAGIC.len()).all(|index| {
                    // SAFETY: `data_address` is the owned DMA block (`BLOCK_BYTES` long) and
                    // the magic is far shorter than that.
                    unsafe { read_volatile(bytes.add(index)) == SUPERBLOCK_MAGIC[index] }
                });
                if valid {
                    STORAGE_VALID_MEDIA.store(true, Ordering::Release);
                }
            }
            _ => {}
        }
    }

    #[cfg(feature = "storage-proof")]
    fn record_flush_success(&self) {
        if STORAGE_VALID_MEDIA.load(Ordering::Acquire)
            && STORAGE_INTERRUPT_COMPLETION.swap(false, Ordering::AcqRel)
            && STORAGE_PROOF_STATE
                .compare_exchange(0, 2, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
        {
            super::proof_line(b"LogOS vNext: storage recovery PASS");
        } else if STORAGE_WRITE_COMPLETE.swap(false, Ordering::AcqRel)
            && STORAGE_INTERRUPT_COMPLETION.swap(false, Ordering::AcqRel)
            && STORAGE_PROOF_STATE
                .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
        {
            super::proof_line(b"LogOS vNext: storage proof PASS");
        }
    }
}

fn region_for(
    probe: &PciProbe,
    capability: logos_storage::VirtioPciCapability,
) -> Result<MmioRegion, DeviceError> {
    let bar = *probe.bars.get(capability.bar as usize).ok_or(DeviceError::MissingBar)?;
    if bar.base == 0
        || u64::from(capability.offset)
            .checked_add(u64::from(capability.length))
            .is_none_or(|end| end > bar.length)
    {
        return Err(DeviceError::MissingBar);
    }
    let address = bar.base.checked_add(capability.offset as u64).ok_or(DeviceError::InvalidBar)?;
    // SAFETY: the capability window was checked to lie inside a BAR the device decodes.
    unsafe { MmioRegion::new(address, capability.length) }
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
                            bars: Self::bars(address, &config),
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
            let value = config_read(address, offset as u8);
            config[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        config
    }

    fn bars(
        address: logos_storage::PciAddress,
        config: &[u8; logos_storage::PCI_CONFIG_BYTES],
    ) -> [PciBar; 6] {
        let mut bars = [PciBar::EMPTY; 6];
        let mut index = 0;
        while index < bars.len() {
            let offset = 0x10 + index * 4;
            let Some(low) = le_u32(config, offset) else { break };
            if low & 1 != 0 {
                index += 1;
                continue;
            }
            let is_64 = low & 0x6 == 0x4 && index < 5;
            let original_high = if is_64 { le_u32(config, offset + 4).unwrap_or(0) } else { 0 };
            config_write(address, offset as u8, u32::MAX);
            if is_64 {
                config_write(address, (offset + 4) as u8, u32::MAX);
            }
            let mask_low = config_read(address, offset as u8);
            let mask_high = if is_64 { config_read(address, (offset + 4) as u8) } else { 0 };
            config_write(address, offset as u8, low);
            if is_64 {
                config_write(address, (offset + 4) as u8, original_high);
            }
            let base = if is_64 {
                (u64::from(original_high) << 32) | u64::from(low & !0xf)
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

    fn enable_device(address: logos_storage::PciAddress) {
        let command = config_read_u16(address, 0x04);
        config_write_u16(address, 0x04, command | 0x0006);
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
    bars: [PciBar; 6],
    msix: Option<MsixCapability>,
}

#[derive(Clone, Copy)]
struct PciBar {
    base: u64,
    length: u64,
}

impl PciBar {
    const EMPTY: Self = Self { base: 0, length: 0 };
}

#[derive(Clone, Copy)]
struct MsixCapability {
    cap_offset: u8,
    table_bar: u8,
    table_offset: u32,
}

fn config_read(address: logos_storage::PciAddress, offset: u8) -> u32 {
    let value = 0x8000_0000u32
        | (u32::from(address.bus()) << 16)
        | (u32::from(address.device()) << 11)
        | (u32::from(address.function()) << 8)
        | u32::from(offset & 0xfc);
    outl(PCI_CONFIG_ADDRESS, value);
    inl(PCI_CONFIG_DATA)
}

fn config_read_u16(address: logos_storage::PciAddress, offset: u8) -> u16 {
    let value = config_read(address, offset & 0xfc);
    let shift = u32::from(offset & 2) * 8;
    ((value >> shift) & 0xffff) as u16
}

fn config_write_u16(address: logos_storage::PciAddress, offset: u8, value: u16) {
    let aligned = offset & 0xfc;
    let current = config_read(address, aligned);
    let shift = u32::from(offset & 2) * 8;
    let mask = 0xffffu32 << shift;
    let updated = (current & !mask) | (u32::from(value) << shift);
    config_write(address, aligned, updated);
}

fn config_write(address: logos_storage::PciAddress, offset: u8, value: u32) {
    let address_value = 0x8000_0000u32
        | (u32::from(address.bus()) << 16)
        | (u32::from(address.device()) << 11)
        | (u32::from(address.function()) << 8)
        | u32::from(offset & 0xfc);
    outl(PCI_CONFIG_ADDRESS, address_value);
    outl(PCI_CONFIG_DATA, value);
}

fn outl(port: u16, value: u32) {
    // SAFETY: port I/O to the PCI configuration ports touches no Rust-visible memory; the
    // kernel runs at ring 0 and is the sole owner of these ports.
    unsafe {
        asm!("out dx, eax", in("dx") port, in("eax") value, options(nomem, nostack, preserves_flags));
    }
}

fn inl(port: u16) -> u32 {
    let value: u32;
    // SAFETY: as for `outl`; reading the PCI data port has no memory effects.
    unsafe {
        asm!("in eax, dx", in("dx") port, out("eax") value, options(nomem, nostack, preserves_flags));
    }
    value
}
