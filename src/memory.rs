//! Bounded memory contracts shared by the physical, virtual, and kernel heaps.
//!
//! The first implementation deliberately uses fixed metadata and host-testable
//! state machines. Architecture code owns the eventual page-table writes and
//! zeroing; this module owns identity, bounds, ownership, and wakeup contracts.

use core::{
    cell::UnsafeCell,
    future::Future,
    ops::{Deref, DerefMut},
    pin::Pin,
    sync::atomic::{AtomicBool, AtomicU8, AtomicU64, AtomicUsize, Ordering},
    task::{Context, Poll, Waker},
};

use crate::boot_resources::{MemoryDescriptor, MemoryMap, PAGE_SIZE};

pub const MAX_MEMORY_RUNS: usize = logos_abi::MAX_MEMORY_DESCRIPTORS;
pub const MAX_MANAGED_FRAMES: usize = logos_abi::MAX_MANAGED_FRAMES;
pub const FRAME_WORDS: usize = MAX_MANAGED_FRAMES.div_ceil(64);
pub const FRAME_SUMMARY_WORDS: usize = FRAME_WORDS.div_ceil(64);
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
pub const MAX_HEAP_SLOTS: usize = 256;
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

/// Dense per-frame state backed by hierarchical free-word metadata.
pub struct PhysicalFrameManager {
    runs: [RunMetadata; MAX_MEMORY_RUNS],
    run_count: usize,
    frame_count: usize,
    free: [u64; FRAME_WORDS],
    summary: [u64; FRAME_SUMMARY_WORDS],
    summary_summary: u64,
    records: [FrameRecord; MAX_MANAGED_FRAMES],
    owner_live: [u32; MAX_QUOTAS],
}

impl PhysicalFrameManager {
    pub const fn empty() -> Self {
        Self {
            runs: [RunMetadata::EMPTY; MAX_MEMORY_RUNS],
            run_count: 0,
            frame_count: 0,
            free: [0; FRAME_WORDS],
            summary: [0; FRAME_SUMMARY_WORDS],
            summary_summary: 0,
            records: [FrameRecord::EMPTY; MAX_MANAGED_FRAMES],
            owner_live: [0; MAX_QUOTAS],
        }
    }

