#![allow(dead_code)]

use logos_abi::{MAX_OBJECT_NAME, NamespaceId, PAGE_SIZE, StoreRequest, VersionSelector};
use logos_core::resource::{ResourceHandle, ResourcePool};

pub const fn storage_lease_owner(caller: u64) -> u64 {
    if caller == 0 { u64::MAX } else { caller }
}

pub struct StorageTransactionPool<const N: usize> {
    leases: ResourcePool<N>,
}

impl<const N: usize> StorageTransactionPool<N> {
    pub const fn new() -> Self {
        Self { leases: ResourcePool::new() }
    }

    pub fn acquire(&mut self, owner: u64) -> Option<ResourceHandle> {
        self.leases.acquire(owner)
    }

    pub fn owns(&self, owner: u64, handle: ResourceHandle) -> bool {
        self.leases.owns(owner, handle)
    }

    pub fn release(&mut self, owner: u64, handle: ResourceHandle) -> bool {
        self.leases.release(owner, handle)
    }

    pub fn reclaim(&mut self, owner: u64) -> usize {
        self.leases.reclaim(owner)
    }
}

impl<const N: usize> Default for StorageTransactionPool<N> {
    fn default() -> Self {
        Self::new()
    }
}

pub struct ReplaceTransaction {
    caller: u64,
    namespace: NamespaceId,
    name: [u8; MAX_OBJECT_NAME],
    name_length: u8,
    length: usize,
    written: usize,
    bytes: [u8; PAGE_SIZE],
}

impl ReplaceTransaction {
    pub fn begin(request: StoreRequest, caller: u64) -> Option<Self> {
        (caller != 0 && request.length != 0 && request.length as usize <= PAGE_SIZE).then_some(
            Self {
                caller,
                namespace: request.namespace,
                name: request.name,
                name_length: request.name_length,
                length: request.length as usize,
                written: 0,
                bytes: [0; PAGE_SIZE],
            },
        )
    }

    pub fn write(&mut self, request: StoreRequest, caller: u64, page: &[u8]) -> bool {
        if request.id == 0
            || self.caller != caller
            || request.offset != self.written as u64
            || request.length == 0
            || !request.offset.checked_add(request.length as u64).is_some_and(|end| {
                end <= self.length as u64 && request.length as usize <= page.len()
            })
        {
            return false;
        }
        let length = request.length as usize;
        self.bytes[self.written..self.written + length].copy_from_slice(&page[..length]);
        self.written += length;
        true
    }

    pub fn complete(&self) -> bool {
        self.written == self.length
    }

    pub fn namespace(&self) -> NamespaceId {
        self.namespace
    }

    pub fn name(&self) -> &[u8] {
        &self.name[..self.name_length as usize]
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes[..self.length]
    }

    pub fn raw_parts(&self) -> (NamespaceId, *const u8, usize, *const u8, usize) {
        (
            self.namespace,
            self.name.as_ptr(),
            self.name_length as usize,
            self.bytes.as_ptr(),
            self.length,
        )
    }

    /// # Safety
    /// `pointer` must point to a live, aligned transaction in the Store context.
    pub unsafe fn complete_at(pointer: *const Self) -> bool {
        unsafe { (*pointer).written == (*pointer).length }
    }

    /// # Safety
    /// `pointer` must point to a live, aligned transaction in the Store context.
    pub unsafe fn raw_parts_at(
        pointer: *const Self,
    ) -> (NamespaceId, *const u8, usize, *const u8, usize) {
        unsafe { (*pointer).raw_parts() }
    }
}

#[derive(Clone, Copy)]
pub struct ReadSelection {
    caller: u64,
    namespace: NamespaceId,
    name: [u8; MAX_OBJECT_NAME],
    name_length: u8,
    version: VersionSelector,
    length: usize,
}

impl ReadSelection {
    pub fn new(request: StoreRequest, length: usize, caller: u64) -> Self {
        Self {
            caller,
            namespace: request.namespace,
            name: request.name,
            name_length: request.name_length,
            version: request.version,
            length,
        }
    }

    pub fn valid_for(&self, request: StoreRequest, caller: u64) -> bool {
        self.caller == caller && request.id != 0
    }

    pub fn namespace(&self) -> NamespaceId {
        self.namespace
    }

    pub fn name(&self) -> &[u8] {
        &self.name[..self.name_length as usize]
    }

    pub fn version(&self) -> VersionSelector {
        self.version
    }

    pub fn length(&self) -> usize {
        self.length
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(operation: logos_abi::StoreOperation, offset: u64, length: u32) -> StoreRequest {
        let mut name = [0; MAX_OBJECT_NAME];
        name[0] = b'x';
        StoreRequest {
            id: 1,
            operation,
            namespace: NamespaceId(1),
            name,
            name_length: 1,
            version: VersionSelector::None,
            offset,
            length,
            page: logos_abi::PageHandle(1),
            deadline: 0,
        }
    }

    #[test]
    fn replace_requires_contiguous_complete_chunks() {
        let mut begin = request(logos_abi::StoreOperation::BeginReplace, 0, 3);
        begin.page = logos_abi::PageHandle(0);
        let mut replace = ReplaceTransaction::begin(begin, 7).unwrap();
        assert!(!replace.write(request(logos_abi::StoreOperation::WriteChunk, 1, 1), 7, b"a"));
        assert!(!replace.write(request(logos_abi::StoreOperation::WriteChunk, 0, 2), 8, b"ab"));
        assert!(replace.write(request(logos_abi::StoreOperation::WriteChunk, 0, 2), 7, b"ab"));
        assert!(!replace.write(request(logos_abi::StoreOperation::WriteChunk, 0, 1), 7, b"c"));
        assert!(replace.write(request(logos_abi::StoreOperation::WriteChunk, 2, 1), 7, b"c"));
        assert!(replace.complete());
        assert_eq!(replace.bytes(), b"abc");
    }

    #[test]
    fn replace_rejects_empty_payloads() {
        let request = request(logos_abi::StoreOperation::BeginReplace, 0, 0);
        assert!(ReplaceTransaction::begin(request, 7).is_none());
    }

    #[test]
    fn read_selection_preserves_namespace_version_and_length() {
        let mut open = request(logos_abi::StoreOperation::OpenRead, 0, 0);
        open.version = VersionSelector::Previous;
        open.page = logos_abi::PageHandle(0);
        let selection = ReadSelection::new(open, 12, 7);
        assert!(selection.valid_for(open, 7));
        assert!(!selection.valid_for(open, 8));
        assert_eq!(selection.namespace(), NamespaceId(1));
        assert_eq!(selection.name(), b"x");
        assert_eq!(selection.version(), VersionSelector::Previous);
        assert_eq!(selection.length(), 12);
    }

    #[test]
    fn transaction_leases_are_owner_and_generation_scoped() {
        let mut pool = StorageTransactionPool::<1>::new();
        let lease = pool.acquire(7).unwrap();

        assert!(pool.owns(7, lease));
        assert!(!pool.owns(8, lease));
        assert_eq!(pool.reclaim(8), 0);
        assert_eq!(pool.reclaim(7), 1);

        let replacement = pool.acquire(7).unwrap();
        assert_ne!(replacement.generation, lease.generation);
        assert!(!pool.owns(7, lease));
    }

    #[test]
    fn core_caller_has_a_reserved_lease_owner() {
        assert_eq!(storage_lease_owner(0), u64::MAX);
        assert_eq!(storage_lease_owner(7), 7);
    }
}
