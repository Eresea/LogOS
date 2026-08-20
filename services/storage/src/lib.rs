#![no_std]

#[cfg(test)]
extern crate std;

mod api;
mod namespace;
mod packages;

pub use api::{StorageApi, error_response};
pub use namespace::{
    DurableNamespace, MAX_COMPONENT_BYTES, MAX_FILE_BLOCKS, MAX_FILE_BYTES, MAX_FILE_EXTENTS,
    MAX_OBJECTS, MAX_PATH_DEPTH, NamespaceError, NamespaceTransaction, ObjectId, ObjectInfo,
    ObjectKind, ObjectList, ObjectNamespace,
};
pub use packages::{
    MAX_PACKAGE_EXTENTS, MAX_PACKAGE_RECORDS, PACKAGE_INSTALL_KIND, PACKAGE_RECORD_BYTES,
    PACKAGE_SNAPSHOT_BYTES, PackageCatalogError, PackageExtent, PackageHandle, PackageInfo,
    PackageInstall, PackageKey,
};

use logos_abi::{
    IpcCapability, IpcStatus, StorageOperation, StorageRequest, StorageResponse, StorageStatus,
};
use logos_storage::{BLOCK_BYTES, Block, BlockError, BlockIndex, BlockStore, ReadMap};

pub const STORAGE_REQUEST_CAPACITY: usize = 8;
const CACHE_SLOTS: usize = logos_abi::STORAGE_CACHE_PAGES;
const CACHE_MAP_MAX_PAGES: usize = 16;

#[allow(dead_code)]
#[derive(Clone, Copy)]
struct CacheSlot {
    block: u64,
    valid: bool,
}

impl CacheSlot {
    const EMPTY: Self = Self { block: 0, valid: false };
}

/// Kernel-owned staging boundary used by the storage service adapter.
pub trait KernelStorageIpc {
    fn send(
        &mut self,
        capability: IpcCapability,
        request: StorageRequest,
        staging: &mut Block,
    ) -> IpcStatus;

    fn receive(
        &mut self,
        capability: IpcCapability,
        response: &mut StorageResponse,
        staging: &mut Block,
    ) -> IpcStatus;
}

pub struct IpcBlockStore<T> {
    transport: T,
    capability: IpcCapability,
    capability_slot: u16,
    generation: u16,
    service_epoch: u64,
    blocks: u64,
    next_request: u32,
    staging: Block,
    cache: [CacheSlot; CACHE_SLOTS],
    pin_count: [u8; CACHE_SLOTS],
    next_cache_slot: usize,
}

impl<T> IpcBlockStore<T> {
    pub fn new(
        transport: T,
        capability: IpcCapability,
        generation: u16,
        service_epoch: u64,
        blocks: u64,
    ) -> Result<Self, BlockError> {
        if generation == 0 || service_epoch == 0 || blocks == 0 {
            return Err(BlockError::InvalidRequest);
        }
        Self::new_with_slot(
            transport,
            capability,
            capability.endpoint as u16,
            generation,
            service_epoch,
            blocks,
        )
    }

    pub fn new_with_slot(
        transport: T,
        capability: IpcCapability,
        capability_slot: u16,
        generation: u16,
        service_epoch: u64,
        blocks: u64,
    ) -> Result<Self, BlockError> {
        if generation == 0 || service_epoch == 0 || blocks == 0 {
            return Err(BlockError::InvalidRequest);
        }
        Ok(Self {
            transport,
            capability,
            capability_slot,
            generation,
            service_epoch,
            blocks,
            next_request: 1,
            staging: Block::zero(),
            cache: [CacheSlot::EMPTY; CACHE_SLOTS],
            pin_count: [0; CACHE_SLOTS],
            next_cache_slot: 0,
        })
    }

    pub fn into_transport(self) -> T {
        self.transport
    }

    fn next_request(&mut self) -> u32 {
        let request = self.next_request;
        self.next_request = self.next_request.wrapping_add(1).max(1);
        request
    }

