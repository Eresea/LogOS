use uefi::mem::memory_map::{MemoryDescriptor, MemoryMap, MemoryType};

const PAGE_SIZE: u64 = 4096;
const RANGES: usize = 8;
const RECYCLED: usize = 8;

pub struct Page(u64);

impl Page {
    pub const fn address(&self) -> u64 {
        self.0
    }
}

pub struct PhysicalMemory {
    ranges: [Option<Range>; RANGES],
    recycled: [Option<Page>; RECYCLED],
    current: usize,
}

#[derive(Clone, Copy)]
struct Range {
    start: u64,
    next: u64,
    end: u64,
}

impl PhysicalMemory {
    pub fn from_memory_map(memory_map: &impl MemoryMap) -> Option<Self> {
        let mut memory =
            Self { ranges: [None; RANGES], recycled: [const { None }; RECYCLED], current: 0 };
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

    pub fn allocate_owned(&mut self) -> Option<Page> {
        self.recycled.iter_mut().find_map(Option::take).or_else(|| self.allocate_page().map(Page))
    }

    pub fn release_page(&mut self, page: Page) -> bool {
        if !self.ranges.iter().flatten().any(|range| page.0 >= range.start && page.0 < range.end) {
            return false;
        }
        let Some(slot) = self.recycled.iter_mut().find(|slot| slot.is_none()) else {
            return false;
        };
        *slot = Some(page);
        true
    }

    fn from_descriptor(entry: &MemoryDescriptor) -> Option<Range> {
        let size = entry.page_count.checked_mul(PAGE_SIZE)?;
        Some(Range {
            start: entry.phys_start,
            next: entry.phys_start,
            end: entry.phys_start.checked_add(size)?,
        })
    }
}

pub fn self_check() -> bool {
    let mut memory = PhysicalMemory {
        ranges: [
            Some(Range { start: 0x1000, next: 0x1000, end: 0x2000 }),
            Some(Range { start: 0x3000, next: 0x3000, end: 0x4000 }),
            None,
            None,
            None,
            None,
            None,
            None,
        ],
        recycled: [const { None }; RECYCLED],
        current: 0,
    };
    let owned = memory.allocate_owned();
    owned.is_some_and(|page| page.address() == 0x1000 && memory.release_page(page))
        && memory.allocate_owned().is_some_and(|page| page.address() == 0x1000)
        && memory.allocate_page() == Some(0x3000)
        && memory.allocate_page().is_none()
}
