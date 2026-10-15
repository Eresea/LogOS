//! Core-owned modern VirtIO-net transport.
//!
//! This module deliberately stops at Ethernet frames. Protocol state and all
//! socket policy live in the Network service. The only memory shared with the
//! service is a copied packet page; the DMA arena below never crosses into a
//! service address space.

use core::{
    arch::asm,
    mem::MaybeUninit,
    ptr::{copy_nonoverlapping, read_volatile, write_volatile},
    sync::atomic::{AtomicBool, AtomicUsize, Ordering, fence},
};

use logos_abi::{NETWORK_DMA_BUFFER_BYTES, NETWORK_MAX_FRAME_BYTES, NETWORK_QUEUE_DESCRIPTORS};

use crate::virtio_queue::{
    clamp_used_length, le_u32, negotiated_queue_size, notify_delta, pending_used, read_shared,
    validate_used_id, write_shared,
};

const PCI_CONFIG_ADDRESS: u16 = 0xcf8;
const PCI_CONFIG_DATA: u16 = 0xcfc;
const PCI_CONFIG_BYTES: usize = 256;
const PCI_CAP_PTR: usize = 0x34;
const PCI_CAP_VENDOR_SPECIFIC: u8 = 0x09;
const PCI_CAP_MSIX: u8 = 0x11;
const VIRTIO_PCI_VENDOR_ID: u16 = 0x1af4;
const VIRTIO_NETWORK_MODERN_DEVICE_ID: u16 = 0x1041;
const VIRTIO_F_VERSION_1: u64 = 1 << 32;
const VIRTIO_NET_F_MAC: u64 = 1 << 5;
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
// QEMU's modern virtio-net transport uses the mergeable-buffer header layout
// for both directions, including the two-byte buffer-count field.
const VIRTIO_NET_HEADER_BYTES: usize = 12;
const VIRTIO_NET_HEADER_BUFFER_COUNT_OFFSET: usize = 10;
const ETHERNET_MIN_FRAME_BYTES: usize = 60;
const ISR_QUEUE_INTERRUPT: u8 = 1;
const RX_QUEUE: u16 = 0;
const TX_QUEUE: u16 = 1;
const NETWORK_VECTOR_INDEX: u16 = 0;
const MAX_CAPABILITIES: usize = 48;
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
    rx_descriptors: [Descriptor; NETWORK_QUEUE_DESCRIPTORS],
    rx_available_flags: u16,
    rx_available_index: u16,
    rx_available_ring: [u16; NETWORK_QUEUE_DESCRIPTORS],
    rx_available_used_event: u16,
    rx_available_padding: u16,
    rx_used_flags: u16,
    rx_used_index: u16,
    rx_used_ring: [UsedElement; NETWORK_QUEUE_DESCRIPTORS],
    rx_used_available_event: u16,
    rx_queue_padding: u16,
    tx_descriptors: [Descriptor; NETWORK_QUEUE_DESCRIPTORS],
    tx_available_flags: u16,
    tx_available_index: u16,
    tx_available_ring: [u16; NETWORK_QUEUE_DESCRIPTORS],
    tx_available_used_event: u16,
    tx_available_padding: u16,
    tx_used_flags: u16,
    tx_used_index: u16,
    tx_used_ring: [UsedElement; NETWORK_QUEUE_DESCRIPTORS],
    tx_used_available_event: u16,
    rx_buffers: [[u8; NETWORK_DMA_BUFFER_BYTES]; NETWORK_QUEUE_DESCRIPTORS],
    tx_buffers: [[u8; NETWORK_DMA_BUFFER_BYTES]; NETWORK_QUEUE_DESCRIPTORS],
}

const DMA_PAGE_BYTES: usize = 4096;
const DMA_QUEUE_PAGES: usize = core::mem::size_of::<QueueMemory>().div_ceil(DMA_PAGE_BYTES);
static DMA_QUEUE_ADDRESS: AtomicUsize = AtomicUsize::new(0);
static mut DEVICE: MaybeUninit<VirtioNetDevice> = MaybeUninit::uninit();
static DEVICE_READY: AtomicBool = AtomicBool::new(false);
static DEVICE_BUSY: AtomicBool = AtomicBool::new(false);