    fn round_trip(&mut self, request: StorageRequest) -> Result<StorageResponse, BlockError>
    where
        T: KernelStorageIpc,
    {
        map_ipc_status(self.transport.send(self.capability, request, &mut self.staging))?;
        let mut response = StorageResponse::new(
            request.request_id,
            StorageStatus::Invalid,
            request.generation,
            0,
            0,
            request.transaction_id,
        );
        map_ipc_status(self.transport.receive(self.capability, &mut response, &mut self.staging))?;
        if response.reserved != 0
            || response.request_id != request.request_id
            || response.generation != request.generation
            || response.transaction_id != request.transaction_id
        {
            return Err(BlockError::InvalidRequest);
        }
        if response.status == StorageStatus::Ok
            && (response.blocks_completed != u16::from(request.is_block_io())
                || response.payload_bytes
                    != if request.is_block_io() { BLOCK_BYTES as u16 } else { 0 })
        {
            return Err(BlockError::InvalidRequest);
        }
        map_storage_status(response.status)?;
        Ok(response)
    }

    fn request(
        &mut self,
        operation: StorageOperation,
        blocks: u16,
        payload_bytes: u16,
    ) -> Result<StorageRequest, BlockError> {
        StorageRequest::new(
            operation,
            self.next_request(),
            self.generation,
            self.capability_slot,
            self.service_epoch,
            0,
            blocks,
            payload_bytes,
            0,
        )
        .ok_or(BlockError::InvalidRequest)
    }

    fn cached_slot(&self, block: u64) -> Option<usize> {
        self.cache.iter().position(|slot| slot.valid && slot.block == block)
    }

    fn reserve_cache_slot(&mut self, block: u64) -> Result<usize, BlockError> {
        if let Some(slot) = self.cached_slot(block) {
            return Ok(slot);
        }
        for offset in 0..CACHE_SLOTS {
            let slot = (self.next_cache_slot + offset) % CACHE_SLOTS;
            if self.pin_count[slot] != 0 {
                continue;
            }
            self.next_cache_slot = (slot + 1) % CACHE_SLOTS;
            self.cache[slot] = CacheSlot { block, valid: true };
            return Ok(slot);
        }
        Err(BlockError::InvalidRequest)
    }

    fn load_cache_slot(&mut self, block: u64, slot: usize) -> Result<(), BlockError>
    where
        T: KernelStorageIpc,
    {
        let mut request = self.request(StorageOperation::Read, 1, BLOCK_BYTES as u16)?;
        request.start_block = block;
        self.round_trip(request)?;
        self.cache[slot] = CacheSlot { block, valid: true };
        unsafe { Self::copy_cache_from(slot, &self.staging) };
        Ok(())
    }

    #[cfg(target_os = "none")]
    unsafe fn copy_cache_to(slot: usize, output: &mut Block) {
        let address = logos_abi::STORAGE_CACHE_BASE + slot * logos_storage::BLOCK_BYTES;
        output.as_bytes_mut().copy_from_slice(unsafe {
            core::slice::from_raw_parts(address as *const u8, logos_storage::BLOCK_BYTES)
        });
    }

    #[cfg(not(target_os = "none"))]
    #[allow(dead_code)]
    unsafe fn copy_cache_to(_slot: usize, _output: &mut Block) {}

    #[cfg(target_os = "none")]
    unsafe fn copy_cache_from(slot: usize, input: &Block) {
        let address = logos_abi::STORAGE_CACHE_BASE + slot * logos_storage::BLOCK_BYTES;
        unsafe {
            core::slice::from_raw_parts_mut(address as *mut u8, logos_storage::BLOCK_BYTES)
                .copy_from_slice(input.as_bytes());
        }
    }

    #[cfg(not(target_os = "none"))]
    unsafe fn copy_cache_from(_slot: usize, _input: &Block) {}
}

impl<T: KernelStorageIpc> BlockStore for IpcBlockStore<T> {
    fn block_count(&self) -> u64 {
        self.blocks
    }

    fn read_block(&mut self, index: BlockIndex, output: &mut Block) -> Result<(), BlockError> {
        if index.get() >= self.blocks {
            return Err(BlockError::OutOfBounds);
        }
        #[cfg(target_os = "none")]
        if let Some(slot) = self.cached_slot(index.get()) {
            unsafe { Self::copy_cache_to(slot, output) };
            return Ok(());
        }
        let mut request = self.request(StorageOperation::Read, 1, BLOCK_BYTES as u16)?;
        request.start_block = index.get();
        self.round_trip(request)?;
        *output = self.staging;
        let slot = self.reserve_cache_slot(index.get())?;
        unsafe { Self::copy_cache_from(slot, &self.staging) };
        Ok(())
    }