    pub fn initialize(&mut self, map: &NormalizedMemoryMap) -> Result<(), FrameError> {
        self.runs.fill(RunMetadata::EMPTY);
        self.run_count = 0;
        self.frame_count = 0;
        self.free.fill(0);
        self.summary.fill(0);
        self.summary_summary = 0;
        self.records.fill(FrameRecord::EMPTY);
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
            if pages == 0
                || self.run_count == MAX_MEMORY_RUNS
                || self.frame_count == MAX_MANAGED_FRAMES
            {
                continue;
            }
            let pages = pages.min(MAX_MANAGED_FRAMES - self.frame_count);
            let bounded = PhysicalRun::new(start, pages as u64).ok_or(FrameError::InvalidMap)?;
            self.runs[self.run_count] =
                RunMetadata { run: bounded, first_slot: self.frame_count, pages };
            for slot in self.frame_count..self.frame_count + pages {
                self.records[slot] = FrameRecord::EMPTY;
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

    pub const fn run_count(&self) -> usize {
        self.run_count
    }

    pub fn free_count(&self) -> usize {
        self.available()
    }

    pub fn available(&self) -> usize {
        self.free[..self.frame_count.div_ceil(64)]
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
            let record = &mut self.records[slot];
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
        let record = self.records[slot];
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
        Some(FrameAddress::from_parts(raw, id, self.records[slot].generation))
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
        let record = self.records[slot];
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
            let record = &mut self.records[slot];
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
        let record = &mut self.records[slot];
        record.owner = OwnerId(0);
        record.state = FREE;
        if let Some(live) = self.owner_live.get_mut(owner.raw() as usize) {
            *live = live.saturating_sub(1);
        }
        self.set_free(slot);
    }

    fn validate_lease(&self, lease: FrameLease) -> Result<usize, FrameError> {
        let slot = usize::try_from(lease.slot).map_err(|_| FrameError::InvalidFrame)?;
        let Some(record) = self.records.get(slot).copied() else {
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
            let mut words = self.summary[summary_index];
            if summary_index == first_summary {
                words &= u64::MAX << (first_word % 64);
            }
            if summary_index == last_summary && last_word % 64 != 63 {
                words &= (1u64 << (last_word % 64 + 1)) - 1;
            }
            while words != 0 {
                let word_index = summary_index * 64 + words.trailing_zeros() as usize;
                let mut bits = self.free[word_index];
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
        self.free[slot / 64] & (1u64 << (slot % 64)) != 0
    }

    fn set_free(&mut self, slot: usize) {
        let word = slot / 64;
        self.free[word] |= 1u64 << (slot % 64);
        self.summary[word / 64] |= 1u64 << (word % 64);
        self.summary_summary |= 1u64 << (word / 64);
    }

    fn clear_free(&mut self, slot: usize) {
        let word = slot / 64;
        self.free[word] &= !(1u64 << (slot % 64));
        if self.free[word] == 0 {
            self.summary[word / 64] &= !(1u64 << (word % 64));
            if self.summary[word / 64] == 0 {
                self.summary_summary &= !(1u64 << (word / 64));
            }
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
    index: u16,
    generation: u32,
    owner: OwnerId,
    bytes: usize,
    pages: Option<FrameBatch>,
}

impl HeapAllocation {
    pub const fn kind(self) -> HeapAllocationKind {
        self.kind
    }

    pub const fn bytes(self) -> usize {
        self.bytes
    }

    pub const fn pages(self) -> Option<FrameBatch> {
        self.pages
    }
}

#[derive(Clone, Copy)]
struct HeapSlot {
    used: bool,
    generation: u32,
    owner: OwnerId,
    bytes: usize,
}

impl HeapSlot {
    const EMPTY: Self =
        Self { used: false, generation: INITIAL_GENERATION, owner: OwnerId(0), bytes: 0 };
}

struct HeapCentral {
    slots: [HeapSlot; MAX_HEAP_SLOTS],
}

impl HeapCentral {
    const fn empty() -> Self {
        Self { slots: [HeapSlot::EMPTY; MAX_HEAP_SLOTS] }
    }
}

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

/// Fixed size-class slabs for small objects and frame batches for large ones.
/// It intentionally exposes handles, not a `GlobalAlloc`, until the physical
/// and virtual contracts are proven in the boot path.
pub struct KernelHeap<'a> {
    frames: &'a SmpFrameAllocator,
    central: TryLock<HeapCentral>,
    caches: [TryLock<HeapMagazine>; MAX_MEMORY_CPUS],
    quotas: [Quota; MAX_QUOTAS],
}

impl<'a> KernelHeap<'a> {
    pub const fn new(frames: &'a SmpFrameAllocator) -> Self {
        Self {
            frames,
            central: TryLock::new(HeapCentral::empty()),
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
        if alignment > bytes {
            return Err(HeapError::InvalidRequest);
        }
        let quota = self.quotas.get_mut(owner.raw() as usize).ok_or(HeapError::InvalidRequest)?;
        if quota.used.checked_add(bytes).is_none_or(|used| used > quota.limit) {
            return Err(HeapError::Quota);
        }
        if bytes > 512 {
            let pages = bytes.div_ceil(PAGE_SIZE as usize);
            let batch = self
                .frames
                .alloc_batch(cpu, owner, pages, FrameState::Dirty)
                .map_err(map_alloc_error)?;
            quota.used += bytes;
            return Ok(HeapAllocation {
                kind: HeapAllocationKind::Pages,
                index: 0,
                generation: 0,
                owner,
                bytes,
                pages: Some(batch),
            });
        }
        let Some(mut central) = self.central.try_lock() else {
            return Err(HeapError::WouldBlock);
        };
        let cached = self.caches[cpu].try_lock().and_then(|mut cache| cache.pop());
        let index = cached
            .filter(|index| central.slots.get(*index as usize).is_some_and(|slot| !slot.used))
            .map(|index| index as usize)
            .or_else(|| central.slots.iter().position(|slot| !slot.used));
        let Some(index) = index else {
            return Err(HeapError::Exhausted);
        };
        let slot = &mut central.slots[index];
        slot.used = true;
        slot.generation = next_generation(slot.generation);
        slot.owner = owner;
        slot.bytes = bytes;
        quota.used += bytes;
        Ok(HeapAllocation {
            kind: HeapAllocationKind::Slab,
            index: index as u16,
            generation: slot.generation,
            owner,
            bytes,
            pages: None,
        })
    }

    pub fn free(&mut self, cpu: usize, allocation: HeapAllocation) -> Result<(), HeapError> {
        if allocation.kind == HeapAllocationKind::Pages {
            let Some(batch) = allocation.pages else {
                return Err(HeapError::InvalidHandle);
            };
            self.frames.free_batch(cpu, batch).map_err(map_alloc_error)?;
            if let Some(quota) = self.quotas.get_mut(allocation.owner.raw() as usize) {
                quota.used = quota.used.saturating_sub(allocation.bytes);
            }
            return Ok(());
        }
        let Some(mut central) = self.central.try_lock() else {
            return Err(HeapError::WouldBlock);
        };
        let Some(slot) = central.slots.get_mut(allocation.index as usize) else {
            return Err(HeapError::InvalidHandle);
        };
        if !slot.used || slot.generation != allocation.generation {
            return Err(HeapError::StaleHandle);
        }
        if slot.owner != allocation.owner {
            return Err(HeapError::WrongOwner);
        }
        slot.used = false;
        if let Some(quota) = self.quotas.get_mut(allocation.owner.raw() as usize) {
            quota.used = quota.used.saturating_sub(allocation.bytes);
        }
        if let Some(mut cache) = self.caches[cpu].try_lock() {
            let _ = cache.push(allocation.index);
        }
        Ok(())
    }

    pub fn live_bytes(&self, owner: OwnerId) -> Option<usize> {
        let central = self.central.try_lock()?;
        Some(
            central
                .slots
                .iter()
                .filter(|slot| slot.used && slot.owner == owner)
                .map(|slot| slot.bytes)
                .sum(),
        )
    }

    pub fn live_objects(&self, owner: OwnerId) -> Option<usize> {
        let central = self.central.try_lock()?;
        Some(central.slots.iter().filter(|slot| slot.used && slot.owner == owner).count())
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
        let mut heap = KernelHeap::new(&allocator);
        heap.set_quota(owner, 8192).unwrap();
        assert_eq!(
            heap.alloc(0, owner, 32, 8, AllocationContext::Interrupt),
            Err(HeapError::InterruptContext)
        );
        let small = heap.alloc(0, owner, 32, 8, AllocationContext::Thread).unwrap();
        assert_eq!(small.kind(), HeapAllocationKind::Slab);
        heap.free(0, small).unwrap();
        let large = heap.alloc(0, owner, 8192, 4096, AllocationContext::Thread).unwrap();
        assert_eq!(large.kind(), HeapAllocationKind::Pages);
        heap.free(0, large).unwrap();
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