impl QueueMemory {
    /// Zeroes every ring, descriptor and buffer.
    fn clear(&mut self) {
        // SAFETY: `QueueMemory` is `repr(C)` plain data (integers and arrays of integers) for
        // which the all-zero pattern is valid, and `self` is a unique reference to all of it.
        unsafe { core::ptr::write_bytes(core::ptr::from_mut(self), 0, 1) };
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum NetworkDeviceError {
    NotFound,
    MissingCapability,
    MalformedCapability,
    CapabilityLoop,
    InvalidBar,
    FeatureNegotiation,
    DeviceRejectedFeatures,
    QueueUnavailable,
    QueueFull,
    InvalidFrame,
    StaleCompletion,
    Timeout,
    Io,
    Unsupported,
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
    unsafe fn new(address: u64, length: u32) -> Result<Self, NetworkDeviceError> {
        if address == 0 || address % 4 != 0 || length < 4 {
            return Err(NetworkDeviceError::InvalidBar);
        }
        Ok(Self { address, length })
    }

    fn ptr<T>(&self, offset: usize) -> Result<*mut T, NetworkDeviceError> {
        let size = core::mem::size_of::<T>();
        if offset.checked_add(size).is_none_or(|end| end > self.length as usize) {
            return Err(NetworkDeviceError::InvalidBar);
        }
        let address =
            self.address.checked_add(offset as u64).ok_or(NetworkDeviceError::InvalidBar)?;
        Ok(address as *mut T)
    }

    fn read<T: Copy>(&self, offset: usize) -> Result<T, NetworkDeviceError> {
        let pointer = self.ptr::<T>(offset)?;
        // SAFETY: `ptr` proved `offset..offset + size_of::<T>()` lies inside the region, and
        // `MmioRegion::new` obliges its caller to keep that window mapped device memory.
        Ok(unsafe { read_volatile(pointer) })
    }

    fn write<T: Copy>(&self, offset: usize, value: T) -> Result<(), NetworkDeviceError> {
        let pointer = self.ptr::<T>(offset)?;
        // SAFETY: as for `read`; the write stays inside the mapped device window.
        unsafe { write_volatile(pointer, value) };
        Ok(())
    }

    fn read_u8(&self, offset: usize) -> Result<u8, NetworkDeviceError> {
        self.read(offset)
    }

    fn read_u16(&self, offset: usize) -> Result<u16, NetworkDeviceError> {
        self.read(offset)
    }

    fn read_u32(&self, offset: usize) -> Result<u32, NetworkDeviceError> {
        self.read(offset)
    }

    fn write_u8(&self, offset: usize, value: u8) -> Result<(), NetworkDeviceError> {
        self.write(offset, value)
    }

    fn write_u16(&self, offset: usize, value: u16) -> Result<(), NetworkDeviceError> {
        self.write(offset, value)
    }

    fn write_u32(&self, offset: usize, value: u32) -> Result<(), NetworkDeviceError> {
        self.write(offset, value)
    }

    fn write_u64(&self, offset: usize, value: u64) -> Result<(), NetworkDeviceError> {
        self.write(offset, value)
    }
}

struct VirtioNetDevice {
    common: MmioRegion,
    notify: MmioRegion,
    isr: MmioRegion,
    device: MmioRegion,
    notify_multiplier: u32,
    queue: &'static mut QueueMemory,
    negotiated_features: u64,
    bars: [PciBar; 6],
    pci_address: logos_storage::PciAddress,
    msix: Option<MsixCapability>,
    rx_used_index: u16,
    tx_used_index: u16,
    tx_generations: [u16; NETWORK_QUEUE_DESCRIPTORS],
    /// TX slots whose buffer is currently owned by the device.
    tx_in_flight: [bool; NETWORK_QUEUE_DESCRIPTORS],
    /// RX buffers currently posted to (owned by) the device.
    rx_posted: [bool; NETWORK_QUEUE_DESCRIPTORS],
    pending_rx: Option<(u16, u16)>,
    mac: [u8; 6],
}

/// The single accessor for the network driver's `DEVICE` slot.
///
/// # Safety
///
/// `DEVICE_READY` must be set (the slot holds an initialized device) and the caller must hold
/// `DEVICE_BUSY`. That flag is the exclusivity lock: it is taken with a compare-exchange, so at
/// most one `&mut` produced here is live at a time, and it is released only after the reference
/// is dead.
unsafe fn device_mut() -> &'static mut VirtioNetDevice {
    // SAFETY: the caller upholds the contract above, so the slot is initialized and unaliased.
    unsafe { &mut *core::ptr::addr_of_mut!(DEVICE).cast::<VirtioNetDevice>() }
}

/// Stores a freshly initialized device into the `DEVICE` slot.
fn install_device(device: VirtioNetDevice) {
    // SAFETY: only called from `initialize` while `DEVICE_READY` is false, so no `device_mut`
    // reference exists; the type has no drop glue, so overwriting a stale device is sound.
    unsafe { core::ptr::addr_of_mut!(DEVICE).write(MaybeUninit::new(device)) };
}

/// The single accessor for the UEFI-allocated DMA arena.
///
/// # Safety
///
/// Called only from `VirtioNetDevice::initialize`. `prepare_dma` allocated, zeroed and
/// page-aligned `DMA_QUEUE_PAGES` pages at `DMA_QUEUE_ADDRESS`, which is never freed. The
/// returned reference is moved into the one device that owns the arena, so it stays unique.
unsafe fn queue_memory() -> Option<&'static mut QueueMemory> {
    let address = DMA_QUEUE_ADDRESS.load(Ordering::Acquire);
    if address == 0 {
        return None;
    }
    // SAFETY: the caller upholds the contract above; `address` is non-zero, page aligned and
    // backs at least `size_of::<QueueMemory>()` bytes.
    Some(unsafe { &mut *(address as *mut QueueMemory) })
}

