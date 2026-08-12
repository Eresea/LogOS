//! Bounded four-level x86-64 page-table construction.

use crate::{
    frame_pool::{FrameAddress, FramePool, FramePoolError},
    loader::{LoadedImage, LoadedPage, MAX_LOAD_PAGES, PAGE_SIZE},
};

#[cfg(any(test, target_os = "uefi"))]
const ENTRY_COUNT: usize = 512;
const PRESENT: u64 = 1;
const WRITABLE: u64 = 1 << 1;
const USER: u64 = 1 << 2;
const HUGE: u64 = 1 << 7;
const NO_EXECUTE: u64 = 1 << 63;
const ADDRESS_MASK: u64 = 0x000f_ffff_ffff_f000;
#[cfg(target_os = "uefi")]
const USER_PML4_INDEX: usize = 2;

/// Maximum table frames needed by one root plus one private path per loaded page.
pub const MAX_PAGE_TABLE_FRAMES: usize = 1 + MAX_LOAD_PAGES * 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PageTableError {
    Capacity,
    Exhausted,
    Memory,
    InvalidMapping,
    InvalidVirtualAddress,
    InvalidFrame,
    InvalidFlags,
    Conflict,
}

/// Architecture-owned storage access for page-table frames.
pub trait PageTableMemory {
    fn clear(&mut self, frame: FrameAddress) -> Result<(), PageTableError>;
    fn read(&self, frame: FrameAddress, index: usize) -> Result<u64, PageTableError>;
    fn write(
        &mut self,
        frame: FrameAddress,
        index: usize,
        value: u64,
    ) -> Result<(), PageTableError>;

    fn seed_kernel_root(&mut self, _root: FrameAddress) -> Result<(), PageTableError> {
        Ok(())
    }
}

pub struct PageTableBuilder {
    frames: [Option<FrameAddress>; MAX_PAGE_TABLE_FRAMES],
    count: usize,
    root: FrameAddress,
    mapped_pages: usize,
}

impl PageTableBuilder {
    pub fn new<M: PageTableMemory>(
        pool: &mut FramePool,
        memory: &mut M,
    ) -> Result<Self, PageTableError> {
        let root = pool.allocate().map_err(map_pool_error)?;
        if memory.clear(root).is_err() {
            let _ = pool.release(root);
            return Err(PageTableError::Memory);
        }
        let mut builder =
            Self { frames: [None; MAX_PAGE_TABLE_FRAMES], count: 1, root, mapped_pages: 0 };
        builder.frames[0] = Some(root);
        if memory.seed_kernel_root(root).is_err() {
            builder.reclaim(pool, memory);
            return Err(PageTableError::Memory);
        }
        Ok(builder)
    }

    pub const fn root(&self) -> FrameAddress {
        self.root
    }

    pub const fn table_count(&self) -> usize {
        self.count
    }

    pub const fn mapped_pages(&self) -> usize {
        self.mapped_pages
    }

    pub fn map_image<M: PageTableMemory>(
        &mut self,
        image: &LoadedImage,
        pool: &mut FramePool,
        memory: &mut M,
    ) -> Result<(), PageTableError> {
        for index in 0..image.page_count() {
            let Some(page) = image.page(index) else {
                return Err(PageTableError::InvalidMapping);
            };
            self.map_page(page, pool, memory)?;
        }
        Ok(())
    }

