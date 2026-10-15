//! VirtIO block queue memory layout, kept hardware-free so its reset contract is host-tested.
//!
//! This module also owns the pure validation of device-supplied queue values (used-ring indices,
//! descriptor ids, lengths, queue sizes, notify offsets). Every value the device writes is
//! untrusted; drivers must route it through these checks before using it as an index or length.

#![allow(dead_code)]

use core::ptr::{read_volatile, write_volatile};

use logos_storage::VirtioBlkHeader;

pub(crate) const QUEUE_SIZE: usize = 8;

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct Descriptor {
    pub(crate) address: u64,
    pub(crate) length: u32,
    pub(crate) flags: u16,
    pub(crate) next: u16,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct UsedElement {
    pub(crate) id: u32,
    pub(crate) length: u32,
}

#[repr(C, align(4096))]
pub(crate) struct DmaBlock {
    pub(crate) bytes: [u8; logos_storage::BLOCK_BYTES],
}

impl DmaBlock {
    const fn new() -> Self {
        Self { bytes: [0; logos_storage::BLOCK_BYTES] }
    }
}

#[repr(C, align(4096))]
pub(crate) struct QueueMemory {
    pub(crate) descriptors: [Descriptor; QUEUE_SIZE * 3],
    pub(crate) available_flags: u16,
    pub(crate) available_index: u16,
    pub(crate) available_ring: [u16; QUEUE_SIZE],
    pub(crate) available_used_event: u16,
    pub(crate) available_padding: u16,
    pub(crate) used_flags: u16,
    pub(crate) used_index: u16,
    pub(crate) used_ring: [UsedElement; QUEUE_SIZE],
    pub(crate) used_available_event: u16,
    pub(crate) headers: [VirtioBlkHeader; QUEUE_SIZE],
    pub(crate) statuses: [u8; QUEUE_SIZE],
    pub(crate) data: DmaBlock,
}

impl QueueMemory {
    const EMPTY_DESCRIPTOR: Descriptor = Descriptor { address: 0, length: 0, flags: 0, next: 0 };
    const EMPTY_USED: UsedElement = UsedElement { id: 0, length: 0 };

    pub(crate) const fn new() -> Self {
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
            data: DmaBlock::new(),
        }
    }

    /// Resets the rings for a device reset. The DMA data block is deliberately
    /// kept: a write payload may already be staged there when a rollover or
    /// timeout reset happens, and discarding it would write zeros to disk.
    pub(crate) fn reset_rings(&mut self) {
        let fresh = Self::new();
        self.descriptors = fresh.descriptors;
        self.available_flags = fresh.available_flags;
        self.available_index = fresh.available_index;
        self.available_ring = fresh.available_ring;
        self.available_used_event = fresh.available_used_event;
        self.available_padding = fresh.available_padding;
        self.used_flags = fresh.used_flags;
        self.used_index = fresh.used_index;
        self.used_ring = fresh.used_ring;
        self.used_available_event = fresh.used_available_event;
        self.headers = fresh.headers;
        self.statuses = fresh.statuses;
    }
}

/// Why a device-supplied used-ring value was rejected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum UsedError {
    /// The device claims more completions than the queue can hold.
    IndexOverrun,
    /// The used id does not name a descriptor chain head of this queue.
    IdOutOfRange,
    /// The used id names a chain that is not currently posted (duplicate or forged completion).
    NotInFlight,
}

/// Number of unread used-ring entries given the device's and the driver's free-running indices.
pub(crate) fn pending_used(
    device_index: u16,
    driver_index: u16,
    queue_size: usize,
) -> Result<usize, UsedError> {
    let pending = usize::from(device_index.wrapping_sub(driver_index));
    if pending > queue_size { Err(UsedError::IndexOverrun) } else { Ok(pending) }
}