/// Marks the device failed after it violated the VirtIO contract. The device is reset (best
/// effort) so it stops using the DMA arena, and the driver reports itself unavailable.
fn mark_failed(device: &VirtioNetDevice) {
    let _ = device.reset();
    DEVICE_READY.store(false, Ordering::Release);
}

pub(crate) fn reserve_frames(pool: &mut crate::frame_pool::FramePool) {
    let address = DMA_QUEUE_ADDRESS.load(Ordering::Acquire);
    if address != 0 {
        super::reserve_storage_frames(pool, address, DMA_QUEUE_PAGES * DMA_PAGE_BYTES);
    }
}

pub(crate) fn prepare_dma() -> bool {
    if DMA_QUEUE_ADDRESS.load(Ordering::Acquire) != 0 {
        return true;
    }
    let Ok(allocation) = uefi::boot::allocate_pages(
        uefi::boot::AllocateType::MaxAddress(u64::from(u32::MAX)),
        uefi::boot::MemoryType::LOADER_DATA,
        DMA_QUEUE_PAGES,
    ) else {
        return false;
    };
    let address = allocation.as_ptr() as usize;
    if address == 0 || address & (DMA_PAGE_BYTES - 1) != 0 || address > u32::MAX as usize {
        return false;
    }
    // SAFETY: `allocate_pages` just returned `DMA_QUEUE_PAGES` exclusively owned pages at
    // `address`, so zeroing exactly that many bytes stays inside the allocation.
    unsafe {
        core::ptr::write_bytes(address as *mut u8, 0, DMA_QUEUE_PAGES * DMA_PAGE_BYTES);
    }
    DMA_QUEUE_ADDRESS.store(address, Ordering::Release);
    true
}

pub(crate) fn initialize(config: logos_abi::NetworkConfig) -> bool {
    if !config.is_enabled() || DEVICE_READY.load(Ordering::Acquire) {
        return false;
    }
    let Ok(device) = VirtioNetDevice::initialize() else {
        return false;
    };
    install_device(device);
    DEVICE_READY.store(true, Ordering::Release);
    true
}

pub(crate) fn reset() {
    if !DEVICE_READY.load(Ordering::Acquire) {
        return;
    }
    if DEVICE_BUSY.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).is_err() {
        return;
    }
    // SAFETY: `DEVICE_READY` was observed set and `DEVICE_BUSY` was just acquired above.
    let result = unsafe { device_mut() }.reset_device();
    if result.is_err() {
        DEVICE_READY.store(false, Ordering::Release);
    }
    DEVICE_BUSY.store(false, Ordering::Release);
}

pub(crate) fn submit_frame(source: usize, length: usize) -> bool {
    if !DEVICE_READY.load(Ordering::Acquire) || source == 0 || length > NETWORK_MAX_FRAME_BYTES {
        return false;
    }
    let submitted = with_device_mut(|device| device.submit_tx(source, length)).is_ok();
    #[cfg(feature = "qemu-proof")]
    if submitted {
        crate::proof::network_tx();
    }
    submitted
}

pub(crate) fn take_frame(destination: usize) -> Option<usize> {
    if !DEVICE_READY.load(Ordering::Acquire) || destination == 0 {
        return None;
    }
    let length =
        with_device_mut(|device| device.take_rx(destination).map_err(|_| NetworkDeviceError::Io))
            .ok();
    #[cfg(feature = "qemu-proof")]
    if length.is_some() {
        crate::proof::network_rx();
    }
    length
}

pub(crate) fn handle_interrupt() {
    if !DEVICE_READY.load(Ordering::Acquire)
        || DEVICE_BUSY.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).is_err()
    {
        return;
    }
    // SAFETY: `DEVICE_READY` was observed set and `DEVICE_BUSY` was just acquired above.
    unsafe { device_mut() }.handle_interrupt();
    DEVICE_BUSY.store(false, Ordering::Release);
}

pub(crate) fn mac() -> Option<[u8; 6]> {
    if !DEVICE_READY.load(Ordering::Acquire) {
        return None;
    }
    with_device_mut(|device| Ok(device.mac)).ok()
}

fn with_device_mut<T>(
    operation: impl FnOnce(&mut VirtioNetDevice) -> Result<T, NetworkDeviceError>,
) -> Result<T, NetworkDeviceError> {
    if !DEVICE_READY.load(Ordering::Acquire) {
        return Err(NetworkDeviceError::NotFound);
    }
    if DEVICE_BUSY.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).is_err() {
        return Err(NetworkDeviceError::Io);
    }
    // SAFETY: `DEVICE_READY` was observed set and `DEVICE_BUSY` was just acquired above; the
    // reference does not outlive `operation`, and the flag is released only afterwards.
    let result = operation(unsafe { device_mut() });
    DEVICE_BUSY.store(false, Ordering::Release);
    result
}