    pub fn map_page<M: PageTableMemory>(
        &mut self,
        page: LoadedPage,
        pool: &mut FramePool,
        memory: &mut M,
    ) -> Result<(), PageTableError> {
        validate_page(page)?;
        let indices = indices(page.virtual_address());
        let mut table = self.root;
        for index in indices.iter().take(3) {
            let entry = memory.read(table, *index)?;
            if entry & PRESENT != 0 {
                if entry & HUGE != 0 || entry & ADDRESS_MASK == 0 {
                    return Err(PageTableError::Conflict);
                }
                table = FrameAddress::from_raw(entry & ADDRESS_MASK);
                if !self.contains(table) {
                    return Err(PageTableError::Memory);
                }
                continue;
            }
            let child = self.allocate_table(pool, memory)?;
            let child_entry = child.raw() | PRESENT | WRITABLE | USER;
            memory.write(table, *index, child_entry)?;
            table = child;
        }
        let leaf_index = indices[3];
        if memory.read(table, leaf_index)? & PRESENT != 0 {
            return Err(PageTableError::Conflict);
        }
        let mut leaf = page.frame().raw() | PRESENT | USER;
        if page.flags().writable {
            leaf |= WRITABLE;
        }
        if !page.flags().executable {
            leaf |= NO_EXECUTE;
        }
        memory.write(table, leaf_index, leaf)?;
        self.mapped_pages += 1;
        Ok(())
    }

    pub fn reclaim<M: PageTableMemory>(&mut self, pool: &mut FramePool, _memory: &mut M) {
        for frame in self.frames[..self.count].iter().flatten() {
            let _ = pool.release(*frame);
        }
        self.frames.fill(None);
        self.count = 0;
        self.mapped_pages = 0;
    }

    fn allocate_table<M: PageTableMemory>(
        &mut self,
        pool: &mut FramePool,
        memory: &mut M,
    ) -> Result<FrameAddress, PageTableError> {
        if self.count == MAX_PAGE_TABLE_FRAMES {
            return Err(PageTableError::Capacity);
        }
        let frame = pool.allocate().map_err(map_pool_error)?;
        if memory.clear(frame).is_err() {
            let _ = pool.release(frame);
            return Err(PageTableError::Memory);
        }
        self.frames[self.count] = Some(frame);
        self.count += 1;
        Ok(frame)
    }

    fn contains(&self, frame: FrameAddress) -> bool {
        self.frames[..self.count].contains(&Some(frame))
    }
}

fn validate_page(page: LoadedPage) -> Result<(), PageTableError> {
    let virtual_address = page.virtual_address();
    if virtual_address == 0
        || virtual_address & (PAGE_SIZE - 1) != 0
        || virtual_address >= 0x0000_8000_0000_0000
    {
        return Err(PageTableError::InvalidVirtualAddress);
    }
    if page.frame().raw() == 0
        || page.frame().raw() & (PAGE_SIZE as u64 - 1) != 0
        || page.frame().raw() & !ADDRESS_MASK != 0
    {
        return Err(PageTableError::InvalidFrame);
    }
    if !page.flags().user || page.flags().writable && page.flags().executable {
        return Err(PageTableError::InvalidFlags);
    }
    Ok(())
}

fn indices(virtual_address: usize) -> [usize; 4] {
    [
        (virtual_address >> 39) & 0x1ff,
        (virtual_address >> 30) & 0x1ff,
        (virtual_address >> 21) & 0x1ff,
        (virtual_address >> 12) & 0x1ff,
    ]
}

fn map_pool_error(error: FramePoolError) -> PageTableError {
    match error {
        FramePoolError::Exhausted => PageTableError::Exhausted,
        FramePoolError::InvalidMap => PageTableError::Memory,
    }
}

#[cfg(target_os = "uefi")]
/// Identity-mapped page-table and image memory access used after UEFI exits.
///
/// UEFI leaves the kernel's current identity mappings active while this
/// adapter constructs the first service roots. Every frame passed to it comes
/// from the retained conventional-memory map and must remain reserved.
pub struct IdentityPageTableMemory;

#[cfg(target_os = "uefi")]
impl IdentityPageTableMemory {
    const fn table(frame: FrameAddress) -> *mut [u64; ENTRY_COUNT] {
        frame.raw() as usize as *mut [u64; ENTRY_COUNT]
    }

    fn valid_index(index: usize) -> Result<(), PageTableError> {
        (index < ENTRY_COUNT).then_some(()).ok_or(PageTableError::Memory)
    }
}

