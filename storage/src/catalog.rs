use crate::{
    Block, BlockError, BlockIndex, BlockStore, COW_MAX_RETIRED_EXTENTS,
    COW_MAX_TRANSACTION_EXTENTS, COW_PROVISIONED_BLANK_MAGIC, COW_SUPERBLOCK_MAGIC, CowError,
    CowExtent,
};

pub const SYSTEM_CATALOG_FORMAT_VERSION: u16 = 5;
pub const SYSTEM_CATALOG_MAX_BLOCKS: u32 = 16;
pub const SYSTEM_CATALOG_MAX_BYTES: usize = SYSTEM_CATALOG_MAX_BLOCKS as usize * crate::BLOCK_BYTES;

const SUPERBLOCK_A: BlockIndex = BlockIndex::new(0);
const SUPERBLOCK_B: BlockIndex = BlockIndex::new(1);
const COMMIT_START: u64 = 2;
const COMMIT_SLOTS: usize = 2;
const DATA_START: u64 = COMMIT_START + COMMIT_SLOTS as u64;
const CHECKSUM_OFFSET: usize = 160;
const COMMIT_CHECKSUM_OFFSET: usize = 64;
const COMMIT_MAGIC: &[u8; 4] = b"LOSC";
const LEGACY_MAGIC: &[u8; 8] = b"LOGOSFS\0";
const MAX_BLOCK_COUNT: u64 = (crate::COW_MAX_BITMAP_BLOCKS * crate::BLOCK_BYTES * 8) as u64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CatalogError {
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
}

