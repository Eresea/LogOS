//! Bounded ELF page admission backed by the fixed frame pool.

use crate::frame_pool::{FrameAddress, FramePool, FramePoolError};
use crate::process::{ElfLoadPlan, MappingFlags};

pub const PAGE_SIZE: usize = 4096;
pub const MAX_LOAD_PAGES: usize =
    crate::process::MAX_IMAGE_BYTES / PAGE_SIZE + crate::process::USER_STACK_PAGES + 2;
pub const USER_STACK_BASE: usize = 0x0000_7000_0000_0000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LoadedPage {
    virtual_address: usize,
    frame: FrameAddress,
    flags: MappingFlags,
}

impl LoadedPage {
    pub const fn virtual_address(self) -> usize {
        self.virtual_address
    }

    pub const fn frame(self) -> FrameAddress {
        self.frame
    }

    pub const fn flags(self) -> MappingFlags {
        self.flags
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LoadError {
    InvalidPlan,
    Capacity,
    Exhausted,
    Overlap,
}

#[derive(Debug)]
pub struct LoadedImage {
    entry: usize,
    stack_top: usize,
    pages: [Option<LoadedPage>; MAX_LOAD_PAGES],
    count: usize,
}

impl LoadedImage {
    pub fn load(plan: ElfLoadPlan, pool: &mut FramePool) -> Result<Self, LoadError> {
        let mut image = Self {
            entry: plan.entry(),
            stack_top: USER_STACK_BASE + crate::process::USER_STACK_PAGES * PAGE_SIZE,
            pages: [None; MAX_LOAD_PAGES],
            count: 0,
        };

        for index in 0..plan.segment_count() {
            let Some(segment) = plan.segment(index) else {
                image.reclaim(pool);
                return Err(LoadError::InvalidPlan);
            };
            let Some(end) = segment.virtual_address().checked_add(segment.memory_size()) else {
                image.reclaim(pool);
                return Err(LoadError::InvalidPlan);
            };
            let start = segment.virtual_address() & !(PAGE_SIZE - 1);
            let end = align_up(end).ok_or_else(|| {
                image.reclaim(pool);
                LoadError::InvalidPlan
            })?;
            for virtual_address in (start..end).step_by(PAGE_SIZE) {
                if image.pages[..image.count]
                    .iter()
                    .flatten()
                    .any(|page| page.virtual_address == virtual_address)
                {
                    image.reclaim(pool);
                    return Err(LoadError::Overlap);
                }
                image.push_page(pool, virtual_address, segment.flags())?;
            }
        }

        for offset in 0..crate::process::USER_STACK_PAGES {
            image.push_page(pool, USER_STACK_BASE + offset * PAGE_SIZE, MappingFlags::DATA)?;
        }
        Ok(image)
    }

    pub const fn entry(&self) -> usize {
        self.entry
    }

    pub const fn stack_top(&self) -> usize {
        self.stack_top
    }

    pub const fn page_count(&self) -> usize {
        self.count
    }

    pub fn page(&self, index: usize) -> Option<LoadedPage> {
        self.pages.get(index).copied().flatten()
    }

    pub fn reclaim(&mut self, pool: &mut FramePool) {
        for page in self.pages[..self.count].iter().flatten() {
            let _ = pool.release(page.frame);
        }
        self.pages.fill(None);
        self.count = 0;
    }

    fn push_page(
        &mut self,
        pool: &mut FramePool,
        virtual_address: usize,
        flags: MappingFlags,
    ) -> Result<(), LoadError> {
        if self.count == MAX_LOAD_PAGES {
            self.reclaim(pool);
            return Err(LoadError::Capacity);
        }
        let frame = pool.allocate().map_err(|error| {
            self.reclaim(pool);
            match error {
                FramePoolError::Exhausted => LoadError::Exhausted,
                FramePoolError::InvalidMap => LoadError::InvalidPlan,
            }
        })?;
        self.pages[self.count] = Some(LoadedPage { virtual_address, frame, flags });
        self.count += 1;
        Ok(())
    }
}

fn align_up(address: usize) -> Option<usize> {
    address.checked_add(PAGE_SIZE - 1).map(|value| value & !(PAGE_SIZE - 1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boot_resources::{MemoryDescriptor, MemoryMap};
    use crate::process::ProcessError;

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
    fn loader_owns_segment_and_stack_frames() {
        let plan = ElfLoadPlan::parse(&image()).unwrap();
        let mut map = MemoryMap::new();
        map.push(MemoryDescriptor::new(0x1000, 16, true).unwrap()).unwrap();
        let mut pool = FramePool::empty();
        pool.initialize(&map).unwrap();
        let mut loaded = LoadedImage::load(plan, &mut pool).unwrap();
        assert_eq!(loaded.entry(), 0x1000);
        assert_eq!(loaded.stack_top(), USER_STACK_BASE + 8 * PAGE_SIZE);
        assert_eq!(loaded.page_count(), 9);
        let first = loaded.page(0).unwrap();
        assert_eq!(first.virtual_address(), 0x1000);
        loaded.reclaim(&mut pool);
        assert_eq!(loaded.page_count(), 0);
        let reused = pool.allocate().unwrap().raw();
        assert!((0x1000..=0x1000 + 15 * PAGE_SIZE as u64).contains(&reused));
    }

    #[test]
    fn loader_reports_frame_exhaustion_without_leaking() {
        let plan = ElfLoadPlan::parse(&image()).unwrap();
        let mut map = MemoryMap::new();
        map.push(MemoryDescriptor::new(0x1000, 2, true).unwrap()).unwrap();
        let mut pool = FramePool::empty();
        pool.initialize(&map).unwrap();
        assert_eq!(LoadedImage::load(plan, &mut pool).unwrap_err(), LoadError::Exhausted);
        assert!(matches!(pool.allocate().unwrap().raw(), 0x1000 | 0x2000));
    }

    #[test]
    fn invalid_elf_plan_is_not_constructible() {
        assert_eq!(ElfLoadPlan::parse(&[0; 64]), Err(ProcessError::InvalidImage));
    }
}