#[cfg(target_os = "uefi")]
impl PageTableMemory for IdentityPageTableMemory {
    fn clear(&mut self, frame: FrameAddress) -> Result<(), PageTableError> {
        if frame.raw() == 0 || frame.raw() & (PAGE_SIZE as u64 - 1) != 0 {
            return Err(PageTableError::Memory);
        }
        // SAFETY: The frame is reserved by FramePool and identity-mapped by
        // the active kernel page tables during this bootstrap phase.
        unsafe { core::ptr::write_bytes(Self::table(frame).cast::<u8>(), 0, PAGE_SIZE) };
        Ok(())
    }

    fn read(&self, frame: FrameAddress, index: usize) -> Result<u64, PageTableError> {
        Self::valid_index(index)?;
        // SAFETY: See `clear`; the table frame remains reserved and mapped.
        Ok(unsafe { (*Self::table(frame))[index] })
    }

    fn write(
        &mut self,
        frame: FrameAddress,
        index: usize,
        value: u64,
    ) -> Result<(), PageTableError> {
        Self::valid_index(index)?;
        // SAFETY: See `clear`; the table frame remains reserved and mapped.
        unsafe { (*Self::table(frame))[index] = value };
        Ok(())
    }

    fn seed_kernel_root(&mut self, root: FrameAddress) -> Result<(), PageTableError> {
        let source = crate::arch::current_cr3() as u64 & ADDRESS_MASK;
        if source == 0 || source == root.raw() {
            return Err(PageTableError::Memory);
        }
        // SAFETY: `source` is the active CR3 root; `root` is a newly allocated
        // reserved frame. Both are identity-mapped during bootstrap.
        unsafe {
            core::ptr::copy_nonoverlapping(
                source as *const u64,
                Self::table(root).cast::<u64>(),
                ENTRY_COUNT,
            );
            // The fixed service image/stack window owns PML4 slot 1. Any
            // firmware branch there is discarded before user mappings grow.
            (*Self::table(root))[USER_PML4_INDEX] = 0;
        };
        Ok(())
    }
}

#[cfg(target_os = "uefi")]
impl crate::loader::PageSink for IdentityPageTableMemory {
    fn clear(&mut self, frame: FrameAddress) -> Result<(), crate::loader::LoadError> {
        <Self as PageTableMemory>::clear(self, frame).map_err(|_| crate::loader::LoadError::Write)
    }

    fn write(
        &mut self,
        frame: FrameAddress,
        offset: usize,
        bytes: &[u8],
    ) -> Result<(), crate::loader::LoadError> {
        if frame.raw() == 0
            || frame.raw() & (PAGE_SIZE as u64 - 1) != 0
            || offset >= PAGE_SIZE
            || bytes.len() > PAGE_SIZE - offset
        {
            return Err(crate::loader::LoadError::Write);
        }
        // SAFETY: The frame is reserved and identity-mapped during bootstrap;
        // the loader supplies only page-local ranges.
        unsafe {
            core::ptr::copy_nonoverlapping(
                bytes.as_ptr(),
                (frame.raw() as usize + offset) as *mut u8,
                bytes.len(),
            )
        };
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        boot_resources::{MemoryDescriptor, MemoryMap},
        loader::LoadedImage,
        process::ElfLoadPlan,
    };
    use std::{boxed::Box, vec::Vec};

    struct TestMemory {
        frames: Vec<FrameAddress>,
        tables: Vec<Box<[u64; ENTRY_COUNT]>>,
    }

    impl TestMemory {
        fn new() -> Self {
            Self { frames: Vec::new(), tables: Vec::new() }
        }

        fn slot(&self, frame: FrameAddress) -> Result<usize, PageTableError> {
            self.frames.iter().position(|entry| *entry == frame).ok_or(PageTableError::Memory)
        }
    }

