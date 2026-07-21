use uefi::mem::memory_map::{MemoryDescriptor, MemoryMap, MemoryType};

const PAGE_SIZE: u64 = 4096;

pub struct PhysicalMemory {
    next: u64,
    end: u64,
}

impl PhysicalMemory {
    pub fn from_memory_map(memory_map: &impl MemoryMap) -> Option<Self> {
        memory_map
            .entries()
            .filter(|entry| entry.ty == MemoryType::CONVENTIONAL)
            .filter_map(Self::from_descriptor)
            // ponytail: one contiguous range; add a free-list when allocations must span regions.
            .max_by_key(|memory| memory.end - memory.next)
    }

    pub fn allocate_page(&mut self) -> Option<u64> {
        let page = self.next;
        self.next = self.next.checked_add(PAGE_SIZE)?;
        (self.next <= self.end).then_some(page)
    }

    fn from_descriptor(entry: &MemoryDescriptor) -> Option<Self> {
        let size = entry.page_count.checked_mul(PAGE_SIZE)?;
        Some(Self { next: entry.phys_start, end: entry.phys_start.checked_add(size)? })
    }
}
