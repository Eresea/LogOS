use logos_abi::{
    IpcBytes, STORAGE_API_FLAG_REPLACE, STORAGE_API_RESPONSE_DATA_BYTES, StorageApiOperation,
    StorageApiRequest, StorageApiResponse, StorageApiStatus,
};
use logos_storage::BlockStore;

use crate::{DurableNamespace, NamespaceError, NamespaceTransaction};

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

pub struct StorageApi<B> {
    namespace: DurableNamespace<B>,
    active: Option<ActiveTransaction>,
    next_transaction: u64,
}

impl<B: BlockStore> StorageApi<B> {
    pub fn new(namespace: DurableNamespace<B>) -> Self {
        Self { namespace, active: None, next_transaction: 1 }
    }

    pub fn into_namespace(self) -> DurableNamespace<B> {
        self.namespace
    }

    pub fn handle(&mut self, message: &IpcBytes) -> Option<IpcBytes> {
        let request = match StorageApiRequest::decode(message) {
            Ok(request) => request,
            Err(_) => return malformed_response(message),
        };
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
            Err(error) => ResponsePayload::empty(map_error(error), id),
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
        NamespaceError::Format(_) | NamespaceError::Block(_) => StorageApiStatus::Io,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use logos_abi::{STORAGE_API_FLAG_REPLACE, StorageApiOperation, StorageApiResponse};
    use logos_storage::MemoryBlockStore;

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
}