impl VirtioNetDevice {
    fn initialize() -> Result<Self, NetworkDeviceError> {
        let probe = PciConfig::find().ok_or(NetworkDeviceError::NotFound)?;
        PciConfig::enable_device(probe.address);
        let common = region_for(&probe, probe.capabilities.common)?;
        let notify = region_for(&probe, probe.capabilities.notify)?;
        let isr = region_for(&probe, probe.capabilities.isr)?;
        let device = region_for(&probe, probe.capabilities.device)?;
        // SAFETY: `initialize` is the only caller and runs while `DEVICE_READY` is false, so no
        // earlier device still uses the arena.
        let queue = unsafe { queue_memory() }.ok_or(NetworkDeviceError::InvalidBar)?;
        queue.clear();
        let mut device = Self {
            common,
            notify,
            isr,
            device,
            notify_multiplier: probe.capabilities.notify_multiplier,
            queue,
            negotiated_features: 0,
            bars: probe.bars,
            pci_address: probe.address,
            msix: probe.msix,
            rx_used_index: 0,
            tx_used_index: 0,
            tx_generations: [0; NETWORK_QUEUE_DESCRIPTORS],
            tx_in_flight: [false; NETWORK_QUEUE_DESCRIPTORS],
            rx_posted: [false; NETWORK_QUEUE_DESCRIPTORS],
            pending_rx: None,
            mac: [0; 6],
        };
        device.reset()?;
        device.common.write_u8(COMMON_DEVICE_STATUS, STATUS_ACKNOWLEDGE)?;
        device.common.write_u8(COMMON_DEVICE_STATUS, STATUS_ACKNOWLEDGE | STATUS_DRIVER)?;
        let features = device.device_features()?;
        if features & VIRTIO_F_VERSION_1 == 0 || features & VIRTIO_NET_F_MAC == 0 {
            return Err(NetworkDeviceError::FeatureNegotiation);
        }
        device.negotiated_features = VIRTIO_F_VERSION_1 | VIRTIO_NET_F_MAC;
        device.driver_features(device.negotiated_features)?;
        device.common.write_u8(
            COMMON_DEVICE_STATUS,
            STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK,
        )?;
        if device.common.read_u8(COMMON_DEVICE_STATUS)? & STATUS_FEATURES_OK == 0 {
            return Err(NetworkDeviceError::DeviceRejectedFeatures);
        }
        device.configure_queue(RX_QUEUE)?;
        device.configure_queue(TX_QUEUE)?;
        device.configure_interrupts()?;
        for index in 0..6 {
            device.mac[index] = device.device.read_u8(index)?;
        }
        device.common.write_u8(
            COMMON_DEVICE_STATUS,
            STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK | STATUS_DRIVER_OK,
        )?;
        device.post_rx_buffers()?;
        Ok(device)
    }

    fn reset(&self) -> Result<(), NetworkDeviceError> {
        self.common.write_u8(COMMON_DEVICE_STATUS, 0)?;
        for _ in 0..1024 {
            if self.common.read_u8(COMMON_DEVICE_STATUS)? == 0 {
                return Ok(());
            }
            core::hint::spin_loop();
        }
        Err(NetworkDeviceError::Timeout)
    }

    fn reset_device(&mut self) -> Result<(), NetworkDeviceError> {
        self.reset()?;
        self.queue.clear();
        self.rx_used_index = 0;
        self.tx_used_index = 0;
        self.tx_generations.fill(0);
        self.tx_in_flight.fill(false);
        self.rx_posted.fill(false);
        self.pending_rx = None;
        self.common.write_u8(COMMON_DEVICE_STATUS, STATUS_ACKNOWLEDGE)?;
        self.common.write_u8(COMMON_DEVICE_STATUS, STATUS_ACKNOWLEDGE | STATUS_DRIVER)?;
        self.driver_features(self.negotiated_features)?;
        self.common.write_u8(
            COMMON_DEVICE_STATUS,
            STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK,
        )?;
        if self.common.read_u8(COMMON_DEVICE_STATUS)? & STATUS_FEATURES_OK == 0 {
            return Err(NetworkDeviceError::DeviceRejectedFeatures);
        }
        self.configure_queue(RX_QUEUE)?;
        self.configure_queue(TX_QUEUE)?;
        self.configure_interrupts()?;
        self.common.write_u8(
            COMMON_DEVICE_STATUS,
            STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK | STATUS_DRIVER_OK,
        )?;
        self.post_rx_buffers()
    }

    fn device_features(&self) -> Result<u64, NetworkDeviceError> {
        self.common.write_u32(COMMON_DEVICE_FEATURE_SELECT, 0)?;
        let low = self.common.read_u32(COMMON_DEVICE_FEATURE)? as u64;
        self.common.write_u32(COMMON_DEVICE_FEATURE_SELECT, 1)?;
        let high = self.common.read_u32(COMMON_DEVICE_FEATURE)? as u64;
        Ok(low | high << 32)
    }

