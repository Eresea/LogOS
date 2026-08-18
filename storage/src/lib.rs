#![no_std]

#[cfg(test)]
extern crate std;

mod journal;
mod pci;
mod request;
mod transport;
mod virtio;

pub use journal::{
    CHECKPOINT_PAYLOAD_BYTES, FORMAT_VERSION, FormatError, JOURNAL_COMMIT_KIND, JournalRecord,
    LEGACY_FORMAT_VERSION, MAX_RECORD_PAYLOAD_BYTES, MAX_RECORDS_PER_TRANSACTION,
    PROVISIONED_BLANK_MAGIC, RecoverySummary, ReplayError, ReplaySink, V2_FORMAT_VERSION, Volume,
    VolumeInfo,
};
pub use pci::{
    PCI_CONFIG_BYTES, PciAddress, PciError, VIRTIO_BLOCK_MODERN_DEVICE_ID, VIRTIO_PCI_VENDOR_ID,
    VirtioPciCapabilities, VirtioPciCapability, VirtioPciDevice,
};
pub use request::{
    BlockCompletion, BlockOperation, BlockRequest, BlockRequestError, BlockRequestId,
    BlockRequestTable, BlockStatus, BufferToken, MAX_BLOCK_REQUESTS, MAX_BLOCKS_PER_REQUEST,
};
pub use transport::{
    DEFAULT_REQUEST_TIMEOUT, DMA_PAGE_BYTES, DmaAddress, DmaArena, DmaError, DmaLease, Expired,
    FeatureError, NegotiatedFeatures, TransportError, TransportRequestId, VIRTIO_BLK_F_FLUSH,
    VIRTIO_F_VERSION_1, VirtioTransport, negotiate_features,
};
pub use virtio::{
    MAX_VIRTIO_QUEUE_DEPTH, SECTORS_PER_LOGOS_BLOCK, VIRTIO_BLK_STATUS_IOERR, VIRTIO_BLK_STATUS_OK,
    VIRTIO_BLK_STATUS_UNSUPP, VIRTIO_BLK_TYPE_FLUSH, VIRTIO_BLK_TYPE_IN, VIRTIO_BLK_TYPE_OUT,
    VIRTIO_SECTOR_BYTES, VirtioBlkChain, VirtioBlkHeader, VirtioBlkQueue, VirtioDataDescriptor,
    VirtioQueueError, encode_virtio_request,
};

/// The logical storage block size used by the first format boundary.
pub const BLOCK_BYTES: usize = 4096;

/// A logical block number. Device-specific sectors remain below this boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BlockIndex(u64);

impl BlockIndex {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

/// One fixed-size logical storage block.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Block {
    bytes: [u8; BLOCK_BYTES],
}

impl Block {
    pub const ZERO: Self = Self { bytes: [0; BLOCK_BYTES] };

    pub const fn zero() -> Self {
        Self::ZERO
    }

    pub const fn from_bytes(bytes: [u8; BLOCK_BYTES]) -> Self {
        Self { bytes }
    }

    pub const fn as_bytes(&self) -> &[u8; BLOCK_BYTES] {
        &self.bytes
    }

    pub fn as_bytes_mut(&mut self) -> &mut [u8; BLOCK_BYTES] {
        &mut self.bytes
    }
}

impl Default for Block {
    fn default() -> Self {
        Self::ZERO
    }
}

/// Errors exposed by the format-facing block seam.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BlockError {
    OutOfBounds,
    Io,
    ReadOnly,
    Unauthorized,
    Stale,
    InvalidRequest,
}

/// Minimal synchronous seam used by the host-tested storage format.
///
/// Hardware adapters can implement this seam without making the format depend
/// on the scheduler or on a particular transport. The future service ABI can
/// layer asynchronous request IDs and page grants above it.
pub trait BlockStore {
    fn block_count(&self) -> u64;
    fn read_block(&mut self, index: BlockIndex, output: &mut Block) -> Result<(), BlockError>;
    fn write_block(&mut self, index: BlockIndex, input: &Block) -> Result<(), BlockError>;
    fn flush(&mut self) -> Result<(), BlockError>;
}

/// Fixed-capacity memory backend for format tests and fault-injection wrappers.
pub struct MemoryBlockStore<const BLOCKS: usize> {
    blocks: [Block; BLOCKS],
    read_only: bool,
}

impl<const BLOCKS: usize> MemoryBlockStore<BLOCKS> {
    pub const fn new() -> Self {
        Self { blocks: [Block::ZERO; BLOCKS], read_only: false }
    }

    pub const fn set_read_only(&mut self, read_only: bool) {
        self.read_only = read_only;
    }

    pub fn block(&self, index: BlockIndex) -> Option<&Block> {
        self.blocks.get(index.get() as usize)
    }
}

impl<const BLOCKS: usize> Default for MemoryBlockStore<BLOCKS> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const BLOCKS: usize> BlockStore for MemoryBlockStore<BLOCKS> {
    fn block_count(&self) -> u64 {
        BLOCKS as u64
    }

    fn read_block(&mut self, index: BlockIndex, output: &mut Block) -> Result<(), BlockError> {
        let Some(block) = self.blocks.get(index.get() as usize) else {
            return Err(BlockError::OutOfBounds);
        };
        *output = *block;
        Ok(())
    }

    fn write_block(&mut self, index: BlockIndex, input: &Block) -> Result<(), BlockError> {
        if self.read_only {
            return Err(BlockError::ReadOnly);
        }
        let Some(block) = self.blocks.get_mut(index.get() as usize) else {
            return Err(BlockError::OutOfBounds);
        };
        *block = *input;
        Ok(())
    }

    fn flush(&mut self) -> Result<(), BlockError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_store_round_trips_fixed_blocks() {
        let mut store = MemoryBlockStore::<2>::new();
        let mut input = Block::zero();
        input.as_bytes_mut()[0] = 0x5a;
        input.as_bytes_mut()[BLOCK_BYTES - 1] = 0xa5;

        store.write_block(BlockIndex::new(1), &input).unwrap();

        let mut output = Block::zero();
        store.read_block(BlockIndex::new(1), &mut output).unwrap();
        assert_eq!(output, input);
        assert_eq!(store.block_count(), 2);
    }

    #[test]
    fn memory_store_rejects_out_of_bounds_access() {
        let mut store = MemoryBlockStore::<1>::new();
        let mut block = Block::zero();

        assert_eq!(store.read_block(BlockIndex::new(1), &mut block), Err(BlockError::OutOfBounds));
        assert_eq!(store.write_block(BlockIndex::new(1), &block), Err(BlockError::OutOfBounds));
    }

    #[test]
    fn memory_store_rejects_writes_when_read_only() {
        let mut store = MemoryBlockStore::<1>::new();
        let block = Block::zero();
        store.set_read_only(true);

        assert_eq!(store.write_block(BlockIndex::new(0), &block), Err(BlockError::ReadOnly));
    }
}
