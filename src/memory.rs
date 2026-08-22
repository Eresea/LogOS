//! Bounded memory contracts shared by the physical, virtual, and kernel heaps.
//!
//! The first implementation deliberately uses fixed metadata and host-testable
//! state machines. Architecture code owns the eventual page-table writes and
//! zeroing; this module owns identity, bounds, ownership, and wakeup contracts.

use core::{
    alloc::{GlobalAlloc, Layout},
    cell::UnsafeCell,
    future::Future,
    mem::{align_of, size_of},
    ops::{Deref, DerefMut},
    pin::Pin,
    slice,
    sync::atomic::{AtomicBool, AtomicU8, AtomicU64, AtomicUsize, Ordering},
    task::{Context, Poll, Waker},
};

use crate::boot_resources::{MemoryDescriptor, MemoryMap, PAGE_SIZE};

pub const MAX_MEMORY_RUNS: usize = logos_abi::MAX_MEMORY_DESCRIPTORS;
#[cfg(test)]
pub const MAX_MANAGED_FRAMES: usize = logos_abi::MAX_MANAGED_FRAMES;
#[cfg(test)]
pub const FRAME_WORDS: usize = MAX_MANAGED_FRAMES.div_ceil(64);
#[cfg(test)]
pub const FRAME_SUMMARY_WORDS: usize = FRAME_WORDS.div_ceil(64);
#[cfg(test)]
const TEST_MAX_MANAGED_FRAMES: usize = 256;
pub const MAX_FRAME_SHARDS: usize = 4;
pub const MAX_MEMORY_CPUS: usize = 8;
pub const FRAME_CACHE_CAPACITY: usize = 16;
pub const FRAME_CACHE_REFILL: usize = 8;
pub const MAX_BATCH_FRAMES: usize = 64;
pub const MAX_WAIT_NODES: usize = 32;
pub const MAX_TLB_QUEUE_ENTRIES: usize = 64;
pub const MAX_ADDRESS_SPACES: usize = 16;
pub const MAX_MAPPINGS_PER_ADDRESS_SPACE: usize = 64;
pub const MAX_RECLAIMERS: usize = 8;
pub const MAX_MEMORY_CLAIMS: usize = 128;
pub const MAX_QUOTAS: usize = 32;

const NO_DEADLINE: u64 = u64::MAX;
const INITIAL_GENERATION: u32 = 1;
const INITIAL_FRAME_GENERATION: u16 = 1;
const FREE: u8 = 0;
const ACTIVE: u8 = 1;
const ZEROED: u8 = 3;
const WOKEN: u8 = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PhysicalRun {
    start: u64,
    pages: u64,
}

impl PhysicalRun {
    pub fn new(start: u64, pages: u64) -> Option<Self> {
        if start % PAGE_SIZE != 0 || pages == 0 {
            return None;
        }
        let bytes = pages.checked_mul(PAGE_SIZE)?;
        start.checked_add(bytes)?;
        Some(Self { start, pages })
    }

    pub const fn start(self) -> u64 {
        self.start
    }

    pub const fn pages(self) -> u64 {
        self.pages
    }

    pub fn end(self) -> Option<u64> {
        self.start.checked_add(self.pages.checked_mul(PAGE_SIZE)?)
    }