    fn driver_features(&self, features: u64) -> Result<(), NetworkDeviceError> {
        self.common.write_u32(COMMON_DRIVER_FEATURE_SELECT, 0)?;
        self.common.write_u32(COMMON_DRIVER_FEATURE, features as u32)?;
        self.common.write_u32(COMMON_DRIVER_FEATURE_SELECT, 1)?;
        self.common.write_u32(COMMON_DRIVER_FEATURE, (features >> 32) as u32)
    }

    fn configure_queue(&mut self, index: u16) -> Result<(), NetworkDeviceError> {
        {
            self.common.write_u16(COMMON_QUEUE_SELECT, index)?;
            // The device reports its maximum; the rings are laid out for exactly
            // `NETWORK_QUEUE_DESCRIPTORS` entries, so program that size and require the device
            // to accept it.
            let maximum = self.common.read_u16(COMMON_QUEUE_SIZE)?;
            let size = negotiated_queue_size(maximum, NETWORK_QUEUE_DESCRIPTORS)
                .ok_or(NetworkDeviceError::QueueUnavailable)?;
            self.common.write_u16(COMMON_QUEUE_SIZE, size)?;
            if self.common.read_u16(COMMON_QUEUE_SIZE)? != size {
                return Err(NetworkDeviceError::QueueUnavailable);
            }
            let (descriptors, driver, device) = if index == RX_QUEUE {
                (
                    self.queue.rx_descriptors.as_ptr(),
                    &self.queue.rx_available_flags,
                    &self.queue.rx_used_flags,
                )
            } else {
                (
                    self.queue.tx_descriptors.as_ptr(),
                    &self.queue.tx_available_flags,
                    &self.queue.tx_used_flags,
                )
            };
            self.common.write_u64(COMMON_QUEUE_DESC, descriptors as u64)?;
            self.common.write_u64(COMMON_QUEUE_DRIVER, driver as *const u16 as u64)?;
            self.common.write_u64(COMMON_QUEUE_DEVICE, device as *const u16 as u64)?;
            self.common.write_u16(COMMON_QUEUE_MSIX_VECTOR, NETWORK_VECTOR_INDEX)?;
            self.common.write_u16(COMMON_QUEUE_ENABLE, 1)
        }
    }

    fn configure_interrupts(&mut self) -> Result<(), NetworkDeviceError> {
        let Some(msix) = self.msix else {
            return Err(NetworkDeviceError::Unsupported);
        };
        let table_bar =
            *self.bars.get(msix.table_bar as usize).ok_or(NetworkDeviceError::InvalidBar)?;
        if table_bar.base == 0
            || u64::from(msix.table_offset)
                .checked_add(u64::from(MSIX_TABLE_ENTRY_BYTES))
                .is_none_or(|end| end > table_bar.length)
        {
            return Err(NetworkDeviceError::InvalidBar);
        }
        let table_address = table_bar
            .base
            .checked_add(u64::from(msix.table_offset))
            .ok_or(NetworkDeviceError::InvalidBar)?;
        // SAFETY: the table entry window was checked above to lie inside the MSI-X table BAR.
        let table = unsafe { MmioRegion::new(table_address, MSIX_TABLE_ENTRY_BYTES)? };
        let apic_id = super::APIC_IDS[0].load(Ordering::Acquire);
        let message_address = 0xfee0_0000u64 | (u64::from(apic_id) << 12);
        table.write_u32(12, 1)?;
        table.write_u32(0, message_address as u32)?;
        table.write_u32(4, (message_address >> 32) as u32)?;
        table.write_u32(8, u32::from(super::NETWORK_VECTOR))?;
        self.common.write_u16(COMMON_MSIX_CONFIG, NETWORK_VECTOR_INDEX)?;
        let control = config_read_u16(self.pci_address, msix.cap_offset.wrapping_add(2));
        config_write_u16(
            self.pci_address,
            msix.cap_offset.wrapping_add(2),
            (control | (1 << 15)) & !(1 << 14),
        );
        table.write_u32(12, 0)?;
        Ok(())
    }

    fn post_rx_buffers(&mut self) -> Result<(), NetworkDeviceError> {
        for index in 0..NETWORK_QUEUE_DESCRIPTORS {
            self.queue.rx_descriptors[index] = Descriptor {
                address: self.queue.rx_buffers[index].as_ptr() as u64,
                length: NETWORK_DMA_BUFFER_BYTES as u32,
                flags: VIRTQ_DESC_F_WRITE,
                next: 0,
            };
            self.queue.rx_available_ring[index] = index as u16;
            self.rx_posted[index] = true;
        }
        fence(Ordering::Release);
        self.queue.rx_available_index = NETWORK_QUEUE_DESCRIPTORS as u16;
        self.notify_queue(RX_QUEUE)
    }

