use uefi::mem::memory_map::{MemoryDescriptor, MemoryMap, MemoryType};

const PAGE_SIZE: u64 = 4096;
const RANGES: usize = 8;

pub struct Page(u64);

impl Page {
    pub const fn address(&self) -> u64 {
        self.0
    }
}

pub struct Contiguous {
    start: u64,
    pages: usize,
}

impl Contiguous {
    pub const fn address(&self) -> u64 {
        self.start
    }

    pub const fn pages(&self) -> usize {
        self.pages
    }

    pub fn release(self, memory: &mut PhysicalMemory) -> bool {
        (0..self.pages).all(|index| {
            u64::try_from(index)
                .ok()
                .and_then(|index| index.checked_mul(PAGE_SIZE))
                .and_then(|offset| self.start.checked_add(offset))
                .is_some_and(|address| memory.release_page(Page(address)))
        })
    }
}

pub struct PhysicalMemory {
    ranges: [Option<Range>; RANGES],
    free_head: u64,
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
        let mut memory = Self { ranges: [None; RANGES], free_head: 0, current: 0 };
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
        if self.free_head != 0 {
            let page = self.free_head;
            self.free_head = unsafe { (page as *const u64).read_volatile() };
            Some(Page(page))
        } else {
            self.allocate_page().map(Page)
        }
    }

    pub fn allocate_contiguous(&mut self, pages: usize) -> Option<Contiguous> {
        let bytes = u64::try_from(pages).ok()?.checked_mul(PAGE_SIZE)?;
        if pages == 0 {
            return None;
        }
        while self.current < RANGES {
            let Some(range) = &mut self.ranges[self.current] else {
                self.current += 1;
                continue;
            };
            let end = range.next.checked_add(bytes)?;
            if end <= range.end {
                let start = range.next;
                range.next = end;
                return Some(Contiguous { start, pages });
            }
            self.current += 1;
        }
        None
    }

    pub fn release_page(&mut self, page: Page) -> bool {
        if !self.ranges.iter().flatten().any(|range| page.0 >= range.start && page.0 < range.end) {
            return false;
        }
        // ponytail: free pages store the next link; add metadata when non-identity mappings arrive.
        unsafe { (page.0 as *mut u64).write_volatile(self.free_head) };
        self.free_head = page.0;
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
            Some(Range { start: 0x1000, next: 0x1000, end: 0x4000 }),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        ],
        free_head: 0,
        current: 0,
    };
    let contiguous = memory.allocate_contiguous(2);
    contiguous.as_ref().is_some_and(|pages| pages.address() == 0x1000 && pages.pages() == 2)
        && contiguous.is_some_and(|pages| pages.release(&mut memory))
        && memory.allocate_owned().is_some_and(|page| page.address() == 0x2000)
        && memory.allocate_page() == Some(0x3000)
        && memory.allocate_page().is_none()
}
