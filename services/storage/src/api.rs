use logos_abi::{
    IpcBytes, STORAGE_API_FLAG_REPLACE, STORAGE_API_RESPONSE_DATA_BYTES, StorageApiOperation,
    StorageApiRequest, StorageApiResponse, StorageApiStatus,
};
use logos_storage::BlockStore;

use crate::{DurableNamespace, MAX_FILE_BYTES, NamespaceError, NamespaceTransaction};

const STAGE_CHUNK_BYTES: usize = 192;
const STAGE_PATH_BYTES: usize = logos_abi::MAX_IPC_BYTES - 26;

pub fn error_response(message: &IpcBytes, status: StorageApiStatus) -> Option<IpcBytes> {
    let request = match StorageApiRequest::decode(message) {
        Ok(request) => request,
        Err(_) => return malformed_response(message),
    };
    StorageApiResponse::encode(status, request.request_id, request.transaction_id, &[], false)
}

struct ResponsePayload {
    status: StorageApiStatus,
    transaction_id: u64,
    data: [u8; STORAGE_API_RESPONSE_DATA_BYTES],
    len: usize,
    more: bool,
}

impl ResponsePayload {
    const fn empty(status: StorageApiStatus, transaction_id: u64) -> Self {
        Self {
            status,
            transaction_id,
            data: [0; STORAGE_API_RESPONSE_DATA_BYTES],
            len: 0,
            more: false,
        }
    }
}

struct ActiveTransaction {
    id: u64,
    transaction: NamespaceTransaction,
}

struct StagedWrite {
    handle: u64,
    path: [u8; STAGE_PATH_BYTES],
    path_len: usize,
    data: [u8; MAX_FILE_BYTES],
    len: usize,
}

pub struct StorageApi<B> {
    namespace: DurableNamespace<B>,
    active: Option<ActiveTransaction>,
    next_transaction: u64,
    next_stage: u64,
    staged: Option<StagedWrite>,
    failed: bool,
}

impl<B: BlockStore> StorageApi<B> {
    pub fn new(namespace: DurableNamespace<B>) -> Self {
        Self {
            namespace,
            active: None,
            next_transaction: 1,
            next_stage: 1,
            staged: None,
            failed: false,
        }
    }

    pub fn into_namespace(self) -> DurableNamespace<B> {
        self.namespace
    }

    pub fn namespace_mut(&mut self) -> &mut DurableNamespace<B> {
        &mut self.namespace
    }

    pub fn handle(&mut self, message: &IpcBytes) -> Option<IpcBytes> {
        let request = match StorageApiRequest::decode(message) {
            Ok(request) => request,
            Err(_) => return malformed_response(message),
        };
        if self.failed {
            return Self::encode(
                request.request_id,
                ResponsePayload::empty(StorageApiStatus::Io, request.transaction_id),
            );
        }
        if request.operation != StorageApiOperation::Write && request.flags != 0 {
            return Self::encode(
                request.request_id,
                ResponsePayload::empty(StorageApiStatus::Invalid, request.transaction_id),
            );
        }
        if request.operation == StorageApiOperation::Write
            && request.flags & !STORAGE_API_FLAG_REPLACE != 0
        {
            return Self::encode(
                request.request_id,
                ResponsePayload::empty(StorageApiStatus::Invalid, request.transaction_id),
            );
        }
        let response = match request.operation {
            StorageApiOperation::Begin => self.begin(&request),
            StorageApiOperation::Commit => self.commit(&request),
            StorageApiOperation::Abort => self.abort(&request),
            StorageApiOperation::List => self.list(&request),
            StorageApiOperation::CreateFile => self.create_file(&request),
            StorageApiOperation::Read => self.read(&request),
            StorageApiOperation::Write => self.write(&request),
            StorageApiOperation::Remove => self.remove(&request),
            StorageApiOperation::Rename => self.rename(&request),
            StorageApiOperation::StageWriteBegin => self.stage_begin(&request),
            StorageApiOperation::StageWriteChunk => self.stage_chunk(&request),
            StorageApiOperation::StageWriteCommit => self.stage_commit(&request),
            StorageApiOperation::StageWriteAbort => self.stage_abort(&request),
            StorageApiOperation::PackageList => self.package_list(&request),
            StorageApiOperation::PackageInfo => self.package_info(&request),
            StorageApiOperation::PackageInstall => self.package_install(&request),
        };
        Self::encode(request.request_id, response)
    }

    fn encode(request_id: u32, response: ResponsePayload) -> Option<IpcBytes> {
        StorageApiResponse::encode(
            response.status,
            request_id,
            response.transaction_id,
            &response.data[..response.len],
            response.more,
        )
    }

