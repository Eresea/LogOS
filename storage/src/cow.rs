use crate::{BLOCK_BYTES, Block, BlockError, BlockIndex, BlockStore};

pub const COW_FORMAT_VERSION: u16 = 4;
pub const COW_SUPERBLOCK_MAGIC: &[u8; 8] = b"LOGOSCOW";
pub const COW_PROVISIONED_BLANK_MAGIC: &[u8; 8] = b"LOGOSBLK";
pub const COW_MAX_BITMAP_BLOCKS: usize = 1;
pub const COW_MAX_RETIRED_EXTENTS: usize = 128;
pub const COW_MAX_TRANSACTION_EXTENTS: usize = 256;
pub const COW_COMMIT_SLOTS: usize = 2;

const SUPERBLOCK_A: BlockIndex = BlockIndex::new(0);
const SUPERBLOCK_B: BlockIndex = BlockIndex::new(1);
const COMMIT_START: u64 = 2;
const DATA_START: u64 = COMMIT_START + COW_COMMIT_SLOTS as u64;
const CHECKSUM_OFFSET: usize = 92;
const RETIRED_MAGIC: &[u8; 4] = b"LOSR";
const COMMIT_MAGIC: &[u8; 4] = b"LOSC";
const LEGACY_MAGIC: &[u8; 8] = b"LOGOSFS\0";
const MAX_BLOCK_COUNT: u64 = (COW_MAX_BITMAP_BLOCKS * BLOCK_BYTES * 8) as u64;
const WRITE_VERIFY_ATTEMPTS: usize = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CowError {
    Block(BlockError),
    NotBlank,
    Unformatted,
    ProvisionedBlank,
    UnsupportedVersion,
    Corrupt,
    TooSmall,
    TooLarge,
    OutOfSpace,
    InvalidRequest,
    GenerationExhausted,
    RetiredExtentCapacity,
}

