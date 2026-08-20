//! Stable physical-frame facade.
//!
//! Existing page-table and loader callers keep their `FrameAddress` and
//! `FramePool` interfaces while the implementation uses normalized runs,
//! indexed bitmap metadata, and generation-safe leases.

use crate::{
    boot_resources::MemoryMap,
    memory::{
        FrameBatch, FrameError, FrameLease, FrameMetadataRegion, FrameState, MemoryExclusion,
        NormalizedMemoryMap, OwnerId, PhysicalFrameManager, normalize_memory_map,
    },
};

pub use crate::memory::FrameAddress;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FramePoolError {
    InvalidMap,
    Exhausted,
}

pub struct FramePool {
    manager: PhysicalFrameManager,
}

impl FramePool {
    pub const fn empty() -> Self {
        Self { manager: PhysicalFrameManager::empty() }
    }

    pub fn initialize(&mut self, memory_map: &MemoryMap) -> Result<(), FramePoolError> {
        self.initialize_with_exclusions(memory_map, &[])
    }

    pub fn initialize_with_exclusions(
        &mut self,
        memory_map: &MemoryMap,
        exclusions: &[MemoryExclusion],
    ) -> Result<(), FramePoolError> {
        let normalized =
            normalize_memory_map(memory_map, exclusions).map_err(|_| FramePoolError::InvalidMap)?;
        self.initialize_normalized(&normalized)
    }

    pub fn initialize_normalized(
        &mut self,
        normalized: &NormalizedMemoryMap,
    ) -> Result<(), FramePoolError> {
        self.manager.initialize(normalized).map_err(map_error)
    }

    pub fn initialize_with_metadata(
        &mut self,
        memory_map: &MemoryMap,
        exclusions: &[MemoryExclusion],
        metadata: FrameMetadataRegion,
    ) -> Result<(), FramePoolError> {
        let normalized =
            normalize_memory_map(memory_map, exclusions).map_err(|_| FramePoolError::InvalidMap)?;
        self.manager.initialize_with_region(&normalized, metadata).map_err(map_error)
    }

    pub const fn capacity(&self) -> usize {
        self.manager.frame_count()
    }

    pub fn available(&self) -> usize {
        self.manager.available()
    }

    pub fn allocate(&mut self) -> Result<FrameAddress, FramePoolError> {
        self.allocate_for(OwnerId::KERNEL)
    }

    pub fn allocate_for(&mut self, owner: OwnerId) -> Result<FrameAddress, FramePoolError> {
        self.manager
            .try_alloc(owner, FrameState::Dirty)
            .map(|lease| lease.address())
            .map_err(map_error)
    }

    pub fn try_alloc(
        &mut self,
        owner: OwnerId,
        state: FrameState,
    ) -> Result<FrameLease, FramePoolError> {
        self.manager.try_alloc(owner, state).map_err(map_error)
    }

    pub fn alloc_batch(
        &mut self,
        owner: OwnerId,
        count: usize,
        state: FrameState,
    ) -> Result<FrameBatch, FramePoolError> {
        self.manager.alloc_batch(owner, count, state).map_err(map_error)
    }

    pub fn free(&mut self, lease: FrameLease) -> Result<(), FramePoolError> {
        self.manager.free(lease).map_err(map_error)
    }

    pub fn free_batch(&mut self, batch: FrameBatch) -> Result<(), FramePoolError> {
        self.manager.free_batch(batch).map_err(map_error)
    }

    /// Reserve a boot-owned frame by physical address.
    pub fn reserve(&mut self, frame: FrameAddress) -> bool {
        self.manager.reserve(frame, OwnerId::KERNEL).is_ok()
    }

    pub fn reserve_batch(&mut self, frames: &[FrameAddress]) -> Result<FrameBatch, FramePoolError> {
        self.manager.reserve_batch(frames, OwnerId::KERNEL).map_err(map_error)
    }

    pub fn release_reservation(&mut self, lease: FrameLease) -> Result<(), FramePoolError> {
        self.manager.release_reservation(lease).map_err(map_error)
    }

    /// Legacy release accepts a generation-stamped address returned by
    /// `allocate`; raw addresses remain supported for old boot code through a
    /// lookup and current-generation validation inside the manager.
    pub fn release(&mut self, frame: FrameAddress) -> Result<(), FramePoolError> {
        self.manager.release_address(frame).map_err(map_error)
    }

    pub const fn manager(&self) -> &PhysicalFrameManager {
        &self.manager
    }
}

impl Default for FramePool {
    fn default() -> Self {
        Self::empty()
    }
}

fn map_error(error: FrameError) -> FramePoolError {
    match error {
        FrameError::Exhausted => FramePoolError::Exhausted,
        FrameError::InvalidMap => FramePoolError::InvalidMap,
        FrameError::InvalidFrame
        | FrameError::NotReservation
        | FrameError::StaleHandle
        | FrameError::WrongOwner
        | FrameError::Capacity
        | FrameError::AlreadyUsed => FramePoolError::InvalidMap,
        FrameError::BatchCapacity => FramePoolError::Exhausted,
    }
}