    fn begin(&mut self, request: &StorageApiRequest<'_>) -> ResponsePayload {
        if request.transaction_id != 0 {
            return ResponsePayload::empty(StorageApiStatus::Invalid, 0);
        }
        if self.active.is_some() {
            return ResponsePayload::empty(StorageApiStatus::Busy, 0);
        }
        let id = self.next_transaction;
        self.next_transaction = self.next_transaction.wrapping_add(1).max(1);
        self.active =
            Some(ActiveTransaction { id, transaction: self.namespace.begin_transaction() });
        ResponsePayload::empty(StorageApiStatus::Ok, id)
    }

    fn commit(&mut self, request: &StorageApiRequest<'_>) -> ResponsePayload {
        let Some(active) = self.active.take() else {
            return ResponsePayload::empty(StorageApiStatus::NoTransaction, request.transaction_id);
        };
        if active.id != request.transaction_id || request.transaction_id == 0 {
            self.active = Some(active);
            return ResponsePayload::empty(StorageApiStatus::Stale, request.transaction_id);
        }
        let id = active.id;
        match active.transaction.commit(&mut self.namespace) {
            Ok(_) => ResponsePayload::empty(StorageApiStatus::Ok, id),
            Err(error) => {
                if error == NamespaceError::Recovery {
                    self.failed = true;
                }
                ResponsePayload::empty(map_error(error), id)
            }
        }
    }

    fn abort(&mut self, request: &StorageApiRequest<'_>) -> ResponsePayload {
        let Some(active) = self.active.take() else {
            return ResponsePayload::empty(StorageApiStatus::NoTransaction, request.transaction_id);
        };
        if active.id != request.transaction_id || request.transaction_id == 0 {
            self.active = Some(active);
            return ResponsePayload::empty(StorageApiStatus::Stale, request.transaction_id);
        }
        active.transaction.abort();
        ResponsePayload::empty(StorageApiStatus::Ok, request.transaction_id)
    }

    fn transaction(&self, id: u64) -> Result<&NamespaceTransaction, StorageApiStatus> {
        let Some(active) = &self.active else {
            return Err(StorageApiStatus::NoTransaction);
        };
        if id == 0 || active.id != id {
            return Err(StorageApiStatus::Stale);
        }
        Ok(&active.transaction)
    }

    fn transaction_mut(&mut self, id: u64) -> Result<&mut NamespaceTransaction, StorageApiStatus> {
        let Some(active) = &mut self.active else {
            return Err(StorageApiStatus::NoTransaction);
        };
        if id == 0 || active.id != id {
            return Err(StorageApiStatus::Stale);
        }
        Ok(&mut active.transaction)
    }

    fn list(&self, request: &StorageApiRequest<'_>) -> ResponsePayload {
        let index = request.offset as usize;
        let list = if request.transaction_id == 0 {
            let parent = match self.namespace.resolve_path(request.path) {
                Ok(parent) => parent,
                Err(error) => {
                    return ResponsePayload::empty(map_error(error), request.transaction_id);
                }
            };
            match self.namespace.list(parent) {
                Ok(list) => list,
                Err(error) => {
                    return ResponsePayload::empty(map_error(error), request.transaction_id);
                }
            }
        } else {
            match self.transaction(request.transaction_id) {
                Ok(transaction) => match transaction.list(request.path) {
                    Ok(list) => list,
                    Err(error) => {
                        return ResponsePayload::empty(map_error(error), request.transaction_id);
                    }
                },
                Err(status) => return ResponsePayload::empty(status, request.transaction_id),
            }
        };
        let Some(id) = list.get(index) else {
            return ResponsePayload::empty(StorageApiStatus::Ok, request.transaction_id);
        };
        let info = if request.transaction_id == 0 {
            match self.namespace.stat(id) {
                Ok(info) => info,
                Err(error) => {
                    return ResponsePayload::empty(map_error(error), request.transaction_id);
                }
            }
        } else {
            match self.transaction(request.transaction_id) {
                Ok(transaction) => match transaction.stat_id(id) {
                    Ok(info) => info,
                    Err(error) => {
                        return ResponsePayload::empty(map_error(error), request.transaction_id);
                    }
                },
                Err(status) => return ResponsePayload::empty(status, request.transaction_id),
            }
        };
        if info.name_bytes().len() > STORAGE_API_RESPONSE_DATA_BYTES {
            return ResponsePayload::empty(StorageApiStatus::TooLarge, request.transaction_id);
        }
        let mut response = ResponsePayload::empty(StorageApiStatus::Ok, request.transaction_id);
        response.data[..info.name_bytes().len()].copy_from_slice(info.name_bytes());
        response.len = info.name_bytes().len();
        response.more = index + 1 < list.len();
        response
    }

    fn package_list(&mut self, request: &StorageApiRequest<'_>) -> ResponsePayload {
        if request.transaction_id != 0 {
            return ResponsePayload::empty(StorageApiStatus::Invalid, request.transaction_id);
        }
        let index = request.offset as usize;
        let info = match self.namespace.package_at(index) {
            Ok(Some(info)) => info,
            Ok(None) => return ResponsePayload::empty(StorageApiStatus::Ok, 0),
            Err(error) => return ResponsePayload::empty(map_error(error), 0),
        };
        let mut response = ResponsePayload::empty(StorageApiStatus::Ok, 0);
        response.len = format_package_info(&info, &mut response.data);
        response.more = self.namespace.package_at(index + 1).is_ok_and(|next| next.is_some());
        response
    }