/// Maps a used-ring `id` to a queue slot. Chain heads sit at `slot * descriptors_per_slot`;
/// `in_flight(slot)` must report whether that slot currently has a posted request.
pub(crate) fn validate_used_id(
    id: u32,
    descriptors_per_slot: usize,
    queue_size: usize,
    in_flight: impl Fn(usize) -> bool,
) -> Result<usize, UsedError> {
    let id = usize::try_from(id).map_err(|_| UsedError::IdOutOfRange)?;
    if descriptors_per_slot == 0 || id % descriptors_per_slot != 0 {
        return Err(UsedError::IdOutOfRange);
    }
    let slot = id / descriptors_per_slot;
    if slot >= queue_size {
        return Err(UsedError::IdOutOfRange);
    }
    if in_flight(slot) { Ok(slot) } else { Err(UsedError::NotInFlight) }
}

/// Clamps a device-reported used length to the device-writable bytes that were posted.
pub(crate) fn clamp_used_length(reported: u32, writable: u32) -> u32 {
    reported.min(writable)
}

/// Chooses the queue size the driver programs. The device reports its maximum; the driver size
/// must be a non-zero power of two within it and within the static queue memory.
pub(crate) fn negotiated_queue_size(device_maximum: u16, driver_size: usize) -> Option<u16> {
    let size = u16::try_from(driver_size).ok()?;
    if size == 0 || !size.is_power_of_two() || device_maximum < size {
        return None;
    }
    Some(size)
}

/// Byte offset of a queue's notify doorbell inside the notify capability region, or `None` when
/// the device-supplied `queue_notify_off` places a 2-byte doorbell outside it.
pub(crate) fn notify_delta(
    region_length: u32,
    queue_notify_off: u16,
    multiplier: u32,
) -> Option<usize> {
    let delta = u64::from(queue_notify_off).checked_mul(u64::from(multiplier))?;
    let end = delta.checked_add(2)?;
    if end > u64::from(region_length) { None } else { usize::try_from(delta).ok() }
}

/// Reads a little-endian `u32` at `offset`, or `None` when it would run past `bytes`.
pub(crate) fn le_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    let end = offset.checked_add(4)?;
    let chunk: [u8; 4] = bytes.get(offset..end)?.try_into().ok()?;
    Some(u32::from_le_bytes(chunk))
}

/// Volatile read of a field in device-shared queue memory.
pub(crate) fn read_shared<T: Copy>(field: &T) -> T {
    // SAFETY: `field` is a live, aligned reference, so the pointer is valid for a read of `T`.
    // Volatile prevents the compiler from caching a value the device may change.
    unsafe { read_volatile(field) }
}

/// Volatile write of a field in device-shared queue memory.
pub(crate) fn write_shared<T: Copy>(field: &mut T, value: T) {
    // SAFETY: `field` is a live, aligned, exclusive reference, so the pointer is valid for a
    // write of `T`. Volatile keeps the store ordered with the surrounding fences.
    unsafe { write_volatile(field, value) }
}

const _: () = assert!(core::mem::align_of::<DmaBlock>() == 4096);
const _: () = assert!(core::mem::offset_of!(QueueMemory, data) % 4096 == 0);

#[cfg(test)]
mod tests {
    use super::*;
    use std::boxed::Box;

    #[test]
    fn reset_keeps_a_staged_write_payload() {
        let mut queue = Box::new(QueueMemory::new());
        queue.data.bytes.fill(0xa5);
        queue.available_index = (QUEUE_SIZE - 1) as u16;
        queue.used_index = 3;
        queue.statuses[2] = 0;

        queue.reset_rings();

        assert!(queue.data.bytes.iter().all(|byte| *byte == 0xa5));
        assert_eq!(queue.available_index, 0);
        assert_eq!(queue.used_index, 0);
        assert_eq!(queue.statuses, [0xff; QUEUE_SIZE]);
    }