    fn read_block_uncached(
        &mut self,
        index: BlockIndex,
        output: &mut Block,
    ) -> Result<(), BlockError> {
        if index.get() >= self.blocks {
            return Err(BlockError::OutOfBounds);
        }
        let mut request = self.request(StorageOperation::Read, 1, BLOCK_BYTES as u16)?;
        request.start_block = index.get();
        self.round_trip(request)?;
        *output = self.staging;
        Ok(())
    }

    fn write_block(&mut self, index: BlockIndex, input: &Block) -> Result<(), BlockError> {
        if index.get() >= self.blocks {
            return Err(BlockError::OutOfBounds);
        }
        self.staging = *input;
        let mut request = self.request(StorageOperation::Write, 1, BLOCK_BYTES as u16)?;
        request.start_block = index.get();
        self.round_trip(request)?;
        let slot = self.reserve_cache_slot(index.get())?;
        unsafe { Self::copy_cache_from(slot, input) };
        Ok(())
    }

    fn flush(&mut self) -> Result<(), BlockError> {
        let request = self.request(StorageOperation::Flush, 0, 0)?;
        self.round_trip(request).map(|_| ())
    }

    fn map_read_blocks(&mut self, start: BlockIndex, blocks: u32) -> Result<ReadMap, BlockError> {
        let blocks = usize::try_from(blocks).map_err(|_| BlockError::InvalidRequest)?;
        if blocks == 0 || blocks > CACHE_MAP_MAX_PAGES {
            return Err(BlockError::InvalidRequest);
        }
        let end = start.get().checked_add(blocks as u64).ok_or(BlockError::OutOfBounds)?;
        if end > self.blocks {
            return Err(BlockError::OutOfBounds);
        }
        let first = (0..=CACHE_SLOTS - blocks)
            .find(|first| (0..blocks).all(|offset| self.pin_count[first + offset] == 0))
            .ok_or(BlockError::InvalidRequest)?;
        for offset in 0..blocks {
            if let Err(error) = self.load_cache_slot(start.get() + offset as u64, first + offset) {
                for rollback in 0..offset {
                    self.pin_count[first + rollback] = 0;
                }
                return Err(error);
            }
            self.pin_count[first + offset] = 1;
        }
        Ok(ReadMap { source_page: first as u64, pages: blocks as u8 })
    }

    fn unmap_read(&mut self, mapping: ReadMap) -> Result<(), BlockError> {
        let first = usize::try_from(mapping.source_page).map_err(|_| BlockError::InvalidRequest)?;
        let pages = usize::from(mapping.pages);
        let end = first.checked_add(pages).ok_or(BlockError::InvalidRequest)?;
        if pages == 0 || end > CACHE_SLOTS {
            return Err(BlockError::InvalidRequest);
        }
        for slot in first..end {
            if self.pin_count[slot] == 0 {
                return Err(BlockError::Stale);
            }
        }
        for slot in first..end {
            self.pin_count[slot] -= 1;
        }
        Ok(())
    }
}

fn map_ipc_status(status: IpcStatus) -> Result<(), BlockError> {
    match status {
        IpcStatus::Ok => Ok(()),
        IpcStatus::Unauthorized => Err(BlockError::Unauthorized),
        IpcStatus::Stale | IpcStatus::Disconnected => Err(BlockError::Stale),
        IpcStatus::Full | IpcStatus::Empty => Err(BlockError::Io),
        IpcStatus::Malformed => Err(BlockError::InvalidRequest),
    }
}