impl From<BlockError> for CowError {
    fn from(error: BlockError) -> Self {
        Self::Block(error)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CowExtent {
    pub start: BlockIndex,
    pub blocks: u32,
}

impl CowExtent {
    pub const EMPTY: Self = Self { start: BlockIndex::new(0), blocks: 0 };

    pub const fn new(start: BlockIndex, blocks: u32) -> Option<Self> {
        if blocks == 0 { None } else { Some(Self { start, blocks }) }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CowRoot {
    pub generation: u64,
    pub metadata_root: BlockIndex,
    pub metadata_blocks: u32,
    pub bitmap_slot: u8,
    pub bitmap_start: BlockIndex,
    pub bitmap_blocks: u16,
    pub data_start: BlockIndex,
    pub data_end: BlockIndex,
    pub retired_root: BlockIndex,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CommitRecord {
    generation: u64,
    metadata_root: BlockIndex,
    metadata_blocks: u32,
    bitmap_slot: u8,
    retired_root: BlockIndex,
    checksum: u32,
}

pub struct CowVolume {
    root: CowRoot,
    active_superblock: u8,
}

pub struct CowTransaction {
    root: CowRoot,
    bitmap_slot: u8,
    retired: [CowExtent; COW_MAX_RETIRED_EXTENTS],
    retired_count: usize,
    allocated: [CowExtent; COW_MAX_TRANSACTION_EXTENTS],
    allocated_count: usize,
    released: [CowExtent; COW_MAX_RETIRED_EXTENTS],
    released_count: usize,
}

impl CowVolume {
    pub fn format<B: BlockStore>(store: &mut B) -> Result<Self, CowError> {
        Self::format_inner(store, false)
    }

    pub fn format_provisioned<B: BlockStore>(store: &mut B) -> Result<Self, CowError> {
        Self::format_inner(store, true)
    }

    fn format_inner<B: BlockStore>(store: &mut B, provisioned: bool) -> Result<Self, CowError> {
        validate_capacity(store.block_count())?;
        let mut first = Block::zero();
        store.read_block(SUPERBLOCK_A, &mut first)?;
        if provisioned {
            if &first.as_bytes()[..COW_PROVISIONED_BLANK_MAGIC.len()] != COW_PROVISIONED_BLANK_MAGIC
                || first.as_bytes()[COW_PROVISIONED_BLANK_MAGIC.len()..]
                    .iter()
                    .any(|byte| *byte != 0)
            {
                return Err(CowError::NotBlank);
            }
        } else {
            let mut block = Block::zero();
            for index in 0..store.block_count() {
                store.read_block(BlockIndex::new(index), &mut block)?;
                if block.as_bytes().iter().any(|byte| *byte != 0) {
                    return Err(CowError::NotBlank);
                }
            }
        }

        let (bitmap_blocks, data_end) = layout(store.block_count())?;
        let root = CowRoot {
            generation: 1,
            metadata_root: BlockIndex::new(DATA_START),
            metadata_blocks: 1,
            bitmap_slot: 0,
            bitmap_start: BlockIndex::new(data_end),
            bitmap_blocks: bitmap_blocks as u16,
            data_start: BlockIndex::new(DATA_START),
            data_end: BlockIndex::new(data_end),
            retired_root: BlockIndex::new(0),
        };
        let mut bitmap = [Block::zero(); COW_MAX_BITMAP_BLOCKS];
        initialize_bitmap(&mut bitmap, root, store.block_count());
        let empty = Block::zero();
        store.write_block(root.metadata_root, &empty)?;
        write_bitmap(store, root, &bitmap)?;
        store.flush()?;
        verify_block(store, root.metadata_root, &empty)?;
        verify_bitmap(store, root, &bitmap)?;
        write_superblock(store, SUPERBLOCK_A, root)?;
        store.flush()?;
        let mut encoded_root = Block::zero();
        encode_root(&mut encoded_root, root);
        verify_block(store, SUPERBLOCK_A, &encoded_root)?;
        write_superblock(store, SUPERBLOCK_B, root)?;
        store.flush()?;
        verify_block(store, SUPERBLOCK_B, &encoded_root)?;
        Ok(Self { root, active_superblock: 1 })
    }

    pub fn open<B: BlockStore>(store: &mut B) -> Result<Self, CowError> {
        validate_capacity(store.block_count())?;
        let first = read_superblock(store, SUPERBLOCK_A);
        let second = read_superblock(store, SUPERBLOCK_B);
        let (mut root, mut active_superblock) = match (first, second) {
            (Ok(Some(a)), Ok(Some(b))) if b.generation > a.generation => (b, 1),
            (Ok(Some(a)), _) => (a, 0),
            (_, Ok(Some(b))) => (b, 1),
            (Ok(None), Ok(None)) => return Err(CowError::Unformatted),
            (Err(CowError::ProvisionedBlank), Ok(None))
            | (Ok(None), Err(CowError::ProvisionedBlank))
            | (Err(CowError::ProvisionedBlank), Err(CowError::ProvisionedBlank)) => {
                return Err(CowError::ProvisionedBlank);
            }
            (Err(error), Ok(None)) | (Ok(None), Err(error)) => return Err(error),
            (Err(error), Err(_)) => return Err(error),
        };

        let mut candidate = None;
        for slot in 0..COW_COMMIT_SLOTS {
            if let Some(commit) = read_commit(store, slot)? {
                if commit.generation > root.generation {
                    candidate = Some(commit);
                }
            }
        }
        if let Some(commit) = candidate {
            root = CowRoot {
                generation: commit.generation,
                metadata_root: commit.metadata_root,
                metadata_blocks: commit.metadata_blocks,
                bitmap_slot: commit.bitmap_slot,
                retired_root: commit.retired_root,
                ..root
            };
            let target = if active_superblock == 0 { SUPERBLOCK_B } else { SUPERBLOCK_A };
            write_superblock(store, target, root)?;
            store.flush()?;
            let mut encoded_root = Block::zero();
            encode_root(&mut encoded_root, root);
            verify_block(store, target, &encoded_root)?;
            active_superblock ^= 1;
        }
        validate_root(root, store.block_count())?;
        Ok(Self { root, active_superblock })
    }

    pub const fn root(&self) -> CowRoot {
        self.root
    }

    pub const fn data_arena(&self) -> (u64, u64) {
        (self.root.data_start.get(), self.root.data_end.get())
    }

    pub const fn package_arena(&self) -> Option<(u64, u64)> {
        let start = self.root.data_start.get() + 16;
        if start < self.root.data_end.get() {
            Some((start, self.root.data_end.get()))
        } else {
            None
        }
    }

    pub fn read_metadata_root<B: BlockStore>(
        &self,
        store: &mut B,
        output: &mut Block,
    ) -> Result<(), CowError> {
        store.read_block(self.root.metadata_root, output).map_err(Into::into)
    }

    pub fn read_metadata<B: BlockStore>(
        &self,
        store: &mut B,
        output: &mut [Block],
    ) -> Result<(), CowError> {
        if output.len() != self.root.metadata_blocks as usize {
            return Err(CowError::InvalidRequest);
        }
        for (offset, block) in output.iter_mut().enumerate() {
            store.read_block(
                BlockIndex::new(self.root.metadata_root.get() + offset as u64),
                block,
            )?;
        }
        Ok(())
    }

    pub fn read_metadata_block<B: BlockStore>(
        &self,
        store: &mut B,
        offset: u32,
        output: &mut Block,
    ) -> Result<(), CowError> {
        if offset >= self.root.metadata_blocks {
            return Err(CowError::InvalidRequest);
        }
        store
            .read_block(BlockIndex::new(self.root.metadata_root.get() + offset as u64), output)
            .map_err(Into::into)
    }

    pub fn begin<B: BlockStore>(&self, store: &mut B) -> Result<CowTransaction, CowError> {
        let mut transaction = CowTransaction {
            root: self.root,
            bitmap_slot: 1u8.saturating_sub(self.root.bitmap_slot),
            retired: [CowExtent::EMPTY; COW_MAX_RETIRED_EXTENTS],
            retired_count: 0,
            allocated: [CowExtent::EMPTY; COW_MAX_TRANSACTION_EXTENTS],
            allocated_count: 0,
            released: [CowExtent::EMPTY; COW_MAX_RETIRED_EXTENTS],
            released_count: 0,
        };
        transaction.reclaim_retired(store)?;
        Ok(transaction)
    }

    pub fn commit<B: BlockStore>(
        &mut self,
        store: &mut B,
        mut transaction: CowTransaction,
        metadata: CowExtent,
    ) -> Result<u64, CowError> {
        if self.root.retired_root.get() != 0 {
            transaction.retire_extent(
                CowExtent::new(self.root.retired_root, 1).ok_or(CowError::InvalidRequest)?,
            )?;
        }
        let retired_root = transaction.write_retired(store)?;
        let mut bitmap = [Block::zero(); COW_MAX_BITMAP_BLOCKS];
        read_bitmap(store, self.root, &mut bitmap)?;
        for extent in transaction.released[..transaction.released_count].iter() {
            for index in extent.start.get()..extent.start.get() + extent.blocks as u64 {
                bitmap_set(&mut bitmap, index, false);
            }
        }
        for extent in transaction.allocated[..transaction.allocated_count].iter() {
            for index in extent.start.get()..extent.start.get() + extent.blocks as u64 {
                bitmap_set(&mut bitmap, index, true);
            }
        }
        if transaction.root != self.root
            || metadata.blocks == 0
            || metadata.start.get() < self.root.data_start.get()
            || metadata.start.get().checked_add(metadata.blocks as u64).is_none()
            || metadata.start.get() + metadata.blocks as u64 > self.root.data_end.get()
            || (metadata.start.get()..metadata.start.get() + metadata.blocks as u64)
                .any(|index| !bitmap_is_set(&bitmap, index))
        {
            return Err(CowError::InvalidRequest);
        }
        let generation =
            self.root.generation.checked_add(1).ok_or(CowError::GenerationExhausted)?;
        let root = CowRoot {
            generation,
            metadata_root: metadata.start,
            metadata_blocks: metadata.blocks,
            bitmap_slot: transaction.bitmap_slot,
            retired_root,
            ..self.root
        };
        write_bitmap(store, root, &bitmap)?;
        store.flush()?;
        verify_extent(store, metadata)?;
        verify_bitmap(store, root, &bitmap)?;
        let commit_slot = (generation as usize) % COW_COMMIT_SLOTS;
        let commit = CommitRecord {
            generation,
            metadata_root: metadata.start,
            metadata_blocks: metadata.blocks,
            bitmap_slot: root.bitmap_slot,
            retired_root,
            checksum: 0,
        };
        write_commit(store, commit_slot, commit)?;
        store.flush()?;
        verify_block(
            store,
            BlockIndex::new(COMMIT_START + commit_slot as u64),
            &encode_commit(commit),
        )?;
        let target = if self.active_superblock == 0 { SUPERBLOCK_B } else { SUPERBLOCK_A };
        write_superblock(store, target, root)?;
        store.flush()?;
        let mut encoded_root = Block::zero();
        encode_root(&mut encoded_root, root);
        verify_block(store, target, &encoded_root)?;
        self.root = root;
        self.active_superblock ^= 1;
        Ok(generation)
    }
}

fn verify_extent<B: BlockStore>(store: &mut B, extent: CowExtent) -> Result<(), CowError> {
    for offset in 0..extent.blocks as u64 {
        verify_cached_block(store, BlockIndex::new(extent.start.get() + offset))?;
    }
    Ok(())
}

fn verify_bitmap<B: BlockStore>(
    store: &mut B,
    root: CowRoot,
    expected: &[Block; COW_MAX_BITMAP_BLOCKS],
) -> Result<(), CowError> {
    let start = root.bitmap_start.get() + root.bitmap_blocks as u64 * root.bitmap_slot as u64;
    for offset in 0..root.bitmap_blocks as u64 {
        verify_block(store, BlockIndex::new(start + offset), &expected[offset as usize])?;
    }
    Ok(())
}

fn verify_cached_block<B: BlockStore>(store: &mut B, index: BlockIndex) -> Result<(), CowError> {
    let mut expected = Block::zero();
    store.read_block(index, &mut expected)?;
    verify_block(store, index, &expected)
}

fn verify_block<B: BlockStore>(
    store: &mut B,
    index: BlockIndex,
    expected: &Block,
) -> Result<(), CowError> {
    for attempt in 0..WRITE_VERIFY_ATTEMPTS {
        let mut actual = Block::zero();
        store.read_block_uncached(index, &mut actual)?;
        if actual == *expected {
            return Ok(());
        }
        if attempt + 1 < WRITE_VERIFY_ATTEMPTS {
            store.write_block(index, expected)?;
            store.flush()?;
        }
    }
    Err(CowError::Block(BlockError::Io))
}

impl CowTransaction {
    pub fn allocate_blocks<B: BlockStore>(
        &mut self,
        store: &mut B,
        blocks: u32,
    ) -> Result<CowExtent, CowError> {
        self.allocate_blocks_in_arena(
            store,
            blocks,
            (self.root.data_start.get(), self.root.data_end.get()),
        )
    }

    /// Allocate only from the caller-provided half-open block range.
    ///
    /// The range must be inside the current data arena. The v4 convenience
    /// method above still scans the complete data arena; format-specific
    /// callers use this method to keep system, user, and package pools apart.
    pub fn allocate_blocks_in_arena<B: BlockStore>(
        &mut self,
        store: &mut B,
        blocks: u32,
        arena: (u64, u64),
    ) -> Result<CowExtent, CowError> {
        if blocks == 0 {
            return Err(CowError::InvalidRequest);
        }
        let (arena_start, arena_end) = arena;
        if arena_start < self.root.data_start.get()
            || arena_start >= arena_end
            || arena_end > self.root.data_end.get()
        {
            return Err(CowError::InvalidRequest);
        }
        let mut bitmap = [Block::zero(); COW_MAX_BITMAP_BLOCKS];
        read_bitmap(store, self.root, &mut bitmap)?;
        let mut run_start = None;
        let mut run_length = 0u32;
        for index in arena_start..arena_end {
            let released = self.released[..self.released_count].iter().any(|extent| {
                index >= extent.start.get() && index < extent.start.get() + extent.blocks as u64
            });
            if released {
                run_start = None;
                run_length = 0;
                continue;
            }
            if !bitmap_is_set(&bitmap, index) {
                if self.allocated[..self.allocated_count].iter().any(|extent| {
                    index >= extent.start.get() && index < extent.start.get() + extent.blocks as u64
                }) {
                    run_start = None;
                    run_length = 0;
                    continue;
                }
                run_start.get_or_insert(index);
                run_length += 1;
                if run_length == blocks {
                    let extent = CowExtent::new(BlockIndex::new(run_start.unwrap()), blocks)
                        .ok_or(CowError::InvalidRequest)?;
                    if self.allocated_count == COW_MAX_TRANSACTION_EXTENTS {
                        return Err(CowError::RetiredExtentCapacity);
                    }
                    self.allocated[self.allocated_count] = extent;
                    self.allocated_count += 1;
                    return Ok(extent);
                }
            } else {
                run_start = None;
                run_length = 0;
            }
        }
        Err(CowError::OutOfSpace)
    }

    pub fn write_block<B: BlockStore>(
        &self,
        store: &mut B,
        index: BlockIndex,
        block: &Block,
    ) -> Result<(), CowError> {
        if index.get() < self.root.data_start.get() || index.get() >= self.root.data_end.get() {
            return Err(CowError::InvalidRequest);
        }
        if !self.allocated[..self.allocated_count].iter().any(|extent| {
            index.get() >= extent.start.get()
                && index.get() < extent.start.get() + extent.blocks as u64
        }) {
            return Err(CowError::InvalidRequest);
        }
        store.write_block(index, block).map_err(Into::into)
    }

    pub fn reserve_extent(&mut self, extent: CowExtent) -> Result<(), CowError> {
        self.reserve_extent_in_arena(extent, (self.root.data_start.get(), self.root.data_end.get()))
    }

    pub fn reserve_extent_in_arena(
        &mut self,
        extent: CowExtent,
        arena: (u64, u64),
    ) -> Result<(), CowError> {
        let (arena_start, arena_end) = arena;
        if extent.start.get() < self.root.data_start.get()
            || extent.start.get().checked_add(extent.blocks as u64).is_none()
            || extent.start.get() + extent.blocks as u64 > self.root.data_end.get()
            || arena_start < self.root.data_start.get()
            || arena_start >= arena_end
            || arena_end > self.root.data_end.get()
            || extent.start.get() < arena_start
            || extent.start.get() + extent.blocks as u64 > arena_end
            || self.allocated_count == COW_MAX_TRANSACTION_EXTENTS
        {
            return Err(CowError::InvalidRequest);
        }
        self.allocated[self.allocated_count] = extent;
        self.allocated_count += 1;
        Ok(())
    }

    pub fn write_extent<B: BlockStore>(
        &self,
        store: &mut B,
        extent: CowExtent,
        blocks: &[Block],
    ) -> Result<(), CowError> {
        if blocks.len() != extent.blocks as usize {
            return Err(CowError::InvalidRequest);
        }
        for (offset, block) in blocks.iter().enumerate() {
            self.write_block(store, BlockIndex::new(extent.start.get() + offset as u64), block)?;
        }
        Ok(())
    }

    pub fn retire_extent(&mut self, extent: CowExtent) -> Result<(), CowError> {
        if extent.start.get() < self.root.data_start.get()
            || extent
                .start
                .get()
                .checked_add(extent.blocks as u64)
                .is_none_or(|end| end > self.root.data_end.get())
        {
            return Err(CowError::InvalidRequest);
        }
        if self.retired_count == COW_MAX_RETIRED_EXTENTS {
            return Err(CowError::RetiredExtentCapacity);
        }
        self.retired[self.retired_count] = extent;
        self.retired_count += 1;
        Ok(())
    }

    fn release_extent(&mut self, extent: CowExtent) -> Result<(), CowError> {
        if self.released_count == COW_MAX_RETIRED_EXTENTS {
            return Err(CowError::RetiredExtentCapacity);
        }
        self.released[self.released_count] = extent;
        self.released_count += 1;
        Ok(())
    }

    fn reclaim_retired<B: BlockStore>(&mut self, store: &mut B) -> Result<(), CowError> {
        if self.root.retired_root.get() == 0 {
            return Ok(());
        }
        let mut block = Block::zero();
        store.read_block(self.root.retired_root, &mut block)?;
        if &block.as_bytes()[..4] != RETIRED_MAGIC {
            return Err(CowError::Corrupt);
        }
        let count = u16::from_le_bytes([block.as_bytes()[4], block.as_bytes()[5]]) as usize;
        if count > COW_MAX_RETIRED_EXTENTS {
            return Err(CowError::Corrupt);
        }
        for index in 0..count {
            let offset = 8 + index * 12;
            let start = get_u64(block.as_bytes(), offset);
            let blocks = get_u32(block.as_bytes(), offset + 8);
            let extent = CowExtent::new(BlockIndex::new(start), blocks).ok_or(CowError::Corrupt)?;
            let end = start.checked_add(blocks as u64).ok_or(CowError::Corrupt)?;
            if start < self.root.data_start.get() || end > self.root.data_end.get() {
                return Err(CowError::Corrupt);
            }
            self.release_extent(extent)?;
        }
        Ok(())
    }

    fn write_retired<B: BlockStore>(&mut self, store: &mut B) -> Result<BlockIndex, CowError> {
        if self.retired_count == 0 {
            return Ok(BlockIndex::new(0));
        }
        let extent = self.allocate_blocks(store, 1)?;
        let mut block = Block::zero();
        block.as_bytes_mut()[..4].copy_from_slice(RETIRED_MAGIC);
        block.as_bytes_mut()[4..6].copy_from_slice(&(self.retired_count as u16).to_le_bytes());
        for index in 0..self.retired_count {
            let offset = 8 + index * 12;
            put_u64(block.as_bytes_mut(), offset, self.retired[index].start.get());
            put_u32(block.as_bytes_mut(), offset + 8, self.retired[index].blocks);
        }
        store.write_block(extent.start, &block)?;
        Ok(extent.start)
    }
}

fn validate_capacity(blocks: u64) -> Result<(), CowError> {
    if blocks <= DATA_START + 2 || blocks > MAX_BLOCK_COUNT {
        Err(if blocks <= DATA_START + 2 { CowError::TooSmall } else { CowError::TooLarge })
    } else {
        Ok(())
    }
}

fn layout(blocks: u64) -> Result<(usize, u64), CowError> {
    let bitmap_blocks = blocks.div_ceil((BLOCK_BYTES * 8) as u64) as usize;
    if bitmap_blocks == 0 || bitmap_blocks > COW_MAX_BITMAP_BLOCKS {
        return Err(CowError::TooLarge);
    }
    let reserved = (bitmap_blocks * 2) as u64;
    let data_end = blocks.checked_sub(reserved).ok_or(CowError::TooSmall)?;
    if data_end <= DATA_START + 1 {
        return Err(CowError::TooSmall);
    }
    Ok((bitmap_blocks, data_end))
}

fn initialize_bitmap(bitmap: &mut [Block; COW_MAX_BITMAP_BLOCKS], root: CowRoot, total: u64) {
    for index in 0..DATA_START {
        bitmap_set(bitmap, index, true);
    }
    for index in root.data_end.get()..total {
        bitmap_set(bitmap, index, true);
    }
    bitmap_set(bitmap, root.metadata_root.get(), true);
}

fn read_bitmap<B: BlockStore>(
    store: &mut B,
    root: CowRoot,
    output: &mut [Block; COW_MAX_BITMAP_BLOCKS],
) -> Result<(), CowError> {
    let slot_start = root.bitmap_start.get() + root.bitmap_blocks as u64 * root.bitmap_slot as u64;
    for index in 0..root.bitmap_blocks as u64 {
        store.read_block(BlockIndex::new(slot_start + index), &mut output[index as usize])?;
    }
    Ok(())
}

fn write_bitmap<B: BlockStore>(
    store: &mut B,
    root: CowRoot,
    bitmap: &[Block; COW_MAX_BITMAP_BLOCKS],
) -> Result<(), CowError> {
    let slot_start = root.data_end.get() + root.bitmap_blocks as u64 * root.bitmap_slot as u64;
    for index in 0..root.bitmap_blocks as u64 {
        store.write_block(BlockIndex::new(slot_start + index), &bitmap[index as usize])?;
    }
    Ok(())
}

fn bitmap_position(index: u64) -> (usize, usize, u8) {
    (
        (index / (BLOCK_BYTES as u64 * 8)) as usize,
        ((index / 8) % BLOCK_BYTES as u64) as usize,
        (index % 8) as u8,
    )
}

fn bitmap_is_set(bitmap: &[Block; COW_MAX_BITMAP_BLOCKS], index: u64) -> bool {
    let (block, byte, bit) = bitmap_position(index);
    bitmap[block].as_bytes()[byte] & (1u8 << bit) != 0
}

fn bitmap_set(bitmap: &mut [Block; COW_MAX_BITMAP_BLOCKS], index: u64, value: bool) {
    let (block, byte, bit) = bitmap_position(index);
    let mask = 1u8 << bit;
    if value {
        bitmap[block].as_bytes_mut()[byte] |= mask;
    } else {
        bitmap[block].as_bytes_mut()[byte] &= !mask;
    }
}

fn write_superblock<B: BlockStore>(
    store: &mut B,
    index: BlockIndex,
    root: CowRoot,
) -> Result<(), CowError> {
    let mut block = Block::zero();
    encode_root(&mut block, root);
    store.write_block(index, &block)?;
    Ok(())
}

fn read_superblock<B: BlockStore>(
    store: &mut B,
    index: BlockIndex,
) -> Result<Option<CowRoot>, CowError> {
    let mut block = Block::zero();
    store.read_block(index, &mut block)?;
    decode_root(&block)
}

fn encode_root(block: &mut Block, root: CowRoot) {
    let bytes = block.as_bytes_mut();
    bytes[..8].copy_from_slice(COW_SUPERBLOCK_MAGIC);
    bytes[8..10].copy_from_slice(&COW_FORMAT_VERSION.to_le_bytes());
    put_u64(bytes, 16, root.generation);
    put_u64(bytes, 24, root.metadata_root.get());
    bytes[32] = root.bitmap_slot;
    put_u64(bytes, 40, root.bitmap_start.get());
    put_u16(bytes, 48, root.bitmap_blocks);
    put_u64(bytes, 56, root.data_start.get());
    put_u64(bytes, 64, root.data_end.get());
    put_u64(bytes, 72, root.retired_root.get());
    put_u32(bytes, 80, root.metadata_blocks);
    put_u32(bytes, CHECKSUM_OFFSET, 0);
    let checksum = crc32c(&bytes[..CHECKSUM_OFFSET]);
    put_u32(bytes, CHECKSUM_OFFSET, checksum);
}

fn decode_root(block: &Block) -> Result<Option<CowRoot>, CowError> {
    let bytes = block.as_bytes();
    if bytes.iter().all(|byte| *byte == 0) {
        return Ok(None);
    }
    if &bytes[..8] != COW_SUPERBLOCK_MAGIC {
        return Err(
            if &bytes[..8] == COW_PROVISIONED_BLANK_MAGIC
                && bytes[8..].iter().all(|byte| *byte == 0)
            {
                CowError::ProvisionedBlank
            } else if &bytes[..8] == LEGACY_MAGIC {
                CowError::UnsupportedVersion
            } else {
                CowError::Corrupt
            },
        );
    }
    if u16::from_le_bytes([bytes[8], bytes[9]]) != COW_FORMAT_VERSION {
        return Err(CowError::UnsupportedVersion);
    }
    let expected = get_u32(bytes, CHECKSUM_OFFSET);
    let mut copy = *block;
    put_u32(copy.as_bytes_mut(), CHECKSUM_OFFSET, 0);
    if crc32c(&copy.as_bytes()[..CHECKSUM_OFFSET]) != expected {
        return Err(CowError::Corrupt);
    }
    Ok(Some(CowRoot {
        generation: get_u64(bytes, 16),
        metadata_root: BlockIndex::new(get_u64(bytes, 24)),
        metadata_blocks: get_u32(bytes, 80),
        bitmap_slot: bytes[32],
        bitmap_start: BlockIndex::new(get_u64(bytes, 40)),
        bitmap_blocks: get_u16(bytes, 48),
        data_start: BlockIndex::new(get_u64(bytes, 56)),
        data_end: BlockIndex::new(get_u64(bytes, 64)),
        retired_root: BlockIndex::new(get_u64(bytes, 72)),
    }))
}

fn validate_root(root: CowRoot, blocks: u64) -> Result<(), CowError> {
    if root.generation == 0
        || root.bitmap_slot >= 2
        || root.bitmap_blocks == 0
        || root.bitmap_blocks as usize > COW_MAX_BITMAP_BLOCKS
        || root.data_start.get() != DATA_START
        || root.data_end.get() >= blocks
        || root.bitmap_start.get() != root.data_end.get()
        || root.metadata_root.get() < root.data_start.get()
        || root.metadata_root.get() >= root.data_end.get()
        || root.metadata_blocks == 0
        || root.metadata_root.get().checked_add(root.metadata_blocks as u64).is_none()
        || root.metadata_root.get() + root.metadata_blocks as u64 > root.data_end.get()
    {
        return Err(CowError::Corrupt);
    }
    Ok(())
}

fn write_commit<B: BlockStore>(
    store: &mut B,
    slot: usize,
    record: CommitRecord,
) -> Result<(), CowError> {
    let block = encode_commit(record);
    store.write_block(BlockIndex::new(COMMIT_START + slot as u64), &block)?;
    Ok(())
}

fn encode_commit(mut record: CommitRecord) -> Block {
    let mut block = Block::zero();
    block.as_bytes_mut()[..4].copy_from_slice(COMMIT_MAGIC);
    put_u64(block.as_bytes_mut(), 8, record.generation);
    put_u64(block.as_bytes_mut(), 16, record.metadata_root.get());
    put_u32(block.as_bytes_mut(), 24, record.metadata_blocks);
    block.as_bytes_mut()[28] = record.bitmap_slot;
    put_u64(block.as_bytes_mut(), 32, record.retired_root.get());
    put_u32(block.as_bytes_mut(), 40, 0);
    record.checksum = crc32c(&block.as_bytes()[..40]);
    put_u32(block.as_bytes_mut(), 40, record.checksum);
    block
}

fn read_commit<B: BlockStore>(
    store: &mut B,
    slot: usize,
) -> Result<Option<CommitRecord>, CowError> {
    let mut block = Block::zero();
    store.read_block(BlockIndex::new(COMMIT_START + slot as u64), &mut block)?;
    if block.as_bytes().iter().all(|byte| *byte == 0) {
        return Ok(None);
    }
    if &block.as_bytes()[..4] != COMMIT_MAGIC {
        return Ok(None);
    }
    let expected = get_u32(block.as_bytes(), 40);
    if crc32c(&block.as_bytes()[..40]) != expected {
        return Ok(None);
    }
    Ok(Some(CommitRecord {
        generation: get_u64(block.as_bytes(), 8),
        metadata_root: BlockIndex::new(get_u64(block.as_bytes(), 16)),
        metadata_blocks: get_u32(block.as_bytes(), 24),
        bitmap_slot: block.as_bytes()[28],
        retired_root: BlockIndex::new(get_u64(block.as_bytes(), 32)),
        checksum: expected,
    }))
}

fn put_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_u64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

fn get_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn get_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([bytes[offset], bytes[offset + 1], bytes[offset + 2], bytes[offset + 3]])
}

fn get_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
        bytes[offset + 4],
        bytes[offset + 5],
        bytes[offset + 6],
        bytes[offset + 7],
    ])
}

fn crc32c(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = 0u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0x82f6_3b78 & mask);
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MemoryBlockStore;
    use std::boxed::Box;

