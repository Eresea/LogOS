#![no_std]

#[cfg(test)]
extern crate std;

mod namespace;

pub use namespace::{
    DurableNamespace, MAX_COMPONENT_BYTES, MAX_FILE_BLOCKS, MAX_FILE_BYTES, MAX_OBJECTS,
    MAX_PATH_DEPTH, NamespaceError, ObjectId, ObjectInfo, ObjectKind, ObjectList, ObjectNamespace,
};

use logos_abi::{
    IpcCapability, IpcStatus, StorageOperation, StorageRequest, StorageResponse, StorageStatus,
};
use logos_storage::{BLOCK_BYTES, Block, BlockError, BlockIndex, BlockStore};

pub const STORAGE_REQUEST_CAPACITY: usize = 8;

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
        if response.request_id != request.request_id
            || response.generation != request.generation
            || response.transaction_id != request.transaction_id
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
}

impl<T: KernelStorageIpc> BlockStore for IpcBlockStore<T> {
    fn block_count(&self) -> u64 {
        self.blocks
    }

    fn read_block(&mut self, index: BlockIndex, output: &mut Block) -> Result<(), BlockError> {
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
        self.round_trip(request).map(|_| ())
    }

    fn flush(&mut self) -> Result<(), BlockError> {
        let request = self.request(StorageOperation::Flush, 0, 0)?;
        self.round_trip(request).map(|_| ())
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
    }

    impl TestKernel {
        fn new(expected: IpcCapability) -> Self {
            Self { store: MemoryBlockStore::new(), expected, pending: None, fault: None }
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
            *response = StorageResponse::new(
                request.request_id,
                self.status(),
                request.generation,
                if request.is_block_io() { 1 } else { 0 },
                request.payload_bytes,
                request.transaction_id,
            );
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