    #[test]
    fn used_index_overrun_is_rejected() {
        assert_eq!(pending_used(5, 5, QUEUE_SIZE), Ok(0));
        assert_eq!(pending_used(8, 0, QUEUE_SIZE), Ok(8));
        assert_eq!(pending_used(9, 0, QUEUE_SIZE), Err(UsedError::IndexOverrun));
        assert_eq!(pending_used(0, 1, QUEUE_SIZE), Err(UsedError::IndexOverrun));
        assert_eq!(pending_used(2, u16::MAX - 1, QUEUE_SIZE), Ok(4));
    }

    #[test]
    fn used_id_out_of_range_is_rejected() {
        let all = |_| true;
        assert_eq!(validate_used_id(0, 3, QUEUE_SIZE, all), Ok(0));
        assert_eq!(validate_used_id(21, 3, QUEUE_SIZE, all), Ok(7));
        assert_eq!(validate_used_id(24, 3, QUEUE_SIZE, all), Err(UsedError::IdOutOfRange));
        assert_eq!(validate_used_id(u32::MAX, 3, QUEUE_SIZE, all), Err(UsedError::IdOutOfRange));
        assert_eq!(validate_used_id(1, 3, QUEUE_SIZE, all), Err(UsedError::IdOutOfRange));
        assert_eq!(validate_used_id(0, 0, QUEUE_SIZE, all), Err(UsedError::IdOutOfRange));
    }

    #[test]
    fn duplicate_completion_is_rejected() {
        let mut posted = [false; QUEUE_SIZE];
        posted[2] = true;
        assert_eq!(validate_used_id(6, 3, QUEUE_SIZE, |slot| posted[slot]), Ok(2));
        posted[2] = false;
        assert_eq!(
            validate_used_id(6, 3, QUEUE_SIZE, |slot| posted[slot]),
            Err(UsedError::NotInFlight)
        );
        assert_eq!(
            validate_used_id(9, 3, QUEUE_SIZE, |slot| posted[slot]),
            Err(UsedError::NotInFlight)
        );
    }

    #[test]
    fn used_length_is_clamped_to_posted_buffer() {
        assert_eq!(clamp_used_length(0, 4097), 0);
        assert_eq!(clamp_used_length(4097, 4097), 4097);
        assert_eq!(clamp_used_length(4098, 4097), 4097);
        assert_eq!(clamp_used_length(u32::MAX, 1), 1);
    }

    #[test]
    fn queue_size_must_fit_static_memory() {
        assert_eq!(negotiated_queue_size(256, QUEUE_SIZE), Some(8));
        assert_eq!(negotiated_queue_size(8, QUEUE_SIZE), Some(8));
        assert_eq!(negotiated_queue_size(7, QUEUE_SIZE), None);
        assert_eq!(negotiated_queue_size(0, QUEUE_SIZE), None);
        assert_eq!(negotiated_queue_size(u16::MAX, 0), None);
        assert_eq!(negotiated_queue_size(u16::MAX, 6), None);
        assert_eq!(negotiated_queue_size(u16::MAX, 1 << 20), None);
    }

    #[test]
    fn notify_offset_must_stay_inside_region() {
        assert_eq!(notify_delta(8, 0, 4), Some(0));
        assert_eq!(notify_delta(8, 1, 4), Some(4));
        assert_eq!(notify_delta(8, 2, 4), None);
        assert_eq!(notify_delta(8, u16::MAX, u32::MAX), None);
        assert_eq!(notify_delta(8, 5, 0), Some(0));
        assert_eq!(notify_delta(1, 0, 0), None);
    }

    #[test]
    fn little_endian_reads_are_bounds_checked() {
        let bytes = [1, 0, 0, 0, 2, 0, 0, 0];
        assert_eq!(le_u32(&bytes, 0), Some(1));
        assert_eq!(le_u32(&bytes, 4), Some(2));
        assert_eq!(le_u32(&bytes, 5), None);
        assert_eq!(le_u32(&bytes, usize::MAX), None);
    }
}