    #[derive(Clone, Copy)]
    enum FailurePoint {
        Write(usize),
        Flush(usize),
    }

    struct CrashStore<const BLOCKS: usize> {
        durable: Box<[Block; BLOCKS]>,
        blocks: Box<[Block; BLOCKS]>,
        failure: Option<FailurePoint>,
        writes: usize,
        flushes: usize,
    }

    impl<const BLOCKS: usize> CrashStore<BLOCKS> {
        fn new() -> Self {
            Self {
                durable: Box::new([Block::ZERO; BLOCKS]),
                blocks: Box::new([Block::ZERO; BLOCKS]),
                failure: None,
                writes: 0,
                flushes: 0,
            }
        }

        fn arm(&mut self, failure: FailurePoint) {
            self.failure = Some(failure);
            self.writes = 0;
            self.flushes = 0;
        }

        fn fail_write(&mut self) -> bool {
            let fail =
                matches!(self.failure, Some(FailurePoint::Write(index)) if index == self.writes);
            self.writes += 1;
            if fail {
                self.failure = None;
            }
            fail
        }

        fn fail_flush(&mut self) -> bool {
            let fail =
                matches!(self.failure, Some(FailurePoint::Flush(index)) if index == self.flushes);
            self.flushes += 1;
            if fail {
                self.failure = None;
            }
            fail
        }