    fn submit_tx(&mut self, source: usize, length: usize) -> Result<(), NetworkDeviceError> {
        if length == 0 || length > NETWORK_MAX_FRAME_BYTES {
            return Err(NetworkDeviceError::InvalidFrame);
        }
        self.reclaim_tx()?;
        let available = self.queue.tx_available_index;
        if available.wrapping_sub(self.tx_used_index) as usize >= NETWORK_QUEUE_DESCRIPTORS {
            return Err(NetworkDeviceError::QueueFull);
        }
        let slot = usize::from(available) % NETWORK_QUEUE_DESCRIPTORS;
        let frame_length = length.max(ETHERNET_MIN_FRAME_BYTES);
        let total = VIRTIO_NET_HEADER_BYTES
            .checked_add(frame_length)
            .ok_or(NetworkDeviceError::InvalidFrame)?;
        if total > NETWORK_DMA_BUFFER_BYTES {
            return Err(NetworkDeviceError::InvalidFrame);
        }
        self.queue.tx_buffers[slot][..VIRTIO_NET_HEADER_BYTES].fill(0);
        self.queue.tx_buffers[slot][VIRTIO_NET_HEADER_BUFFER_COUNT_OFFSET..VIRTIO_NET_HEADER_BYTES]
            .copy_from_slice(&1u16.to_le_bytes());
        // SAFETY: `source` is a non-zero Core-validated frame buffer of at least `length` bytes
        // (`submit_frame` bounds `length` by `NETWORK_MAX_FRAME_BYTES`); the destination has
        // room for `total <= NETWORK_DMA_BUFFER_BYTES` bytes, checked above, and the regions
        // are distinct allocations.
        unsafe {
            copy_nonoverlapping(
                source as *const u8,
                self.queue.tx_buffers[slot].as_mut_ptr().add(VIRTIO_NET_HEADER_BYTES),
                length,
            );
        }
        self.queue.tx_buffers[slot]
            [VIRTIO_NET_HEADER_BYTES + length..VIRTIO_NET_HEADER_BYTES + frame_length]
            .fill(0);
        self.queue.tx_descriptors[slot] = Descriptor {
            address: self.queue.tx_buffers[slot].as_ptr() as u64,
            length: total as u32,
            flags: 0,
            next: 0,
        };
        self.queue.tx_available_ring[usize::from(available) % NETWORK_QUEUE_DESCRIPTORS] =
            slot as u16;
        self.tx_generations[slot] = available;
        self.tx_in_flight[slot] = true;
        fence(Ordering::Release);
        write_shared(&mut self.queue.tx_available_index, available.wrapping_add(1));
        self.notify_queue(TX_QUEUE)
    }

    fn reclaim_tx(&mut self) -> Result<(), NetworkDeviceError> {
        let device_used = read_shared(&self.queue.tx_used_index);
        if pending_used(device_used, self.tx_used_index, NETWORK_QUEUE_DESCRIPTORS).is_err() {
            return Err(self.device_violation());
        }
        while self.tx_used_index != device_used {
            let sequence = self.tx_used_index;
            let slot = usize::from(sequence) % NETWORK_QUEUE_DESCRIPTORS;
            let element = read_shared(&self.queue.tx_used_ring[slot]);
            let in_flight = &self.tx_in_flight;
            let valid = validate_used_id(element.id, 1, NETWORK_QUEUE_DESCRIPTORS, |candidate| {
                in_flight[candidate]
            });
            if valid != Ok(slot)
                || self.tx_generations[slot] != sequence
                || element.length as usize > NETWORK_DMA_BUFFER_BYTES
            {
                return Err(self.device_violation());
            }
            self.tx_generations[slot] = 0;
            self.tx_in_flight[slot] = false;
            self.tx_used_index = self.tx_used_index.wrapping_add(1);
        }
        Ok(())
    }

    /// Records that the device broke the used-ring contract and takes it offline.
    fn device_violation(&self) -> NetworkDeviceError {
        mark_failed(self);
        NetworkDeviceError::StaleCompletion
    }

    fn handle_interrupt(&mut self) {
        let Ok(status) = self.isr.read_u8(0) else {
            return;
        };
        if status & ISR_QUEUE_INTERRUPT == 0 {
            return;
        }
        let _ = self.reclaim_tx();
        let _ = self.reap_rx();
    }

    fn reap_rx(&mut self) -> Result<(), NetworkDeviceError> {
        if self.pending_rx.is_some() {
            return Ok(());
        }
        let device_used = read_shared(&self.queue.rx_used_index);
        match pending_used(device_used, self.rx_used_index, NETWORK_QUEUE_DESCRIPTORS) {
            Err(_) => return Err(self.device_violation()),
            Ok(0) => return Err(NetworkDeviceError::Timeout),
            Ok(_) => {}
        }
        let slot = usize::from(self.rx_used_index) % NETWORK_QUEUE_DESCRIPTORS;
        let element = read_shared(&self.queue.rx_used_ring[slot]);
        let posted = &self.rx_posted;
        let Ok(buffer) = validate_used_id(element.id, 1, NETWORK_QUEUE_DESCRIPTORS, |candidate| {
            posted[candidate]
        }) else {
            return Err(self.device_violation());
        };
        if (element.length as usize) < VIRTIO_NET_HEADER_BYTES {
            return Err(self.device_violation());
        }
        let length = clamp_used_length(element.length, NETWORK_DMA_BUFFER_BYTES as u32);
        let Ok(length) = u16::try_from(length) else {
            return Err(self.device_violation());
        };
        self.rx_posted[buffer] = false;
        self.pending_rx = Some((buffer as u16, length));
        self.rx_used_index = self.rx_used_index.wrapping_add(1);
        Ok(())
    }

