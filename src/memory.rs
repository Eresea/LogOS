use uefi::mem::memory_map::{MemoryDescriptor, MemoryMap, MemoryType};

const PAGE_SIZE: u64 = 4096;
const RANGES: usize = 8;

pub struct PhysicalMemory {
    ranges: [Option<Range>; RANGES],
    current: usize,
}

#[derive(Clone, Copy)]
struct Range {
    next: u64,
    end: u64,
}

impl PhysicalMemory {
    pub fn from_memory_map(memory_map: &impl MemoryMap) -> Option<Self> {
        let mut memory = Self { ranges: [None; RANGES], current: 0 };
        for range in memory_map
            .entries()
            .filter(|entry| entry.ty == MemoryType::CONVENTIONAL)
            .filter_map(Self::from_descriptor)
        {
            if let Some(slot) = memory.ranges.iter_mut().find(|slot| slot.is_none()) {
                // ponytail: retain eight ranges; add dynamic metadata when firmware exposes more.
                *slot = Some(range);
            }
        }
        memory.ranges[0].is_some().then_some(memory)
    }

    pub fn allocate_page(&mut self) -> Option<u64> {
        while self.current < RANGES {
            let Some(range) = &mut self.ranges[self.current] else {
                self.current += 1;
                continue;
            };
            let page = range.next;
            let Some(next) = range.next.checked_add(PAGE_SIZE) else {
                self.current += 1;
                continue;
            };
            if next <= range.end {
                range.next = next;
                return Some(page);
            }
            self.current += 1;
        }
        None
    }

    fn from_descriptor(entry: &MemoryDescriptor) -> Option<Range> {
        let size = entry.page_count.checked_mul(PAGE_SIZE)?;
        Some(Range { next: entry.phys_start, end: entry.phys_start.checked_add(size)? })
    }
}

pub fn self_check() -> bool {
    let mut memory = PhysicalMemory {
        ranges: [
            Some(Range { next: 0x1000, end: 0x2000 }),
            Some(Range { next: 0x3000, end: 0x4000 }),
            None,
            None,
            None,
            None,
            None,
            None,
        ],
        current: 0,
    };
    memory.allocate_page() == Some(0x1000)
        && memory.allocate_page() == Some(0x3000)
        && memory.allocate_page().is_none()
}