    fn package_info(&mut self, request: &StorageApiRequest<'_>) -> ResponsePayload {
        if request.transaction_id != 0 {
            return ResponsePayload::empty(StorageApiStatus::Invalid, request.transaction_id);
        }
        let info = match self.namespace.lookup_package_name(request.path) {
            Ok(info) => info,
            Err(error) => return ResponsePayload::empty(map_error(error), 0),
        };
        let mut response = ResponsePayload::empty(StorageApiStatus::Ok, 0);
        response.len = format_package_info(&info, &mut response.data);
        response
    }

    fn package_install(&mut self, request: &StorageApiRequest<'_>) -> ResponsePayload {
        if request.transaction_id != 0 {
            return ResponsePayload::empty(StorageApiStatus::Invalid, request.transaction_id);
        }
        if self.active.is_some() || self.staged.is_some() {
            return ResponsePayload::empty(StorageApiStatus::Busy, 0);
        }
        let handle = match self.namespace.install_package_file(request.path) {
            Ok(handle) => handle,
            Err(error) => return ResponsePayload::empty(map_error(error), 0),
        };
        let mut response = ResponsePayload::empty(StorageApiStatus::Ok, 0);
        append_bytes(&mut response.data, &mut response.len, b"installed generation ");
        append_u32(&mut response.data, &mut response.len, handle.generation);
        append_bytes(&mut response.data, &mut response.len, b"\r\n");
        response
    }

    fn create_file(&mut self, request: &StorageApiRequest<'_>) -> ResponsePayload {
        match self.transaction_mut(request.transaction_id) {
            Ok(transaction) => match transaction.create_file(request.path) {
                Ok(_) => ResponsePayload::empty(StorageApiStatus::Ok, request.transaction_id),
                Err(error) => ResponsePayload::empty(map_error(error), request.transaction_id),
            },
            Err(status) => ResponsePayload::empty(status, request.transaction_id),
        }
    }

    fn read(&self, request: &StorageApiRequest<'_>) -> ResponsePayload {
        let mut response = ResponsePayload::empty(StorageApiStatus::Ok, request.transaction_id);
        if request.transaction_id == 0 {
            let id = match self.namespace.resolve_path(request.path) {
                Ok(id) => id,
                Err(error) => return ResponsePayload::empty(map_error(error), 0),
            };
            let info = match self.namespace.stat(id) {
                Ok(info) => info,
                Err(error) => return ResponsePayload::empty(map_error(error), 0),
            };
            let count = match self.namespace.read(id, request.offset as usize, &mut response.data) {
                Ok(count) => count,
                Err(error) => return ResponsePayload::empty(map_error(error), 0),
            };
            response.more = request.offset as usize + count < info.length as usize;
            response.len = count;
        } else {
            let transaction = match self.transaction(request.transaction_id) {
                Ok(transaction) => transaction,
                Err(status) => return ResponsePayload::empty(status, request.transaction_id),
            };
            let info = match transaction.stat(request.path) {
                Ok(info) => info,
                Err(error) => {
                    return ResponsePayload::empty(map_error(error), request.transaction_id);
                }
            };
            let count =
                match transaction.read(request.path, request.offset as usize, &mut response.data) {
                    Ok(count) => count,
                    Err(error) => {
                        return ResponsePayload::empty(map_error(error), request.transaction_id);
                    }
                };
            response.more = request.offset as usize + count < info.length as usize;
            response.len = count;
        }
        response
    }

    fn write(&mut self, request: &StorageApiRequest<'_>) -> ResponsePayload {
        match self.transaction_mut(request.transaction_id) {
            Ok(transaction) => match transaction.write(
                request.path,
                request.offset as usize,
                request.data,
                request.flags & STORAGE_API_FLAG_REPLACE != 0,
            ) {
                Ok(_) => ResponsePayload::empty(StorageApiStatus::Ok, request.transaction_id),
                Err(error) => ResponsePayload::empty(map_error(error), request.transaction_id),
            },
            Err(status) => ResponsePayload::empty(status, request.transaction_id),
        }
    }

    fn remove(&mut self, request: &StorageApiRequest<'_>) -> ResponsePayload {
        match self.transaction_mut(request.transaction_id) {
            Ok(transaction) => match transaction.remove(request.path) {
                Ok(()) => ResponsePayload::empty(StorageApiStatus::Ok, request.transaction_id),
                Err(error) => ResponsePayload::empty(map_error(error), request.transaction_id),
            },
            Err(status) => ResponsePayload::empty(status, request.transaction_id),
        }
    }