    impl PageTableMemory for TestMemory {
        fn clear(&mut self, frame: FrameAddress) -> Result<(), PageTableError> {
            if let Some(index) = self.frames.iter().position(|entry| *entry == frame) {
                self.tables[index].fill(0);
            } else {
                self.frames.push(frame);
                self.tables.push(Box::new([0; ENTRY_COUNT]));
            }
            Ok(())
        }

        fn read(&self, frame: FrameAddress, index: usize) -> Result<u64, PageTableError> {
            if index >= ENTRY_COUNT {
                return Err(PageTableError::Memory);
            }
            Ok(self.tables[self.slot(frame)?][index])
        }

        fn write(
            &mut self,
            frame: FrameAddress,
            index: usize,
            value: u64,
        ) -> Result<(), PageTableError> {
            if index >= ENTRY_COUNT {
                return Err(PageTableError::Memory);
            }
            let slot = self.slot(frame)?;
            self.tables[slot][index] = value;
            Ok(())
        }
    }

    fn image() -> [u8; 128] {
        let mut image = [0; 128];
        image[..4].copy_from_slice(b"\x7fELF");
        image[4] = 2;
        image[5] = 1;
        image[16..18].copy_from_slice(&2u16.to_le_bytes());
        image[18..20].copy_from_slice(&0x3eu16.to_le_bytes());
        image[24..32].copy_from_slice(&0x1000u64.to_le_bytes());
        image[32..40].copy_from_slice(&64u64.to_le_bytes());
        image[54..56].copy_from_slice(&56u16.to_le_bytes());
        image[56..58].copy_from_slice(&1u16.to_le_bytes());
        image[64..68].copy_from_slice(&1u32.to_le_bytes());
        image[68..72].copy_from_slice(&5u32.to_le_bytes());
        image[80..88].copy_from_slice(&0x1000u64.to_le_bytes());
        image[96..104].copy_from_slice(&1u64.to_le_bytes());
        image[104..112].copy_from_slice(&0x1000u64.to_le_bytes());
        image[112..120].copy_from_slice(&0x1000u64.to_le_bytes());
        image[120] = 0xc3;
        image
    }

    #[test]
    fn builder_maps_loaded_pages_and_reclaims_tables() {
        let image = image();
        let plan = ElfLoadPlan::parse(&image).unwrap();
        let mut map = MemoryMap::new();
        map.push(MemoryDescriptor::new(0x1000, 64, true).unwrap()).unwrap();
        let mut pool = FramePool::empty();
        pool.initialize(&map).unwrap();
        let loaded = LoadedImage::load(plan, &mut pool).unwrap();
        let mut memory = TestMemory::new();
        let mut tables = PageTableBuilder::new(&mut pool, &mut memory).unwrap();

        tables.map_image(&loaded, &mut pool, &mut memory).unwrap();

        assert_eq!(tables.mapped_pages(), loaded.page_count());
        assert!(tables.table_count() > 1);
        let root = tables.root();
        assert_ne!(memory.read(root, 0).unwrap(), 0);
        tables.reclaim(&mut pool, &mut memory);
        assert_eq!(tables.table_count(), 0);
        assert!(pool.allocate().is_ok());
    }

    #[test]
    fn duplicate_virtual_pages_are_rejected() {
        let image = image();
        let plan = ElfLoadPlan::parse(&image).unwrap();
        let mut map = MemoryMap::new();
        map.push(MemoryDescriptor::new(0x1000, 32, true).unwrap()).unwrap();
        let mut pool = FramePool::empty();
        pool.initialize(&map).unwrap();
        let loaded = LoadedImage::load(plan, &mut pool).unwrap();
        let mut memory = TestMemory::new();
        let mut tables = PageTableBuilder::new(&mut pool, &mut memory).unwrap();
        let page = loaded.page(0).unwrap();
        tables.map_page(page, &mut pool, &mut memory).unwrap();
        assert_eq!(tables.map_page(page, &mut pool, &mut memory), Err(PageTableError::Conflict));
    }
}