impl From<BlockError> for CatalogError {
    fn from(error: BlockError) -> Self {
        Self::Block(error)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SystemCatalogRoot {
    pub generation: u64,
    pub metadata_start: BlockIndex,
    pub metadata_blocks: u32,
    pub metadata_bytes: u32,
    pub catalog_start: BlockIndex,
    pub catalog_blocks: u32,
    pub catalog_bytes: u32,
    pub bitmap_slot: u8,
    pub bitmap_start: BlockIndex,
    pub bitmap_blocks: u16,
    pub system_start: BlockIndex,
    pub system_end: BlockIndex,
    pub user_start: BlockIndex,
    pub user_end: BlockIndex,
    pub package_start: BlockIndex,
    pub package_end: BlockIndex,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CommitRecord {
    generation: u64,
    metadata_start: BlockIndex,
    metadata_blocks: u32,
    metadata_bytes: u32,
    catalog_start: BlockIndex,
    catalog_blocks: u32,
    catalog_bytes: u32,
    bitmap_slot: u8,
}

pub struct SystemCatalogVolume {
    root: SystemCatalogRoot,
    active_superblock: u8,
}

impl SystemCatalogVolume {
    pub fn format<B: BlockStore>(
        store: &mut B,
        system_blocks: u64,
        package_start: u64,
    ) -> Result<Self, CatalogError> {
        Self::format_inner(store, system_blocks, package_start, false)
    }

    pub fn format_provisioned<B: BlockStore>(
        store: &mut B,
        system_blocks: u64,
        package_start: u64,
    ) -> Result<Self, CatalogError> {
        Self::format_inner(store, system_blocks, package_start, true)
    }

    fn format_inner<B: BlockStore>(
        store: &mut B,
        system_blocks: u64,
        package_start: u64,
        provisioned: bool,
    ) -> Result<Self, CatalogError> {
        let data_end = validate_capacity(store.block_count())?;
        validate_pools(store.block_count(), data_end, system_blocks, package_start)?;
        ensure_blank(store, provisioned)?;

        let root = empty_root(store.block_count(), data_end, system_blocks, package_start)?;
        let bitmap = [Block::zero(); crate::COW_MAX_BITMAP_BLOCKS];
        write_bitmap(store, root, &bitmap)?;
        store.flush()?;
        write_superblock(store, SUPERBLOCK_A, root)?;
        store.flush()?;
        verify_superblock(store, SUPERBLOCK_A, root)?;
        write_superblock(store, SUPERBLOCK_B, root)?;
        store.flush()?;
        verify_superblock(store, SUPERBLOCK_B, root)?;
        Ok(Self { root, active_superblock: 1 })
    }

    pub fn open<B: BlockStore>(store: &mut B) -> Result<Self, CatalogError> {
        let data_end = validate_capacity(store.block_count())?;
        let first = read_superblock(store, SUPERBLOCK_A);
        let second = read_superblock(store, SUPERBLOCK_B);
        let (mut root, mut active_superblock) = match (first, second) {
            (Ok(Some(a)), Ok(Some(b))) if b.generation > a.generation => (b, 1),
            (Ok(Some(a)), _) => (a, 0),
            (_, Ok(Some(b))) => (b, 1),
            (Ok(None), Ok(None)) => return Err(CatalogError::Unformatted),
            (Err(CatalogError::ProvisionedBlank), Ok(None))
            | (Ok(None), Err(CatalogError::ProvisionedBlank))
            | (Err(CatalogError::ProvisionedBlank), Err(CatalogError::ProvisionedBlank)) => {
                return Err(CatalogError::ProvisionedBlank);
            }
            (Err(error), Ok(None)) | (Ok(None), Err(error)) => return Err(error),
            (Err(error), Err(_)) => return Err(error),
        };
        validate_root(root, store.block_count(), data_end)?;

        let mut candidate = None;
        for slot in 0..COMMIT_SLOTS {
            let Some(commit) = (match read_commit(store, slot) {
                Ok(commit) => commit,
                Err(CatalogError::Corrupt) => continue,
                Err(error) => return Err(error),
            }) else {
                continue;
            };
            if commit.generation > root.generation
                && candidate
                    .is_none_or(|current: CommitRecord| commit.generation > current.generation)
                && validate_commit(root, commit).is_ok()
            {
                candidate = Some(commit);
            }
        }
        if let Some(commit) = candidate {
            root.generation = commit.generation;
            root.metadata_start = commit.metadata_start;
            root.metadata_blocks = commit.metadata_blocks;
            root.metadata_bytes = commit.metadata_bytes;
            root.catalog_start = commit.catalog_start;
            root.catalog_blocks = commit.catalog_blocks;
            root.catalog_bytes = commit.catalog_bytes;
            root.bitmap_slot = commit.bitmap_slot;
            let target = if active_superblock == 0 { SUPERBLOCK_B } else { SUPERBLOCK_A };
            write_superblock(store, target, root)?;
            store.flush()?;
            verify_superblock(store, target, root)?;
            active_superblock ^= 1;
        }
        Ok(Self { root, active_superblock })
    }

    pub const fn root(&self) -> SystemCatalogRoot {
        self.root
    }

    pub const fn system_arena(&self) -> (u64, u64) {
        (self.root.system_start.get(), self.root.system_end.get())
    }

    pub const fn user_arena(&self) -> (u64, u64) {
        (self.root.user_start.get(), self.root.user_end.get())
    }

    pub const fn package_arena(&self) -> (u64, u64) {
        (self.root.package_start.get(), self.root.package_end.get())
    }

    pub fn read_catalog<B: BlockStore>(
        &self,
        store: &mut B,
        output: &mut [u8],
    ) -> Result<usize, CatalogError> {
        if self.root.catalog_blocks == 0 {
            return Err(CatalogError::Unformatted);
        }
        if self.root.catalog_bytes as usize > output.len()
            || self.root.catalog_bytes as usize
                > self.root.catalog_blocks as usize * crate::BLOCK_BYTES
        {
            return Err(CatalogError::InvalidRequest);
        }
        for offset in 0..self.root.catalog_blocks as usize {
            let mut block = Block::zero();
            store.read_block(
                BlockIndex::new(self.root.catalog_start.get() + offset as u64),
                &mut block,
            )?;
            let start = offset * crate::BLOCK_BYTES;
            let end = (start + crate::BLOCK_BYTES).min(self.root.catalog_bytes as usize);
            if start < end {
                output[start..end].copy_from_slice(&block.as_bytes()[..end - start]);
            }
        }
        Ok(self.root.catalog_bytes as usize)
    }

    pub fn read_metadata<B: BlockStore>(
        &self,
        store: &mut B,
        output: &mut [u8],
    ) -> Result<usize, CatalogError> {
        if self.root.metadata_blocks == 0 {
            return Err(CatalogError::Unformatted);
        }
        if self.root.metadata_bytes as usize > output.len()
            || self.root.metadata_bytes as usize
                > self.root.metadata_blocks as usize * crate::BLOCK_BYTES
        {
            return Err(CatalogError::InvalidRequest);
        }
        for offset in 0..self.root.metadata_blocks as usize {
            let mut block = Block::zero();
            store.read_block(
                BlockIndex::new(self.root.metadata_start.get() + offset as u64),
                &mut block,
            )?;
            let start = offset * crate::BLOCK_BYTES;
            let end = (start + crate::BLOCK_BYTES).min(self.root.metadata_bytes as usize);
            if start < end {
                output[start..end].copy_from_slice(&block.as_bytes()[..end - start]);
            }
        }
        Ok(self.root.metadata_bytes as usize)
    }

    pub fn replace_catalog<B: BlockStore>(
        &mut self,
        store: &mut B,
        snapshot: &[u8],
    ) -> Result<u64, CatalogError> {
        let previous_generation = self.root.generation;
        match self.replace_catalog_inner(store, snapshot) {
            Ok(generation) => Ok(generation),
            Err(error) if matches!(error, CatalogError::Block(_) | CatalogError::Corrupt) => {
                let recovered = Self::open(store)?;
                let generation = recovered.root.generation;
                *self = recovered;
                if generation > previous_generation { Ok(generation) } else { Err(error) }
            }
            Err(error) => Err(error),
        }
    }

    fn replace_catalog_inner<B: BlockStore>(
        &mut self,
        store: &mut B,
        snapshot: &[u8],
    ) -> Result<u64, CatalogError> {
        if snapshot.is_empty() || snapshot.len() > SYSTEM_CATALOG_MAX_BYTES {
            return Err(CatalogError::InvalidRequest);
        }
        let blocks = snapshot.len().div_ceil(crate::BLOCK_BYTES) as u32;
        if blocks == 0 || blocks > SYSTEM_CATALOG_MAX_BLOCKS {
            return Err(CatalogError::TooLarge);
        }
        let mut bitmap = [Block::zero(); crate::COW_MAX_BITMAP_BLOCKS];
        read_bitmap(store, self.root, &mut bitmap)?;
        let extent = allocate_in_arena(&bitmap, self.system_arena(), blocks)?;
        for offset in 0..blocks as usize {
            let start = offset * crate::BLOCK_BYTES;
            let end = (start + crate::BLOCK_BYTES).min(snapshot.len());
            let mut block = Block::zero();
            block.as_bytes_mut()[..end - start].copy_from_slice(&snapshot[start..end]);
            store.write_block(BlockIndex::new(extent.get() + offset as u64), &block)?;
        }
        for offset in 0..blocks as u64 {
            bitmap_set(&mut bitmap, extent.get() + offset, true);
        }
        if self.root.catalog_blocks != 0 {
            for offset in 0..self.root.catalog_blocks as u64 {
                bitmap_set(&mut bitmap, self.root.catalog_start.get() + offset, false);
            }
        }
        let generation =
            self.root.generation.checked_add(1).ok_or(CatalogError::GenerationExhausted)?;
        let bitmap_slot = 1u8.saturating_sub(self.root.bitmap_slot);
        let mut root = self.root;
        root.generation = generation;
        root.catalog_start = BlockIndex::new(extent.get());
        root.catalog_blocks = blocks;
        root.catalog_bytes = snapshot.len() as u32;
        root.bitmap_slot = bitmap_slot;
        write_bitmap(store, root, &bitmap)?;
        store.flush()?;
        verify_catalog(store, root, snapshot)?;
        verify_bitmap(store, root, &bitmap)?;
        let commit_slot = generation as usize % COMMIT_SLOTS;
        write_commit(
            store,
            commit_slot,
            CommitRecord {
                generation,
                metadata_start: root.metadata_start,
                metadata_blocks: root.metadata_blocks,
                metadata_bytes: root.metadata_bytes,
                catalog_start: root.catalog_start,
                catalog_blocks: root.catalog_blocks,
                catalog_bytes: root.catalog_bytes,
                bitmap_slot,
            },
        )?;
        store.flush()?;
        let target = if self.active_superblock == 0 { SUPERBLOCK_B } else { SUPERBLOCK_A };
        write_superblock(store, target, root)?;
        store.flush()?;
        verify_superblock(store, target, root)?;
        self.root = root;
        self.active_superblock ^= 1;
        Ok(generation)
    }

    pub fn replace_metadata<B: BlockStore>(
        &mut self,
        store: &mut B,
        snapshot: &[u8],
    ) -> Result<u64, CatalogError> {
        if snapshot.is_empty() || snapshot.len() > MAX_RECORD_BYTES {
            return Err(CatalogError::InvalidRequest);
        }
        let blocks = snapshot.len().div_ceil(crate::BLOCK_BYTES) as u32;
        let mut bitmap = [Block::zero(); crate::COW_MAX_BITMAP_BLOCKS];
        read_bitmap(store, self.root, &mut bitmap)?;
        let extent = allocate_in_arena(&bitmap, self.system_arena(), blocks)?;
        for offset in 0..blocks as usize {
            let start = offset * crate::BLOCK_BYTES;
            let end = (start + crate::BLOCK_BYTES).min(snapshot.len());
            let mut block = Block::zero();
            block.as_bytes_mut()[..end - start].copy_from_slice(&snapshot[start..end]);
            store.write_block(BlockIndex::new(extent.get() + offset as u64), &block)?;
        }
        for offset in 0..blocks as u64 {
            bitmap_set(&mut bitmap, extent.get() + offset, true);
        }
        if self.root.metadata_blocks != 0 {
            for offset in 0..self.root.metadata_blocks as u64 {
                bitmap_set(&mut bitmap, self.root.metadata_start.get() + offset, false);
            }
        }
        let generation =
            self.root.generation.checked_add(1).ok_or(CatalogError::GenerationExhausted)?;
        let bitmap_slot = 1u8.saturating_sub(self.root.bitmap_slot);
        let mut root = self.root;
        root.generation = generation;
        root.metadata_start = BlockIndex::new(extent.get());
        root.metadata_blocks = blocks;
        root.metadata_bytes = snapshot.len() as u32;
        root.bitmap_slot = bitmap_slot;
        write_bitmap(store, root, &bitmap)?;
        store.flush()?;
        verify_metadata(store, root, snapshot)?;
        verify_bitmap(store, root, &bitmap)?;
        let commit_slot = generation as usize % COMMIT_SLOTS;
        write_commit(
            store,
            commit_slot,
            CommitRecord {
                generation,
                metadata_start: root.metadata_start,
                metadata_blocks: root.metadata_blocks,
                metadata_bytes: root.metadata_bytes,
                catalog_start: root.catalog_start,
                catalog_blocks: root.catalog_blocks,
                catalog_bytes: root.catalog_bytes,
                bitmap_slot,
            },
        )?;
        store.flush()?;
        let target = if self.active_superblock == 0 { SUPERBLOCK_B } else { SUPERBLOCK_A };
        write_superblock(store, target, root)?;
        store.flush()?;
        verify_superblock(store, target, root)?;
        self.root = root;
        self.active_superblock ^= 1;
        Ok(generation)
    }

    pub fn begin<B: BlockStore>(
        &self,
        store: &mut B,
    ) -> Result<SystemCatalogTransaction, CowError> {
        let mut bitmap = [Block::zero(); crate::COW_MAX_BITMAP_BLOCKS];
        read_bitmap(store, self.root, &mut bitmap).map_err(catalog_to_cow)?;
        Ok(SystemCatalogTransaction {
            root: self.root,
            bitmap,
            bitmap_slot: 1u8.saturating_sub(self.root.bitmap_slot),
            allocated: [CowExtent::EMPTY; COW_MAX_TRANSACTION_EXTENTS],
            allocated_count: 0,
            released: [CowExtent::EMPTY; COW_MAX_RETIRED_EXTENTS],
            released_count: 0,
        })
    }

    pub fn commit<B: BlockStore>(
        &mut self,
        store: &mut B,
        transaction: SystemCatalogTransaction,
        metadata: CowExtent,
        metadata_bytes: usize,
    ) -> Result<u64, CowError> {
        if transaction.root != self.root
            || metadata.blocks == 0
            || metadata_bytes == 0
            || metadata_bytes > metadata.blocks as usize * crate::BLOCK_BYTES
            || !extent_in_arena(metadata, self.system_arena())
            || !transaction.contains_allocated(metadata)
        {
            return Err(CowError::InvalidRequest);
        }
        let mut bitmap = transaction.bitmap;
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
        let generation =
            self.root.generation.checked_add(1).ok_or(CowError::GenerationExhausted)?;
        let root = SystemCatalogRoot {
            generation,
            metadata_start: metadata.start,
            metadata_blocks: metadata.blocks,
            metadata_bytes: metadata_bytes as u32,
            bitmap_slot: transaction.bitmap_slot,
            ..self.root
        };
        write_bitmap(store, root, &bitmap).map_err(catalog_to_cow)?;
        store.flush().map_err(CowError::Block)?;
        for extent in transaction.allocated[..transaction.allocated_count].iter() {
            verify_extent(store, *extent).map_err(catalog_to_cow)?;
        }
        verify_bitmap(store, root, &bitmap).map_err(catalog_to_cow)?;
        let commit_slot = generation as usize % COMMIT_SLOTS;
        write_commit(
            store,
            commit_slot,
            CommitRecord {
                generation,
                metadata_start: root.metadata_start,
                metadata_blocks: root.metadata_blocks,
                metadata_bytes: root.metadata_bytes,
                catalog_start: root.catalog_start,
                catalog_blocks: root.catalog_blocks,
                catalog_bytes: root.catalog_bytes,
                bitmap_slot: root.bitmap_slot,
            },
        )
        .map_err(catalog_to_cow)?;
        store.flush().map_err(CowError::Block)?;
        let target = if self.active_superblock == 0 { SUPERBLOCK_B } else { SUPERBLOCK_A };
        write_superblock(store, target, root).map_err(catalog_to_cow)?;
        store.flush().map_err(CowError::Block)?;
        verify_superblock(store, target, root).map_err(catalog_to_cow)?;
        self.root = root;
        self.active_superblock ^= 1;
        Ok(generation)
    }
}

pub struct SystemCatalogTransaction {
    root: SystemCatalogRoot,
    bitmap: [Block; crate::COW_MAX_BITMAP_BLOCKS],
    bitmap_slot: u8,
    allocated: [CowExtent; COW_MAX_TRANSACTION_EXTENTS],
    allocated_count: usize,
    released: [CowExtent; COW_MAX_RETIRED_EXTENTS],
    released_count: usize,
}

impl SystemCatalogTransaction {
    pub fn allocate_blocks<B: BlockStore>(
        &mut self,
        _store: &mut B,
        blocks: u32,
    ) -> Result<CowExtent, CowError> {
        self.allocate_in_arena(blocks, (self.root.user_start.get(), self.root.user_end.get()))
    }

    pub fn allocate_metadata_blocks<B: BlockStore>(
        &mut self,
        _store: &mut B,
        blocks: u32,
    ) -> Result<CowExtent, CowError> {
        self.allocate_in_arena(blocks, (self.root.system_start.get(), self.root.system_end.get()))
    }

    fn allocate_in_arena(&mut self, blocks: u32, arena: (u64, u64)) -> Result<CowExtent, CowError> {
        if blocks == 0 || arena.0 >= arena.1 {
            return Err(CowError::InvalidRequest);
        }
        let mut start = None;
        let mut length = 0u32;
        for index in arena.0..arena.1 {
            let occupied = bitmap_is_set(&self.bitmap, index)
                || self.allocated[..self.allocated_count].iter().any(|extent| {
                    index >= extent.start.get() && index < extent.start.get() + extent.blocks as u64
                })
                || self.released[..self.released_count].iter().any(|extent| {
                    index >= extent.start.get() && index < extent.start.get() + extent.blocks as u64
                });
            if occupied {
                start = None;
                length = 0;
                continue;
            }
            start.get_or_insert(index);
            length += 1;
            if length == blocks {
                let extent = CowExtent::new(BlockIndex::new(start.unwrap()), blocks)
                    .ok_or(CowError::InvalidRequest)?;
                self.record_allocated(extent)?;
                return Ok(extent);
            }
        }
        Err(CowError::OutOfSpace)
    }

    pub fn reserve_extent(&mut self, extent: CowExtent) -> Result<(), CowError> {
        if !extent_in_arena(extent, (self.root.system_start.get(), self.root.package_end.get())) {
            return Err(CowError::InvalidRequest);
        }
        self.record_allocated(extent)
    }

    pub fn retire_extent(&mut self, extent: CowExtent) -> Result<(), CowError> {
        if !extent_in_arena(extent, (self.root.system_start.get(), self.root.package_end.get()))
            || self.released_count == self.released.len()
        {
            return Err(CowError::InvalidRequest);
        }
        self.released[self.released_count] = extent;
        self.released_count += 1;
        Ok(())
    }

    fn record_allocated(&mut self, extent: CowExtent) -> Result<(), CowError> {
        if self.allocated_count == self.allocated.len() {
            return Err(CowError::RetiredExtentCapacity);
        }
        self.allocated[self.allocated_count] = extent;
        self.allocated_count += 1;
        Ok(())
    }

    fn contains_allocated(&self, target: CowExtent) -> bool {
        self.allocated[..self.allocated_count].contains(&target)
    }

    pub fn write_block<B: BlockStore>(
        &self,
        store: &mut B,
        index: BlockIndex,
        block: &Block,
    ) -> Result<(), CowError> {
        if !self.allocated[..self.allocated_count].iter().any(|extent| {
            index.get() >= extent.start.get()
                && index.get() < extent.start.get() + extent.blocks as u64
        }) {
            return Err(CowError::InvalidRequest);
        }
        store.write_block(index, block).map_err(CowError::Block)
    }
}

fn extent_in_arena(extent: CowExtent, arena: (u64, u64)) -> bool {
    extent.blocks != 0
        && extent.start.get() >= arena.0
        && extent.start.get().checked_add(extent.blocks as u64).is_some_and(|end| end <= arena.1)
}

fn verify_extent<B: BlockStore>(store: &mut B, extent: CowExtent) -> Result<(), CatalogError> {
    for offset in 0..extent.blocks as u64 {
        let mut block = Block::zero();
        store.read_block_uncached(BlockIndex::new(extent.start.get() + offset), &mut block)?;
    }
    Ok(())
}

fn catalog_to_cow(error: CatalogError) -> CowError {
    match error {
        CatalogError::Block(error) => CowError::Block(error),
        CatalogError::OutOfSpace => CowError::OutOfSpace,
        CatalogError::GenerationExhausted => CowError::GenerationExhausted,
        CatalogError::TooLarge => CowError::TooLarge,
        CatalogError::InvalidRequest => CowError::InvalidRequest,
        CatalogError::NotBlank => CowError::NotBlank,
        CatalogError::Unformatted => CowError::Unformatted,
        CatalogError::ProvisionedBlank => CowError::ProvisionedBlank,
        CatalogError::UnsupportedVersion => CowError::UnsupportedVersion,
        CatalogError::Corrupt => CowError::Corrupt,
        CatalogError::TooSmall => CowError::TooSmall,
    }
}

const MAX_RECORD_BYTES: usize = MAX_BLOCK_COUNT as usize * crate::BLOCK_BYTES;

fn validate_capacity(blocks: u64) -> Result<u64, CatalogError> {
    if blocks <= DATA_START + 2 {
        return Err(CatalogError::TooSmall);
    }
    if blocks > MAX_BLOCK_COUNT {
        return Err(CatalogError::TooLarge);
    }
    Ok(blocks - 2)
}

fn validate_pools(
    blocks: u64,
    data_end: u64,
    system_blocks: u64,
    package_start: u64,
) -> Result<(), CatalogError> {
    let system_end = DATA_START.checked_add(system_blocks).ok_or(CatalogError::InvalidRequest)?;
    if system_blocks == 0
        || system_end >= package_start
        || package_start >= data_end
        || data_end >= blocks
        || system_end > data_end
    {
        return Err(CatalogError::InvalidRequest);
    }
    Ok(())
}

fn empty_root(
    blocks: u64,
    data_end: u64,
    system_blocks: u64,
    package_start: u64,
) -> Result<SystemCatalogRoot, CatalogError> {
    let system_end = DATA_START.checked_add(system_blocks).ok_or(CatalogError::InvalidRequest)?;
    validate_pools(blocks, data_end, system_blocks, package_start)?;
    Ok(SystemCatalogRoot {
        generation: 1,
        metadata_start: BlockIndex::new(0),
        metadata_blocks: 0,
        metadata_bytes: 0,
        catalog_start: BlockIndex::new(0),
        catalog_blocks: 0,
        catalog_bytes: 0,
        bitmap_slot: 0,
        bitmap_start: BlockIndex::new(data_end),
        bitmap_blocks: 1,
        system_start: BlockIndex::new(DATA_START),
        system_end: BlockIndex::new(system_end),
        user_start: BlockIndex::new(system_end),
        user_end: BlockIndex::new(package_start),
        package_start: BlockIndex::new(package_start),
        package_end: BlockIndex::new(data_end),
    })
}

fn validate_root(root: SystemCatalogRoot, blocks: u64, data_end: u64) -> Result<(), CatalogError> {
    if root.generation == 0
        || root.bitmap_slot >= 2
        || root.bitmap_blocks != 1
        || root.bitmap_start.get() != data_end
        || root.package_end.get() != data_end
        || root.system_start.get() != DATA_START
        || root.system_start.get() >= root.system_end.get()
        || root.system_end.get() != root.user_start.get()
        || root.user_start.get() >= root.user_end.get()
        || root.user_end.get() != root.package_start.get()
        || root.package_start.get() >= root.package_end.get()
        || root.package_end.get() >= blocks
    {
        return Err(CatalogError::Corrupt);
    }
    if root.catalog_blocks == 0 {
        if root.catalog_bytes != 0 || root.catalog_start.get() != 0 {
            return Err(CatalogError::Corrupt);
        }
    } else if root.catalog_start.get() < root.system_start.get()
        || root
            .catalog_start
            .get()
            .checked_add(root.catalog_blocks as u64)
            .is_none_or(|end| end > root.system_end.get())
        || root.catalog_bytes == 0
        || root.catalog_bytes as usize > root.catalog_blocks as usize * crate::BLOCK_BYTES
    {
        return Err(CatalogError::Corrupt);
    }
    if root.metadata_blocks != 0
        && (root.metadata_start.get() < root.system_start.get()
            || root
                .metadata_start
                .get()
                .checked_add(root.metadata_blocks as u64)
                .is_none_or(|end| end > root.system_end.get())
            || root.metadata_bytes == 0
            || root.metadata_bytes as usize > root.metadata_blocks as usize * crate::BLOCK_BYTES)
    {
        return Err(CatalogError::Corrupt);
    }
    if root.metadata_blocks == 0 && (root.metadata_start.get() != 0 || root.metadata_bytes != 0) {
        return Err(CatalogError::Corrupt);
    }
    Ok(())
}

fn validate_commit(root: SystemCatalogRoot, commit: CommitRecord) -> Result<(), CatalogError> {
    if commit.generation == 0
        || commit.bitmap_slot >= 2
        || (commit.catalog_blocks == 0
            && (commit.catalog_start.get() != 0 || commit.catalog_bytes != 0))
        || (commit.metadata_blocks == 0
            && (commit.metadata_start.get() != 0 || commit.metadata_bytes != 0))
        || (commit.metadata_blocks != 0
            && (commit.metadata_start.get() < root.system_start.get()
                || commit
                    .metadata_start
                    .get()
                    .checked_add(commit.metadata_blocks as u64)
                    .is_none_or(|end| end > root.system_end.get())
                || commit.metadata_bytes == 0
                || commit.metadata_bytes as usize
                    > commit.metadata_blocks as usize * crate::BLOCK_BYTES))
        || (commit.catalog_blocks != 0
            && (commit.catalog_start.get() < root.system_start.get()
                || commit
                    .catalog_start
                    .get()
                    .checked_add(commit.catalog_blocks as u64)
                    .is_none_or(|end| end > root.system_end.get())
                || commit.catalog_bytes == 0
                || commit.catalog_bytes as usize
                    > commit.catalog_blocks as usize * crate::BLOCK_BYTES))
    {
        return Err(CatalogError::Corrupt);
    }
    Ok(())
}

fn ensure_blank<B: BlockStore>(store: &mut B, provisioned: bool) -> Result<(), CatalogError> {
    let mut first = Block::zero();
    store.read_block(SUPERBLOCK_A, &mut first)?;
    if provisioned {
        if &first.as_bytes()[..COW_PROVISIONED_BLANK_MAGIC.len()] != COW_PROVISIONED_BLANK_MAGIC
            || first.as_bytes()[COW_PROVISIONED_BLANK_MAGIC.len()..].iter().any(|byte| *byte != 0)
        {
            return Err(CatalogError::NotBlank);
        }
        return Ok(());
    }
    let mut block = Block::zero();
    for index in 0..store.block_count() {
        store.read_block(BlockIndex::new(index), &mut block)?;
        if block.as_bytes().iter().any(|byte| *byte != 0) {
            return Err(CatalogError::NotBlank);
        }
    }
    Ok(())
}

fn allocate_in_arena(
    bitmap: &[Block; crate::COW_MAX_BITMAP_BLOCKS],
    arena: (u64, u64),
    blocks: u32,
) -> Result<BlockIndex, CatalogError> {
    let mut start = None;
    let mut length = 0u32;
    for index in arena.0..arena.1 {
        if bitmap_is_set(bitmap, index) {
            start = None;
            length = 0;
            continue;
        }
        start.get_or_insert(index);
        length += 1;
        if length == blocks {
            return Ok(BlockIndex::new(start.unwrap()));
        }
    }
    Err(CatalogError::OutOfSpace)
}

fn read_bitmap<B: BlockStore>(
    store: &mut B,
    root: SystemCatalogRoot,
    output: &mut [Block; crate::COW_MAX_BITMAP_BLOCKS],
) -> Result<(), CatalogError> {
    store.read_block(
        BlockIndex::new(root.bitmap_start.get() + root.bitmap_slot as u64),
        &mut output[0],
    )?;
    Ok(())
}

fn write_bitmap<B: BlockStore>(
    store: &mut B,
    root: SystemCatalogRoot,
    bitmap: &[Block; crate::COW_MAX_BITMAP_BLOCKS],
) -> Result<(), CatalogError> {
    store.write_block(
        BlockIndex::new(root.bitmap_start.get() + root.bitmap_slot as u64),
        &bitmap[0],
    )?;
    Ok(())
}

fn verify_bitmap<B: BlockStore>(
    store: &mut B,
    root: SystemCatalogRoot,
    expected: &[Block; crate::COW_MAX_BITMAP_BLOCKS],
) -> Result<(), CatalogError> {
    let mut actual = Block::zero();
    store.read_block_uncached(
        BlockIndex::new(root.bitmap_start.get() + root.bitmap_slot as u64),
        &mut actual,
    )?;
    if actual != expected[0] {
        return Err(CatalogError::Block(BlockError::Io));
    }
    Ok(())
}

fn verify_catalog<B: BlockStore>(
    store: &mut B,
    root: SystemCatalogRoot,
    expected: &[u8],
) -> Result<(), CatalogError> {
    for offset in 0..root.catalog_blocks as usize {
        let mut actual = Block::zero();
        store.read_block_uncached(
            BlockIndex::new(root.catalog_start.get() + offset as u64),
            &mut actual,
        )?;
        let start = offset * crate::BLOCK_BYTES;
        let end = (start + crate::BLOCK_BYTES).min(expected.len());
        if actual.as_bytes()[..end - start] != expected[start..end] {
            return Err(CatalogError::Block(BlockError::Io));
        }
    }
    Ok(())
}

fn verify_metadata<B: BlockStore>(
    store: &mut B,
    root: SystemCatalogRoot,
    expected: &[u8],
) -> Result<(), CatalogError> {
    for offset in 0..root.metadata_blocks as usize {
        let mut actual = Block::zero();
        store.read_block_uncached(
            BlockIndex::new(root.metadata_start.get() + offset as u64),
            &mut actual,
        )?;
        let start = offset * crate::BLOCK_BYTES;
        let end = (start + crate::BLOCK_BYTES).min(expected.len());
        if actual.as_bytes()[..end - start] != expected[start..end] {
            return Err(CatalogError::Block(BlockError::Io));
        }
    }
    Ok(())
}

fn verify_superblock<B: BlockStore>(
    store: &mut B,
    index: BlockIndex,
    root: SystemCatalogRoot,
) -> Result<(), CatalogError> {
    let mut actual = Block::zero();
    store.read_block_uncached(index, &mut actual)?;
    let mut expected = Block::zero();
    encode_root(&mut expected, root);
    if actual != expected {
        return Err(CatalogError::Block(BlockError::Io));
    }
    Ok(())
}

fn write_superblock<B: BlockStore>(
    store: &mut B,
    index: BlockIndex,
    root: SystemCatalogRoot,
) -> Result<(), CatalogError> {
    let mut block = Block::zero();
    encode_root(&mut block, root);
    store.write_block(index, &block)?;
    Ok(())
}

fn read_superblock<B: BlockStore>(
    store: &mut B,
    index: BlockIndex,
) -> Result<Option<SystemCatalogRoot>, CatalogError> {
    let mut block = Block::zero();
    store.read_block(index, &mut block)?;
    decode_root(&block)
}

fn encode_root(block: &mut Block, root: SystemCatalogRoot) {
    let bytes = block.as_bytes_mut();
    bytes[..8].copy_from_slice(COW_SUPERBLOCK_MAGIC);
    bytes[8..10].copy_from_slice(&SYSTEM_CATALOG_FORMAT_VERSION.to_le_bytes());
    put_u64(bytes, 16, root.generation);
    put_u64(bytes, 24, root.metadata_start.get());
    put_u32(bytes, 32, root.metadata_blocks);
    put_u32(bytes, 36, root.metadata_bytes);
    put_u64(bytes, 40, root.catalog_start.get());
    put_u32(bytes, 48, root.catalog_blocks);
    put_u32(bytes, 52, root.catalog_bytes);
    bytes[56] = root.bitmap_slot;
    put_u64(bytes, 64, root.bitmap_start.get());
    put_u16(bytes, 72, root.bitmap_blocks);
    put_u64(bytes, 80, root.system_start.get());
    put_u64(bytes, 88, root.system_end.get());
    put_u64(bytes, 96, root.user_start.get());
    put_u64(bytes, 104, root.user_end.get());
    put_u64(bytes, 112, root.package_start.get());
    put_u64(bytes, 120, root.package_end.get());
    put_u32(bytes, CHECKSUM_OFFSET, 0);
    let checksum = crc32c(&bytes[..CHECKSUM_OFFSET]);
    put_u32(bytes, CHECKSUM_OFFSET, checksum);
}

fn decode_root(block: &Block) -> Result<Option<SystemCatalogRoot>, CatalogError> {
    let bytes = block.as_bytes();
    if bytes.iter().all(|byte| *byte == 0) {
        return Ok(None);
    }
    if &bytes[..8] != COW_SUPERBLOCK_MAGIC {
        return Err(
            if &bytes[..8] == COW_PROVISIONED_BLANK_MAGIC
                && bytes[8..].iter().all(|byte| *byte == 0)
            {
                CatalogError::ProvisionedBlank
            } else if &bytes[..8] == LEGACY_MAGIC {
                CatalogError::UnsupportedVersion
            } else {
                CatalogError::Corrupt
            },
        );
    }
    if u16::from_le_bytes([bytes[8], bytes[9]]) != SYSTEM_CATALOG_FORMAT_VERSION {
        return Err(CatalogError::UnsupportedVersion);
    }
    let expected = get_u32(bytes, CHECKSUM_OFFSET);
    let mut copy = *block;
    put_u32(copy.as_bytes_mut(), CHECKSUM_OFFSET, 0);
    if crc32c(&copy.as_bytes()[..CHECKSUM_OFFSET]) != expected {
        return Err(CatalogError::Corrupt);
    }
    Ok(Some(SystemCatalogRoot {
        generation: get_u64(bytes, 16),
        metadata_start: BlockIndex::new(get_u64(bytes, 24)),
        metadata_blocks: get_u32(bytes, 32),
        metadata_bytes: get_u32(bytes, 36),
        catalog_start: BlockIndex::new(get_u64(bytes, 40)),
        catalog_blocks: get_u32(bytes, 48),
        catalog_bytes: get_u32(bytes, 52),
        bitmap_slot: bytes[56],
        bitmap_start: BlockIndex::new(get_u64(bytes, 64)),
        bitmap_blocks: get_u16(bytes, 72),
        system_start: BlockIndex::new(get_u64(bytes, 80)),
        system_end: BlockIndex::new(get_u64(bytes, 88)),
        user_start: BlockIndex::new(get_u64(bytes, 96)),
        user_end: BlockIndex::new(get_u64(bytes, 104)),
        package_start: BlockIndex::new(get_u64(bytes, 112)),
        package_end: BlockIndex::new(get_u64(bytes, 120)),
    }))
}

fn write_commit<B: BlockStore>(
    store: &mut B,
    slot: usize,
    commit: CommitRecord,
) -> Result<(), CatalogError> {
    let mut block = Block::zero();
    let bytes = block.as_bytes_mut();
    bytes[..4].copy_from_slice(COMMIT_MAGIC);
    put_u64(bytes, 8, commit.generation);
    put_u64(bytes, 16, commit.metadata_start.get());
    put_u32(bytes, 24, commit.metadata_blocks);
    put_u32(bytes, 28, commit.metadata_bytes);
    put_u64(bytes, 32, commit.catalog_start.get());
    put_u32(bytes, 40, commit.catalog_blocks);
    put_u32(bytes, 44, commit.catalog_bytes);
    bytes[48] = commit.bitmap_slot;
    put_u32(bytes, COMMIT_CHECKSUM_OFFSET, 0);
    let checksum = crc32c(&bytes[..COMMIT_CHECKSUM_OFFSET]);
    put_u32(bytes, COMMIT_CHECKSUM_OFFSET, checksum);
    store.write_block(BlockIndex::new(COMMIT_START + slot as u64), &block)?;
    Ok(())
}

fn read_commit<B: BlockStore>(
    store: &mut B,
    slot: usize,
) -> Result<Option<CommitRecord>, CatalogError> {
    let mut block = Block::zero();
    store.read_block(BlockIndex::new(COMMIT_START + slot as u64), &mut block)?;
    let bytes = block.as_bytes();
    if bytes.iter().all(|byte| *byte == 0) {
        return Ok(None);
    }
    if &bytes[..4] != COMMIT_MAGIC {
        return Err(CatalogError::Corrupt);
    }
    let expected = get_u32(bytes, COMMIT_CHECKSUM_OFFSET);
    let mut copy = block;
    put_u32(copy.as_bytes_mut(), COMMIT_CHECKSUM_OFFSET, 0);
    if crc32c(&copy.as_bytes()[..COMMIT_CHECKSUM_OFFSET]) != expected {
        return Err(CatalogError::Corrupt);
    }
    Ok(Some(CommitRecord {
        generation: get_u64(bytes, 8),
        metadata_start: BlockIndex::new(get_u64(bytes, 16)),
        metadata_blocks: get_u32(bytes, 24),
        metadata_bytes: get_u32(bytes, 28),
        catalog_start: BlockIndex::new(get_u64(bytes, 32)),
        catalog_blocks: get_u32(bytes, 40),
        catalog_bytes: get_u32(bytes, 44),
        bitmap_slot: bytes[48],
    }))
}

fn bitmap_position(index: u64) -> (usize, usize, u8) {
    (
        (index / (crate::BLOCK_BYTES as u64 * 8)) as usize,
        ((index / 8) % crate::BLOCK_BYTES as u64) as usize,
        (index % 8) as u8,
    )
}

fn bitmap_is_set(bitmap: &[Block; crate::COW_MAX_BITMAP_BLOCKS], index: u64) -> bool {
    let (block, byte, bit) = bitmap_position(index);
    bitmap[block].as_bytes()[byte] & (1u8 << bit) != 0
}

fn bitmap_set(bitmap: &mut [Block; crate::COW_MAX_BITMAP_BLOCKS], index: u64, value: bool) {
    let (block, byte, bit) = bitmap_position(index);
    let mask = 1u8 << bit;
    if value {
        bitmap[block].as_bytes_mut()[byte] |= mask;
    } else {
        bitmap[block].as_bytes_mut()[byte] &= !mask;
    }
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
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn get_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}

fn crc32c(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for byte in bytes {
        crc ^= *byte as u32;
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

    struct MemoryStore<const BLOCKS: usize> {
        blocks: [Block; BLOCKS],
        fail_write: Option<usize>,
    }

    impl<const BLOCKS: usize> MemoryStore<BLOCKS> {
        fn new() -> Self {
            Self { blocks: [Block::zero(); BLOCKS], fail_write: None }
        }
    }

    impl<const BLOCKS: usize> BlockStore for MemoryStore<BLOCKS> {
        fn block_count(&self) -> u64 {
            BLOCKS as u64
        }

        fn read_block(&mut self, index: BlockIndex, output: &mut Block) -> Result<(), BlockError> {
            *output = *self.blocks.get(index.get() as usize).ok_or(BlockError::OutOfBounds)?;
            Ok(())
        }

        fn read_block_uncached(
            &mut self,
            index: BlockIndex,
            output: &mut Block,
        ) -> Result<(), BlockError> {
            self.read_block(index, output)
        }

        fn write_block(&mut self, index: BlockIndex, input: &Block) -> Result<(), BlockError> {
            if self.fail_write == Some(0) {
                return Err(BlockError::Io);
            }
            if let Some(remaining) = self.fail_write.as_mut() {
                *remaining -= 1;
            }
            *self.blocks.get_mut(index.get() as usize).ok_or(BlockError::OutOfBounds)? = *input;
            Ok(())
        }

        fn flush(&mut self) -> Result<(), BlockError> {
            Ok(())
        }
    }

    #[test]
    fn format_persists_pool_boundaries_and_catalog_bytes() {
        let mut store = MemoryStore::<128>::new();
        let mut volume = SystemCatalogVolume::format(&mut store, 8, 32).unwrap();
        assert_eq!(volume.system_arena(), (4, 12));
        assert_eq!(volume.user_arena(), (12, 32));
        assert_eq!(volume.package_arena(), (32, 126));
        assert_eq!(volume.read_catalog(&mut store, &mut [0; 8]), Err(CatalogError::Unformatted));

        let snapshot = b"LOGOS user catalog";
        assert_eq!(volume.replace_catalog(&mut store, snapshot), Ok(2));
        let mut output = [0; 32];
        assert_eq!(volume.read_catalog(&mut store, &mut output), Ok(snapshot.len()));
        assert_eq!(&output[..snapshot.len()], snapshot);

        let metadata = b"namespace metadata";
        assert_eq!(volume.replace_metadata(&mut store, metadata), Ok(3));
        assert_eq!(volume.read_metadata(&mut store, &mut output), Ok(metadata.len()));
        assert_eq!(&output[..metadata.len()], metadata);

        let reopened = SystemCatalogVolume::open(&mut store).unwrap();
        assert_eq!(reopened.root().generation, 3);
        assert_eq!(reopened.root().catalog_bytes as usize, snapshot.len());
        assert_eq!(reopened.root().metadata_bytes as usize, metadata.len());
    }

    #[test]
    fn v4_media_is_rejected_by_the_v5_catalog_opener() {
        let mut store = MemoryStore::<128>::new();
        let mut block = Block::zero();
        block.as_bytes_mut()[..8].copy_from_slice(COW_SUPERBLOCK_MAGIC);
        block.as_bytes_mut()[8..10].copy_from_slice(&4u16.to_le_bytes());
        store.write_block(BlockIndex::new(0), &block).unwrap();
        assert!(matches!(
            SystemCatalogVolume::open(&mut store),
            Err(CatalogError::UnsupportedVersion)
        ));
    }

    #[test]
    fn torn_superblock_publication_recovers_the_committed_catalog() {
        let mut store = MemoryStore::<128>::new();
        let mut volume = SystemCatalogVolume::format(&mut store, 8, 32).unwrap();
        volume.replace_catalog(&mut store, b"first").unwrap();
        store.fail_write = Some(3);
        assert_eq!(
            volume.replace_catalog(&mut store, b"second"),
            Err(CatalogError::Block(BlockError::Io))
        );
        store.fail_write = None;

        let reopened = SystemCatalogVolume::open(&mut store).unwrap();
        let mut output = [0; 16];
        assert_eq!(reopened.read_catalog(&mut store, &mut output), Ok(6));
        assert_eq!(&output[..6], b"second");
    }

    #[test]
    fn corrupt_stale_commit_record_does_not_hide_a_valid_root() {
        let mut store = MemoryStore::<128>::new();
        let mut volume = SystemCatalogVolume::format(&mut store, 8, 32).unwrap();
        volume.replace_catalog(&mut store, b"catalog").unwrap();
        store.blocks[COMMIT_START as usize].as_bytes_mut()[0] ^= 0xa5;

        let reopened = SystemCatalogVolume::open(&mut store).unwrap();
        assert_eq!(reopened.root().generation, volume.root().generation);
    }

    #[test]
    fn system_pool_exhaustion_does_not_use_user_or_package_blocks() {
        let mut store = MemoryStore::<128>::new();
        let mut volume = SystemCatalogVolume::format(&mut store, 2, 12).unwrap();
        let first = [0x5a; crate::BLOCK_BYTES + 1];
        volume.replace_catalog(&mut store, &first).unwrap();
        assert_eq!(volume.replace_catalog(&mut store, b"two"), Err(CatalogError::OutOfSpace));
        assert!(volume.root().catalog_start.get() < volume.root().system_end.get());
        assert!(volume.root().system_end.get() <= volume.root().user_start.get());
        assert!(volume.root().user_end.get() <= volume.root().package_start.get());
    }
}