    fn rename(&mut self, request: &StorageApiRequest<'_>) -> ResponsePayload {
        match self.transaction_mut(request.transaction_id) {
            Ok(transaction) => match transaction.rename(request.path, request.secondary_path) {
                Ok(()) => ResponsePayload::empty(StorageApiStatus::Ok, request.transaction_id),
                Err(error) => ResponsePayload::empty(map_error(error), request.transaction_id),
            },
            Err(status) => ResponsePayload::empty(status, request.transaction_id),
        }
    }

    fn stage_begin(&mut self, request: &StorageApiRequest<'_>) -> ResponsePayload {
        if request.transaction_id != 0 || request.path.len() > STAGE_PATH_BYTES {
            return ResponsePayload::empty(StorageApiStatus::Invalid, request.transaction_id);
        }
        let handle = self.next_stage;
        self.next_stage = self.next_stage.wrapping_add(1).max(1);
        let mut path = [0; STAGE_PATH_BYTES];
        path[..request.path.len()].copy_from_slice(request.path);
        self.staged = Some(StagedWrite {
            handle,
            path,
            path_len: request.path.len(),
            data: [0; MAX_FILE_BYTES],
            len: 0,
        });
        ResponsePayload::empty(StorageApiStatus::Ok, handle)
    }

    fn stage_chunk(&mut self, request: &StorageApiRequest<'_>) -> ResponsePayload {
        let Some(stage) = &mut self.staged else {
            return ResponsePayload::empty(StorageApiStatus::NoTransaction, request.transaction_id);
        };
        if stage.handle != request.transaction_id {
            return ResponsePayload::empty(StorageApiStatus::Stale, request.transaction_id);
        }
        if request.data.len() > STAGE_CHUNK_BYTES || request.offset as usize != stage.len {
            return ResponsePayload::empty(StorageApiStatus::Invalid, stage.handle);
        }
        let end = stage.len.saturating_add(request.data.len());
        if end > MAX_FILE_BYTES {
            return ResponsePayload::empty(StorageApiStatus::TooLarge, stage.handle);
        }
        stage.data[stage.len..end].copy_from_slice(request.data);
        stage.len = end;
        ResponsePayload::empty(StorageApiStatus::Ok, stage.handle)
    }

    fn stage_commit(&mut self, request: &StorageApiRequest<'_>) -> ResponsePayload {
        let Some(stage) = self.staged.take() else {
            return ResponsePayload::empty(StorageApiStatus::NoTransaction, request.transaction_id);
        };
        if stage.handle != request.transaction_id {
            self.staged = Some(stage);
            return ResponsePayload::empty(StorageApiStatus::Stale, request.transaction_id);
        }
        let path = &stage.path[..stage.path_len];
        let mut transaction = self.namespace.begin_transaction();
        let result = match transaction.stat(path) {
            Ok(_) => transaction.write(path, 0, &stage.data[..stage.len], true).map(|_| ()),
            Err(NamespaceError::NotFound) => transaction.create_file(path).and_then(|_| {
                transaction.write(path, 0, &stage.data[..stage.len], true).map(|_| ())
            }),
            Err(error) => Err(error),
        };
        match result {
            Ok(()) => match transaction.commit(&mut self.namespace) {
                Ok(_) => ResponsePayload::empty(StorageApiStatus::Ok, stage.handle),
                Err(error) => {
                    if error == NamespaceError::Recovery {
                        self.failed = true;
                    }
                    ResponsePayload::empty(map_error(error), stage.handle)
                }
            },
            Err(error) => {
                transaction.abort();
                ResponsePayload::empty(map_error(error), stage.handle)
            }
        }
    }

    fn stage_abort(&mut self, request: &StorageApiRequest<'_>) -> ResponsePayload {
        let Some(stage) = self.staged.take() else {
            return ResponsePayload::empty(StorageApiStatus::NoTransaction, request.transaction_id);
        };
        if stage.handle != request.transaction_id {
            self.staged = Some(stage);
            return ResponsePayload::empty(StorageApiStatus::Stale, request.transaction_id);
        }
        ResponsePayload::empty(StorageApiStatus::Ok, request.transaction_id)
    }
}

fn format_package_info(
    info: &crate::packages::PackageInfo,
    output: &mut [u8; STORAGE_API_RESPONSE_DATA_BYTES],
) -> usize {
    let mut length = 0;
    if let Some(manifest) = info.manifest {
        append_bytes(output, &mut length, manifest.name.as_bytes());
        append_bytes(output, &mut length, b" ");
        append_version(output, &mut length, manifest.version);
        if manifest.dependency_count() != 0 {
            append_bytes(output, &mut length, b" deps=");
            for index in 0..manifest.dependency_count() {
                if index != 0 {
                    append_bytes(output, &mut length, b",");
                }
                let dependency = manifest.dependency(index).expect("dependency count is bounded");
                append_bytes(output, &mut length, dependency.name.as_bytes());
                append_bytes(output, &mut length, b"(");
                append_bytes(output, &mut length, dependency.range());
                append_bytes(output, &mut length, b")");
            }
        }
    } else {
        append_bytes(output, &mut length, service_name(info.handle.service));
        append_bytes(output, &mut length, b" legacy-");
        append_u32(output, &mut length, info.package_version);
    }
    append_bytes(output, &mut length, b"\r\n");
    length
}