    fn take_rx(&mut self, destination: usize) -> Result<usize, NetworkDeviceError> {
        self.reap_rx()?;
        let Some((slot, length)) = self.pending_rx.take() else {
            return Err(NetworkDeviceError::Timeout);
        };
        let frame_length = usize::from(length)
            .checked_sub(VIRTIO_NET_HEADER_BYTES)
            .ok_or(NetworkDeviceError::InvalidFrame)?;
        let buffer =
            self.queue.rx_buffers.get(usize::from(slot)).ok_or(NetworkDeviceError::InvalidFrame)?;
        let payload = buffer
            .get(VIRTIO_NET_HEADER_BYTES..VIRTIO_NET_HEADER_BYTES + frame_length)
            .ok_or(NetworkDeviceError::InvalidFrame)?;
        // SAFETY: `destination` is a non-zero Core-provided receive page with room for a full
        // frame (`take_frame` callers pass the Network service's private packet page);
        // `payload` is a bounds-checked slice of the DMA buffer, so the regions are distinct
        // and `frame_length` bytes are readable.
        unsafe {
            copy_nonoverlapping(payload.as_ptr(), destination as *mut u8, frame_length);
        }
        let available = self.queue.rx_available_index;
        self.queue.rx_available_ring[usize::from(available) % NETWORK_QUEUE_DESCRIPTORS] = slot;
        self.rx_posted[usize::from(slot) % NETWORK_QUEUE_DESCRIPTORS] = true;
        fence(Ordering::Release);
        write_shared(&mut self.queue.rx_available_index, available.wrapping_add(1));
        self.notify_queue(RX_QUEUE)?;
        Ok(frame_length)
    }

    fn notify_queue(&self, queue: u16) -> Result<(), NetworkDeviceError> {
        self.common.write_u16(COMMON_QUEUE_SELECT, queue)?;
        let offset = self.common.read_u16(COMMON_QUEUE_NOTIFY_OFF)?;
        let delta = notify_delta(self.notify.length, offset, self.notify_multiplier)
            .ok_or(NetworkDeviceError::InvalidBar)?;
        self.notify.write_u16(delta, queue)
    }
}

fn region_for(
    probe: &PciProbe,
    capability: logos_storage::VirtioPciCapability,
) -> Result<MmioRegion, NetworkDeviceError> {
    let bar = *probe.bars.get(capability.bar as usize).ok_or(NetworkDeviceError::InvalidBar)?;
    if bar.base == 0
        || u64::from(capability.offset)
            .checked_add(u64::from(capability.length))
            .is_none_or(|end| end > bar.length)
    {
        return Err(NetworkDeviceError::InvalidBar);
    }
    let address =
        bar.base.checked_add(u64::from(capability.offset)).ok_or(NetworkDeviceError::InvalidBar)?;
    // SAFETY: the capability window was checked to lie inside a BAR the device decodes.
    unsafe { MmioRegion::new(address, capability.length) }
}

struct PciConfig;

impl PciConfig {
    fn find() -> Option<PciProbe> {
        for bus in 0..=u8::MAX {
            for device in 0..32u8 {
                for function in 0..8u8 {
                    let address = logos_storage::PciAddress::new(bus, device, function)?;
                    let config = Self::read_config(address);
                    let vendor = u16::from_le_bytes([config[0], config[1]]);
                    let device_id = u16::from_le_bytes([config[2], config[3]]);
                    if vendor != VIRTIO_PCI_VENDOR_ID
                        || device_id != VIRTIO_NETWORK_MODERN_DEVICE_ID
                    {
                        continue;
                    }
                    let capabilities = parse_capabilities(&config).ok()?;
                    return Some(PciProbe {
                        address,
                        capabilities,
                        bars: Self::bars(address, &config),
                        msix: Self::msix(&config),
                    });
                }
            }
        }
        None
    }