        fn power_loss(&mut self) {
            self.blocks.copy_from_slice(&self.durable[..]);
        }
    }

    impl<const BLOCKS: usize> BlockStore for CrashStore<BLOCKS> {
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
            if self.fail_write() {
                return Err(BlockError::Io);
            }
            let Some(block) = self.blocks.get_mut(index.get() as usize) else {
                return Err(BlockError::OutOfBounds);
            };
            *block = *input;
            Ok(())
        }

        fn flush(&mut self) -> Result<(), BlockError> {
            if self.fail_flush() {
                return Err(BlockError::Io);
            }
            self.durable.copy_from_slice(&self.blocks[..]);
            Ok(())
        }
    }

    struct StaleReadbackStore<const BLOCKS: usize> {
        inner: MemoryBlockStore<BLOCKS>,
        stale: Option<BlockIndex>,
        stale_reads: usize,
    }

    impl<const BLOCKS: usize> BlockStore for StaleReadbackStore<BLOCKS> {
        fn block_count(&self) -> u64 {
            self.inner.block_count()
        }

        fn read_block(&mut self, index: BlockIndex, output: &mut Block) -> Result<(), BlockError> {
            self.inner.read_block(index, output)
        }

        fn read_block_uncached(
            &mut self,
            index: BlockIndex,
            output: &mut Block,
        ) -> Result<(), BlockError> {
            if self.stale == Some(index) && self.stale_reads != 0 {
                self.stale_reads -= 1;
                *output = Block::zero();
                Ok(())
            } else {
                self.inner.read_block(index, output)
            }
        }

        fn write_block(&mut self, index: BlockIndex, input: &Block) -> Result<(), BlockError> {
            self.inner.write_block(index, input)
        }

        fn flush(&mut self) -> Result<(), BlockError> {
            self.inner.flush()
        }
    }

    #[test]
    fn cow_root_publishes_after_data_and_recovers_commit_record() {
        let mut store = MemoryBlockStore::<128>::new();
        let mut volume = CowVolume::format(&mut store).unwrap();
        let mut transaction = volume.begin(&mut store).unwrap();
        let metadata = transaction.allocate_blocks(&mut store, 1).unwrap();
        let mut block = Block::zero();
        block.as_bytes_mut()[0] = 0x5a;
        transaction.write_block(&mut store, metadata.start, &block).unwrap();
        transaction.retire_extent(CowExtent::new(volume.root().metadata_root, 1).unwrap()).unwrap();
        let generation = volume.commit(&mut store, transaction, metadata).unwrap();
        assert_eq!(generation, 2);

        let reopened = CowVolume::open(&mut store).unwrap();
        assert_eq!(reopened.root().generation, 2);
        let mut output = Block::zero();
        reopened.read_metadata_root(&mut store, &mut output).unwrap();
        assert_eq!(output.as_bytes()[0], 0x5a);
    }

    #[test]
    fn commit_rejects_a_stale_metadata_readback_before_publication() {
        let mut store = StaleReadbackStore {
            inner: MemoryBlockStore::<128>::new(),
            stale: None,
            stale_reads: 0,
        };
        let mut volume = CowVolume::format(&mut store).unwrap();
        let mut transaction = volume.begin(&mut store).unwrap();
        let metadata = transaction.allocate_blocks(&mut store, 1).unwrap();
        let mut block = Block::zero();
        block.as_bytes_mut()[0] = 0x5a;
        transaction.write_block(&mut store, metadata.start, &block).unwrap();
        transaction.retire_extent(CowExtent::new(volume.root().metadata_root, 1).unwrap()).unwrap();
        store.stale = Some(metadata.start);
        store.stale_reads = WRITE_VERIFY_ATTEMPTS;

        assert_eq!(
            volume.commit(&mut store, transaction, metadata),
            Err(CowError::Block(BlockError::Io))
        );
        assert_eq!(volume.root().generation, 1);
    }

    #[test]
    fn commit_retries_a_transient_stale_metadata_readback() {
        let mut store = StaleReadbackStore {
            inner: MemoryBlockStore::<128>::new(),
            stale: None,
            stale_reads: 0,
        };
        let mut volume = CowVolume::format(&mut store).unwrap();
        let mut transaction = volume.begin(&mut store).unwrap();
        let metadata = transaction.allocate_blocks(&mut store, 1).unwrap();
        let mut block = Block::zero();
        block.as_bytes_mut()[0] = 0x5a;
        transaction.write_block(&mut store, metadata.start, &block).unwrap();
        transaction.retire_extent(CowExtent::new(volume.root().metadata_root, 1).unwrap()).unwrap();
        store.stale = Some(metadata.start);
        store.stale_reads = 1;

        assert_eq!(volume.commit(&mut store, transaction, metadata), Ok(2));
        assert_eq!(volume.root().generation, 2);
    }

    #[test]
    fn reclaimed_extents_are_reusable_only_after_publication() {
        let mut store = MemoryBlockStore::<128>::new();
        let mut volume = CowVolume::format(&mut store).unwrap();
        let mut transaction = volume.begin(&mut store).unwrap();
        let first = transaction.allocate_blocks(&mut store, 2).unwrap();
        let blocker = transaction.allocate_blocks(&mut store, 2).unwrap();
        let metadata = transaction.allocate_blocks(&mut store, 1).unwrap();
        transaction.write_block(&mut store, metadata.start, &Block::zero()).unwrap();
        volume.commit(&mut store, transaction, metadata).unwrap();

        let root = volume.root();
        let mut bitmap = Block::zero();
        store
            .read_block(
                BlockIndex::new(root.bitmap_start.get() + root.bitmap_slot as u64),
                &mut bitmap,
            )
            .unwrap();
        assert!(bitmap_is_set(&[bitmap; COW_MAX_BITMAP_BLOCKS], blocker.start.get()));
        assert!(bitmap_is_set(&[bitmap; COW_MAX_BITMAP_BLOCKS], blocker.start.get() + 1));

        let mut transaction = volume.begin(&mut store).unwrap();
        transaction.release_extent(first).unwrap();
        let metadata = transaction.allocate_blocks(&mut store, 1).unwrap();
        transaction.write_block(&mut store, metadata.start, &Block::zero()).unwrap();
        volume.commit(&mut store, transaction, metadata).unwrap();

        let mut transaction = volume.begin(&mut store).unwrap();
        let root = volume.root();
        let mut bitmap = Block::zero();
        store
            .read_block(
                BlockIndex::new(root.bitmap_start.get() + root.bitmap_slot as u64),
                &mut bitmap,
            )
            .unwrap();
        assert!(bitmap_is_set(&[bitmap; COW_MAX_BITMAP_BLOCKS], blocker.start.get()));
        assert!(bitmap_is_set(&[bitmap; COW_MAX_BITMAP_BLOCKS], blocker.start.get() + 1));
        let reused = transaction.allocate_blocks(&mut store, 2).unwrap();
        assert_eq!(reused, first);
        let after_blocker = transaction.allocate_blocks(&mut store, 1).unwrap();
        assert!(
            after_blocker.start.get() + after_blocker.blocks as u64 <= blocker.start.get()
                || after_blocker.start.get() >= blocker.start.get() + blocker.blocks as u64,
            "first={first:?} blocker={blocker:?} reused={reused:?} after={after_blocker:?}"
        );
    }

    #[test]
    fn arena_scoped_allocation_keeps_pools_disjoint_and_reports_exhaustion() {
        let mut store = MemoryBlockStore::<128>::new();
        let volume = CowVolume::format(&mut store).unwrap();
        let mut transaction = volume.begin(&mut store).unwrap();
        let system = (volume.root().data_start.get() + 1, volume.root().data_start.get() + 3);
        let user = (system.1, system.1 + 3);

        let system_extent = transaction.allocate_blocks_in_arena(&mut store, 2, system).unwrap();
        assert_eq!(system_extent.start.get(), system.0);
        assert_eq!(
            transaction.allocate_blocks_in_arena(&mut store, 1, system),
            Err(CowError::OutOfSpace)
        );

        let user_extent = transaction.allocate_blocks_in_arena(&mut store, 2, user).unwrap();
        assert_eq!(user_extent.start.get(), user.0);
        assert!(system_extent.start.get() + system_extent.blocks as u64 <= user_extent.start.get());
        assert_eq!(
            transaction.allocate_blocks_in_arena(&mut store, 1, (user.1, user.1)),
            Err(CowError::InvalidRequest)
        );
        assert_eq!(
            transaction
                .reserve_extent_in_arena(CowExtent::new(BlockIndex::new(user.1), 1).unwrap(), user),
            Err(CowError::InvalidRequest)
        );
    }

    #[test]
    fn old_media_is_not_reinterpreted() {
        let mut store = MemoryBlockStore::<32>::new();
        let mut marker = Block::zero();
        marker.as_bytes_mut()[..8].copy_from_slice(b"LOGOSFS\0");
        store.write_block(BlockIndex::new(0), &marker).unwrap();
        assert!(matches!(CowVolume::open(&mut store), Err(CowError::UnsupportedVersion)));
    }

    #[test]
    fn provisioned_blank_media_is_reported_for_v4_formatting() {
        let mut store = MemoryBlockStore::<32>::new();
        let mut marker = Block::zero();
        marker.as_bytes_mut()[..COW_PROVISIONED_BLANK_MAGIC.len()]
            .copy_from_slice(COW_PROVISIONED_BLANK_MAGIC);
        store.write_block(BlockIndex::new(0), &marker).unwrap();
        assert!(matches!(CowVolume::open(&mut store), Err(CowError::ProvisionedBlank)));
        CowVolume::format_provisioned(&mut store).unwrap();
        assert_eq!(CowVolume::open(&mut store).unwrap().root().generation, 1);
    }

    #[test]
    fn interrupted_root_publication_keeps_previous_root() {
        let mut store = MemoryBlockStore::<128>::new();
        let volume = CowVolume::format(&mut store).unwrap();
        let old = volume.root().metadata_root;
        let mut transaction = volume.begin(&mut store).unwrap();
        let metadata = transaction.allocate_blocks(&mut store, 1).unwrap();
        let mut block = Block::zero();
        block.as_bytes_mut()[0] = 0xa5;
        transaction.write_block(&mut store, metadata.start, &block).unwrap();
        assert_ne!(metadata.start, old);
        let reopened = CowVolume::open(&mut store).unwrap();
        assert_eq!(reopened.root().generation, 1);
    }

    #[test]
    fn commit_failure_points_reopen_to_a_valid_root() {
        for failure in [
            FailurePoint::Write(0),
            FailurePoint::Write(1),
            FailurePoint::Write(2),
            FailurePoint::Write(3),
            FailurePoint::Flush(0),
            FailurePoint::Flush(1),
            FailurePoint::Flush(2),
        ] {
            let mut store = CrashStore::<128>::new();
            let mut volume = CowVolume::format(&mut store).unwrap();
            let mut transaction = volume.begin(&mut store).unwrap();
            let metadata = transaction.allocate_blocks(&mut store, 1).unwrap();
            let mut block = Block::zero();
            block.as_bytes_mut()[0] = 0x5a;
            transaction.write_block(&mut store, metadata.start, &block).unwrap();
            transaction
                .retire_extent(CowExtent::new(volume.root().metadata_root, 1).unwrap())
                .unwrap();

            store.arm(failure);
            let _ = volume.commit(&mut store, transaction, metadata);
            store.power_loss();

            let reopened = CowVolume::open(&mut store).expect("crash point must recover");
            let expected_generation = match failure {
                FailurePoint::Write(3) | FailurePoint::Flush(2) => 2,
                FailurePoint::Write(_) | FailurePoint::Flush(_) => 1,
            };
            assert_eq!(reopened.root().generation, expected_generation);
        }
    }

    #[test]
    fn metadata_page_write_failure_keeps_previous_root_visible() {
        let mut store = CrashStore::<128>::new();
        let volume = CowVolume::format(&mut store).unwrap();
        let mut transaction = volume.begin(&mut store).unwrap();
        let metadata = transaction.allocate_blocks(&mut store, 1).unwrap();
        store.arm(FailurePoint::Write(0));
        assert_eq!(
            transaction.write_block(&mut store, metadata.start, &Block::zero()),
            Err(CowError::Block(BlockError::Io))
        );
        let reopened = CowVolume::open(&mut store).unwrap();
        assert_eq!(reopened.root().generation, 1);
    }
}