fn append_version(
    output: &mut [u8; STORAGE_API_RESPONSE_DATA_BYTES],
    length: &mut usize,
    version: logos_package::SemanticVersion,
) {
    append_u32(output, length, version.major);
    append_bytes(output, length, b".");
    append_u32(output, length, version.minor);
    append_bytes(output, length, b".");
    append_u32(output, length, version.patch);
}

fn append_u32(output: &mut [u8; STORAGE_API_RESPONSE_DATA_BYTES], length: &mut usize, value: u32) {
    let mut digits = [0; 10];
    let mut count = 0;
    let mut value = value;
    loop {
        digits[count] = b'0' + (value % 10) as u8;
        count += 1;
        value /= 10;
        if value == 0 {
            break;
        }
    }
    while count != 0 {
        count -= 1;
        append_bytes(output, length, &digits[count..count + 1]);
    }
}

fn append_bytes(
    output: &mut [u8; STORAGE_API_RESPONSE_DATA_BYTES],
    length: &mut usize,
    bytes: &[u8],
) {
    let amount = bytes.len().min(output.len().saturating_sub(*length));
    output[*length..*length + amount].copy_from_slice(&bytes[..amount]);
    *length += amount;
}

fn service_name(service: logos_abi::ServiceId) -> &'static [u8] {
    match service {
        logos_abi::ServiceId::Input => b"input",
        logos_abi::ServiceId::Display => b"display",
        logos_abi::ServiceId::Terminal => b"terminal",
        logos_abi::ServiceId::Session => b"session",
        logos_abi::ServiceId::Flow => b"flow",
        logos_abi::ServiceId::Storage => b"storage",
        logos_abi::ServiceId::Network => b"network",
        logos_abi::ServiceId::Fetch => b"fetch",
    }
}

fn malformed_response(message: &IpcBytes) -> Option<IpcBytes> {
    if message.kind != logos_abi::MessageKind::StorageRequest {
        return None;
    }
    let bytes = message.as_bytes()?;
    let request_id_bytes = bytes.get(4..8)?;
    let request_id = u32::from_le_bytes(request_id_bytes.try_into().ok()?);
    if request_id == 0 {
        return None;
    }
    StorageApiResponse::encode(StorageApiStatus::Invalid, request_id, 0, &[], false)
}