    fn read_config(address: logos_storage::PciAddress) -> [u8; PCI_CONFIG_BYTES] {
        let mut config = [0; PCI_CONFIG_BYTES];
        for offset in (0..PCI_CONFIG_BYTES).step_by(4) {
            let value = config_read(address, offset as u8);
            config[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        config
    }

    fn bars(address: logos_storage::PciAddress, config: &[u8; PCI_CONFIG_BYTES]) -> [PciBar; 6] {
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

    fn msix(config: &[u8; PCI_CONFIG_BYTES]) -> Option<MsixCapability> {
        let mut offset = config[PCI_CAP_PTR];
        let mut seen = [false; PCI_CONFIG_BYTES];
        for _ in 0..MAX_CAPABILITIES {
            if offset == 0 || usize::from(offset) + 2 > config.len() || seen[offset as usize] {
                break;
            }
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
                if (control & 0x07ff) != 0 {
                    return Some(MsixCapability {
                        cap_offset: offset,
                        table_bar: (table & 0x7) as u8,
                        table_offset: table & !0x7,
                    });
                }
            }
            offset = next;
        }
        None
    }
}

fn parse_capabilities(
    config: &[u8; PCI_CONFIG_BYTES],
) -> Result<logos_storage::VirtioPciCapabilities, NetworkDeviceError> {
    let mut cursor = config[PCI_CAP_PTR] as usize;
    let mut seen = [false; PCI_CONFIG_BYTES];
    let mut capabilities = [None; 5];
    let mut notify_multiplier = 0;
    for _ in 0..MAX_CAPABILITIES {
        if cursor == 0 {
            break;
        }
        if cursor + 3 >= config.len() || seen[cursor] {
            return Err(NetworkDeviceError::CapabilityLoop);
        }
        seen[cursor] = true;
        if config[cursor] != PCI_CAP_VENDOR_SPECIFIC {
            cursor = config[cursor + 1] as usize;
            continue;
        }
        let next = config[cursor + 1] as usize;
        let length = config[cursor + 2] as usize;
        if length < 16 || cursor + length > config.len() {
            return Err(NetworkDeviceError::MalformedCapability);
        }
        let cfg_type = config[cursor + 3] as usize;
        if cfg_type == 5 {
            cursor = next;
            continue;
        }
        if cfg_type >= capabilities.len() {
            return Err(NetworkDeviceError::MalformedCapability);
        }
        let capability_length = length;
        let bar = config[cursor + 4];
        let (Some(offset), Some(length)) =
            (le_u32(config, cursor + 8), le_u32(config, cursor + 12))
        else {
            return Err(NetworkDeviceError::MalformedCapability);
        };
        if bar >= 6 || length == 0 || offset.checked_add(length).is_none() {
            return Err(NetworkDeviceError::MalformedCapability);
        }
        // The multiplier lives in the capability structure itself, so gate on the structure
        // length (not the BAR window length) and bounds-check the read.
        if cfg_type == 2 && capability_length >= 20 {
            notify_multiplier =
                le_u32(config, cursor + 16).ok_or(NetworkDeviceError::MalformedCapability)?;
        }
        capabilities[cfg_type] = Some(logos_storage::VirtioPciCapability { bar, offset, length });
        cursor = next;
    }
    if cursor != 0 {
        return Err(NetworkDeviceError::CapabilityLoop);
    }
    Ok(logos_storage::VirtioPciCapabilities {
        common: capabilities[1].ok_or(NetworkDeviceError::MissingCapability)?,
        notify: capabilities[2].ok_or(NetworkDeviceError::MissingCapability)?,
        notify_multiplier,
        isr: capabilities[3].ok_or(NetworkDeviceError::MissingCapability)?,
        device: capabilities[4].ok_or(NetworkDeviceError::MissingCapability)?,
    })
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
    let address_value = 0x8000_0000u32
        | (u32::from(address.bus()) << 16)
        | (u32::from(address.device()) << 11)
        | (u32::from(address.function()) << 8)
        | u32::from(offset & 0xfc);
    // SAFETY: port I/O to the PCI configuration ports (0xcf8/0xcfc) touches no Rust-visible
    // memory; the kernel runs at ring 0 and is the sole owner of these ports.
    unsafe {
        asm!("out dx, eax", in("dx") PCI_CONFIG_ADDRESS, in("eax") address_value, options(nostack, preserves_flags));
        let value: u32;
        asm!("in eax, dx", in("dx") PCI_CONFIG_DATA, out("eax") value, options(nostack, preserves_flags));
        value
    }
}

fn config_write(address: logos_storage::PciAddress, offset: u8, value: u32) {
    let address_value = 0x8000_0000u32
        | (u32::from(address.bus()) << 16)
        | (u32::from(address.device()) << 11)
        | (u32::from(address.function()) << 8)
        | u32::from(offset & 0xfc);
    // SAFETY: port I/O to the PCI configuration ports (0xcf8/0xcfc) touches no Rust-visible
    // memory; the kernel runs at ring 0 and is the sole owner of these ports.
    unsafe {
        asm!("out dx, eax", in("dx") PCI_CONFIG_ADDRESS, in("eax") address_value, options(nostack, preserves_flags));
        asm!("out dx, eax", in("dx") PCI_CONFIG_DATA, in("eax") value, options(nostack, preserves_flags));
    }
}

fn config_read_u16(address: logos_storage::PciAddress, offset: u8) -> u16 {
    let value = config_read(address, offset);
    (value >> ((offset & 2) * 8)) as u16
}

fn config_write_u16(address: logos_storage::PciAddress, offset: u8, value: u16) {
    let current = config_read(address, offset);
    let shift = (offset & 2) * 8;
    let merged = (current & !(0xffff << shift)) | (u32::from(value) << shift);
    config_write(address, offset, merged);
}
