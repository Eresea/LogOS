use crate::request::validate_request;
use crate::{
    BLOCK_BYTES, BlockCompletion, BlockOperation, BlockRequest, BlockRequestId, BlockStatus,
    BufferToken,
};

pub const VIRTIO_SECTOR_BYTES: u64 = 512;
pub const SECTORS_PER_LOGOS_BLOCK: u64 = (BLOCK_BYTES as u64) / VIRTIO_SECTOR_BYTES;
pub const MAX_VIRTIO_QUEUE_DEPTH: usize = 8;

pub const VIRTIO_BLK_TYPE_IN: u32 = 0;
pub const VIRTIO_BLK_TYPE_OUT: u32 = 1;
pub const VIRTIO_BLK_TYPE_FLUSH: u32 = 4;

pub const VIRTIO_BLK_STATUS_OK: u8 = 0;
pub const VIRTIO_BLK_STATUS_IOERR: u8 = 1;
pub const VIRTIO_BLK_STATUS_UNSUPP: u8 = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VirtioBlkHeader {
    pub request_type: u32,
    pub reserved: u32,
    pub sector: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VirtioDataDescriptor {
    pub buffer: BufferToken,
    pub length: u32,
    pub device_writable: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VirtioBlkChain {
    pub request_id: BlockRequestId,
    pub header: VirtioBlkHeader,
    pub data: Option<VirtioDataDescriptor>,
    pub blocks: u16,
}

impl VirtioBlkChain {
    pub const fn descriptor_count(self) -> u8 {
        if self.data.is_some() { 3 } else { 2 }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VirtioQueueError {
    Full,
    Duplicate,
    Stale,
    InvalidStatus,
    InvalidRequest,
}

pub fn encode_virtio_request(
    request_id: BlockRequestId,
    request: BlockRequest,
) -> Result<VirtioBlkChain, VirtioQueueError> {
    validate_request(request).map_err(|_| VirtioQueueError::InvalidRequest)?;
    let sector = request
        .start
        .get()
        .checked_mul(SECTORS_PER_LOGOS_BLOCK)
        .ok_or(VirtioQueueError::InvalidRequest)?;
    let (request_type, data) = match request.operation {
        BlockOperation::Read => (
            VIRTIO_BLK_TYPE_IN,
            Some(VirtioDataDescriptor {
                buffer: request.buffer,
                length: (request.blocks as u32) * BLOCK_BYTES as u32,
                device_writable: true,
            }),
        ),
        BlockOperation::Write => (
            VIRTIO_BLK_TYPE_OUT,
            Some(VirtioDataDescriptor {
                buffer: request.buffer,
                length: (request.blocks as u32) * BLOCK_BYTES as u32,
                device_writable: false,
            }),
        ),
        BlockOperation::Flush => (VIRTIO_BLK_TYPE_FLUSH, None),
    };

    Ok(VirtioBlkChain {
        request_id,
        header: VirtioBlkHeader { request_type, reserved: 0, sector },
        data,
        blocks: request.blocks,
    })
}

/// Host-tested queue ownership model for the future PCI/MMIO adapter.
pub struct VirtioBlkQueue<const DEPTH: usize = MAX_VIRTIO_QUEUE_DEPTH> {
    chains: [Option<VirtioBlkChain>; DEPTH],
}

impl<const DEPTH: usize> VirtioBlkQueue<DEPTH> {
    pub const fn new() -> Self {
        Self { chains: [None; DEPTH] }
    }

    pub const fn capacity(&self) -> usize {
        DEPTH
    }

    pub fn submit(
        &mut self,
        request_id: BlockRequestId,
        request: BlockRequest,
    ) -> Result<VirtioBlkChain, VirtioQueueError> {
        if self.chains.iter().any(|chain| chain.is_some_and(|chain| chain.request_id == request_id))
        {
            return Err(VirtioQueueError::Duplicate);
        }
        let chain = encode_virtio_request(request_id, request)?;
        let Some(slot) = self.chains.iter_mut().find(|slot| slot.is_none()) else {
            return Err(VirtioQueueError::Full);
        };
        *slot = Some(chain);
        Ok(chain)
    }

    pub fn complete(
        &mut self,
        request_id: BlockRequestId,
        device_status: u8,
    ) -> Result<(VirtioBlkChain, BlockCompletion), VirtioQueueError> {
        let Some(slot) = self
            .chains
            .iter_mut()
            .find(|slot| slot.is_some_and(|chain| chain.request_id == request_id))
        else {
            return Err(VirtioQueueError::Stale);
        };
        let chain = slot.as_ref().copied().expect("matching queue slot contains a chain");
        let (status, blocks_completed) = match device_status {
            VIRTIO_BLK_STATUS_OK => (BlockStatus::Success, chain.blocks),
            VIRTIO_BLK_STATUS_IOERR => (BlockStatus::Io, 0),
            VIRTIO_BLK_STATUS_UNSUPP => (BlockStatus::Unsupported, 0),
            _ => return Err(VirtioQueueError::InvalidStatus),
        };
        slot.take();
        Ok((chain, BlockCompletion { status, blocks_completed }))
    }
}

impl<const DEPTH: usize> Default for VirtioBlkQueue<DEPTH> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BlockIndex, BlockRequestTable};

    fn buffer() -> BufferToken {
        BufferToken::new(0x1000).unwrap()
    }

    fn request_id() -> BlockRequestId {
        let mut table = BlockRequestTable::<1>::new();
        table.submit(BlockRequest::flush()).unwrap()
    }

    #[test]
    fn read_and_write_chains_translate_logical_blocks() {
        let id = request_id();
        let read =
            encode_virtio_request(id, BlockRequest::read(BlockIndex::new(2), 2, buffer())).unwrap();
        assert_eq!(read.header.request_type, VIRTIO_BLK_TYPE_IN);
        assert_eq!(read.header.sector, 2 * SECTORS_PER_LOGOS_BLOCK);
        assert_eq!(read.descriptor_count(), 3);
        assert_eq!(read.data.unwrap().length, 2 * BLOCK_BYTES as u32);
        assert!(read.data.unwrap().device_writable);

        let write = encode_virtio_request(id, BlockRequest::write(BlockIndex::new(3), 1, buffer()))
            .unwrap();
        assert_eq!(write.header.request_type, VIRTIO_BLK_TYPE_OUT);
        assert!(!write.data.unwrap().device_writable);
    }

    #[test]
    fn flush_chain_has_header_and_status_only() {
        let chain = encode_virtio_request(request_id(), BlockRequest::flush()).unwrap();
        assert_eq!(chain.header.request_type, VIRTIO_BLK_TYPE_FLUSH);
        assert_eq!(chain.descriptor_count(), 2);
        assert!(chain.data.is_none());
    }

    #[test]
    fn queue_accepts_out_of_order_completions_and_maps_status() {
        let mut ids = BlockRequestTable::<2>::new();
        let first = ids.submit(BlockRequest::flush()).unwrap();
        let second = ids.submit(BlockRequest::flush()).unwrap();
        let mut queue = VirtioBlkQueue::<2>::new();
        queue.submit(first, BlockRequest::flush()).unwrap();
        queue.submit(second, BlockRequest::flush()).unwrap();

        let (completed, result) = queue.complete(second, VIRTIO_BLK_STATUS_UNSUPP).unwrap();
        assert_eq!(completed.request_id, second);
        assert_eq!(result.status, BlockStatus::Unsupported);
        assert_eq!(result.blocks_completed, 0);
        assert_eq!(
            queue.complete(first, VIRTIO_BLK_STATUS_OK).unwrap().1.status,
            BlockStatus::Success
        );
    }

    #[test]
    fn queue_rejects_duplicates_full_and_unknown_status() {
        let mut ids = BlockRequestTable::<2>::new();
        let first = ids.submit(BlockRequest::flush()).unwrap();
        let second = ids.submit(BlockRequest::flush()).unwrap();
        let mut queue = VirtioBlkQueue::<1>::new();
        queue.submit(first, BlockRequest::flush()).unwrap();
        assert_eq!(queue.submit(first, BlockRequest::flush()), Err(VirtioQueueError::Duplicate));
        assert_eq!(queue.submit(second, BlockRequest::flush()), Err(VirtioQueueError::Full));
        assert_eq!(queue.complete(first, 0xff), Err(VirtioQueueError::InvalidStatus));
        assert_eq!(
            queue.complete(first, VIRTIO_BLK_STATUS_OK).unwrap().1.status,
            BlockStatus::Success
        );
    }

    #[test]
    fn sector_overflow_is_rejected_before_queue_submission() {
        let id = request_id();
        assert_eq!(
            encode_virtio_request(id, BlockRequest::read(BlockIndex::new(u64::MAX), 1, buffer())),
            Err(VirtioQueueError::InvalidRequest)
        );
    }
}