fn map_error(error: NamespaceError) -> StorageApiStatus {
    match error {
        NamespaceError::Capacity | NamespaceError::GenerationExhausted => {
            StorageApiStatus::Capacity
        }
        NamespaceError::InvalidName
        | NamespaceError::InvalidPath
        | NamespaceError::InvalidRecord => StorageApiStatus::Invalid,
        NamespaceError::NotFound => StorageApiStatus::NotFound,
        NamespaceError::NotDirectory => StorageApiStatus::NotDirectory,
        NamespaceError::AlreadyExists => StorageApiStatus::AlreadyExists,
        NamespaceError::IsDirectory => StorageApiStatus::IsDirectory,
        NamespaceError::Root => StorageApiStatus::Root,
        NamespaceError::NotEmpty => StorageApiStatus::NotEmpty,
        NamespaceError::Stale => StorageApiStatus::Stale,
        NamespaceError::TooLarge => StorageApiStatus::TooLarge,
        NamespaceError::Recovery => StorageApiStatus::Io,
        NamespaceError::Format(_) | NamespaceError::Block(_) => StorageApiStatus::Io,
        NamespaceError::Unsupported => StorageApiStatus::Unsupported,
        NamespaceError::Package(
            crate::packages::PackageCatalogError::VersionConflict
            | crate::packages::PackageCatalogError::MissingDependency
            | crate::packages::PackageCatalogError::DependencyConflict,
        ) => StorageApiStatus::Invalid,
        NamespaceError::Package(_) => StorageApiStatus::Io,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packages::{MAX_PACKAGE_EXTENTS, PackageExtent, PackageHandle, PackageInfo};
    use logos_abi::{STORAGE_API_FLAG_REPLACE, ServiceId, StorageApiOperation, StorageApiResponse};
    use logos_package::{
        PACKAGE_HEADER_BYTES, PackageManifest, PackageName, SemanticVersion, ServicePackageHeader,
        crc32c,
    };
    use logos_storage::{Block, BlockError, BlockIndex, BlockStore, MemoryBlockStore};
    use std::boxed::Box;

    struct HeapStore(Box<MemoryBlockStore<96>>);

    impl BlockStore for HeapStore {
        fn block_count(&self) -> u64 {
            self.0.block_count()
        }

        fn read_block(&mut self, index: BlockIndex, output: &mut Block) -> Result<(), BlockError> {
            self.0.read_block(index, output)
        }

        fn write_block(&mut self, index: BlockIndex, input: &Block) -> Result<(), BlockError> {
            self.0.write_block(index, input)
        }

        fn flush(&mut self) -> Result<(), BlockError> {
            self.0.flush()
        }
    }

    struct RecoveryFailStore {
        inner: MemoryBlockStore<16>,
        failed: bool,
    }

    impl RecoveryFailStore {
        fn new() -> Self {
            Self { inner: MemoryBlockStore::new(), failed: false }
        }

        fn fail(&mut self) {
            self.failed = true;
        }
    }

    impl BlockStore for RecoveryFailStore {
        fn block_count(&self) -> u64 {
            self.inner.block_count()
        }

        fn read_block(&mut self, index: BlockIndex, output: &mut Block) -> Result<(), BlockError> {
            if self.failed {
                return Err(BlockError::Io);
            }
            self.inner.read_block(index, output)
        }

        fn write_block(&mut self, index: BlockIndex, input: &Block) -> Result<(), BlockError> {
            if self.failed {
                return Err(BlockError::Io);
            }
            self.inner.write_block(index, input)
        }

        fn flush(&mut self) -> Result<(), BlockError> {
            if self.failed {
                return Err(BlockError::Io);
            }
            self.inner.flush()
        }
    }

    fn request(
        operation: StorageApiOperation,
        transaction_id: u64,
        path: &[u8],
        secondary_path: &[u8],
        data: &[u8],
        flags: u8,
        request_id: u32,
    ) -> IpcBytes {
        StorageApiRequest::encode(
            operation,
            flags,
            request_id,
            transaction_id,
            0,
            path,
            secondary_path,
            data,
        )
        .unwrap()
    }

    fn status(message: &IpcBytes) -> StorageApiResponse<'_> {
        StorageApiResponse::decode(message).unwrap()
    }

    #[test]
    fn malformed_request_with_identity_gets_an_error_reply() {
        let mut message = IpcBytes::empty(logos_abi::MessageKind::StorageRequest);
        message.len = 8;
        message.bytes[4..8].copy_from_slice(&7u32.to_le_bytes());
        let response =
            StorageApi::new(DurableNamespace::format(MemoryBlockStore::<64>::new()).unwrap())
                .handle(&message)
                .unwrap();
        assert_eq!(status(&response).status, StorageApiStatus::Invalid);
        assert_eq!(status(&response).request_id, 7);
    }

    #[test]
    fn error_response_preserves_identity_for_malformed_requests() {
        let mut message = IpcBytes::empty(logos_abi::MessageKind::StorageRequest);
        message.len = 8;
        message.bytes[4..8].copy_from_slice(&7u32.to_le_bytes());
        let response = error_response(&message, StorageApiStatus::Io).unwrap();
        assert_eq!(status(&response).status, StorageApiStatus::Invalid);
        assert_eq!(status(&response).request_id, 7);
    }

    #[test]
    fn api_commits_replace_write_and_reopens() {
        let namespace = DurableNamespace::format(MemoryBlockStore::<16>::new()).unwrap();
        let mut api = StorageApi::new(namespace);
        let begin_message =
            api.handle(&request(StorageApiOperation::Begin, 0, b"", b"", b"", 0, 1)).unwrap();
        let begin = status(&begin_message);
        assert_eq!(begin.status, StorageApiStatus::Ok);
        let txid = begin.transaction_id;
        assert_eq!(
            status(
                &api.handle(&request(
                    StorageApiOperation::CreateFile,
                    txid,
                    b"/proof",
                    b"",
                    b"",
                    0,
                    2
                ))
                .unwrap()
            )
            .status,
            StorageApiStatus::Ok
        );
        assert_eq!(
            status(
                &api.handle(&request(
                    StorageApiOperation::Write,
                    txid,
                    b"/proof",
                    b"",
                    b"durable",
                    STORAGE_API_FLAG_REPLACE,
                    3
                ))
                .unwrap()
            )
            .status,
            StorageApiStatus::Ok
        );
        assert_eq!(
            status(
                &api.handle(&request(StorageApiOperation::Commit, txid, b"", b"", b"", 0, 4))
                    .unwrap()
            )
            .status,
            StorageApiStatus::Ok
        );
        let namespace = api.into_namespace();
        let store = namespace.into_store();
        let namespace = DurableNamespace::open(store).unwrap();
        let mut api = StorageApi::new(namespace);
        let read_message =
            api.handle(&request(StorageApiOperation::Read, 0, b"/proof", b"", b"", 0, 5)).unwrap();
        let read = status(&read_message);
        assert_eq!(read.data, b"durable");
    }

    #[test]
    fn api_replace_writes_survive_reopen_cycles() {
        let mut api =
            StorageApi::new(DurableNamespace::format(MemoryBlockStore::<16>::new()).unwrap());
        let contents =
            [b"first".as_slice(), b"second replacement", b"third", b"final durable content"];

        for (cycle, expected) in contents.iter().enumerate() {
            let request_id = (cycle * 5 + 1) as u32;
            let begin_message = api
                .handle(&request(StorageApiOperation::Begin, 0, b"", b"", b"", 0, request_id))
                .unwrap();
            let begin = status(&begin_message);
            assert_eq!(begin.status, StorageApiStatus::Ok);
            let transaction_id = begin.transaction_id;
            if cycle == 0 {
                assert_eq!(
                    status(
                        &api.handle(&request(
                            StorageApiOperation::CreateFile,
                            transaction_id,
                            b"/cycle",
                            b"",
                            b"",
                            0,
                            request_id + 1,
                        ))
                        .unwrap(),
                    )
                    .status,
                    StorageApiStatus::Ok
                );
            }
            assert_eq!(
                status(
                    &api.handle(&request(
                        StorageApiOperation::Write,
                        transaction_id,
                        b"/cycle",
                        b"",
                        expected,
                        STORAGE_API_FLAG_REPLACE,
                        request_id + 2,
                    ))
                    .unwrap(),
                )
                .status,
                StorageApiStatus::Ok
            );
            assert_eq!(
                status(
                    &api.handle(&request(
                        StorageApiOperation::Commit,
                        transaction_id,
                        b"",
                        b"",
                        b"",
                        0,
                        request_id + 3,
                    ))
                    .unwrap(),
                )
                .status,
                StorageApiStatus::Ok
            );

            let namespace = api.into_namespace();
            let store = namespace.into_store();
            let namespace = DurableNamespace::open(store).unwrap();
            api = StorageApi::new(namespace);
            let read_message = api
                .handle(&request(
                    StorageApiOperation::Read,
                    0,
                    b"/cycle",
                    b"",
                    b"",
                    0,
                    request_id + 4,
                ))
                .unwrap();
            let read = status(&read_message);
            assert_eq!(read.status, StorageApiStatus::Ok);
            assert_eq!(read.data, *expected);
        }
    }

    #[test]
    fn api_rejects_stale_and_abort_discards_changes() {
        let namespace = DurableNamespace::format(MemoryBlockStore::<16>::new()).unwrap();
        let mut api = StorageApi::new(namespace);
        let begin_message =
            api.handle(&request(StorageApiOperation::Begin, 0, b"", b"", b"", 0, 1)).unwrap();
        let begin = status(&begin_message);
        let txid = begin.transaction_id;
        assert_eq!(
            status(
                &api.handle(&request(StorageApiOperation::Begin, 0, b"", b"", b"", 0, 2)).unwrap()
            )
            .status,
            StorageApiStatus::Busy
        );
        assert_eq!(
            status(
                &api.handle(&request(
                    StorageApiOperation::CreateFile,
                    txid + 1,
                    b"/lost",
                    b"",
                    b"",
                    0,
                    3
                ))
                .unwrap()
            )
            .status,
            StorageApiStatus::Stale
        );
        assert_eq!(
            status(
                &api.handle(&request(StorageApiOperation::Abort, txid, b"", b"", b"", 0, 4))
                    .unwrap()
            )
            .status,
            StorageApiStatus::Ok
        );
        assert_eq!(
            status(
                &api.handle(&request(StorageApiOperation::Read, 0, b"/lost", b"", b"", 0, 5))
                    .unwrap()
            )
            .status,
            StorageApiStatus::NotFound
        );
    }

    #[test]
    fn staged_write_is_invisible_ordered_and_atomic() {
        let namespace = DurableNamespace::format(MemoryBlockStore::<16>::new()).unwrap();
        let mut api = StorageApi::new(namespace);
        let begin_message = api
            .handle(&request(StorageApiOperation::StageWriteBegin, 0, b"/stage", b"", b"", 0, 1))
            .unwrap();
        let begin = status(&begin_message);
        let handle = begin.transaction_id;
        assert_eq!(begin.status, StorageApiStatus::Ok);
        assert_eq!(
            status(
                &api.handle(&request(StorageApiOperation::Read, 0, b"/stage", b"", b"", 0, 2))
                    .unwrap()
            )
            .status,
            StorageApiStatus::NotFound
        );
        assert_eq!(
            status(
                &api.handle(&request(
                    StorageApiOperation::StageWriteChunk,
                    handle,
                    b"",
                    b"",
                    b"hello",
                    0,
                    3
                ))
                .unwrap()
            )
            .status,
            StorageApiStatus::Ok
        );
        let second = StorageApiRequest::encode(
            StorageApiOperation::StageWriteChunk,
            0,
            4,
            handle,
            5,
            b"",
            b"",
            b" world",
        )
        .unwrap();
        assert_eq!(status(&api.handle(&second).unwrap()).status, StorageApiStatus::Ok);
        assert_eq!(
            status(
                &api.handle(&request(
                    StorageApiOperation::StageWriteCommit,
                    handle,
                    b"",
                    b"",
                    b"",
                    0,
                    5
                ))
                .unwrap()
            )
            .status,
            StorageApiStatus::Ok
        );
        let read_message =
            api.handle(&request(StorageApiOperation::Read, 0, b"/stage", b"", b"", 0, 6)).unwrap();
        let read = status(&read_message);
        assert_eq!(read.data, b"hello world");

        let replacement_message = api
            .handle(&request(StorageApiOperation::StageWriteBegin, 0, b"/stage", b"", b"", 0, 7))
            .unwrap();
        let replacement = status(&replacement_message).transaction_id;
        assert_eq!(
            status(
                &api.handle(&request(
                    StorageApiOperation::StageWriteChunk,
                    replacement,
                    b"",
                    b"",
                    b"new",
                    0,
                    8
                ))
                .unwrap()
            )
            .status,
            StorageApiStatus::Ok
        );
        assert_eq!(
            status(
                &api.handle(&request(
                    StorageApiOperation::StageWriteAbort,
                    replacement,
                    b"",
                    b"",
                    b"",
                    0,
                    9
                ))
                .unwrap()
            )
            .status,
            StorageApiStatus::Ok
        );
        let read_message =
            api.handle(&request(StorageApiOperation::Read, 0, b"/stage", b"", b"", 0, 10)).unwrap();
        let read = status(&read_message);
        assert_eq!(read.data, b"hello world");
    }

    #[test]
    fn recovery_failure_latches_storage_api_closed() {
        let namespace = DurableNamespace::format(RecoveryFailStore::new()).unwrap();
        let mut api = StorageApi::new(namespace);
        let begin_message =
            api.handle(&request(StorageApiOperation::Begin, 0, b"", b"", b"", 0, 1)).unwrap();
        let begin = status(&begin_message);
        let transaction_id = begin.transaction_id;
        assert_eq!(begin.status, StorageApiStatus::Ok);
        assert_eq!(
            status(
                &api.handle(&request(
                    StorageApiOperation::CreateFile,
                    transaction_id,
                    b"/failed",
                    b"",
                    b"",
                    0,
                    2,
                ))
                .unwrap(),
            )
            .status,
            StorageApiStatus::Ok
        );
        api.namespace.block_store_mut().fail();
        assert_eq!(
            status(
                &api.handle(&request(
                    StorageApiOperation::Commit,
                    transaction_id,
                    b"",
                    b"",
                    b"",
                    0,
                    3,
                ))
                .unwrap(),
            )
            .status,
            StorageApiStatus::Io
        );
        assert_eq!(
            status(
                &api.handle(&request(StorageApiOperation::Begin, 0, b"", b"", b"", 0, 4)).unwrap(),
            )
            .status,
            StorageApiStatus::Io
        );
    }

    #[test]
    fn package_summary_format_is_bounded_and_includes_manifest_version() {
        let manifest = PackageManifest::for_service(
            PackageName::parse(b"flow-addon").unwrap(),
            SemanticVersion::new(2, 1, 0),
            ServiceId::Flow,
        );
        let info = PackageInfo {
            handle: PackageHandle { service: ServiceId::Flow, generation: 1 },
            package_version: 0,
            manifest: Some(manifest),
            bytes: 1,
            crc32c: 0,
            extents: [PackageExtent { start: 1, blocks: 1 }; MAX_PACKAGE_EXTENTS],
            extent_count: 1,
        };
        let mut output = [0; STORAGE_API_RESPONSE_DATA_BYTES];
        let length = format_package_info(&info, &mut output);
        assert_eq!(&output[..length], b"flow-addon 2.1.0\r\n");
    }

    #[test]
    fn package_install_request_imports_an_existing_file() {
        let mut namespace =
            DurableNamespace::format(HeapStore(Box::new(MemoryBlockStore::new()))).unwrap();
        let payload = b"elf";
        let header =
            ServicePackageHeader::new(ServiceId::Flow, 3, payload.len(), crc32c(payload)).unwrap();
        let mut package = [0; PACKAGE_HEADER_BYTES + 3];
        header.encode(&mut package).unwrap();
        package[PACKAGE_HEADER_BYTES..].copy_from_slice(payload);
        let source = namespace.create_file(namespace.root(), b"flow.pkg").unwrap();
        namespace.write(source, 0, &package).unwrap();
        let mut api = StorageApi::new(namespace);

        let response = api
            .handle(&request(StorageApiOperation::PackageInstall, 0, b"/flow.pkg", b"", b"", 0, 9))
            .unwrap();
        let response = status(&response);
        assert_eq!(response.status, StorageApiStatus::Ok);
        assert_eq!(response.data, b"installed generation 1\r\n");
        assert!(api.into_namespace().lookup_package(ServiceId::Flow).is_ok());
    }
}