    fn from_descriptor(descriptor: MemoryDescriptor) -> Option<Self> {
        Self::new(descriptor.physical_start, descriptor.pages)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ExclusionKind {
    Reserved = 1,
    Firmware = 2,
    Kernel = 3,
    Framebuffer = 4,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MemoryExclusion {
    range: PhysicalRun,
    kind: ExclusionKind,
}

impl MemoryExclusion {
    pub fn new(start: u64, pages: u64, kind: ExclusionKind) -> Option<Self> {
        let range = PhysicalRun::new(start, pages)?;
        Some(Self { range, kind })
    }

    pub const fn range(self) -> PhysicalRun {
        self.range
    }

    pub const fn kind(self) -> ExclusionKind {
        self.kind
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NormalizationError {
    InvalidDescriptor,
    Capacity,
}

#[derive(Clone, Copy)]
pub struct NormalizedMemoryMap {
    runs: [Option<PhysicalRun>; MAX_MEMORY_RUNS],
    count: usize,
    total_pages: u64,
}

impl NormalizedMemoryMap {
    pub const fn empty() -> Self {
        Self { runs: [None; MAX_MEMORY_RUNS], count: 0, total_pages: 0 }
    }

    pub const fn len(&self) -> usize {
        self.count
    }

    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }

    pub const fn total_pages(&self) -> u64 {
        self.total_pages
    }

    pub fn get(&self, index: usize) -> Option<PhysicalRun> {
        self.runs.get(index).copied().flatten()
    }
}

/// Convert copied UEFI descriptors into sorted, disjoint conventional runs.
/// Non-available descriptors are always exclusions; explicit exclusions cover
/// kernel, framebuffer, firmware, and reserved ranges discovered later.
pub fn normalize_memory_map(
    memory_map: &MemoryMap,
    explicit_exclusions: &[MemoryExclusion],
) -> Result<NormalizedMemoryMap, NormalizationError> {
    if explicit_exclusions.len() > MAX_MEMORY_RUNS {
        return Err(NormalizationError::Capacity);
    }

    let mut available = [None; MAX_MEMORY_RUNS];
    let mut available_count = 0;
    let mut exclusions = [None; MAX_MEMORY_RUNS];
    let mut exclusion_count = 0;

    for index in 0..memory_map.len() {
        let Some(descriptor) = memory_map.get(index) else {
            return Err(NormalizationError::InvalidDescriptor);
        };
        let Some(range) = PhysicalRun::from_descriptor(descriptor) else {
            return Err(NormalizationError::InvalidDescriptor);
        };
        if descriptor.available {
            push_range(&mut available, &mut available_count, range)?;
        } else {
            push_range(&mut exclusions, &mut exclusion_count, range)?;
        }
    }
    for exclusion in explicit_exclusions {
        push_range(&mut exclusions, &mut exclusion_count, exclusion.range)?;
    }

    sort_ranges(&mut available, available_count);
    sort_ranges(&mut exclusions, exclusion_count);
    let available_count = merge_ranges(&mut available, available_count)?;
    let exclusion_count = merge_ranges(&mut exclusions, exclusion_count)?;

    let mut current = [None; MAX_MEMORY_RUNS];
    let mut current_count = available_count;
    current[..available_count].copy_from_slice(&available[..available_count]);
    let mut scratch = [None; MAX_MEMORY_RUNS];

    for exclusion in exclusions[..exclusion_count].iter().flatten().copied() {
        let mut scratch_count = 0;
        for candidate in current[..current_count].iter().flatten().copied() {
            subtract(candidate, exclusion, &mut scratch, &mut scratch_count)?;
        }
        current[..scratch_count].copy_from_slice(&scratch[..scratch_count]);
        if scratch_count < current_count {
            current[scratch_count..current_count].fill(None);
        }
        current_count = scratch_count;
        if current_count == 0 {
            break;
        }
    }

    sort_ranges(&mut current, current_count);
    let current_count = merge_ranges(&mut current, current_count)?;
    let mut normalized = NormalizedMemoryMap::empty();
    for range in current[..current_count].iter().flatten().copied() {
        if normalized.count == MAX_MEMORY_RUNS {
            return Err(NormalizationError::Capacity);
        }
        normalized.total_pages = normalized
            .total_pages
            .checked_add(range.pages)
            .ok_or(NormalizationError::InvalidDescriptor)?;
        normalized.runs[normalized.count] = Some(range);
        normalized.count += 1;
    }
    Ok(normalized)
}

fn push_range(
    ranges: &mut [Option<PhysicalRun>; MAX_MEMORY_RUNS],
    count: &mut usize,
    range: PhysicalRun,
) -> Result<(), NormalizationError> {
    let Some(slot) = ranges.get_mut(*count) else {
        return Err(NormalizationError::Capacity);
    };
    *slot = Some(range);
    *count += 1;
    Ok(())
}

fn sort_ranges(ranges: &mut [Option<PhysicalRun>; MAX_MEMORY_RUNS], count: usize) {
    for index in 1..count {
        let value = ranges[index];
        let mut position = index;
        while position > 0
            && ranges[position - 1].is_some_and(|candidate| candidate.start > value.unwrap().start)
        {
            ranges[position] = ranges[position - 1];
            position -= 1;
        }
        ranges[position] = value;
    }
}

fn merge_ranges(
    ranges: &mut [Option<PhysicalRun>; MAX_MEMORY_RUNS],
    count: usize,
) -> Result<usize, NormalizationError> {
    let mut output: usize = 0;
    for index in 0..count {
        let Some(range) = ranges[index] else {
            continue;
        };
        if output == 0 {
            ranges[0] = Some(range);
            output = 1;
            continue;
        }
        let previous = ranges[output - 1].unwrap();
        let previous_end = previous.end().ok_or(NormalizationError::InvalidDescriptor)?;
        if range.start <= previous_end {
            let end = core::cmp::max(
                previous_end,
                range.end().ok_or(NormalizationError::InvalidDescriptor)?,
            );
            ranges[output - 1] = Some(
                PhysicalRun::new(previous.start, (end - previous.start) / PAGE_SIZE)
                    .ok_or(NormalizationError::InvalidDescriptor)?,
            );
        } else {
            if output == MAX_MEMORY_RUNS {
                return Err(NormalizationError::Capacity);
            }
            ranges[output] = Some(range);
            output += 1;
        }
    }
    if output < count {
        ranges[output..count].fill(None);
    }
    Ok(output)
}

fn subtract(
    candidate: PhysicalRun,
    exclusion: PhysicalRun,
    output: &mut [Option<PhysicalRun>; MAX_MEMORY_RUNS],
    count: &mut usize,
) -> Result<(), NormalizationError> {
    let candidate_end = candidate.end().ok_or(NormalizationError::InvalidDescriptor)?;
    let exclusion_end = exclusion.end().ok_or(NormalizationError::InvalidDescriptor)?;
    if exclusion_end <= candidate.start || exclusion.start >= candidate_end {
        return push_range(output, count, candidate);
    }
    if exclusion.start > candidate.start {
        push_range(
            output,
            count,
            PhysicalRun::new(candidate.start, (exclusion.start - candidate.start) / PAGE_SIZE)
                .ok_or(NormalizationError::InvalidDescriptor)?,
        )?;
    }
    if exclusion_end < candidate_end {
        push_range(
            output,
            count,
            PhysicalRun::new(exclusion_end, (candidate_end - exclusion_end) / PAGE_SIZE)
                .ok_or(NormalizationError::InvalidDescriptor)?,
        )?;
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameId {
    run: u16,
    offset: u32,
}

impl FrameId {
    pub const fn new(run: usize, offset: usize) -> Option<Self> {
        if run > u16::MAX as usize || offset > u32::MAX as usize {
            return None;
        }
        Some(Self { run: run as u16, offset: offset as u32 })
    }

    pub const fn run(self) -> usize {
        self.run as usize
    }

    pub const fn offset(self) -> usize {
        self.offset as usize
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OwnerId(u16);

impl OwnerId {
    pub const KERNEL: Self = Self(1);

    pub const fn service(service: logos_abi::ServiceId) -> Self {
        Self(2 + service.index() as u16)
    }

    pub const fn new(raw: u16) -> Option<Self> {
        if raw == 0 { None } else { Some(Self(raw)) }
    }

    pub const fn raw(self) -> u16 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FrameState {
    Zeroed,
    Dirty,
    Reserved,
}

#[derive(Clone, Copy, Debug)]
pub struct FrameAddress {
    raw: u64,
    id: Option<FrameId>,
    generation: u16,
}

impl FrameAddress {
    pub(crate) const fn from_raw(raw: u64) -> Self {
        Self { raw, id: None, generation: 0 }
    }

    const fn from_parts(raw: u64, id: FrameId, generation: u16) -> Self {
        Self { raw, id: Some(id), generation }
    }

    pub const fn raw(self) -> u64 {
        self.raw
    }

    pub const fn id(self) -> Option<FrameId> {
        self.id
    }
}

impl PartialEq for FrameAddress {
    fn eq(&self, other: &Self) -> bool {
        self.raw == other.raw
    }
}

impl Eq for FrameAddress {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameLease {
    address: FrameAddress,
    id: FrameId,
    generation: u16,
    owner: OwnerId,
    state: FrameState,
    home_cpu: u8,
    slot: u32,
}

impl FrameLease {
    pub const fn address(self) -> FrameAddress {
        self.address
    }

    pub const fn id(self) -> FrameId {
        self.id
    }

    pub const fn generation(self) -> u16 {
        self.generation
    }

    pub const fn owner(self) -> OwnerId {
        self.owner
    }

    pub const fn state(self) -> FrameState {
        self.state
    }

    pub const fn home_cpu(self) -> usize {
        self.home_cpu as usize
    }

    const fn with_home_cpu(self, cpu: usize) -> Self {
        Self { home_cpu: cpu as u8, ..self }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FrameError {
    InvalidMap,
    Capacity,
    Exhausted,
    InvalidFrame,
    StaleHandle,
    WrongOwner,
    AlreadyUsed,
    NotReservation,
    BatchCapacity,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FrameRecord {
    generation: u16,
    owner: OwnerId,
    state: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameMetadataLayout {
    records_offset: usize,
    records_len: usize,
    free_offset: usize,
    free_len: usize,
    summary_offset: usize,
    summary_len: usize,
    heap_slots_offset: usize,
    heap_slots_len: usize,
    heap_records_offset: usize,
    heap_records_len: usize,
    heap_leases_offset: usize,
    heap_leases_len: usize,
    bytes: usize,
}

impl FrameMetadataLayout {
    pub fn for_frames(frame_count: usize) -> Option<Self> {
        let records_offset = align_up(0, align_of::<FrameRecord>())?;
        let records_bytes = frame_count.checked_mul(size_of::<FrameRecord>())?;
        let free_len = frame_count.div_ceil(64);
        let free_offset = align_up(records_offset.checked_add(records_bytes)?, align_of::<u64>())?;
        let free_bytes = free_len.checked_mul(size_of::<u64>())?;
        let summary_len = free_len.div_ceil(64);
        let summary_offset = align_up(free_offset.checked_add(free_bytes)?, align_of::<u64>())?;
        let summary_bytes = summary_len.checked_mul(size_of::<u64>())?;
        let heap_slots_len = frame_count;
        let heap_slots_offset =
            align_up(summary_offset.checked_add(summary_bytes)?, align_of::<HeapSlot>())?;
        let heap_slots_bytes = heap_slots_len.checked_mul(size_of::<HeapSlot>())?;
        let heap_records_len = frame_count;
        let heap_records_offset = align_up(
            heap_slots_offset.checked_add(heap_slots_bytes)?,
            align_of::<HeapPageRecord>(),
        )?;
        let heap_records_bytes = heap_records_len.checked_mul(size_of::<HeapPageRecord>())?;
        let heap_leases_len = frame_count;
        let heap_leases_offset = align_up(
            heap_records_offset.checked_add(heap_records_bytes)?,
            align_of::<HeapLeaseRecord>(),
        )?;
        let heap_leases_bytes = heap_leases_len.checked_mul(size_of::<HeapLeaseRecord>())?;
        let bytes = heap_leases_offset.checked_add(heap_leases_bytes)?;
        Some(Self {
            records_offset,
            records_len: frame_count,
            free_offset,
            free_len,
            summary_offset,
            summary_len,
            heap_slots_offset,
            heap_slots_len,
            heap_records_offset,
            heap_records_len,
            heap_leases_offset,
            heap_leases_len,
            bytes,
        })
    }

    pub const fn bytes(self) -> usize {
        self.bytes
    }

    pub const fn records_len(self) -> usize {
        self.records_len
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameMetadataRegion {
    base: u64,
    bytes: u64,
}

impl FrameMetadataRegion {
    pub fn new(base: u64, bytes: u64) -> Option<Self> {
        if base == 0 || base % PAGE_SIZE != 0 || bytes < PAGE_SIZE || bytes % PAGE_SIZE != 0 {
            return None;
        }
        base.checked_add(bytes).map(|_| Self { base, bytes })
    }

    pub const fn base(self) -> u64 {
        self.base
    }

    pub const fn bytes(self) -> u64 {
        self.bytes
    }

    pub fn capacity(self) -> Option<usize> {
        let bytes = usize::try_from(self.bytes).ok()?;
        let mut low = 0usize;
        let mut high = bytes / size_of::<FrameRecord>();
        while low < high {
            let middle = low + (high - low).div_ceil(2);
            if FrameMetadataLayout::for_frames(middle)?.bytes() <= bytes {
                low = middle;
            } else {
                high = middle - 1;
            }
        }
        Some(low)
    }

    pub fn layout(self, frame_count: usize) -> Option<FrameMetadataLayout> {
        let layout = FrameMetadataLayout::for_frames(frame_count)?;
        (layout.bytes() <= usize::try_from(self.bytes).ok()?).then_some(layout)
    }
}

pub fn frame_metadata_pages_for_frames(frame_count: u64) -> Option<u64> {
    let frame_count = usize::try_from(frame_count).ok()?;
    let bytes = FrameMetadataLayout::for_frames(frame_count)?.bytes() as u64;
    bytes.checked_add(PAGE_SIZE - 1).map(|bytes| bytes / PAGE_SIZE)
}

fn align_up(value: usize, alignment: usize) -> Option<usize> {
    value.checked_add(alignment - 1).map(|value| value / alignment * alignment)
}

impl FrameRecord {
    const EMPTY: Self =
        Self { generation: INITIAL_FRAME_GENERATION, owner: OwnerId(0), state: FREE };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RunMetadata {
    run: PhysicalRun,
    first_slot: usize,
    pages: usize,
}

impl RunMetadata {
    const EMPTY: Self = Self { run: PhysicalRun { start: 0, pages: 0 }, first_slot: 0, pages: 0 };
}

struct FrameMetadata {
    records: *mut FrameRecord,
    records_len: usize,
    free: *mut u64,
    free_len: usize,
    summary: *mut u64,
    summary_len: usize,
    heap_slots: *mut HeapSlot,
    heap_slots_len: usize,
    heap_records: *mut HeapPageRecord,
    heap_records_len: usize,
    heap_leases: *mut HeapLeaseRecord,
    heap_leases_len: usize,
}

unsafe impl Send for FrameMetadata {}

impl FrameMetadata {
    const EMPTY: Self = Self {
        records: core::ptr::null_mut(),
        records_len: 0,
        free: core::ptr::null_mut(),
        free_len: 0,
        summary: core::ptr::null_mut(),
        summary_len: 0,
        heap_slots: core::ptr::null_mut(),
        heap_slots_len: 0,
        heap_records: core::ptr::null_mut(),
        heap_records_len: 0,
        heap_leases: core::ptr::null_mut(),
        heap_leases_len: 0,
    };

    fn configure(
        &mut self,
        region: FrameMetadataRegion,
        frame_count: usize,
    ) -> Result<(), FrameError> {
        let layout = region.layout(frame_count).ok_or(FrameError::Capacity)?;
        let base = region.base() as *mut u8;
        // SAFETY: the region is page-aligned, sized by `layout`, and reserved
        // exclusively for allocator metadata before publication.
        unsafe {
            self.records = base.add(layout.records_offset).cast();
            self.free = base.add(layout.free_offset).cast();
            self.summary = base.add(layout.summary_offset).cast();
            self.heap_slots = base.add(layout.heap_slots_offset).cast();
            self.heap_records = base.add(layout.heap_records_offset).cast();
            self.heap_leases = base.add(layout.heap_leases_offset).cast();
        }
        self.records_len = layout.records_len;
        self.free_len = layout.free_len;
        self.summary_len = layout.summary_len;
        self.heap_slots_len = layout.heap_slots_len;
        self.heap_records_len = layout.heap_records_len;
        self.heap_leases_len = layout.heap_leases_len;
        Ok(())
    }

    fn clear(&mut self) {
        // SAFETY: all slices were established by `configure`.
        unsafe {
            slice::from_raw_parts_mut(self.records, self.records_len).fill(FrameRecord::EMPTY);
            slice::from_raw_parts_mut(self.free, self.free_len).fill(0);
            slice::from_raw_parts_mut(self.summary, self.summary_len).fill(0);
            slice::from_raw_parts_mut(self.heap_slots, self.heap_slots_len).fill(HeapSlot::empty());
            slice::from_raw_parts_mut(self.heap_records, self.heap_records_len)
                .fill(HeapPageRecord::empty());
            slice::from_raw_parts_mut(self.heap_leases, self.heap_leases_len)
                .fill(HeapLeaseRecord::empty());
        }
    }

    fn records(&self) -> &[FrameRecord] {
        // SAFETY: all slices were established by `configure`.
        unsafe { slice::from_raw_parts(self.records, self.records_len) }
    }

    fn records_mut(&mut self) -> &mut [FrameRecord] {
        // SAFETY: all slices were established by `configure`.
        unsafe { slice::from_raw_parts_mut(self.records, self.records_len) }
    }

    fn free(&self) -> &[u64] {
        // SAFETY: all slices were established by `configure`.
        unsafe { slice::from_raw_parts(self.free, self.free_len) }
    }

    fn free_mut(&mut self) -> &mut [u64] {
        // SAFETY: all slices were established by `configure`.
        unsafe { slice::from_raw_parts_mut(self.free, self.free_len) }
    }

    fn summary(&self) -> &[u64] {
        // SAFETY: all slices were established by `configure`.
        unsafe { slice::from_raw_parts(self.summary, self.summary_len) }
    }

    fn summary_mut(&mut self) -> &mut [u64] {
        // SAFETY: all slices were established by `configure`.
        unsafe { slice::from_raw_parts_mut(self.summary, self.summary_len) }
    }
}

#[cfg(test)]
const TEST_METADATA_BYTES: usize = {
    let records_bytes = TEST_MAX_MANAGED_FRAMES * size_of::<FrameRecord>();
    let free_len = TEST_MAX_MANAGED_FRAMES.div_ceil(64);
    let free_offset = records_bytes.div_ceil(8) * 8;
    let summary_offset = (free_offset + free_len * size_of::<u64>()).div_ceil(8) * 8;
    let heap_slots_offset = (summary_offset + free_len.div_ceil(64) * size_of::<u64>())
        .div_ceil(align_of::<HeapSlot>())
        * align_of::<HeapSlot>();
    let heap_records_offset = (heap_slots_offset + TEST_MAX_MANAGED_FRAMES * size_of::<HeapSlot>())
        .div_ceil(align_of::<HeapPageRecord>())
        * align_of::<HeapPageRecord>();
    let heap_leases_offset = (heap_records_offset
        + TEST_MAX_MANAGED_FRAMES * size_of::<HeapPageRecord>())
    .div_ceil(align_of::<HeapLeaseRecord>())
        * align_of::<HeapLeaseRecord>();
    let bytes = heap_leases_offset + TEST_MAX_MANAGED_FRAMES * size_of::<HeapLeaseRecord>();
    bytes.div_ceil(PAGE_SIZE as usize) * PAGE_SIZE as usize
};

#[cfg(test)]
#[repr(align(4096))]
struct TestFrameMetadata {
    bytes: [u8; TEST_METADATA_BYTES],
}

#[cfg(test)]
impl TestFrameMetadata {
    const fn empty() -> Self {
        Self { bytes: [0; TEST_METADATA_BYTES] }
    }

    fn region(&mut self) -> FrameMetadataRegion {
        FrameMetadataRegion::new(self.bytes.as_mut_ptr() as u64, self.bytes.len() as u64).unwrap()
    }
}

/// Dense per-frame state backed by hierarchical free-word metadata.
pub struct PhysicalFrameManager {
    runs: [RunMetadata; MAX_MEMORY_RUNS],
    run_count: usize,
    frame_count: usize,
    metadata: FrameMetadata,
    owner_live: [u32; MAX_QUOTAS],
    #[cfg(test)]
    test_metadata: TestFrameMetadata,
}

impl PhysicalFrameManager {
    pub const fn empty() -> Self {
        Self {
            runs: [RunMetadata::EMPTY; MAX_MEMORY_RUNS],
            run_count: 0,
            frame_count: 0,
            metadata: FrameMetadata::EMPTY,
            owner_live: [0; MAX_QUOTAS],
            #[cfg(test)]
            test_metadata: TestFrameMetadata::empty(),
        }
    }

    pub fn initialize(&mut self, map: &NormalizedMemoryMap) -> Result<(), FrameError> {
        #[cfg(test)]
        {
            let region = self.test_metadata.region();
            self.initialize_with_region(map, region)
        }
        #[cfg(not(test))]
        {
            let _ = map;
            Err(FrameError::Capacity)
        }
    }

    pub fn initialize_with_region(
        &mut self,
        map: &NormalizedMemoryMap,
        region: FrameMetadataRegion,
    ) -> Result<(), FrameError> {
        let frame_capacity =
            usize::try_from(map.total_pages()).map_err(|_| FrameError::Capacity)?;
        self.metadata.configure(region, frame_capacity)?;
        self.runs.fill(RunMetadata::EMPTY);
        self.run_count = 0;
        self.frame_count = 0;
        self.metadata.clear();
        self.owner_live.fill(0);
        for run_index in 0..map.len() {
            let Some(run) = map.get(run_index) else {
                return Err(FrameError::InvalidMap);
            };
            let mut start = run.start;
            let mut pages = usize::try_from(run.pages).map_err(|_| FrameError::Capacity)?;
            if start == 0 {
                start = PAGE_SIZE;
                pages = pages.saturating_sub(1);
            }
            if pages == 0 || self.run_count == MAX_MEMORY_RUNS {
                continue;
            }
            let bounded = PhysicalRun::new(start, pages as u64).ok_or(FrameError::InvalidMap)?;
            self.runs[self.run_count] =
                RunMetadata { run: bounded, first_slot: self.frame_count, pages };
            for slot in self.frame_count..self.frame_count + pages {
                self.metadata.records_mut()[slot] = FrameRecord::EMPTY;
                self.set_free(slot);
            }
            self.frame_count += pages;
            self.run_count += 1;
        }
        Ok(())
    }

    pub const fn frame_count(&self) -> usize {
        self.frame_count
    }

    #[cfg(target_os = "uefi")]
    fn heap_metadata(
        &mut self,
    ) -> (&mut [HeapSlot], &mut [HeapPageRecord], &mut [HeapLeaseRecord]) {
        let slots = unsafe {
            slice::from_raw_parts_mut(self.metadata.heap_slots, self.metadata.heap_slots_len)
        };
        let records = unsafe {
            slice::from_raw_parts_mut(self.metadata.heap_records, self.metadata.heap_records_len)
        };
        let leases = unsafe {
            slice::from_raw_parts_mut(self.metadata.heap_leases, self.metadata.heap_leases_len)
        };
        (slots, records, leases)
    }

    pub const fn run_count(&self) -> usize {
        self.run_count
    }

    pub fn free_count(&self) -> usize {
        self.available()
    }

    pub fn available(&self) -> usize {
        self.metadata.free()[..self.frame_count.div_ceil(64)]
            .iter()
            .map(|word| word.count_ones() as usize)
            .sum()
    }

    pub fn try_alloc(
        &mut self,
        owner: OwnerId,
        state: FrameState,
    ) -> Result<FrameLease, FrameError> {
        self.try_alloc_in_range(owner, state, 0, self.frame_count)
    }

    pub fn try_alloc_in_range(
        &mut self,
        owner: OwnerId,
        state: FrameState,
        first: usize,
        last: usize,
    ) -> Result<FrameLease, FrameError> {
        if owner.0 == 0 || first > last || last > self.frame_count {
            return Err(FrameError::InvalidFrame);
        }
        let Some(slot) = self.find_free_slot(first, last) else {
            return Err(FrameError::Exhausted);
        };
        self.claim(slot, owner, state)
    }

    pub fn alloc_batch(
        &mut self,
        owner: OwnerId,
        count: usize,
        state: FrameState,
    ) -> Result<FrameBatch, FrameError> {
        if count == 0 || count > MAX_BATCH_FRAMES {
            return Err(FrameError::BatchCapacity);
        }
        if self.available() < count {
            return Err(FrameError::Exhausted);
        }
        let mut batch = FrameBatch::empty();
        for _ in 0..count {
            let lease = match self.try_alloc(owner, state) {
                Ok(lease) => lease,
                Err(error) => {
                    let _ = self.free_batch(batch);
                    return Err(error);
                }
            };
            batch.push(lease).map_err(|_| FrameError::BatchCapacity)?;
        }
        Ok(batch)
    }

    pub fn free(&mut self, lease: FrameLease) -> Result<(), FrameError> {
        let slot = self.validate_lease(lease)?;
        self.release_slot(slot, lease.owner);
        Ok(())
    }

    pub fn validate(&self, lease: FrameLease) -> Result<(), FrameError> {
        self.validate_lease(lease).map(|_| ())
    }

    fn refresh(&mut self, lease: FrameLease) -> Result<FrameLease, FrameError> {
        let slot = self.validate_lease(lease)?;
        let generation = {
            let record = &mut self.metadata.records_mut()[slot];
            record.generation = next_frame_generation(record.generation);
            record.generation
        };
        let id = self.id_for_slot(slot).ok_or(FrameError::InvalidFrame)?;
        let address = self.address(id).ok_or(FrameError::InvalidFrame)?;
        Ok(FrameLease { address, id, generation, ..lease })
    }

    pub fn release_address(&mut self, address: FrameAddress) -> Result<(), FrameError> {
        let id = address
            .id
            .or_else(|| self.id_for_address(address.raw))
            .ok_or(FrameError::InvalidFrame)?;
        let slot = self.slot_for_id(id).ok_or(FrameError::InvalidFrame)?;
        let record = self.metadata.records()[slot];
        if record.state == FREE {
            return Err(FrameError::StaleHandle);
        }
        if address.generation != 0 && address.generation != record.generation {
            return Err(FrameError::StaleHandle);
        }
        let state = match record.state {
            ZEROED => FrameState::Zeroed,
            2 => FrameState::Reserved,
            _ => FrameState::Dirty,
        };
        let lease = FrameLease {
            address: self.address(id).ok_or(FrameError::InvalidFrame)?,
            id,
            generation: record.generation,
            owner: record.owner,
            state,
            home_cpu: 0,
            slot: slot as u32,
        };
        self.free(lease)
    }

    pub fn free_batch(&mut self, batch: FrameBatch) -> Result<(), FrameError> {
        for index in 0..batch.len {
            let Some(lease) = batch.frames[index] else {
                return Err(FrameError::InvalidFrame);
            };
            self.validate_lease(lease)?;
        }
        for index in 0..batch.len {
            self.free(batch.frames[index].unwrap())?;
        }
        Ok(())
    }

    pub fn reserve(
        &mut self,
        address: FrameAddress,
        owner: OwnerId,
    ) -> Result<FrameLease, FrameError> {
        let id = self.id_for_address(address.raw).ok_or(FrameError::InvalidFrame)?;
        let slot = self.slot_for_id(id).ok_or(FrameError::InvalidFrame)?;
        if !self.is_free(slot) {
            return Err(FrameError::AlreadyUsed);
        }
        self.clear_free(slot);
        self.claim_used(slot, owner, FrameState::Reserved)
    }

    pub fn release_reservation(&mut self, lease: FrameLease) -> Result<(), FrameError> {
        let slot = self.validate_lease(lease)?;
        if lease.state != FrameState::Reserved {
            return Err(FrameError::NotReservation);
        }
        self.release_slot(slot, lease.owner);
        Ok(())
    }

    pub fn reserve_batch(
        &mut self,
        addresses: &[FrameAddress],
        owner: OwnerId,
    ) -> Result<FrameBatch, FrameError> {
        if addresses.is_empty() || addresses.len() > MAX_BATCH_FRAMES {
            return Err(FrameError::BatchCapacity);
        }
        let mut batch = FrameBatch::empty();
        for address in addresses {
            let lease = match self.reserve(*address, owner) {
                Ok(lease) => lease,
                Err(error) => {
                    let _ = self.free_batch(batch);
                    return Err(error);
                }
            };
            batch.push(lease).map_err(|_| FrameError::BatchCapacity)?;
        }
        Ok(batch)
    }

    pub fn address(&self, id: FrameId) -> Option<FrameAddress> {
        let slot = self.slot_for_id(id)?;
        let run = self.runs.get(id.run())?;
        let offset = id.offset() as u64;
        let raw = run.run.start.checked_add(offset.checked_mul(PAGE_SIZE)?)?;
        Some(FrameAddress::from_parts(raw, id, self.metadata.records()[slot].generation))
    }

    pub fn id_for_address(&self, raw: u64) -> Option<FrameId> {
        if raw % PAGE_SIZE != 0 {
            return None;
        }
        let mut low = 0;
        let mut high = self.run_count;
        while low < high {
            let middle = (low + high) / 2;
            let run = self.runs[middle].run;
            if raw < run.start {
                high = middle;
            } else if raw >= run.end()? {
                low = middle + 1;
            } else {
                let offset = usize::try_from((raw - run.start) / PAGE_SIZE).ok()?;
                return FrameId::new(middle, offset);
            }
        }
        None
    }

    pub fn state(&self, id: FrameId) -> Option<FrameState> {
        let slot = self.slot_for_id(id)?;
        let record = self.metadata.records()[slot];
        match record.state {
            FREE => None,
            ACTIVE => Some(FrameState::Dirty),
            ZEROED => Some(FrameState::Zeroed),
            _ => Some(FrameState::Reserved),
        }
    }

    pub fn owner_live(&self, owner: OwnerId) -> u32 {
        self.owner_live.get(owner.raw() as usize).copied().unwrap_or(0)
    }

    pub fn fragmentation(&self) -> FragmentationSnapshot {
        let mut free_frames = 0;
        let mut largest_free_run = 0;
        let mut current_run = 0;
        for slot in 0..self.frame_count {
            if self.is_free(slot) {
                free_frames += 1;
                current_run += 1;
                largest_free_run = largest_free_run.max(current_run);
            } else {
                current_run = 0;
            }
        }
        FragmentationSnapshot { free_frames, largest_free_run }
    }

    fn claim(
        &mut self,
        slot: usize,
        owner: OwnerId,
        state: FrameState,
    ) -> Result<FrameLease, FrameError> {
        self.clear_free(slot);
        self.claim_used(slot, owner, state)
    }

    fn claim_used(
        &mut self,
        slot: usize,
        owner: OwnerId,
        state: FrameState,
    ) -> Result<FrameLease, FrameError> {
        let generation = {
            let record = &mut self.metadata.records_mut()[slot];
            record.generation = next_frame_generation(record.generation);
            record.owner = owner;
            record.state = match state {
                FrameState::Reserved => 2,
                FrameState::Zeroed => ZEROED,
                FrameState::Dirty => ACTIVE,
            };
            record.generation
        };
        let owner_slot = owner.raw() as usize;
        if let Some(live) = self.owner_live.get_mut(owner_slot) {
            *live = live.saturating_add(1);
        }
        let id = self.id_for_slot(slot).ok_or(FrameError::InvalidFrame)?;
        let address = self.address(id).ok_or(FrameError::InvalidFrame)?;
        Ok(FrameLease { address, id, generation, owner, state, home_cpu: 0, slot: slot as u32 })
    }

    fn release_slot(&mut self, slot: usize, owner: OwnerId) {
        let record = &mut self.metadata.records_mut()[slot];
        record.owner = OwnerId(0);
        record.state = FREE;
        if let Some(live) = self.owner_live.get_mut(owner.raw() as usize) {
            *live = live.saturating_sub(1);
        }
        self.set_free(slot);
    }

    fn validate_lease(&self, lease: FrameLease) -> Result<usize, FrameError> {
        let slot = usize::try_from(lease.slot).map_err(|_| FrameError::InvalidFrame)?;
        let Some(record) = self.metadata.records().get(slot).copied() else {
            return Err(FrameError::InvalidFrame);
        };
        if record.state == FREE {
            return Err(FrameError::StaleHandle);
        }
        if record.generation != lease.generation {
            return Err(FrameError::StaleHandle);
        }
        if record.owner != lease.owner {
            return Err(FrameError::WrongOwner);
        }
        let state = match record.state {
            ACTIVE => FrameState::Dirty,
            ZEROED => FrameState::Zeroed,
            2 => FrameState::Reserved,
            _ => return Err(FrameError::StaleHandle),
        };
        if state != lease.state {
            return Err(FrameError::StaleHandle);
        }
        Ok(slot)
    }

    fn id_for_slot(&self, slot: usize) -> Option<FrameId> {
        let mut low = 0;
        let mut high = self.run_count;
        while low < high {
            let middle = (low + high) / 2;
            let run = self.runs[middle];
            if slot < run.first_slot {
                high = middle;
            } else if slot >= run.first_slot + run.pages {
                low = middle + 1;
            } else {
                return FrameId::new(middle, slot - run.first_slot);
            }
        }
        None
    }

    fn slot_for_id(&self, id: FrameId) -> Option<usize> {
        let run = self.runs.get(id.run())?;
        (id.offset() < run.pages).then_some(run.first_slot + id.offset())
    }

    fn find_free_slot(&self, first: usize, last: usize) -> Option<usize> {
        if first == last {
            return None;
        }
        let first_word = first / 64;
        let last_word = (last - 1) / 64;
        let first_summary = first_word / 64;
        let last_summary = last_word / 64;
        for summary_index in first_summary..=last_summary {
            let mut words = self.metadata.summary()[summary_index];
            if summary_index == first_summary {
                words &= u64::MAX << (first_word % 64);
            }
            if summary_index == last_summary && last_word % 64 != 63 {
                words &= (1u64 << (last_word % 64 + 1)) - 1;
            }
            while words != 0 {
                let word_index = summary_index * 64 + words.trailing_zeros() as usize;
                let mut bits = self.metadata.free()[word_index];
                if word_index == first_word {
                    bits &= u64::MAX << (first % 64);
                }
                if word_index == last_word && last % 64 != 0 {
                    bits &= (1u64 << (last % 64)) - 1;
                }
                if bits != 0 {
                    return Some(word_index * 64 + bits.trailing_zeros() as usize);
                }
                words &= words - 1;
            }
        }
        None
    }

    fn is_free(&self, slot: usize) -> bool {
        self.metadata.free()[slot / 64] & (1u64 << (slot % 64)) != 0
    }

    fn set_free(&mut self, slot: usize) {
        let word = slot / 64;
        self.metadata.free_mut()[word] |= 1u64 << (slot % 64);
        self.metadata.summary_mut()[word / 64] |= 1u64 << (word % 64);
    }

    fn clear_free(&mut self, slot: usize) {
        let word = slot / 64;
        self.metadata.free_mut()[word] &= !(1u64 << (slot % 64));
        if self.metadata.free()[word] == 0 {
            self.metadata.summary_mut()[word / 64] &= !(1u64 << (word % 64));
        }
    }
}

impl Default for PhysicalFrameManager {
    fn default() -> Self {
        Self::empty()
    }
}

fn next_generation(current: u32) -> u32 {
    let next = current.wrapping_add(1);
    if next == 0 { INITIAL_GENERATION } else { next }
}

fn next_frame_generation(current: u16) -> u16 {
    let next = current.wrapping_add(1);
    if next == 0 { INITIAL_FRAME_GENERATION } else { next }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameBatch {
    frames: [Option<FrameLease>; MAX_BATCH_FRAMES],
    len: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FragmentationSnapshot {
    pub free_frames: usize,
    pub largest_free_run: usize,
}

impl FrameBatch {
    pub const fn empty() -> Self {
        Self { frames: [None; MAX_BATCH_FRAMES], len: 0 }
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn get(&self, index: usize) -> Option<FrameLease> {
        self.frames.get(index).copied().flatten()
    }

    fn push(&mut self, lease: FrameLease) -> Result<(), FrameError> {
        let Some(slot) = self.frames.get_mut(self.len) else {
            return Err(FrameError::BatchCapacity);
        };
        *slot = Some(lease);
        self.len += 1;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AllocationError {
    WouldBlock,
    Exhausted,
    InvalidCpu,
    InvalidOwner,
    StaleHandle,
    WrongOwner,
    BatchCapacity,
    Cancelled,
    Deadline,
    InterruptContext,
}

#[repr(align(64))]
struct CpuCache {
    frames: [Option<FrameLease>; FRAME_CACHE_CAPACITY],
    len: usize,
}

impl CpuCache {
    const fn empty() -> Self {
        Self { frames: [None; FRAME_CACHE_CAPACITY], len: 0 }
    }

    fn pop_matching(&mut self, owner: OwnerId, state: FrameState) -> Option<FrameLease> {
        for index in (0..self.len).rev() {
            let Some(lease) = self.frames[index] else {
                continue;
            };
            if lease.owner != owner || lease.state != state {
                continue;
            }
            self.len -= 1;
            let result = self.frames[index].take();
            if index != self.len {
                self.frames[index] = self.frames[self.len].take();
            }
            return result;
        }
        None
    }

    fn pop(&mut self) -> Option<FrameLease> {
        if self.len == 0 {
            return None;
        }
        self.len -= 1;
        self.frames[self.len].take()
    }

    fn contains(&self, lease: FrameLease) -> bool {
        self.frames[..self.len].contains(&Some(lease))
    }

    fn push(&mut self, lease: FrameLease) -> Result<(), FrameLease> {
        let Some(slot) = self.frames.get_mut(self.len) else {
            return Err(lease);
        };
        *slot = Some(lease);
        self.len += 1;
        Ok(())
    }
}

#[repr(align(64))]
struct RemoteQueue {
    lock: TryLock<RemoteQueueState>,
}

struct RemoteQueueState {
    frames: [Option<FrameLease>; FRAME_CACHE_CAPACITY],
    len: usize,
}

impl RemoteQueue {
    const fn empty() -> Self {
        Self {
            lock: TryLock::new(RemoteQueueState { frames: [None; FRAME_CACHE_CAPACITY], len: 0 }),
        }
    }

    fn push(&self, lease: FrameLease) -> Result<(), FrameLease> {
        let Some(mut guard) = self.lock.try_lock() else {
            return Err(lease);
        };
        if guard.len == FRAME_CACHE_CAPACITY {
            return Err(lease);
        }
        let index = guard.len;
        guard.frames[index] = Some(lease);
        guard.len += 1;
        Ok(())
    }

    fn contains(&self, lease: FrameLease) -> Option<bool> {
        let guard = self.lock.try_lock()?;
        Some(guard.frames[..guard.len].contains(&Some(lease)))
    }

    fn pop(&self) -> Option<FrameLease> {
        let mut guard = self.lock.try_lock()?;
        if guard.len == 0 {
            return None;
        }
        guard.len -= 1;
        let index = guard.len;
        guard.frames[index].take()
    }
}

#[repr(align(64))]
struct ShardLock {
    lock: TryLock<ShardState>,
}

struct ShardState;

impl ShardLock {
    const fn empty() -> Self {
        Self { lock: TryLock::new(ShardState) }
    }
}

#[repr(align(64))]
struct WaitNode {
    state: AtomicU8,
    shard: AtomicU8,
    deadline: AtomicU64,
    waker_lock: AtomicBool,
    waker: UnsafeCell<Option<Waker>>,
}

impl WaitNode {
    const fn empty() -> Self {
        Self {
            state: AtomicU8::new(FREE),
            shard: AtomicU8::new(0),
            deadline: AtomicU64::new(NO_DEADLINE),
            waker_lock: AtomicBool::new(false),
            waker: UnsafeCell::new(None),
        }
    }

    fn claim(&self) -> bool {
        self.state.compare_exchange(FREE, ACTIVE, Ordering::AcqRel, Ordering::Acquire).is_ok()
    }

    fn set_waker(&self, waker: &Waker) {
        if self
            .waker_lock
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            // SAFETY: the small lock protects the only mutable field.
            unsafe { *self.waker.get() = Some(waker.clone()) };
            self.waker_lock.store(false, Ordering::Release);
        }
        self.state.store(ACTIVE, Ordering::Release);
    }

    fn wake(&self) {
        if self.state.compare_exchange(ACTIVE, WOKEN, Ordering::AcqRel, Ordering::Acquire).is_err()
        {
            return;
        }
        if self
            .waker_lock
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            // SAFETY: the small lock protects the only mutable field.
            let waker = unsafe { (*self.waker.get()).take() };
            self.waker_lock.store(false, Ordering::Release);
            if let Some(waker) = waker {
                waker.wake();
            }
        }
    }

    fn expire(&self) {
        self.wake();
    }

    fn cancel(&self) {
        self.state.store(FREE, Ordering::Release);
        self.deadline.store(NO_DEADLINE, Ordering::Release);
        if self
            .waker_lock
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            // SAFETY: the small lock protects the only mutable field.
            unsafe { *self.waker.get() = None };
            self.waker_lock.store(false, Ordering::Release);
        }
    }
}

unsafe impl Sync for WaitNode {}

#[repr(align(64))]
struct WaitQueue {
    nodes: AtomicU64,
}

impl WaitQueue {
    const fn empty() -> Self {
        Self { nodes: AtomicU64::new(0) }
    }

    fn add(&self, node: usize) {
        self.nodes.fetch_or(1u64 << node, Ordering::AcqRel);
    }

    fn remove(&self, node: usize) {
        self.nodes.fetch_and(!(1u64 << node), Ordering::AcqRel);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WakePolicy {
    One,
    All,
}

#[derive(Default)]
pub struct AllocationStats {
    allocations: AtomicU64,
    frees: AtomicU64,
    exhausted: AtomicU64,
    would_block: AtomicU64,
    contention: AtomicU64,
    remote_frees: AtomicU64,
    invalid_handles: AtomicU64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AllocationStatsSnapshot {
    pub allocations: u64,
    pub frees: u64,
    pub exhausted: u64,
    pub would_block: u64,
    pub contention: u64,
    pub remote_frees: u64,
    pub invalid_handles: u64,
}

impl AllocationStats {
    pub fn snapshot(&self) -> AllocationStatsSnapshot {
        AllocationStatsSnapshot {
            allocations: self.allocations.load(Ordering::Relaxed),
            frees: self.frees.load(Ordering::Relaxed),
            exhausted: self.exhausted.load(Ordering::Relaxed),
            would_block: self.would_block.load(Ordering::Relaxed),
            contention: self.contention.load(Ordering::Relaxed),
            remote_frees: self.remote_frees.load(Ordering::Relaxed),
            invalid_handles: self.invalid_handles.load(Ordering::Relaxed),
        }
    }
}

pub struct LatencyHistogram {
    buckets: [AtomicU64; 16],
    samples: AtomicU64,
}

impl LatencyHistogram {
    pub const fn new() -> Self {
        Self { buckets: [const { AtomicU64::new(0) }; 16], samples: AtomicU64::new(0) }
    }

    pub fn record(&self, ticks: u64) {
        let bucket = ticks.min(15) as usize;
        self.buckets[bucket].fetch_add(1, Ordering::Relaxed);
        self.samples.fetch_add(1, Ordering::Relaxed);
    }

    pub fn percentile(&self, numerator: u64, denominator: u64) -> Option<u64> {
        if denominator == 0 || numerator > denominator {
            return None;
        }
        let samples = self.samples.load(Ordering::Acquire);
        if samples == 0 {
            return None;
        }
        let rank = samples.saturating_mul(numerator).div_ceil(denominator).max(1);
        let mut cumulative = 0;
        for (index, bucket) in self.buckets.iter().enumerate() {
            cumulative += bucket.load(Ordering::Acquire);
            if cumulative >= rank {
                return Some(index as u64);
            }
        }
        Some(15)
    }
}

impl Default for LatencyHistogram {
    fn default() -> Self {
        Self::new()
    }
}

/// SMP facade: disjoint bitmap-word shards, cache-line separated magazines,
/// bounded remote-free queues, and preallocated async wait nodes.
pub struct SmpFrameAllocator {
    manager: UnsafeCell<PhysicalFrameManager>,
    shards: [ShardLock; MAX_FRAME_SHARDS],
    shard_first: [usize; MAX_FRAME_SHARDS],
    shard_last: [usize; MAX_FRAME_SHARDS],
    caches: [TryLock<CpuCache>; MAX_MEMORY_CPUS],
    remote: [RemoteQueue; MAX_MEMORY_CPUS],
    waits: [WaitNode; MAX_WAIT_NODES],
    wait_queues: [WaitQueue; MAX_FRAME_SHARDS],
    stats: AllocationStats,
    latency: LatencyHistogram,
    clock: AtomicU64,
    initialized: AtomicBool,
}

unsafe impl Sync for SmpFrameAllocator {}

impl SmpFrameAllocator {
    pub const fn empty() -> Self {
        Self {
            manager: UnsafeCell::new(PhysicalFrameManager::empty()),
            shards: [const { ShardLock::empty() }; MAX_FRAME_SHARDS],
            shard_first: [0; MAX_FRAME_SHARDS],
            shard_last: [0; MAX_FRAME_SHARDS],
            caches: [const { TryLock::new(CpuCache::empty()) }; MAX_MEMORY_CPUS],
            remote: [const { RemoteQueue::empty() }; MAX_MEMORY_CPUS],
            waits: [const { WaitNode::empty() }; MAX_WAIT_NODES],
            wait_queues: [const { WaitQueue::empty() }; MAX_FRAME_SHARDS],
            stats: AllocationStats {
                allocations: AtomicU64::new(0),
                frees: AtomicU64::new(0),
                exhausted: AtomicU64::new(0),
                would_block: AtomicU64::new(0),
                contention: AtomicU64::new(0),
                remote_frees: AtomicU64::new(0),
                invalid_handles: AtomicU64::new(0),
            },
            latency: LatencyHistogram::new(),
            clock: AtomicU64::new(0),
            initialized: AtomicBool::new(false),
        }
    }

    pub fn initialize(&mut self, map: &NormalizedMemoryMap) -> Result<(), FrameError> {
        // SAFETY: initialization is exclusively borrowed and precedes publication.
        unsafe { &mut *self.manager.get() }.initialize(map)?;
        self.finish_initialize()
    }

    pub fn initialize_with_region(
        &mut self,
        map: &NormalizedMemoryMap,
        metadata: FrameMetadataRegion,
    ) -> Result<(), FrameError> {
        // SAFETY: initialization is exclusively borrowed and precedes publication.
        unsafe { &mut *self.manager.get() }.initialize_with_region(map, metadata)?;
        self.finish_initialize()
    }

    fn finish_initialize(&mut self) -> Result<(), FrameError> {
        let count = unsafe { (&*self.manager.get()).frame_count() };
        for shard in 0..MAX_FRAME_SHARDS {
            let first = (count * shard).div_ceil(MAX_FRAME_SHARDS);
            let last = (count * (shard + 1)).div_ceil(MAX_FRAME_SHARDS);
            self.shard_first[shard] = first.div_ceil(64) * 64;
            self.shard_last[shard] =
                if shard + 1 == MAX_FRAME_SHARDS { count } else { last.div_ceil(64) * 64 }
                    .min(count);
        }
        self.initialized.store(true, Ordering::Release);
        Ok(())
    }

    pub fn stats(&self) -> AllocationStatsSnapshot {
        self.stats.snapshot()
    }

    pub fn capacity(&self) -> usize {
        // SAFETY: frame-count metadata is immutable after initialization.
        unsafe { (&*self.manager.get()).frame_count() }
    }

    pub fn available(&self) -> usize {
        // SAFETY: callers use this snapshot for capacity reporting only.
        unsafe { (&*self.manager.get()).available() }
    }

    #[cfg(target_os = "uefi")]
    #[allow(clippy::mut_from_ref)]
    pub(crate) fn heap_metadata(
        &self,
    ) -> Option<(&mut [HeapSlot], &mut [HeapPageRecord], &mut [HeapLeaseRecord])> {
        if !self.initialized.load(Ordering::Acquire) {
            return None;
        }
        // SAFETY: heap metadata is reserved during allocator initialization and
        // is exclusively owned by the kernel heap after this handoff.
        Some(unsafe { (&mut *self.manager.get()).heap_metadata() })
    }

    /// Flush cached frees into the shared frame metadata.
    ///
    /// The synchronous `FramePool` facade uses this to preserve its exact
    /// availability semantics while the SMP path keeps per-CPU magazines.
    pub fn flush_cpu_cache(&self, cpu: usize) {
        if cpu >= MAX_MEMORY_CPUS {
            return;
        }
        loop {
            let Some(lease) = self.caches[cpu].try_lock().and_then(|mut cache| cache.pop()) else {
                return;
            };
            let Some(shard) = self.shard_for_slot(lease.slot as usize) else {
                continue;
            };
            let Some(_guard) = self.shards[shard].lock.try_lock() else {
                let _ = self.caches[cpu].try_lock().and_then(|mut cache| cache.push(lease).ok());
                return;
            };
            // SAFETY: the shard owns this frame's metadata.
            if unsafe { &mut *self.manager.get() }.free(lease).is_err() {
                let _ = self.caches[cpu].try_lock().and_then(|mut cache| cache.push(lease).ok());
                return;
            }
            self.wake_shard(shard, WakePolicy::One);
        }
    }

    pub(crate) fn manager(&self) -> &PhysicalFrameManager {
        // SAFETY: this shared view matches the pre-existing FramePool manager
        // inspection API; mutation remains owned by the allocator paths.
        unsafe { &*self.manager.get() }
    }

    pub(crate) fn reserve(
        &mut self,
        address: FrameAddress,
        owner: OwnerId,
    ) -> Result<FrameLease, FrameError> {
        self.flush_cpu_cache(0);
        // SAFETY: exclusive access is required for boot/control reservations.
        unsafe { &mut *self.manager.get() }.reserve(address, owner)
    }

    pub(crate) fn reserve_batch(
        &mut self,
        addresses: &[FrameAddress],
        owner: OwnerId,
    ) -> Result<FrameBatch, FrameError> {
        self.flush_cpu_cache(0);
        // SAFETY: exclusive access is required for boot/control reservations.
        unsafe { &mut *self.manager.get() }.reserve_batch(addresses, owner)
    }

    pub(crate) fn release_reservation(&mut self, lease: FrameLease) -> Result<(), FrameError> {
        self.flush_cpu_cache(0);
        // SAFETY: exclusive access is required for boot/control reservations.
        unsafe { &mut *self.manager.get() }.release_reservation(lease)
    }

    pub(crate) fn release_address(&mut self, address: FrameAddress) -> Result<(), FrameError> {
        self.flush_cpu_cache(0);
        // SAFETY: exclusive access is required for the synchronous release path.
        unsafe { &mut *self.manager.get() }.release_address(address)
    }

    pub fn latency(&self) -> &LatencyHistogram {
        &self.latency
    }

    pub fn try_alloc(
        &self,
        cpu: usize,
        owner: OwnerId,
        state: FrameState,
    ) -> Result<FrameLease, AllocationError> {
        self.alloc_inner(cpu, owner, state, false)
    }

    /// IRQ-safe path: cache plus try-locked metadata refresh; it never spins.
    pub fn try_alloc_irq(
        &self,
        cpu: usize,
        owner: OwnerId,
        state: FrameState,
    ) -> Result<FrameLease, AllocationError> {
        if cpu >= MAX_MEMORY_CPUS || owner.0 == 0 {
            return Err(AllocationError::InvalidCpu);
        }
        let cached =
            self.caches[cpu].try_lock().and_then(|mut cache| cache.pop_matching(owner, state));
        let Some(lease) = cached else {
            if self.caches[cpu].try_lock().is_none() {
                self.stats.contention.fetch_add(1, Ordering::Relaxed);
            }
            self.stats.would_block.fetch_add(1, Ordering::Relaxed);
            return Err(AllocationError::WouldBlock);
        };
        match self.refresh_cached(cpu, lease) {
            Ok(lease) => {
                self.stats.allocations.fetch_add(1, Ordering::Relaxed);
                Ok(lease)
            }
            Err(AllocationError::WouldBlock) => {
                let _ = self.caches[cpu].try_lock().and_then(|mut cache| cache.push(lease).ok());
                self.stats.would_block.fetch_add(1, Ordering::Relaxed);
                Err(AllocationError::WouldBlock)
            }
            Err(error) => Err(error),
        }
    }

    pub fn alloc_batch(
        &self,
        cpu: usize,
        owner: OwnerId,
        count: usize,
        state: FrameState,
    ) -> Result<FrameBatch, AllocationError> {
        if count == 0 || count > MAX_BATCH_FRAMES {
            return Err(AllocationError::BatchCapacity);
        }
        let mut batch = FrameBatch::empty();
        for _ in 0..count {
            let lease = match self.try_alloc(cpu, owner, state) {
                Ok(lease) => lease,
                Err(error) => {
                    let _ = self.free_batch(cpu, batch);
                    return Err(error);
                }
            };
            batch.push(lease).map_err(|_| AllocationError::BatchCapacity)?;
        }
        Ok(batch)
    }

    pub fn free(&self, cpu: usize, lease: FrameLease) -> Result<(), AllocationError> {
        if cpu >= MAX_MEMORY_CPUS {
            return Err(AllocationError::InvalidCpu);
        }
        let home = lease.home_cpu();
        if home >= MAX_MEMORY_CPUS {
            return Err(AllocationError::InvalidCpu);
        }
        let validation_shard =
            self.shard_for_slot(lease.slot as usize).ok_or(AllocationError::StaleHandle)?;
        let validation = {
            let Some(_guard) = self.shards[validation_shard].lock.try_lock() else {
                self.stats.contention.fetch_add(1, Ordering::Relaxed);
                return Err(AllocationError::WouldBlock);
            };
            // SAFETY: validation only reads the frame record owned by this shard.
            unsafe { &*self.manager.get() }.validate(lease)
        };
        match validation {
            Ok(()) => {}
            Err(FrameError::StaleHandle) => return Err(AllocationError::StaleHandle),
            Err(FrameError::WrongOwner) => return Err(AllocationError::WrongOwner),
            Err(_) => return Err(AllocationError::StaleHandle),
        }
        if home != cpu {
            match self.remote[home].contains(lease) {
                Some(true) => return Err(AllocationError::StaleHandle),
                Some(false) => {}
                None => return Err(AllocationError::WouldBlock),
            }
            if self.remote[home].push(lease).is_ok() {
                self.stats.remote_frees.fetch_add(1, Ordering::Relaxed);
                return Ok(());
            }
        } else if let Some(mut cache) = self.caches[home].try_lock() {
            if cache.contains(lease) {
                return Err(AllocationError::StaleHandle);
            }
            if cache.push(lease).is_ok() {
                self.stats.frees.fetch_add(1, Ordering::Relaxed);
                return Ok(());
            }
        }

        let shard = self.shard_for_slot(lease.slot as usize).ok_or(AllocationError::StaleHandle)?;
        let result = {
            let Some(_guard) = self.shards[shard].lock.try_lock() else {
                self.stats.contention.fetch_add(1, Ordering::Relaxed);
                return Err(AllocationError::WouldBlock);
            };
            // SAFETY: the shard owns the bitmap word containing this frame.
            unsafe { &mut *self.manager.get() }.free(lease)
        };
        match result {
            Ok(()) => {
                self.stats.frees.fetch_add(1, Ordering::Relaxed);
                self.wake_shard(shard, WakePolicy::One);
                Ok(())
            }
            Err(FrameError::StaleHandle) => {
                self.stats.invalid_handles.fetch_add(1, Ordering::Relaxed);
                Err(AllocationError::StaleHandle)
            }
            Err(FrameError::WrongOwner) => Err(AllocationError::WrongOwner),
            Err(_) => Err(AllocationError::StaleHandle),
        }
    }

    pub fn free_batch(&self, cpu: usize, batch: FrameBatch) -> Result<(), AllocationError> {
        for index in 0..batch.len {
            self.free(cpu, batch.frames[index].unwrap())?;
        }
        Ok(())
    }

    pub fn alloc_async(
        &self,
        cpu: usize,
        owner: OwnerId,
        state: FrameState,
        deadline: Option<u64>,
    ) -> AllocationFuture<'_> {
        let node = self.waits.iter().position(WaitNode::claim);
        if let Some(index) = node {
            self.waits[index].deadline.store(deadline.unwrap_or(NO_DEADLINE), Ordering::Release);
        }
        AllocationFuture {
            allocator: self,
            cpu,
            owner,
            state,
            deadline: deadline.unwrap_or(NO_DEADLINE),
            node,
            done: false,
        }
    }

    pub fn wake_waiters(&self, shard: usize, policy: WakePolicy) {
        self.wake_shard(shard, policy);
    }

    pub fn advance_time(&self, now: u64) {
        self.clock.fetch_max(now, Ordering::Release);
        for (index, node) in self.waits.iter().enumerate() {
            if node.state.load(Ordering::Acquire) != FREE
                && node.deadline.load(Ordering::Acquire) <= now
            {
                node.expire();
                for queue in &self.wait_queues {
                    queue.remove(index);
                }
            }
        }
    }

    fn alloc_inner(
        &self,
        cpu: usize,
        owner: OwnerId,
        state: FrameState,
        irq: bool,
    ) -> Result<FrameLease, AllocationError> {
        if cpu >= MAX_MEMORY_CPUS || owner.0 == 0 {
            return Err(AllocationError::InvalidCpu);
        }
        if !self.initialized.load(Ordering::Acquire) {
            return Err(AllocationError::Exhausted);
        }
        self.drain_remote(cpu);
        let cached = if let Some(mut cache) = self.caches[cpu].try_lock() {
            cache.pop_matching(owner, state)
        } else {
            self.stats.contention.fetch_add(1, Ordering::Relaxed);
            return Err(AllocationError::WouldBlock);
        };
        if let Some(lease) = cached {
            match self.refresh_cached(cpu, lease) {
                Ok(lease) => {
                    self.stats.allocations.fetch_add(1, Ordering::Relaxed);
                    return Ok(lease);
                }
                Err(AllocationError::WouldBlock) => {
                    let _ =
                        self.caches[cpu].try_lock().and_then(|mut cache| cache.push(lease).ok());
                    return Err(AllocationError::WouldBlock);
                }
                Err(_) => {}
            }
        }
        if irq {
            return Err(AllocationError::WouldBlock);
        }
        let mut contended = false;
        for offset in 0..MAX_FRAME_SHARDS {
            let shard = (cpu + offset) % MAX_FRAME_SHARDS;
            let Some(_guard) = self.shards[shard].lock.try_lock() else {
                contended = true;
                self.stats.contention.fetch_add(1, Ordering::Relaxed);
                continue;
            };
            let first = self.shard_first[shard];
            let last = self.shard_last[shard];
            // SAFETY: the shard owns this disjoint bitmap-word interval.
            let result =
                unsafe { &mut *self.manager.get() }.try_alloc_in_range(owner, state, first, last);
            match result {
                Ok(lease) => {
                    self.stats.allocations.fetch_add(1, Ordering::Relaxed);
                    return Ok(lease.with_home_cpu(cpu));
                }
                Err(FrameError::Exhausted | FrameError::InvalidFrame) => continue,
                Err(_) => return Err(AllocationError::StaleHandle),
            }
        }
        if contended {
            self.stats.would_block.fetch_add(1, Ordering::Relaxed);
            Err(AllocationError::WouldBlock)
        } else {
            self.stats.exhausted.fetch_add(1, Ordering::Relaxed);
            Err(AllocationError::Exhausted)
        }
    }

    fn drain_remote(&self, cpu: usize) {
        for _ in 0..FRAME_CACHE_REFILL {
            let Some(lease) = self.remote[cpu].pop() else {
                break;
            };
            let Some(shard) = self.shard_for_slot(lease.slot as usize) else {
                continue;
            };
            let result = {
                let Some(_guard) = self.shards[shard].lock.try_lock() else {
                    let _ = self.remote[cpu].push(lease);
                    break;
                };
                // SAFETY: the shard owns this frame's metadata.
                unsafe { &mut *self.manager.get() }.refresh(lease)
            };
            if let Ok(lease) = result {
                let lease = lease.with_home_cpu(cpu);
                if let Some(mut cache) = self.caches[cpu].try_lock() {
                    if cache.push(lease).is_err() {
                        self.return_remote_or_central(cpu, lease);
                    }
                } else {
                    self.return_remote_or_central(cpu, lease);
                }
                self.stats.frees.fetch_add(1, Ordering::Relaxed);
                self.wake_shard(shard, WakePolicy::One);
            }
        }
    }

    fn shard_for_slot(&self, slot: usize) -> Option<usize> {
        (0..MAX_FRAME_SHARDS)
            .find(|shard| slot >= self.shard_first[*shard] && slot < self.shard_last[*shard])
    }

    fn refresh_cached(&self, cpu: usize, lease: FrameLease) -> Result<FrameLease, AllocationError> {
        let shard = self.shard_for_slot(lease.slot as usize).ok_or(AllocationError::StaleHandle)?;
        let Some(_guard) = self.shards[shard].lock.try_lock() else {
            self.stats.contention.fetch_add(1, Ordering::Relaxed);
            return Err(AllocationError::WouldBlock);
        };
        // SAFETY: this shard owns the frame's metadata and bitmap word.
        unsafe { &mut *self.manager.get() }
            .refresh(lease)
            .map(|lease| lease.with_home_cpu(cpu))
            .map_err(|error| match error {
                FrameError::WrongOwner => AllocationError::WrongOwner,
                _ => AllocationError::StaleHandle,
            })
    }

    fn return_remote_or_central(&self, cpu: usize, lease: FrameLease) {
        if self.remote[cpu].push(lease).is_ok() {
            return;
        }
        let Some(shard) = self.shard_for_slot(lease.slot as usize) else {
            return;
        };
        let result = {
            let Some(_guard) = self.shards[shard].lock.try_lock() else {
                return;
            };
            // SAFETY: the shard owns this frame's metadata.
            unsafe { &mut *self.manager.get() }.free(lease)
        };
        if result.is_ok() {
            self.stats.frees.fetch_add(1, Ordering::Relaxed);
            self.wake_shard(shard, WakePolicy::One);
        }
    }

    fn wake_shard(&self, shard: usize, policy: WakePolicy) {
        if shard >= MAX_FRAME_SHARDS {
            return;
        }
        let mut mask = self.wait_queues[shard].nodes.load(Ordering::Acquire);
        while mask != 0 {
            let index = mask.trailing_zeros() as usize;
            mask &= mask - 1;
            self.waits[index].wake();
            if matches!(policy, WakePolicy::One) {
                break;
            }
        }
    }
}

impl Default for SmpFrameAllocator {
    fn default() -> Self {
        Self::empty()
    }
}

pub struct AllocationFuture<'a> {
    allocator: &'a SmpFrameAllocator,
    cpu: usize,
    owner: OwnerId,
    state: FrameState,
    deadline: u64,
    node: Option<usize>,
    done: bool,
}

impl AllocationFuture<'_> {
    pub fn cancel(&mut self) {
        self.done = true;
        self.release_node();
    }

    fn release_node(&mut self) {
        let Some(index) = self.node.take() else {
            return;
        };
        for queue in &self.allocator.wait_queues {
            queue.remove(index);
        }
        self.allocator.waits[index].cancel();
    }
}

impl Future for AllocationFuture<'_> {
    type Output = Result<FrameLease, AllocationError>;

    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        if self.done {
            return Poll::Ready(Err(AllocationError::Cancelled));
        }
        if self.deadline != NO_DEADLINE && self.allocator.now() >= self.deadline {
            self.cancel();
            return Poll::Ready(Err(AllocationError::Deadline));
        }
        match self.allocator.try_alloc(self.cpu, self.owner, self.state) {
            Ok(lease) => {
                self.done = true;
                self.release_node();
                Poll::Ready(Ok(lease))
            }
            Err(AllocationError::WouldBlock | AllocationError::Exhausted) => {
                if let Some(index) = self.node {
                    let shard = self.cpu % MAX_FRAME_SHARDS;
                    self.allocator.waits[index].shard.store(shard as u8, Ordering::Release);
                    self.allocator.waits[index].set_waker(context.waker());
                    self.allocator.wait_queues[shard].add(index);
                }
                Poll::Pending
            }
            Err(error) => {
                self.done = true;
                self.release_node();
                Poll::Ready(Err(error))
            }
        }
    }
}

impl Drop for AllocationFuture<'_> {
    fn drop(&mut self) {
        self.release_node();
    }
}

impl SmpFrameAllocator {
    fn now(&self) -> u64 {
        self.clock.load(Ordering::Acquire)
    }
}

pub struct TryLock<T> {
    held: AtomicBool,
    value: UnsafeCell<T>,
}

unsafe impl<T: Send> Sync for TryLock<T> {}

impl<T> TryLock<T> {
    pub const fn new(value: T) -> Self {
        Self { held: AtomicBool::new(false), value: UnsafeCell::new(value) }
    }

    pub fn try_lock(&self) -> Option<TryLockGuard<'_, T>> {
        self.held
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .ok()
            .map(|_| TryLockGuard { lock: self })
    }
}

pub struct TryLockGuard<'a, T> {
    lock: &'a TryLock<T>,
}

impl<T> Deref for TryLockGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        // SAFETY: the guard is the unique holder of the lock.
        unsafe { &*self.lock.value.get() }
    }
}

impl<T> DerefMut for TryLockGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        // SAFETY: the guard is the unique holder of the lock.
        unsafe { &mut *self.lock.value.get() }
    }
}

impl<T> Drop for TryLockGuard<'_, T> {
    fn drop(&mut self) {
        self.lock.held.store(false, Ordering::Release);
    }
}

// --- Virtual memory -----------------------------------------------------

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AddressSpaceId {
    slot: u8,
    generation: u32,
}

impl AddressSpaceId {
    pub const fn slot(self) -> usize {
        self.slot as usize
    }

    pub const fn generation(self) -> u32 {
        self.generation
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VmFlags {
    pub writable: bool,
    pub executable: bool,
    pub user: bool,
}

impl VmFlags {
    pub const CODE: Self = Self { writable: false, executable: true, user: true };
    pub const DATA: Self = Self { writable: true, executable: false, user: true };
    pub const READ_ONLY: Self = Self { writable: false, executable: false, user: true };

    const fn valid(self) -> bool {
        self.user && !(self.writable && self.executable)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PageSize {
    Size4K,
    Size2M,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PageMapping {
    virtual_address: u64,
    frame: FrameId,
    flags: VmFlags,
    size: PageSize,
}

impl PageMapping {
    pub const fn virtual_address(self) -> u64 {
        self.virtual_address
    }

    pub const fn frame(self) -> FrameId {
        self.frame
    }

    pub const fn flags(self) -> VmFlags {
        self.flags
    }

    pub const fn size(self) -> PageSize {
        self.size
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VirtualMemoryError {
    Capacity,
    InvalidAddressSpace,
    InvalidAddress,
    InvalidFrame,
    InvalidFlags,
    Conflict,
    NotMapped,
    UnsupportedPageSize,
}

#[derive(Clone, Copy)]
struct AddressSpaceSlot {
    generation: u32,
    live: bool,
    mappings: [Option<PageMapping>; MAX_MAPPINGS_PER_ADDRESS_SPACE],
}

impl AddressSpaceSlot {
    const EMPTY: Self = Self {
        generation: INITIAL_GENERATION,
        live: false,
        mappings: [None; MAX_MAPPINGS_PER_ADDRESS_SPACE],
    };
}

pub struct VirtualMemoryManager {
    spaces: [AddressSpaceSlot; MAX_ADDRESS_SPACES],
}

impl VirtualMemoryManager {
    pub const fn new() -> Self {
        Self { spaces: [AddressSpaceSlot::EMPTY; MAX_ADDRESS_SPACES] }
    }

    pub fn create(&mut self) -> Result<AddressSpaceId, VirtualMemoryError> {
        let Some((slot, state)) = self.spaces.iter_mut().enumerate().find(|(_, state)| !state.live)
        else {
            return Err(VirtualMemoryError::Capacity);
        };
        state.live = true;
        state.mappings = [None; MAX_MAPPINGS_PER_ADDRESS_SPACE];
        Ok(AddressSpaceId { slot: slot as u8, generation: state.generation })
    }

    pub fn destroy(&mut self, id: AddressSpaceId) -> Result<(), VirtualMemoryError> {
        let state = self.current_mut(id)?;
        state.live = false;
        state.mappings = [None; MAX_MAPPINGS_PER_ADDRESS_SPACE];
        state.generation = next_generation(state.generation);
        Ok(())
    }

    pub fn map(
        &mut self,
        id: AddressSpaceId,
        virtual_address: u64,
        frame: FrameId,
        flags: VmFlags,
    ) -> Result<PageMapping, VirtualMemoryError> {
        self.map_page(id, virtual_address, frame, flags, PageSize::Size4K)
    }

    pub fn map_page(
        &mut self,
        id: AddressSpaceId,
        virtual_address: u64,
        frame: FrameId,
        flags: VmFlags,
        size: PageSize,
    ) -> Result<PageMapping, VirtualMemoryError> {
        if size != PageSize::Size4K {
            return Err(VirtualMemoryError::UnsupportedPageSize);
        }
        let mapping = Self::validate_mapping(virtual_address, frame, flags, size)?;
        let state = self.current_mut(id)?;
        if state
            .mappings
            .iter()
            .flatten()
            .any(|existing| existing.virtual_address == virtual_address)
        {
            return Err(VirtualMemoryError::Conflict);
        }
        let Some(slot) = state.mappings.iter_mut().find(|slot| slot.is_none()) else {
            return Err(VirtualMemoryError::Capacity);
        };
        *slot = Some(mapping);
        Ok(mapping)
    }

    pub fn unmap(
        &mut self,
        id: AddressSpaceId,
        virtual_address: u64,
    ) -> Result<PageMapping, VirtualMemoryError> {
        let state = self.current_mut(id)?;
        let Some(slot) = state
            .mappings
            .iter_mut()
            .find(|slot| slot.is_some_and(|mapping| mapping.virtual_address == virtual_address))
        else {
            return Err(VirtualMemoryError::NotMapped);
        };
        slot.take().ok_or(VirtualMemoryError::NotMapped)
    }

    pub fn protect(
        &mut self,
        id: AddressSpaceId,
        virtual_address: u64,
        flags: VmFlags,
    ) -> Result<(), VirtualMemoryError> {
        if !flags.valid() {
            return Err(VirtualMemoryError::InvalidFlags);
        }
        let state = self.current_mut(id)?;
        let Some(mapping) = state
            .mappings
            .iter_mut()
            .flatten()
            .find(|mapping| mapping.virtual_address == virtual_address)
        else {
            return Err(VirtualMemoryError::NotMapped);
        };
        mapping.flags = flags;
        Ok(())
    }

    pub fn clone_space(
        &mut self,
        source: AddressSpaceId,
    ) -> Result<AddressSpaceId, VirtualMemoryError> {
        let source_state = *self.current(source)?;
        let target = self.create()?;
        let target_state = self.current_mut(target)?;
        target_state.mappings = source_state.mappings;
        Ok(target)
    }

    pub fn query(&self, id: AddressSpaceId, virtual_address: u64) -> Option<PageMapping> {
        self.current(id)
            .ok()?
            .mappings
            .iter()
            .flatten()
            .find(|mapping| mapping.virtual_address == virtual_address)
            .copied()
    }

    fn validate_mapping(
        virtual_address: u64,
        frame: FrameId,
        flags: VmFlags,
        size: PageSize,
    ) -> Result<PageMapping, VirtualMemoryError> {
        if !flags.valid() {
            return Err(VirtualMemoryError::InvalidFlags);
        }
        let alignment = match size {
            PageSize::Size4K => PAGE_SIZE,
            PageSize::Size2M => 2 * 1024 * 1024,
        };
        if virtual_address == 0
            || virtual_address % alignment != 0
            || virtual_address >= 0x0000_8000_0000_0000
        {
            return Err(VirtualMemoryError::InvalidAddress);
        }
        Ok(PageMapping { virtual_address, frame, flags, size })
    }

    fn current(&self, id: AddressSpaceId) -> Result<&AddressSpaceSlot, VirtualMemoryError> {
        let Some(state) = self.spaces.get(id.slot as usize) else {
            return Err(VirtualMemoryError::InvalidAddressSpace);
        };
        if !state.live || state.generation != id.generation {
            return Err(VirtualMemoryError::InvalidAddressSpace);
        }
        Ok(state)
    }

    fn current_mut(
        &mut self,
        id: AddressSpaceId,
    ) -> Result<&mut AddressSpaceSlot, VirtualMemoryError> {
        let Some(state) = self.spaces.get_mut(id.slot as usize) else {
            return Err(VirtualMemoryError::InvalidAddressSpace);
        };
        if !state.live || state.generation != id.generation {
            return Err(VirtualMemoryError::InvalidAddressSpace);
        }
        Ok(state)
    }
}

impl Default for VirtualMemoryManager {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(align(64))]
pub struct PageTableFrameCache {
    frames: [Option<FrameLease>; FRAME_CACHE_CAPACITY],
    len: usize,
}

impl PageTableFrameCache {
    pub const fn empty() -> Self {
        Self { frames: [None; FRAME_CACHE_CAPACITY], len: 0 }
    }

    pub fn push(&mut self, frame: FrameLease) -> Result<(), FrameLease> {
        let Some(slot) = self.frames.get_mut(self.len) else {
            return Err(frame);
        };
        *slot = Some(frame);
        self.len += 1;
        Ok(())
    }

    pub fn pop(&mut self) -> Option<FrameLease> {
        if self.len == 0 {
            return None;
        }
        self.len -= 1;
        self.frames[self.len].take()
    }
}

// --- TLB coordination ---------------------------------------------------

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TlbInvalidation {
    pub sequence: u64,
    pub address_space: AddressSpaceId,
    pub virtual_address: u64,
    pub pages: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TlbTicket {
    pub sequence: u64,
    pub targets: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TlbBatchEntry {
    pub address_space: AddressSpaceId,
    pub virtual_address: u64,
    pub pages: u16,
}

pub const MAX_TLB_BATCH_ENTRIES: usize = 16;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TlbBatch {
    entries: [Option<TlbBatchEntry>; MAX_TLB_BATCH_ENTRIES],
    len: usize,
}

impl TlbBatch {
    pub const fn new() -> Self {
        Self { entries: [None; MAX_TLB_BATCH_ENTRIES], len: 0 }
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn push(&mut self, entry: TlbBatchEntry) -> Result<(), TlbError> {
        if entry.pages == 0 || entry.virtual_address % PAGE_SIZE != 0 {
            return Err(TlbError::InvalidBatch);
        }
        if let Some(previous) = self.entries[..self.len].iter_mut().rev().flatten().next()
            && previous.address_space == entry.address_space
            && previous.virtual_address.checked_add(previous.pages as u64 * PAGE_SIZE)
                == Some(entry.virtual_address)
            && previous.pages.checked_add(entry.pages).is_some()
        {
            previous.pages += entry.pages;
            return Ok(());
        }
        let Some(slot) = self.entries.get_mut(self.len) else {
            return Err(TlbError::InvalidBatch);
        };
        *slot = Some(entry);
        self.len += 1;
        Ok(())
    }

    pub fn get(&self, index: usize) -> Option<TlbBatchEntry> {
        self.entries.get(index).copied().flatten()
    }
}

impl Default for TlbBatch {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TlbError {
    InvalidCpu,
    QueueFull,
    InvalidBatch,
}

struct TlbQueueState {
    entries: [Option<TlbInvalidation>; MAX_TLB_QUEUE_ENTRIES],
    head: usize,
    tail: usize,
}

impl TlbQueueState {
    const fn empty() -> Self {
        Self { entries: [None; MAX_TLB_QUEUE_ENTRIES], head: 0, tail: 0 }
    }
}

#[repr(align(64))]
struct TlbQueue {
    lock: TryLock<TlbQueueState>,
}

impl TlbQueue {
    const fn empty() -> Self {
        Self { lock: TryLock::new(TlbQueueState::empty()) }
    }
}

pub struct TlbCoordinator {
    queues: [TlbQueue; MAX_MEMORY_CPUS],
    acknowledgements: [AtomicU64; MAX_MEMORY_CPUS],
    sequence: AtomicU64,
}

impl TlbCoordinator {
    pub const fn new() -> Self {
        Self {
            queues: [const { TlbQueue::empty() }; MAX_MEMORY_CPUS],
            acknowledgements: [const { AtomicU64::new(0) }; MAX_MEMORY_CPUS],
            sequence: AtomicU64::new(0),
        }
    }

    pub fn enqueue(
        &self,
        source_cpu: usize,
        targets: u8,
        address_space: AddressSpaceId,
        virtual_address: u64,
        pages: u16,
    ) -> Result<TlbTicket, TlbError> {
        if source_cpu >= MAX_MEMORY_CPUS
            || targets == 0
            || pages == 0
            || virtual_address % PAGE_SIZE != 0
        {
            return Err(TlbError::InvalidCpu);
        }
        let sequence = self.sequence.fetch_add(1, Ordering::AcqRel) + 1;
        let entry = TlbInvalidation { sequence, address_space, virtual_address, pages };
        for cpu in 0..MAX_MEMORY_CPUS {
            if targets & (1u8 << cpu) == 0 {
                continue;
            }
            let Some(mut queue) = self.queues[cpu].lock.try_lock() else {
                return Err(TlbError::QueueFull);
            };
            if queue.tail - queue.head == MAX_TLB_QUEUE_ENTRIES {
                return Err(TlbError::QueueFull);
            }
            let index = queue.tail % MAX_TLB_QUEUE_ENTRIES;
            queue.entries[index] = Some(entry);
            queue.tail += 1;
        }
        Ok(TlbTicket { sequence, targets })
    }

    pub fn enqueue_batch(
        &self,
        source_cpu: usize,
        targets: u8,
        batch: TlbBatch,
    ) -> Result<TlbTicket, TlbError> {
        if source_cpu >= MAX_MEMORY_CPUS || targets == 0 || batch.len == 0 {
            return Err(TlbError::InvalidBatch);
        }
        let sequence = self.sequence.fetch_add(1, Ordering::AcqRel) + 1;
        for cpu in 0..MAX_MEMORY_CPUS {
            if targets & (1u8 << cpu) == 0 {
                continue;
            }
            let Some(mut queue) = self.queues[cpu].lock.try_lock() else {
                return Err(TlbError::QueueFull);
            };
            if queue.tail - queue.head + batch.len > MAX_TLB_QUEUE_ENTRIES {
                return Err(TlbError::QueueFull);
            }
            for index in 0..batch.len {
                let entry = batch.entries[index].unwrap();
                let queue_index = queue.tail % MAX_TLB_QUEUE_ENTRIES;
                queue.entries[queue_index] = Some(TlbInvalidation {
                    sequence,
                    address_space: entry.address_space,
                    virtual_address: entry.virtual_address,
                    pages: entry.pages,
                });
                queue.tail += 1;
            }
        }
        Ok(TlbTicket { sequence, targets })
    }

    pub fn drain(&self, cpu: usize, output: &mut [TlbInvalidation]) -> Result<usize, TlbError> {
        if cpu >= MAX_MEMORY_CPUS {
            return Err(TlbError::InvalidCpu);
        }
        let Some(mut queue) = self.queues[cpu].lock.try_lock() else {
            return Err(TlbError::QueueFull);
        };
        let mut count = 0;
        while queue.head != queue.tail && count < output.len() {
            let index = queue.head % MAX_TLB_QUEUE_ENTRIES;
            output[count] = queue.entries[index].take().unwrap();
            queue.head += 1;
            count += 1;
        }
        Ok(count)
    }

    pub fn acknowledge(&self, cpu: usize, sequence: u64) -> Result<(), TlbError> {
        if cpu >= MAX_MEMORY_CPUS {
            return Err(TlbError::InvalidCpu);
        }
        self.acknowledgements[cpu].fetch_max(sequence, Ordering::Release);
        Ok(())
    }

    pub fn complete(&self, ticket: TlbTicket) -> bool {
        (0..MAX_MEMORY_CPUS).all(|cpu| {
            ticket.targets & (1u8 << cpu) == 0
                || self.acknowledgements[cpu].load(Ordering::Acquire) >= ticket.sequence
        })
    }
}

impl Default for TlbCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

// --- Kernel heap, pressure, and observability --------------------------

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AllocationContext {
    Thread,
    Interrupt,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HeapError {
    InterruptContext,
    WouldBlock,
    Exhausted,
    Quota,
    InvalidHandle,
    WrongOwner,
    StaleHandle,
    InvalidRequest,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HeapAllocationKind {
    Slab,
    Pages,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HeapAllocation {
    kind: HeapAllocationKind,
    index: u32,
    generation: u32,
    owner: OwnerId,
    bytes: usize,
    address: usize,
    pages: Option<FrameBatch>,
}

impl HeapAllocation {
    pub const fn kind(self) -> HeapAllocationKind {
        self.kind
    }

    pub const fn bytes(self) -> usize {
        self.bytes
    }

    pub const fn address(self) -> usize {
        self.address
    }

    pub const fn pages(self) -> Option<FrameBatch> {
        self.pages
    }
}

#[derive(Clone, Copy)]
pub struct HeapSlot {
    used: bool,
    generation: u32,
    owner: OwnerId,
    bytes: usize,
    address: usize,
    frame: Option<FrameLease>,
}

impl HeapSlot {
    const fn empty() -> Self {
        Self {
            used: false,
            generation: INITIAL_GENERATION,
            owner: OwnerId(0),
            bytes: 0,
            address: 0,
            frame: None,
        }
    }
}

struct HeapCentral {
    slots: *mut HeapSlot,
    len: usize,
}

impl HeapCentral {
    fn from_slice(slots: &mut [HeapSlot]) -> Self {
        Self { slots: slots.as_mut_ptr(), len: slots.len() }
    }

    fn slots(&self) -> &[HeapSlot] {
        // SAFETY: the metadata slice outlives the heap and is exclusively
        // owned by this central allocator after initialization.
        unsafe { slice::from_raw_parts(self.slots, self.len) }
    }

    fn slots_mut(&mut self) -> &mut [HeapSlot] {
        // SAFETY: the central lock serializes mutable access to the metadata.
        unsafe { slice::from_raw_parts_mut(self.slots, self.len) }
    }
}

unsafe impl Send for HeapCentral {}

#[derive(Clone, Copy)]
struct Quota {
    limit: usize,
    used: usize,
}

#[repr(align(64))]
struct HeapMagazine {
    slots: [Option<u16>; FRAME_CACHE_CAPACITY],
    len: usize,
}

impl HeapMagazine {
    const fn empty() -> Self {
        Self { slots: [None; FRAME_CACHE_CAPACITY], len: 0 }
    }

    fn pop(&mut self) -> Option<u16> {
        if self.len == 0 {
            return None;
        }
        self.len -= 1;
        self.slots[self.len].take()
    }

    fn push(&mut self, slot: u16) -> bool {
        let Some(target) = self.slots.get_mut(self.len) else {
            return false;
        };
        *target = Some(slot);
        self.len += 1;
        true
    }
}

impl Quota {
    const EMPTY: Self = Self { limit: usize::MAX, used: 0 };
}

#[derive(Clone, Copy)]
pub struct HeapPageRecord {
    used: bool,
    generation: u32,
    owner: OwnerId,
    bytes: usize,
    address: usize,
    lease_start: u32,
    lease_count: u16,
}

impl HeapPageRecord {
    pub const fn empty() -> Self {
        Self {
            used: false,
            generation: INITIAL_GENERATION,
            owner: OwnerId(0),
            bytes: 0,
            address: 0,
            lease_start: 0,
            lease_count: 0,
        }
    }
}

#[derive(Clone, Copy)]
pub struct HeapLeaseRecord {
    used: bool,
    lease: Option<FrameLease>,
}

impl HeapLeaseRecord {
    const fn empty() -> Self {
        Self { used: false, lease: None }
    }
}

/// Bounded page-backed heap handles for small and large objects.
///
/// Small allocations use one frame-backed slot and large allocations use a
/// frame batch. The address is the current identity-mapped frame address;
/// this type intentionally exposes handles, not a `GlobalAlloc`, until the
/// virtual heap contract is proven in the boot path.
pub struct KernelHeap<'a> {
    frames: &'a SmpFrameAllocator,
    central: TryLock<HeapCentral>,
    page_records: &'a mut [HeapPageRecord],
    page_leases: &'a mut [HeapLeaseRecord],
    caches: [TryLock<HeapMagazine>; MAX_MEMORY_CPUS],
    quotas: [Quota; MAX_QUOTAS],
}

impl<'a> KernelHeap<'a> {
    pub fn new(
        frames: &'a SmpFrameAllocator,
        heap_slots: &'a mut [HeapSlot],
        page_records: &'a mut [HeapPageRecord],
        page_leases: &'a mut [HeapLeaseRecord],
    ) -> Self {
        Self {
            frames,
            central: TryLock::new(HeapCentral::from_slice(heap_slots)),
            page_records,
            page_leases,
            caches: [const { TryLock::new(HeapMagazine::empty()) }; MAX_MEMORY_CPUS],
            quotas: [Quota::EMPTY; MAX_QUOTAS],
        }
    }

    pub fn set_quota(&mut self, owner: OwnerId, bytes: usize) -> Result<(), HeapError> {
        let quota = self.quotas.get_mut(owner.raw() as usize).ok_or(HeapError::InvalidRequest)?;
        quota.limit = bytes;
        Ok(())
    }

    pub fn alloc(
        &mut self,
        cpu: usize,
        owner: OwnerId,
        bytes: usize,
        alignment: usize,
        context: AllocationContext,
    ) -> Result<HeapAllocation, HeapError> {
        if matches!(context, AllocationContext::Interrupt) {
            return Err(HeapError::InterruptContext);
        }
        if cpu >= MAX_MEMORY_CPUS || owner.0 == 0 || bytes == 0 || !alignment.is_power_of_two() {
            return Err(HeapError::InvalidRequest);
        }
        if alignment > bytes || alignment > PAGE_SIZE as usize {
            return Err(HeapError::InvalidRequest);
        }
        let quota_index = owner.raw() as usize;
        let quota = self.quotas.get(quota_index).ok_or(HeapError::InvalidRequest)?;
        if quota.used.checked_add(bytes).is_none_or(|used| used > quota.limit) {
            return Err(HeapError::Quota);
        }
        if bytes > 512 {
            let Some(index) = self.page_records.iter().position(|record| !record.used) else {
                return Err(HeapError::Exhausted);
            };
            let pages = bytes.div_ceil(PAGE_SIZE as usize);
            let Some(lease_start) = self.find_lease_range(pages) else {
                return Err(HeapError::Exhausted);
            };
            let batch = self
                .frames
                .alloc_batch(cpu, owner, pages, FrameState::Dirty)
                .map_err(map_alloc_error)?;
            let address = batch
                .get(0)
                .map(|lease| lease.address().raw() as usize)
                .ok_or(HeapError::InvalidHandle)?;
            for offset in 0..pages {
                self.page_leases[lease_start + offset] =
                    HeapLeaseRecord { used: true, lease: batch.get(offset) };
            }
            let record = &mut self.page_records[index];
            record.used = true;
            record.generation = next_generation(record.generation);
            record.owner = owner;
            record.bytes = bytes;
            record.address = address;
            record.lease_start = lease_start as u32;
            record.lease_count = pages as u16;
            self.quotas[quota_index].used += bytes;
            return Ok(HeapAllocation {
                kind: HeapAllocationKind::Pages,
                index: index as u32,
                generation: record.generation,
                owner,
                bytes,
                address,
                pages: Some(batch),
            });
        }
        let Some(mut central) = self.central.try_lock() else {
            return Err(HeapError::WouldBlock);
        };
        let cached = self.caches[cpu].try_lock().and_then(|mut cache| cache.pop());
        let index = cached
            .filter(|index| central.slots().get(*index as usize).is_some_and(|slot| !slot.used))
            .map(|index| index as usize)
            .or_else(|| central.slots().iter().position(|slot| !slot.used));
        let Some(index) = index else {
            return Err(HeapError::Exhausted);
        };
        let batch =
            self.frames.alloc_batch(cpu, owner, 1, FrameState::Dirty).map_err(map_alloc_error)?;
        let address = batch
            .get(0)
            .map(|lease| lease.address().raw() as usize)
            .ok_or(HeapError::InvalidHandle)?;
        let slot = &mut central.slots_mut()[index];
        slot.used = true;
        slot.generation = next_generation(slot.generation);
        slot.owner = owner;
        slot.bytes = bytes;
        slot.address = address;
        slot.frame = batch.get(0);
        self.quotas[quota_index].used += bytes;
        Ok(HeapAllocation {
            kind: HeapAllocationKind::Slab,
            index: index as u32,
            generation: slot.generation,
            owner,
            bytes,
            address,
            pages: None,
        })
    }

    /// Allocate a small kernel object without entering the blocking allocator.
    ///
    /// This path only consumes a pre-existing physical-frame cache entry and a
    /// pre-existing heap slot. It never grows metadata, waits, or invokes
    /// reclaim, so callers may use it from interrupt context.
    pub fn try_alloc_irq(
        &mut self,
        cpu: usize,
        owner: OwnerId,
        layout: Layout,
    ) -> Result<HeapAllocation, HeapError> {
        if cpu >= MAX_MEMORY_CPUS || owner.0 == 0 || layout.size() == 0 {
            return Err(HeapError::InvalidRequest);
        }
        if layout.size() > 512
            || !layout.align().is_power_of_two()
            || layout.align() > layout.size()
            || layout.align() > PAGE_SIZE as usize
        {
            return Err(HeapError::WouldBlock);
        }
        let quota_index = owner.raw() as usize;
        let quota = self.quotas.get(quota_index).ok_or(HeapError::InvalidRequest)?;
        if quota.used.checked_add(layout.size()).is_none_or(|used| used > quota.limit) {
            return Err(HeapError::Quota);
        }
        let Some(mut central) = self.central.try_lock() else {
            return Err(HeapError::WouldBlock);
        };
        let Some(index) = central.slots().iter().position(|slot| !slot.used) else {
            return Err(HeapError::Exhausted);
        };
        let frame =
            self.frames.try_alloc_irq(cpu, owner, FrameState::Dirty).map_err(map_alloc_error)?;
        let address = frame.address().raw() as usize;
        let slot = &mut central.slots_mut()[index];
        slot.used = true;
        slot.generation = next_generation(slot.generation);
        slot.owner = owner;
        slot.bytes = layout.size();
        slot.address = address;
        slot.frame = Some(frame);
        self.quotas[quota_index].used += layout.size();
        Ok(HeapAllocation {
            kind: HeapAllocationKind::Slab,
            index: index as u32,
            generation: slot.generation,
            owner,
            bytes: layout.size(),
            address,
            pages: None,
        })
    }

    pub fn free(&mut self, cpu: usize, allocation: HeapAllocation) -> Result<(), HeapError> {
        if allocation.kind == HeapAllocationKind::Pages {
            let Some(record) = self.page_records.get(allocation.index as usize).copied() else {
                return Err(HeapError::InvalidHandle);
            };
            if !record.used || record.generation != allocation.generation {
                return Err(HeapError::StaleHandle);
            }
            if record.owner != allocation.owner {
                return Err(HeapError::WrongOwner);
            }
            if record.address != allocation.address || record.bytes != allocation.bytes {
                return Err(HeapError::InvalidHandle);
            }
            let bytes = record.bytes;
            let batch = self.page_batch(record)?;
            if allocation.pages != Some(batch) {
                return Err(HeapError::InvalidHandle);
            }
            self.frames.free_batch(cpu, batch).map_err(map_alloc_error)?;
            for offset in 0..record.lease_count as usize {
                self.page_leases[record.lease_start as usize + offset] = HeapLeaseRecord::empty();
            }
            let record = &mut self.page_records[allocation.index as usize];
            record.used = false;
            record.owner = OwnerId(0);
            record.bytes = 0;
            record.address = 0;
            record.lease_start = 0;
            record.lease_count = 0;
            if let Some(quota) = self.quotas.get_mut(allocation.owner.raw() as usize) {
                quota.used = quota.used.saturating_sub(bytes);
            }
            return Ok(());
        }
        let Some(mut central) = self.central.try_lock() else {
            return Err(HeapError::WouldBlock);
        };
        let Some(slot) = central.slots_mut().get_mut(allocation.index as usize) else {
            return Err(HeapError::InvalidHandle);
        };
        if !slot.used || slot.generation != allocation.generation {
            return Err(HeapError::StaleHandle);
        }
        if slot.owner != allocation.owner {
            return Err(HeapError::WrongOwner);
        }
        if slot.address != allocation.address || slot.bytes != allocation.bytes {
            return Err(HeapError::InvalidHandle);
        }
        let Some(frame) = slot.frame else {
            return Err(HeapError::InvalidHandle);
        };
        self.frames.free(cpu, frame).map_err(map_alloc_error)?;
        slot.used = false;
        slot.frame = None;
        slot.address = 0;
        if let Some(quota) = self.quotas.get_mut(allocation.owner.raw() as usize) {
            quota.used = quota.used.saturating_sub(allocation.bytes);
        }
        if let Some(mut cache) = self.caches[cpu].try_lock() {
            let _ = cache.push(allocation.index as u16);
        }
        Ok(())
    }

    /// Release a slab-backed allocation by its identity-mapped address.
    ///
    /// Large frame-batch allocations remain handle-owned until their pointer
    /// record is added to the global allocator path.
    pub fn free_address(&mut self, cpu: usize, address: usize) -> Result<(), HeapError> {
        if address == 0 {
            return Err(HeapError::InvalidHandle);
        }
        let slab_allocation = {
            let central = self.central.try_lock().ok_or(HeapError::WouldBlock)?;
            central
                .slots()
                .iter()
                .enumerate()
                .find(|(_, slot)| slot.used && slot.address == address)
                .map(|(index, slot)| HeapAllocation {
                    kind: HeapAllocationKind::Slab,
                    index: index as u32,
                    generation: slot.generation,
                    owner: slot.owner,
                    bytes: slot.bytes,
                    address: slot.address,
                    pages: None,
                })
        };
        if let Some(allocation) = slab_allocation {
            return self.free(cpu, allocation);
        }
        if let Some((index, record)) = self
            .page_records
            .iter()
            .enumerate()
            .find(|(_, record)| record.used && record.address == address)
        {
            let batch = self.page_batch(*record)?;
            let allocation = HeapAllocation {
                kind: HeapAllocationKind::Pages,
                index: index as u32,
                generation: record.generation,
                owner: record.owner,
                bytes: record.bytes,
                address: record.address,
                pages: Some(batch),
            };
            return self.free(cpu, allocation);
        }
        Err(HeapError::InvalidHandle)
    }

    fn find_lease_range(&self, pages: usize) -> Option<usize> {
        if pages == 0 || pages > self.page_leases.len() {
            return None;
        }
        (0..=self.page_leases.len() - pages).find(|start| {
            self.page_leases[*start..*start + pages].iter().all(|record| !record.used)
        })
    }

    fn page_batch(&self, record: HeapPageRecord) -> Result<FrameBatch, HeapError> {
        let start = record.lease_start as usize;
        let count = record.lease_count as usize;
        let end = start.checked_add(count).ok_or(HeapError::InvalidHandle)?;
        let leases = self.page_leases.get(start..end).ok_or(HeapError::InvalidHandle)?;
        let mut batch = FrameBatch::empty();
        for slot in leases {
            if !slot.used {
                return Err(HeapError::InvalidHandle);
            }
            batch
                .push(slot.lease.ok_or(HeapError::InvalidHandle)?)
                .map_err(|_| HeapError::InvalidHandle)?;
        }
        Ok(batch)
    }

    pub fn live_bytes(&self, owner: OwnerId) -> Option<usize> {
        let central = self.central.try_lock()?;
        let slab_bytes = central
            .slots()
            .iter()
            .filter(|slot| slot.used && slot.owner == owner)
            .map(|slot| slot.bytes)
            .sum::<usize>();
        let page_bytes = self
            .page_records
            .iter()
            .filter(|record| record.used && record.owner == owner)
            .map(|record| record.bytes)
            .sum::<usize>();
        Some(slab_bytes.saturating_add(page_bytes))
    }

    pub fn live_objects(&self, owner: OwnerId) -> Option<usize> {
        let central = self.central.try_lock()?;
        let slab_objects =
            central.slots().iter().filter(|slot| slot.used && slot.owner == owner).count();
        let page_objects =
            self.page_records.iter().filter(|record| record.used && record.owner == owner).count();
        Some(slab_objects + page_objects)
    }
}

/// `GlobalAlloc` adapter for a caller-owned kernel heap.
///
/// The adapter uses the current CPU on UEFI and CPU zero in host tests. It is
/// installed after the post-UEFI frame allocator has been bound.
pub struct KernelGlobalAllocator<'a> {
    heap: TryLock<Option<KernelHeap<'a>>>,
    pressure: TryLock<PressureManager>,
}

#[cfg(target_os = "uefi")]
#[global_allocator]
pub(crate) static KERNEL_GLOBAL_ALLOCATOR: KernelGlobalAllocator<'static> =
    KernelGlobalAllocator::empty();

impl<'a> KernelGlobalAllocator<'a> {
    pub const fn new(heap: KernelHeap<'a>) -> Self {
        Self { heap: TryLock::new(Some(heap)), pressure: TryLock::new(PressureManager::new()) }
    }

    pub const fn empty() -> Self {
        Self { heap: TryLock::new(None), pressure: TryLock::new(PressureManager::new()) }
    }

    pub fn bind(&self, heap: KernelHeap<'a>) -> Result<(), HeapError> {
        let Some(mut bound) = self.heap.try_lock() else {
            return Err(HeapError::WouldBlock);
        };
        if bound.is_some() {
            return Err(HeapError::InvalidRequest);
        }
        *bound = Some(heap);
        Ok(())
    }

    pub fn is_bound(&self) -> bool {
        self.heap.try_lock().is_some_and(|heap| heap.is_some())
    }

    pub fn register_reclaimer(&self, callback: ReclaimCallback) -> Result<(), FrameError> {
        let Some(mut pressure) = self.pressure.try_lock() else {
            return Err(FrameError::Capacity);
        };
        pressure.register_reclaimer(callback)
    }
}

#[cfg(target_os = "uefi")]
pub(crate) fn bind_kernel_global_allocator(frames: &SmpFrameAllocator) -> Result<(), HeapError> {
    let frames: &'static SmpFrameAllocator = unsafe { core::mem::transmute(frames) };
    let (slots, records, leases) = frames.heap_metadata().ok_or(HeapError::InvalidRequest)?;
    let (slots, records, leases): (
        &'static mut [HeapSlot],
        &'static mut [HeapPageRecord],
        &'static mut [HeapLeaseRecord],
    ) = unsafe { core::mem::transmute((slots, records, leases)) };
    KERNEL_GLOBAL_ALLOCATOR
        .register_reclaimer(reclaim_kernel_frame_caches)
        .map_err(|_| HeapError::InvalidRequest)?;
    KERNEL_GLOBAL_ALLOCATOR.bind(KernelHeap::new(frames, slots, records, leases))
}

#[cfg(target_os = "uefi")]
fn reclaim_kernel_frame_caches(_level: PressureLevel) -> usize {
    let Some(bound) = KERNEL_GLOBAL_ALLOCATOR.heap.try_lock() else {
        return 0;
    };
    let Some(heap) = bound.as_ref() else {
        return 0;
    };
    let before = heap.frames.available();
    for cpu in 0..MAX_MEMORY_CPUS {
        heap.frames.flush_cpu_cache(cpu);
    }
    heap.frames.available().saturating_sub(before)
}

#[cfg(target_os = "uefi")]
pub(crate) fn kernel_global_allocator_bound() -> bool {
    KERNEL_GLOBAL_ALLOCATOR.is_bound()
}

fn global_allocator_cpu() -> usize {
    #[cfg(target_os = "uefi")]
    {
        crate::current_cpu()
    }
    #[cfg(not(target_os = "uefi"))]
    {
        0
    }
}

unsafe impl GlobalAlloc for KernelGlobalAllocator<'_> {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        match self.alloc_layout(layout) {
            Ok(pointer) => pointer,
            Err(HeapError::Exhausted) => {
                let reclaimed = self.notify_reclaim(PressureLevel::Warning);
                if reclaimed == 0 {
                    let _ = self.notify_reclaim(PressureLevel::Critical);
                }
                match self.alloc_layout(layout) {
                    Ok(pointer) => pointer,
                    Err(HeapError::Exhausted) => {
                        #[cfg(target_os = "uefi")]
                        crate::arch_fatal(b"LogOS vNext: kernel allocation");
                        #[cfg(not(target_os = "uefi"))]
                        return core::ptr::null_mut();
                    }
                    Err(_) => core::ptr::null_mut(),
                }
            }
            Err(_) => core::ptr::null_mut(),
        }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, _layout: Layout) {
        if pointer.is_null() {
            return;
        }
        if let Some(mut bound) = self.heap.try_lock() {
            if let Some(heap) = bound.as_mut() {
                let _ = heap.free_address(global_allocator_cpu(), pointer as usize);
            }
        }
    }
}

impl KernelGlobalAllocator<'_> {
    /// Nonblocking IRQ allocation. This never calls `GlobalAlloc`, waits, or
    /// invokes pressure reclaim; callers must handle `WouldBlock`/`Exhausted`.
    pub fn try_alloc_irq(&self, layout: Layout) -> Result<*mut u8, HeapError> {
        if layout.size() == 0 {
            return Ok(layout.align() as *mut u8);
        }
        let Some(mut bound) = self.heap.try_lock() else {
            return Err(HeapError::WouldBlock);
        };
        let Some(heap) = bound.as_mut() else {
            return Err(HeapError::InvalidRequest);
        };
        heap.try_alloc_irq(global_allocator_cpu(), OwnerId::KERNEL, layout)
            .map(|allocation| allocation.address() as *mut u8)
    }

    fn alloc_layout(&self, layout: Layout) -> Result<*mut u8, HeapError> {
        if layout.size() == 0 {
            return Ok(layout.align() as *mut u8);
        }
        let Some(mut bound) = self.heap.try_lock() else {
            return Err(HeapError::WouldBlock);
        };
        let Some(heap) = bound.as_mut() else {
            return Err(HeapError::InvalidRequest);
        };
        heap.alloc(
            global_allocator_cpu(),
            OwnerId::KERNEL,
            layout.size(),
            layout.align(),
            AllocationContext::Thread,
        )
        .map(|allocation| allocation.address() as *mut u8)
    }

    fn notify_reclaim(&self, level: PressureLevel) -> usize {
        self.pressure.try_lock().map(|pressure| pressure.notify(level)).unwrap_or(0)
    }
}

fn map_alloc_error(error: AllocationError) -> HeapError {
    match error {
        AllocationError::WouldBlock => HeapError::WouldBlock,
        AllocationError::Exhausted => HeapError::Exhausted,
        AllocationError::StaleHandle => HeapError::StaleHandle,
        AllocationError::WrongOwner => HeapError::WrongOwner,
        _ => HeapError::InvalidHandle,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PressureLevel {
    Normal,
    Warning,
    Critical,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClaimKind {
    Pinned,
    Dma,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MemoryClaim {
    pub frame: FrameId,
    pub owner: OwnerId,
    pub kind: ClaimKind,
}

pub type ReclaimCallback = fn(PressureLevel) -> usize;

pub struct PressureManager {
    claims: [Option<MemoryClaim>; MAX_MEMORY_CLAIMS],
    reclaimers: [Option<ReclaimCallback>; MAX_RECLAIMERS],
    reservations: AtomicUsize,
}

impl PressureManager {
    pub const fn new() -> Self {
        Self {
            claims: [None; MAX_MEMORY_CLAIMS],
            reclaimers: [None; MAX_RECLAIMERS],
            reservations: AtomicUsize::new(0),
        }
    }

    pub fn track(&mut self, claim: MemoryClaim) -> Result<(), FrameError> {
        let Some(slot) = self.claims.iter_mut().find(|slot| slot.is_none()) else {
            return Err(FrameError::Capacity);
        };
        *slot = Some(claim);
        Ok(())
    }

    pub fn untrack(&mut self, claim: MemoryClaim) -> bool {
        let Some(slot) = self.claims.iter_mut().find(|slot| **slot == Some(claim)) else {
            return false;
        };
        *slot = None;
        true
    }

    pub fn register_reclaimer(&mut self, callback: ReclaimCallback) -> Result<(), FrameError> {
        let Some(slot) = self.reclaimers.iter_mut().find(|slot| slot.is_none()) else {
            return Err(FrameError::Capacity);
        };
        *slot = Some(callback);
        Ok(())
    }

    pub fn notify(&self, level: PressureLevel) -> usize {
        self.reclaimers.iter().flatten().map(|callback| callback(level)).sum()
    }

    pub fn reserve_pages(&self, pages: usize) {
        self.reservations.fetch_add(pages, Ordering::AcqRel);
    }

    pub fn release_reserved_pages(&self, pages: usize) {
        self.reservations.fetch_sub(pages, Ordering::AcqRel);
    }

    pub fn reserved_pages(&self) -> usize {
        self.reservations.load(Ordering::Acquire)
    }
}

impl Default for PressureManager {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BootTiming {
    pub started: u64,
    pub completed: u64,
}

pub struct BootPhaseTimer {
    started: AtomicU64,
    completed: AtomicU64,
}

impl BootPhaseTimer {
    pub const fn new() -> Self {
        Self { started: AtomicU64::new(0), completed: AtomicU64::new(0) }
    }

    pub fn start(&self, ticks: u64) {
        self.started.store(ticks, Ordering::Release);
    }

    pub fn complete(&self, ticks: u64) {
        self.completed.store(ticks, Ordering::Release);
    }

    pub fn snapshot(&self) -> BootTiming {
        BootTiming {
            started: self.started.load(Ordering::Acquire),
            completed: self.completed.load(Ordering::Acquire),
        }
    }
}

impl Default for BootPhaseTimer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boot_resources::MemoryDescriptor;
    use std::{
        pin::Pin,
        sync::{Arc, atomic::AtomicUsize},
        task::{Context, Wake, Waker},
        thread,
    };

    fn map(entries: &[(u64, u64, bool)]) -> MemoryMap {
        let mut map = MemoryMap::new();
        for (start, pages, available) in entries {
            map.push(MemoryDescriptor::new(*start, *pages, *available).unwrap()).unwrap();
        }
        map
    }

    static RECLAIM_CALLS: AtomicUsize = AtomicUsize::new(0);

    fn reclaim_none(_level: PressureLevel) -> usize {
        RECLAIM_CALLS.fetch_add(1, Ordering::Relaxed);
        0
    }

    fn manager(entries: &[(u64, u64, bool)]) -> PhysicalFrameManager {
        let normalized = normalize_memory_map(&map(entries), &[]).unwrap();
        let mut manager = PhysicalFrameManager::empty();
        manager.initialize(&normalized).unwrap();
        manager
    }

    #[test]
    fn normalization_sorts_merges_and_excludes_ranges() {
        let source =
            map(&[(0x9000, 3, true), (0x1000, 4, true), (0x5000, 1, false), (0x7000, 2, true)]);
        let exclusion = MemoryExclusion::new(0x2000, 1, ExclusionKind::Kernel).unwrap();
        let normalized = normalize_memory_map(&source, &[exclusion]).unwrap();
        assert_eq!(normalized.get(0), PhysicalRun::new(0x1000, 1));
        assert_eq!(normalized.get(1), PhysicalRun::new(0x3000, 2));
        assert_eq!(normalized.get(2), PhysicalRun::new(0x7000, 5));
    }

    #[test]
    fn metadata_sizing_is_page_rounded_and_capacity_checked() {
        let pages = frame_metadata_pages_for_frames(128).unwrap();
        assert!(pages > 0);
        let bytes = pages * PAGE_SIZE;
        let region = FrameMetadataRegion::new(0x20_0000, bytes).unwrap();
        assert!(region.capacity().unwrap() >= 128);
        assert!(region.layout(128).unwrap().bytes() <= bytes as usize);
        assert!(FrameMetadataRegion::new(u64::MAX - PAGE_SIZE + 1, PAGE_SIZE).is_none());
    }

    #[test]
    fn metadata_sizing_rejects_unrepresentable_frame_counts() {
        assert!(frame_metadata_pages_for_frames(u64::MAX).is_none());
    }

    #[test]
    fn frame_ids_are_compact_and_lookup_is_indexed() {
        let manager = manager(&[(0x1000, 4, true), (0x9000, 2, true)]);
        let id = manager.id_for_address(0x9000).unwrap();
        assert_eq!(id.run(), 1);
        assert_eq!(id.offset(), 0);
        assert_eq!(manager.address(id).unwrap().raw(), 0x9000);
    }

    #[test]
    fn generation_and_owner_reject_stale_frame_leases() {
        let mut manager = manager(&[(0x1000, 2, true)]);
        let owner = OwnerId::new(2).unwrap();
        let first = manager.try_alloc(owner, FrameState::Dirty).unwrap();
        manager.free(first).unwrap();
        assert_eq!(manager.free(first), Err(FrameError::StaleHandle));
        let second = manager.try_alloc(owner, FrameState::Dirty).unwrap();
        assert_ne!(first.generation(), second.generation());
        assert_eq!(manager.free(first), Err(FrameError::StaleHandle));
    }

    #[test]
    fn reservation_and_batch_operations_are_bounded() {
        let mut manager = manager(&[(0x1000, 8, true)]);
        let owner = OwnerId::KERNEL;
        let reserved = manager.reserve(FrameAddress::from_raw(0x1000), owner).unwrap();
        assert_eq!(reserved.state(), FrameState::Reserved);
        manager.release_reservation(reserved).unwrap();
        let batch = manager.alloc_batch(owner, 4, FrameState::Zeroed).unwrap();
        assert_eq!(batch.len(), 4);
        manager.free_batch(batch).unwrap();
    }

    #[test]
    fn virtual_operations_preserve_space_generations() {
        let mut vm = VirtualMemoryManager::new();
        let space = vm.create().unwrap();
        let frame = FrameId::new(0, 0).unwrap();
        vm.map(space, 0x4000, frame, VmFlags::DATA).unwrap();
        vm.protect(space, 0x4000, VmFlags::READ_ONLY).unwrap();
        assert_eq!(vm.query(space, 0x4000).unwrap().flags(), VmFlags::READ_ONLY);
        vm.destroy(space).unwrap();
        assert_eq!(vm.query(space, 0x4000), None);
        let replacement = vm.create().unwrap();
        assert_ne!(space.generation(), replacement.generation());
    }

    #[test]
    fn tlb_ticket_completes_only_after_all_acknowledgements() {
        let coordinator = TlbCoordinator::new();
        let space = AddressSpaceId { slot: 0, generation: 1 };
        let ticket = coordinator.enqueue(0, 0b11, space, 0x4000, 1).unwrap();
        let mut entries =
            [TlbInvalidation { sequence: 0, address_space: space, virtual_address: 0, pages: 0 };
                2];
        let count = coordinator.drain(1, &mut entries).unwrap();
        assert_eq!(count, 1);
        coordinator.acknowledge(0, ticket.sequence).unwrap();
        assert!(!coordinator.complete(ticket));
        coordinator.acknowledge(1, ticket.sequence).unwrap();
        assert!(coordinator.complete(ticket));
    }

    #[test]
    fn smp_allocator_reuses_frames_and_records_stats() {
        let normalized = normalize_memory_map(&map(&[(0x1000, 64, true)]), &[]).unwrap();
        let mut allocator = SmpFrameAllocator::empty();
        allocator.initialize(&normalized).unwrap();
        let owner = OwnerId::new(2).unwrap();
        let lease = allocator.try_alloc(0, owner, FrameState::Dirty).unwrap();
        allocator.free(1, lease).unwrap();
        let replacement = allocator.try_alloc(0, owner, FrameState::Dirty).unwrap();
        assert_ne!(lease.generation(), replacement.generation());
        assert_eq!(allocator.free(0, lease), Err(AllocationError::StaleHandle));
        allocator.free(0, replacement).unwrap();
        assert!(allocator.stats().remote_frees >= 1);
    }

    #[test]
    fn async_deadline_wakes_a_preallocated_waiter() {
        let normalized = normalize_memory_map(&map(&[(0x1000, 1, true)]), &[]).unwrap();
        let mut allocator = SmpFrameAllocator::empty();
        allocator.initialize(&normalized).unwrap();
        let owner = OwnerId::new(2).unwrap();
        let held = allocator.try_alloc(0, owner, FrameState::Dirty).unwrap();
        let mut future = allocator.alloc_async(0, owner, FrameState::Dirty, Some(10));
        struct WakeCounter(AtomicUsize);
        impl Wake for WakeCounter {
            fn wake(self: Arc<Self>) {
                self.0.fetch_add(1, Ordering::Relaxed);
            }
        }
        let wake = Arc::new(WakeCounter(AtomicUsize::new(0)));
        let waker = Waker::from(Arc::clone(&wake));
        let mut context = Context::from_waker(&waker);
        let first = Pin::new(&mut future).poll(&mut context);
        assert_eq!(first, Poll::Pending);
        allocator.advance_time(10);
        assert_eq!(
            Pin::new(&mut future).poll(&mut context),
            Poll::Ready(Err(AllocationError::Deadline))
        );
        assert_eq!(wake.0.load(Ordering::Relaxed), 1);
        allocator.free(0, held).unwrap();
    }

    #[test]
    fn heap_enforces_interrupt_and_quota_boundaries() {
        let normalized = normalize_memory_map(&map(&[(0x1000, 16, true)]), &[]).unwrap();
        let mut allocator = SmpFrameAllocator::empty();
        allocator.initialize(&normalized).unwrap();
        let owner = OwnerId::new(2).unwrap();
        let mut heap_slots = [HeapSlot::empty(); 2];
        let mut page_records = [HeapPageRecord::empty(); 2];
        let mut page_leases = [HeapLeaseRecord::empty(); 16];
        let mut heap =
            KernelHeap::new(&allocator, &mut heap_slots, &mut page_records, &mut page_leases);
        heap.set_quota(owner, 8192).unwrap();
        assert_eq!(
            heap.alloc(0, owner, 32, 8, AllocationContext::Interrupt),
            Err(HeapError::InterruptContext)
        );
        let small = heap.alloc(0, owner, 32, 8, AllocationContext::Thread).unwrap();
        assert_eq!(small.kind(), HeapAllocationKind::Slab);
        assert_ne!(small.address(), 0);
        assert_eq!(small.address() % PAGE_SIZE as usize, 0);
        heap.free_address(0, small.address()).unwrap();
        assert_eq!(heap.free_address(0, small.address()), Err(HeapError::InvalidHandle));
        let large = heap.alloc(0, owner, 8192, 4096, AllocationContext::Thread).unwrap();
        assert_eq!(large.kind(), HeapAllocationKind::Pages);
        assert_ne!(large.address(), 0);
        assert_eq!(large.address() % PAGE_SIZE as usize, 0);
        heap.free_address(0, large.address()).unwrap();
        assert_eq!(heap.free_address(0, large.address()), Err(HeapError::InvalidHandle));
    }

    #[test]
    fn irq_heap_allocation_only_consumes_cached_small_frames() {
        let normalized = normalize_memory_map(&map(&[(0x1000, 16, true)]), &[]).unwrap();
        let mut allocator = SmpFrameAllocator::empty();
        allocator.initialize(&normalized).unwrap();
        let owner = OwnerId::KERNEL;
        let cached = allocator.try_alloc(0, owner, FrameState::Dirty).unwrap();
        allocator.free(0, cached).unwrap();

        let mut heap_slots = [HeapSlot::empty(); 2];
        let mut page_records = [HeapPageRecord::empty(); 2];
        let mut page_leases = [HeapLeaseRecord::empty(); 16];
        let mut heap =
            KernelHeap::new(&allocator, &mut heap_slots, &mut page_records, &mut page_leases);
        heap.set_quota(owner, 8192).unwrap();
        let small = Layout::from_size_align(32, 8).unwrap();
        let allocation = heap.try_alloc_irq(0, owner, small).unwrap();
        assert_eq!(allocation.kind(), HeapAllocationKind::Slab);
        heap.free(0, allocation).unwrap();

        let large = Layout::from_size_align(8192, PAGE_SIZE as usize).unwrap();
        assert_eq!(heap.try_alloc_irq(0, owner, large), Err(HeapError::WouldBlock));
    }

    #[test]
    fn heap_page_metadata_scales_with_reserved_capacity() {
        let normalized = normalize_memory_map(&map(&[(0x1000, 32, true)]), &[]).unwrap();
        let mut allocator = SmpFrameAllocator::empty();
        allocator.initialize(&normalized).unwrap();
        let owner = OwnerId::new(2).unwrap();
        let mut heap_slots = [HeapSlot::empty(); 17];
        let mut page_records = [HeapPageRecord::empty(); 17];
        let mut page_leases = [HeapLeaseRecord::empty(); 17];
        let mut heap =
            KernelHeap::new(&allocator, &mut heap_slots, &mut page_records, &mut page_leases);
        heap.set_quota(owner, 17 * 513).unwrap();
        let mut allocations = [None; 17];
        for allocation in &mut allocations {
            *allocation = Some(heap.alloc(0, owner, 513, 8, AllocationContext::Thread).unwrap());
        }
        assert!(heap.alloc(0, owner, 513, 8, AllocationContext::Thread).is_err());
        for allocation in allocations.into_iter().flatten() {
            heap.free(0, allocation).unwrap();
        }
        assert_eq!(heap.live_objects(owner), Some(0));
    }

    #[test]
    fn global_allocator_adapter_tracks_layout_addresses() {
        let normalized = normalize_memory_map(&map(&[(0x1000, 16, true)]), &[]).unwrap();
        let mut allocator = SmpFrameAllocator::empty();
        allocator.initialize(&normalized).unwrap();
        let unbound = KernelGlobalAllocator::empty();
        let small_layout = Layout::from_size_align(32, 8).unwrap();
        assert!(!unbound.is_bound());
        assert!(unsafe { GlobalAlloc::alloc(&unbound, small_layout) }.is_null());
        let mut heap_slots = [HeapSlot::empty(); 2];
        let mut page_records = [HeapPageRecord::empty(); 2];
        let mut page_leases = [HeapLeaseRecord::empty(); 16];
        let heap =
            KernelHeap::new(&allocator, &mut heap_slots, &mut page_records, &mut page_leases);
        let global = KernelGlobalAllocator::empty();
        assert!(!global.is_bound());
        global.bind(heap).unwrap();
        assert!(global.is_bound());
        let mut second_slots = [HeapSlot::empty(); 1];
        let mut second_records = [HeapPageRecord::empty(); 1];
        let mut second_leases = [HeapLeaseRecord::empty(); 8];
        assert_eq!(
            global.bind(KernelHeap::new(
                &allocator,
                &mut second_slots,
                &mut second_records,
                &mut second_leases,
            )),
            Err(HeapError::InvalidRequest)
        );

        let small = unsafe { GlobalAlloc::alloc(&global, small_layout) };
        assert!(!small.is_null());
        assert_eq!(small as usize % small_layout.align(), 0);
        unsafe { GlobalAlloc::dealloc(&global, small, small_layout) };

        let large_layout = Layout::from_size_align(8192, PAGE_SIZE as usize).unwrap();
        let large = unsafe { GlobalAlloc::alloc(&global, large_layout) };
        assert!(!large.is_null());
        assert_eq!(large as usize % large_layout.align(), 0);
        unsafe { GlobalAlloc::dealloc(&global, large, large_layout) };

        let unsupported = Layout::from_size_align(32, (PAGE_SIZE * 2) as usize).unwrap();
        assert!(unsafe { GlobalAlloc::alloc(&global, unsupported) }.is_null());
    }

    #[test]
    fn global_allocator_retries_warning_then_critical_reclaim() {
        let normalized = normalize_memory_map(&map(&[(0x1000, 16, true)]), &[]).unwrap();
        let mut allocator = SmpFrameAllocator::empty();
        allocator.initialize(&normalized).unwrap();
        let mut heap_slots = [HeapSlot::empty(); 2];
        let mut page_records = [HeapPageRecord::empty(); 2];
        let mut page_leases = [HeapLeaseRecord::empty(); 16];
        let global = KernelGlobalAllocator::new(KernelHeap::new(
            &allocator,
            &mut heap_slots,
            &mut page_records,
            &mut page_leases,
        ));
        global.register_reclaimer(reclaim_none).unwrap();
        RECLAIM_CALLS.store(0, Ordering::Relaxed);

        let layout = Layout::from_size_align(8192, PAGE_SIZE as usize).unwrap();
        let first = unsafe { GlobalAlloc::alloc(&global, layout) };
        let second = unsafe { GlobalAlloc::alloc(&global, layout) };
        assert!(!first.is_null() && !second.is_null());
        assert!(unsafe { GlobalAlloc::alloc(&global, layout) }.is_null());
        assert_eq!(RECLAIM_CALLS.load(Ordering::Relaxed), 2);
        unsafe {
            GlobalAlloc::dealloc(&global, first, layout);
            GlobalAlloc::dealloc(&global, second, layout);
        }
    }

    #[test]
    fn tlb_batches_coalesce_adjacent_invalidations() {
        let space = AddressSpaceId { slot: 0, generation: 1 };
        let mut batch = TlbBatch::new();
        batch
            .push(TlbBatchEntry { address_space: space, virtual_address: 0x4000, pages: 1 })
            .unwrap();
        batch
            .push(TlbBatchEntry { address_space: space, virtual_address: 0x5000, pages: 2 })
            .unwrap();
        assert_eq!(batch.len(), 1);
        assert_eq!(batch.get(0).unwrap().pages, 3);
        let coordinator = TlbCoordinator::new();
        let ticket = coordinator.enqueue_batch(0, 0b10, batch).unwrap();
        let mut entries =
            [TlbInvalidation { sequence: 0, address_space: space, virtual_address: 0, pages: 0 };
                2];
        assert_eq!(coordinator.drain(1, &mut entries).unwrap(), 1);
        assert_eq!(entries[0].sequence, ticket.sequence);
        assert_eq!(entries[0].pages, 3);
    }

    #[test]
    fn concurrent_alloc_free_stress_keeps_generation_checks_active() {
        let normalized = normalize_memory_map(&map(&[(0x1000, 128, true)]), &[]).unwrap();
        let mut allocator = Arc::new(SmpFrameAllocator::empty());
        // Initialization remains single-owner before worker publication.
        Arc::get_mut(&mut allocator).unwrap().initialize(&normalized).unwrap();
        let mut workers = [const { None }; 4];
        for (cpu, worker) in workers.iter_mut().enumerate() {
            let allocator = Arc::clone(&allocator);
            *worker = Some(thread::spawn(move || {
                let owner = OwnerId::new((cpu + 2) as u16).unwrap();
                for _ in 0..100 {
                    loop {
                        match allocator.try_alloc(cpu, owner, FrameState::Dirty) {
                            Ok(lease) => {
                                loop {
                                    match allocator.free(cpu, lease) {
                                        Ok(()) => break,
                                        Err(AllocationError::WouldBlock) => thread::yield_now(),
                                        Err(error) => panic!("unexpected free error: {error:?}"),
                                    }
                                }
                                break;
                            }
                            Err(AllocationError::WouldBlock) => thread::yield_now(),
                            Err(error) => panic!("unexpected allocation error: {error:?}"),
                        }
                    }
                }
            }));
        }
        for worker in workers.into_iter().flatten() {
            worker.join().unwrap();
        }
        assert_eq!(allocator.stats().allocations, 400);
    }
}
