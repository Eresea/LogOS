//! VirtIO block queue memory layout, kept hardware-free so its reset contract is host-tested.

#![allow(dead_code)]

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
}