fn map_storage_status(status: StorageStatus) -> Result<(), BlockError> {
    match status {
        StorageStatus::Ok => Ok(()),
        StorageStatus::Io => Err(BlockError::Io),
        StorageStatus::OutOfBounds => Err(BlockError::OutOfBounds),
        StorageStatus::ReadOnly => Err(BlockError::ReadOnly),
        StorageStatus::Unauthorized => Err(BlockError::Unauthorized),
        StorageStatus::Stale => Err(BlockError::Stale),
        StorageStatus::Invalid
        | StorageStatus::Full
        | StorageStatus::Recovery
        | StorageStatus::Unsupported => Err(BlockError::InvalidRequest),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use logos_abi::{IpcRights, StorageStatus};
    use logos_storage::{JournalRecord, MemoryBlockStore, ReplayError, ReplaySink, Volume};

    struct TestKernel {
        store: MemoryBlockStore<32>,
        expected: IpcCapability,
        pending: Option<StorageRequest>,
        fault: Option<StorageStatus>,
        fail_read_after: Option<usize>,
        malformed_response: bool,
    }

    impl TestKernel {
        fn new(expected: IpcCapability) -> Self {
            Self {
                store: MemoryBlockStore::new(),
                expected,
                pending: None,
                fault: None,
                fail_read_after: None,
                malformed_response: false,
            }
        }

        fn status(&self) -> StorageStatus {
            self.fault.unwrap_or(StorageStatus::Ok)
        }
    }

    impl KernelStorageIpc for TestKernel {
        fn send(
            &mut self,
            capability: IpcCapability,
            request: StorageRequest,
            staging: &mut Block,
        ) -> IpcStatus {
            if capability != self.expected {
                return IpcStatus::Unauthorized;
            }
            if request.generation != capability.generation
                || request.service_epoch != capability.service_epoch
            {
                return IpcStatus::Stale;
            }
            if request.operation == StorageOperation::Write
                && self.store.write_block(BlockIndex::new(request.start_block), staging).is_err()
            {
                return IpcStatus::Ok;
            }
            self.pending = Some(request);
            IpcStatus::Ok
        }

        fn receive(
            &mut self,
            capability: IpcCapability,
            response: &mut StorageResponse,
            staging: &mut Block,
        ) -> IpcStatus {
            if capability != self.expected {
                return IpcStatus::Unauthorized;
            }
            let Some(request) = self.pending.take() else { return IpcStatus::Empty };
            if request.operation == StorageOperation::Read {
                let _ = self.store.read_block(BlockIndex::new(request.start_block), staging);
            }
            let status = if request.operation == StorageOperation::Read {
                match self.fail_read_after.as_mut() {
                    Some(remaining) if *remaining == 0 => StorageStatus::Io,
                    Some(remaining) => {
                        *remaining -= 1;
                        self.status()
                    }
                    None => self.status(),
                }
            } else {
                self.status()
            };
            *response = StorageResponse::new(
                request.request_id,
                status,
                request.generation,
                if request.is_block_io() { 1 } else { 0 },
                request.payload_bytes,
                request.transaction_id,
            );
            if self.malformed_response {
                response.blocks_completed = 0;
                response.payload_bytes = 0;
            }
            IpcStatus::Ok
        }
    }

    fn capability() -> IpcCapability {
        IpcCapability::new(0, IpcRights::Send, 3, 9).unwrap()
    }

    #[test]
    fn block_store_round_trips_through_private_staging() {
        let mut kernel = TestKernel::new(capability());
        let mut expected = Block::zero();
        expected.as_bytes_mut()[0] = 0x5a;
        kernel.store.write_block(BlockIndex::new(1), &expected).unwrap();
        let mut store = IpcBlockStore::new(kernel, capability(), 3, 9, 4).unwrap();
        let mut output = Block::zero();
        store.read_block(BlockIndex::new(1), &mut output).unwrap();
        assert_eq!(output, expected);
        store.flush().unwrap();
        let kernel = store.into_transport();
        assert_eq!(kernel.pending, None);
    }

    #[test]
    fn unauthorized_and_stale_capabilities_are_rejected() {
        let kernel = TestKernel::new(capability());
        let wrong = IpcCapability::new(0, IpcRights::Send, 4, 9).unwrap();
        let mut store = IpcBlockStore::new(kernel, wrong, 4, 9, 4).unwrap();
        let mut output = Block::zero();
        assert_eq!(
            store.read_block(BlockIndex::new(0), &mut output),
            Err(BlockError::Unauthorized)
        );
    }

    #[test]
    fn io_and_read_only_failures_propagate_as_typed_errors() {
        let mut kernel = TestKernel::new(capability());
        kernel.fault = Some(StorageStatus::Io);
        let mut store = IpcBlockStore::new(kernel, capability(), 3, 9, 4).unwrap();
        assert_eq!(store.flush(), Err(BlockError::Io));

        let mut kernel = TestKernel::new(capability());
        kernel.fault = Some(StorageStatus::ReadOnly);
        let mut store = IpcBlockStore::new(kernel, capability(), 3, 9, 4).unwrap();
        assert_eq!(
            store.write_block(BlockIndex::new(0), &Block::zero()),
            Err(BlockError::ReadOnly)
        );
    }

    #[test]
    fn malformed_success_response_is_rejected() {
        let mut kernel = TestKernel::new(capability());
        kernel.malformed_response = true;
        let mut store = IpcBlockStore::new(kernel, capability(), 3, 9, 4).unwrap();
        let mut output = Block::zero();

        assert_eq!(
            store.read_block(BlockIndex::new(0), &mut output),
            Err(BlockError::InvalidRequest)
        );
    }

    #[test]
    fn cache_read_maps_pin_slots_until_unmap() {
        let kernel = TestKernel::new(capability());
        let mut store = IpcBlockStore::new(kernel, capability(), 3, 9, 32).unwrap();
        let first = store.map_read_blocks(BlockIndex::new(0), 16).unwrap();
        let second = store.map_read_blocks(BlockIndex::new(16), 16).unwrap();
        assert_eq!(first, ReadMap { source_page: 0, pages: 16 });
        assert_eq!(second, ReadMap { source_page: 16, pages: 16 });
        assert_eq!(store.map_read_blocks(BlockIndex::new(0), 1), Err(BlockError::InvalidRequest));
        store.unmap_read(first).unwrap();
        assert_eq!(
            store.map_read_blocks(BlockIndex::new(0), 1),
            Ok(ReadMap { source_page: 0, pages: 1 })
        );
        assert_eq!(store.unmap_read(first), Err(BlockError::Stale));
        store.unmap_read(second).unwrap();
    }

    #[test]
    fn failed_cache_read_rolls_back_partial_pins() {
        let mut kernel = TestKernel::new(capability());
        kernel.fail_read_after = Some(1);
        let mut store = IpcBlockStore::new(kernel, capability(), 3, 9, 32).unwrap();

        assert_eq!(store.map_read_blocks(BlockIndex::new(0), 2), Err(BlockError::Io));
        assert!(store.pin_count.iter().all(|count| *count == 0));

        store.transport.fail_read_after = None;
        let mapping = store.map_read_blocks(BlockIndex::new(0), 2).unwrap();
        store.unmap_read(mapping).unwrap();
    }

    struct Sink {
        records: u8,
    }

    impl ReplaySink for Sink {
        fn record(
            &mut self,
            _transaction_id: u64,
            _kind: u16,
            _payload: &[u8],
        ) -> Result<(), ReplayError> {
            self.records = self.records.saturating_add(1);
            Ok(())
        }
    }

    #[test]
    fn service_restart_reopens_and_replays_a_committed_transaction_once() {
        let capability = capability();
        let kernel = TestKernel::new(capability);
        let mut store = IpcBlockStore::new(kernel, capability, 3, 9, 32).unwrap();
        let mut volume = Volume::format(&mut store).unwrap();
        let payload = [0x5a; 8];
        let transaction =
            volume.commit(&mut store, &[JournalRecord { kind: 7, payload: &payload }]).unwrap();
        assert_eq!(transaction, 1);

        let kernel = store.into_transport();
        let mut reopened_store = IpcBlockStore::new(kernel, capability, 3, 9, 32).unwrap();
        let mut reopened = Volume::open(&mut reopened_store).unwrap();
        let mut sink = Sink { records: 0 };
        let summary = reopened.recover(&mut reopened_store, &mut sink).unwrap();
        assert_eq!(summary.committed_transactions, 1);
        assert_eq!(summary.replayed_records, 1);
        assert_eq!(sink.records, 1);
    }
}
