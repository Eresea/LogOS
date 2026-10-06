use logos_abi::{
    IpcBytes, IpcStatus, MessageKind, STORAGE_API_FLAG_REPLACE, StorageApiOperation,
    StorageApiRequest, StorageApiResponse, StorageApiStatus,
};

use crate::{
    MAX_FLOW_BYTES, MAX_OUTPUT_BYTES, MAX_STORAGE_DATA_BYTES, PendingOutput, Port, StorageCommand,
    Transport, root_relative_path,
};

#[derive(Clone, Copy)]
enum StorageWork {
    List,
    Touch,
    Cat,
    Write,
    TouchWrite,
    Remove,
    Move,
    AbortProof,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum StoragePhase {
    Begin,
    Operation,
    Commit,
    Abort,
    StageBegin,
    StageChunk,
    StageCommit,
    StageAbort,
    Read,
    List,
    Idle,
}

pub struct StorageClient {
    work: StorageWork,
    phase: StoragePhase,
    busy: bool,
    done: bool,
    sent: bool,
    request_id: u32,
    transaction_id: u64,
    cursor: u32,
    path: [u8; MAX_FLOW_BYTES],
    path_len: usize,
    secondary_path: [u8; MAX_FLOW_BYTES],
    secondary_len: usize,
    data: [u8; MAX_STORAGE_DATA_BYTES],
    data_len: usize,
    result: [u8; MAX_OUTPUT_BYTES],
    result_len: usize,
    failure: StorageApiStatus,
    last_status: StorageApiStatus,
    cancelled: bool,
}

impl Default for StorageClient {
    fn default() -> Self {
        Self::new()
    }
}

impl StorageClient {
    pub const fn new() -> Self {
        Self {
            work: StorageWork::List,
            phase: StoragePhase::Idle,
            busy: false,
            done: false,
            sent: false,
            request_id: 1,
            transaction_id: 0,
            cursor: 0,
            path: [0; MAX_FLOW_BYTES],
            path_len: 0,
            secondary_path: [0; MAX_FLOW_BYTES],
            secondary_len: 0,
            data: [0; MAX_STORAGE_DATA_BYTES],
            data_len: 0,
            result: [0; MAX_OUTPUT_BYTES],
            result_len: 0,
            failure: StorageApiStatus::Invalid,
            last_status: StorageApiStatus::Invalid,
            cancelled: false,
        }
    }

    pub fn start(&mut self, command: StorageCommand<'_>) -> bool {
        let (work, phase, path, secondary, data) = match command {
            StorageCommand::List { path } => {
                (StorageWork::List, StoragePhase::List, path, &[][..], &[][..])
            }
            StorageCommand::Touch { path } => {
                (StorageWork::Touch, StoragePhase::Begin, path, &[][..], &[][..])
            }
            StorageCommand::Cat { path } => {
                (StorageWork::Cat, StoragePhase::Read, path, &[][..], &[][..])
            }
            StorageCommand::Write { path, data } => {
                (StorageWork::Write, StoragePhase::Begin, path, &[][..], data)
            }
            StorageCommand::TouchWrite { path, data } => {
                (StorageWork::TouchWrite, StoragePhase::StageBegin, path, &[][..], data)
            }
            StorageCommand::WriteVariables { .. } => return false,
            StorageCommand::Remove { path } => {
                (StorageWork::Remove, StoragePhase::Begin, path, &[][..], &[][..])
            }
            StorageCommand::Move { from, to } => {
                (StorageWork::Move, StoragePhase::Begin, from, to, &[][..])
            }
        };
        self.start_work(work, phase, path, secondary, data)
    }

    pub fn start_touch_write(&mut self, path: &[u8], data: &[u8]) -> bool {
        self.start_work(StorageWork::TouchWrite, StoragePhase::StageBegin, path, &[], data)
    }

    pub fn start_proof_abort(&mut self, path: &[u8]) -> bool {
        self.failure = StorageApiStatus::Ok;
        self.start_work(StorageWork::AbortProof, StoragePhase::Begin, path, &[], &[])
    }

    fn start_work(
        &mut self,
        work: StorageWork,
        phase: StoragePhase,
        path: &[u8],
        secondary: &[u8],
        data: &[u8],
    ) -> bool {
        if self.busy || self.done {
            return false;
        }
        self.path_len = 0;
        self.secondary_len = 0;
        self.data_len = 0;
        self.result_len = 0;
        self.cursor = 0;
        self.transaction_id = 0;
        self.last_status = StorageApiStatus::Invalid;
        self.cancelled = false;
        let Some(path_len) = root_relative_path(path, &mut self.path).map(|path| path.len()) else {
            return false;
        };
        let Some(secondary_len) =
            root_relative_path(secondary, &mut self.secondary_path).map(|path| path.len())
        else {
            return false;
        };
        if data.len() > self.data.len() {
            return false;
        }
        self.path_len = path_len;
        self.secondary_len = secondary_len;
        self.data[..data.len()].copy_from_slice(data);
        self.data_len = data.len();
        self.work = work;
        self.phase = phase;
        self.busy = true;
        self.done = false;
        self.sent = false;
        true
    }

    pub fn active(&self) -> bool {
        self.busy
    }

    pub fn cancel(&mut self) {
        if !self.busy {
            return;
        }
        self.cancelled = true;
        self.failure = StorageApiStatus::Unsupported;
        if !self.sent {
            if self.transaction_id == 0 {
                self.fail(StorageApiStatus::Unsupported);
            } else {
                self.phase = if matches!(
                    self.phase,
                    StoragePhase::StageBegin | StoragePhase::StageChunk | StoragePhase::StageCommit
                ) {
                    StoragePhase::StageAbort
                } else {
                    StoragePhase::Abort
                };
                self.next_request();
            }
        }
    }

    fn next_request(&mut self) {
        self.request_id = self.request_id.wrapping_add(1).max(1);
        self.sent = false;
    }

    fn request(&self) -> Option<IpcBytes> {
        let (operation, transaction_id, flags, offset, path, secondary, data) = match self.phase {
            StoragePhase::Begin => (StorageApiOperation::Begin, 0, 0, 0, &[][..], &[][..], &[][..]),
            StoragePhase::Operation => {
                let operation = match self.work {
                    StorageWork::Touch => StorageApiOperation::CreateFile,
                    StorageWork::Write | StorageWork::TouchWrite => StorageApiOperation::Write,
                    StorageWork::Remove => StorageApiOperation::Remove,
                    StorageWork::Move => StorageApiOperation::Rename,
                    StorageWork::AbortProof => StorageApiOperation::CreateFile,
                    StorageWork::List | StorageWork::Cat => StorageApiOperation::Read,
                };
                (
                    operation,
                    self.transaction_id,
                    if matches!(self.work, StorageWork::Write) {
                        STORAGE_API_FLAG_REPLACE
                    } else {
                        0
                    },
                    0,
                    &self.path[..self.path_len],
                    &self.secondary_path[..self.secondary_len],
                    &self.data[..self.data_len],
                )
            }
            StoragePhase::Commit => {
                (StorageApiOperation::Commit, self.transaction_id, 0, 0, &[][..], &[][..], &[][..])
            }
            StoragePhase::Abort => {
                (StorageApiOperation::Abort, self.transaction_id, 0, 0, &[][..], &[][..], &[][..])
            }
            StoragePhase::StageBegin => (
                StorageApiOperation::StageWriteBegin,
                0,
                0,
                0,
                &self.path[..self.path_len],
                &[][..],
                &[][..],
            ),
            StoragePhase::StageChunk => {
                let start = self.cursor as usize;
                let end = (start + 192).min(self.data_len);
                (
                    StorageApiOperation::StageWriteChunk,
                    self.transaction_id,
                    0,
                    self.cursor,
                    &[][..],
                    &[][..],
                    &self.data[start..end],
                )
            }
            StoragePhase::StageCommit => (
                StorageApiOperation::StageWriteCommit,
                self.transaction_id,
                0,
                0,
                &[][..],
                &[][..],
                &[][..],
            ),
            StoragePhase::StageAbort => (
                StorageApiOperation::StageWriteAbort,
                self.transaction_id,
                0,
                0,
                &[][..],
                &[][..],
                &[][..],
            ),
            StoragePhase::Read => (
                StorageApiOperation::Read,
                0,
                0,
                self.cursor,
                &self.path[..self.path_len],
                &[][..],
                &[][..],
            ),
            StoragePhase::List => (
                StorageApiOperation::List,
                0,
                0,
                self.cursor,
                &self.path[..self.path_len],
                &[][..],
                &[][..],
            ),
            StoragePhase::Idle => return None,
        };
        StorageApiRequest::encode(
            operation,
            flags,
            self.request_id,
            transaction_id,
            offset,
            path,
            secondary,
            data,
        )
    }

    pub fn drive<T: Transport>(&mut self, transport: &mut T) -> bool {
        if !self.busy {
            return false;
        }
        if !self.sent {
            let Some(request) = self.request() else {
                self.fail(StorageApiStatus::Invalid);
                return true;
            };
            match transport.send(Port::Storage, &request) {
                IpcStatus::Ok => {}
                IpcStatus::Full => return false,
                status => {
                    self.fail(storage_ipc_error(status));
                    return true;
                }
            }
            self.sent = true;
            return true;
        }
        let mut message = IpcBytes::empty(MessageKind::StorageResponse);
        match transport.receive(Port::Storage, &mut message) {
            IpcStatus::Ok => {}
            IpcStatus::Empty => return false,
            status => {
                self.fail(storage_ipc_error(status));
                return true;
            }
        }
        self.sent = false;
        let Ok(response) = StorageApiResponse::decode(&message) else {
            self.fail(StorageApiStatus::Invalid);
            return true;
        };
        if response.request_id != self.request_id {
            self.fail(StorageApiStatus::Stale);
            return true;
        }
        if self.cancelled {
            if self.transaction_id == 0 {
                self.transaction_id = response.transaction_id;
            }
            if self.transaction_id == 0 {
                self.fail(StorageApiStatus::Unsupported);
            } else {
                self.phase = StoragePhase::Abort;
                self.next_request();
            }
            return true;
        }
        self.handle_response(response);
        true
    }

    fn handle_response(&mut self, response: StorageApiResponse<'_>) {
        if response.status != StorageApiStatus::Ok {
            if matches!(
                self.phase,
                StoragePhase::Operation | StoragePhase::StageChunk | StoragePhase::StageCommit
            ) && self.transaction_id != 0
            {
                self.failure = response.status;
                self.phase =
                    if matches!(self.phase, StoragePhase::StageChunk | StoragePhase::StageCommit) {
                        StoragePhase::StageAbort
                    } else {
                        StoragePhase::Abort
                    };
                self.next_request();
            } else {
                self.fail(response.status);
            }
            return;
        }
        match self.phase {
            StoragePhase::Begin => {
                if response.transaction_id == 0 {
                    self.fail(StorageApiStatus::Invalid);
                } else {
                    self.transaction_id = response.transaction_id;
                    self.phase = StoragePhase::Operation;
                    self.next_request();
                }
            }
            StoragePhase::StageBegin => {
                if response.transaction_id == 0 {
                    self.fail(StorageApiStatus::Invalid);
                } else {
                    self.transaction_id = response.transaction_id;
                    self.phase = if self.data_len == 0 {
                        StoragePhase::StageCommit
                    } else {
                        StoragePhase::StageChunk
                    };
                    self.next_request();
                }
            }
            StoragePhase::StageChunk => {
                self.cursor = self.cursor.saturating_add(
                    (self.data_len.saturating_sub(self.cursor as usize).min(192)) as u32,
                );
                self.phase = if self.cursor as usize >= self.data_len {
                    StoragePhase::StageCommit
                } else {
                    StoragePhase::StageChunk
                };
                self.next_request();
            }
            StoragePhase::StageCommit => self.succeed(),
            StoragePhase::StageAbort => self.fail(self.failure),
            StoragePhase::Operation => {
                self.phase = if self.operation_aborts() {
                    StoragePhase::Abort
                } else {
                    StoragePhase::Commit
                };
                self.next_request();
            }
            StoragePhase::Commit => self.succeed(),
            StoragePhase::Abort => self.fail(self.failure),
            StoragePhase::Read => {
                if response.data.is_empty() && response.more {
                    self.fail(StorageApiStatus::Invalid);
                } else {
                    self.append(response.data);
                    self.cursor = self.cursor.saturating_add(response.data.len() as u32);
                    if response.more && self.result_len < self.result.len() {
                        self.next_request();
                    } else {
                        self.succeed();
                    }
                }
            }
            StoragePhase::List => {
                if response.data.len() + 2 <= self.result.len() - self.result_len {
                    self.append(response.data);
                    self.append(b"\r\n");
                }
                self.cursor = self.cursor.saturating_add(1);
                if response.more && self.result_len < self.result.len() {
                    self.next_request();
                } else {
                    self.succeed();
                }
            }
            StoragePhase::Idle => self.fail(StorageApiStatus::Invalid),
        }
    }

    fn append(&mut self, bytes: &[u8]) {
        let count = bytes.len().min(self.result.len() - self.result_len);
        self.result[self.result_len..self.result_len + count].copy_from_slice(&bytes[..count]);
        self.result_len += count;
    }

    /// Only the storage-proof `AbortProof` work aborts its transaction after
    /// the operation succeeds; every Flow command commits.
    fn operation_aborts(&self) -> bool {
        matches!(self.work, StorageWork::AbortProof)
    }

    fn fail(&mut self, status: StorageApiStatus) {
        self.last_status = status;
        self.result_len = 0;
        if self.cancelled {
            self.append(b"command cancelled\r\n");
            self.cancelled = false;
        } else {
            self.append(status_text(status));
        }
        self.phase = StoragePhase::Idle;
        self.busy = false;
        self.done = true;
    }

    fn succeed(&mut self) {
        self.last_status = StorageApiStatus::Ok;
        if matches!(
            self.work,
            StorageWork::Touch
                | StorageWork::Write
                | StorageWork::TouchWrite
                | StorageWork::Remove
                | StorageWork::Move
        ) {
            self.append(b"ok\r\n");
        } else if matches!(self.work, StorageWork::Cat) {
            self.append(b"\r\n");
        }
        self.phase = StoragePhase::Idle;
        self.busy = false;
        self.done = true;
    }

    pub fn done(&self) -> bool {
        self.done
    }

    pub fn take_result<T: Transport>(&mut self, transport: &mut T, pending: &mut PendingOutput) {
        if self.done {
            if self.result[..self.result_len] == *b"LogOS-Fetch\r\n" {
                transport.proof_line(b"LogOS vNext: fetch contents verified");
            }
            pending.stage(&self.result[..self.result_len]);
            self.done = false;
        }
    }

    pub fn discard_result(&mut self) -> StorageApiStatus {
        self.done = false;
        self.last_status
    }

    pub fn result_equals(&self, expected: &[u8]) -> bool {
        self.result[..self.result_len] == *expected
    }
}

pub fn storage_ipc_error(status: IpcStatus) -> StorageApiStatus {
    match status {
        IpcStatus::Stale => StorageApiStatus::Stale,
        IpcStatus::Malformed => StorageApiStatus::Invalid,
        IpcStatus::Disconnected => StorageApiStatus::Unavailable,
        IpcStatus::Unauthorized => StorageApiStatus::PermissionDenied,
        IpcStatus::Full => StorageApiStatus::Busy,
        IpcStatus::Ok | IpcStatus::Empty => StorageApiStatus::Io,
    }
}

pub fn status_text(status: StorageApiStatus) -> &'static [u8] {
    match status {
        StorageApiStatus::Invalid => b"invalid storage request\r\n",
        StorageApiStatus::NotFound => b"not found\r\n",
        StorageApiStatus::AlreadyExists => b"already exists\r\n",
        StorageApiStatus::Busy => b"storage busy\r\n",
        StorageApiStatus::Capacity => b"storage capacity exhausted\r\n",
        StorageApiStatus::Io => b"storage I/O error\r\n",
        StorageApiStatus::Unsupported => b"storage unsupported\r\n",
        StorageApiStatus::Unavailable => b"storage service unavailable\r\n",
        StorageApiStatus::PermissionDenied => b"storage access denied\r\n",
        StorageApiStatus::ReadOnly => b"storage is read-only\r\n",
        StorageApiStatus::Recovery => b"storage recovery required\r\n",
        StorageApiStatus::Corrupt => b"storage format is invalid\r\n",
        StorageApiStatus::NotDirectory => b"not a directory\r\n",
        StorageApiStatus::IsDirectory => b"is a directory\r\n",
        StorageApiStatus::Root => b"cannot modify root\r\n",
        StorageApiStatus::NotEmpty => b"directory not empty\r\n",
        StorageApiStatus::Stale => b"stale transaction\r\n",
        StorageApiStatus::TooLarge => b"data too large\r\n",
        StorageApiStatus::NoTransaction => b"no transaction\r\n",
        _ => b"storage error\r\n",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::fake::FakeTransport;
    use std::{vec, vec::Vec};

    /// What the client asked for on one wire request.
    #[derive(Debug, PartialEq)]
    struct Seen {
        operation: StorageApiOperation,
        transaction_id: u64,
        offset: u32,
        flags: u8,
        path: Vec<u8>,
        data: Vec<u8>,
    }

    fn request_id(transport: &FakeTransport) -> u32 {
        let message: IpcBytes = transport.last_sent(Port::Storage);
        StorageApiRequest::decode(&message).unwrap().request_id
    }

    /// Send the client's next request, answer it, let the client consume the
    /// answer, and report what the request was.
    fn exchange(
        client: &mut StorageClient,
        transport: &mut FakeTransport,
        status: StorageApiStatus,
        transaction_id: u64,
        data: &[u8],
        more: bool,
    ) -> Seen {
        assert!(client.drive(transport), "request is sent");
        let message: IpcBytes = transport.last_sent(Port::Storage);
        let request = StorageApiRequest::decode(&message).unwrap();
        let seen = Seen {
            operation: request.operation,
            transaction_id: request.transaction_id,
            offset: request.offset,
            flags: request.flags,
            path: request.path.to_vec(),
            data: request.data.to_vec(),
        };
        let reply =
            StorageApiResponse::encode(status, request.request_id, transaction_id, data, more)
                .unwrap();
        transport.reply(Port::Storage, &reply);
        assert!(client.drive(transport), "reply is consumed");
        seen
    }

    fn ok(
        client: &mut StorageClient,
        transport: &mut FakeTransport,
        transaction_id: u64,
        data: &[u8],
        more: bool,
    ) -> Seen {
        exchange(client, transport, StorageApiStatus::Ok, transaction_id, data, more)
    }

    fn output(client: &mut StorageClient, transport: &mut FakeTransport) -> Vec<u8> {
        assert!(client.done() && !client.active());
        let mut pending = PendingOutput::new();
        client.take_result(transport, &mut pending);
        pending.staged().to_vec()
    }

    #[test]
    fn cat_reads_every_chunk_then_appends_a_newline() {
        let mut transport = FakeTransport::new();
        let mut client = StorageClient::new();
        assert!(client.start(StorageCommand::Cat { path: b"note" }));
        let first = ok(&mut client, &mut transport, 0, b"hel", true);
        assert_eq!(first.operation, StorageApiOperation::Read);
        assert_eq!(first.path, b"/note");
        let second = ok(&mut client, &mut transport, 0, b"lo", false);
        assert_eq!(second.offset, 3, "the next read resumes after the bytes already received");
        assert_eq!(output(&mut client, &mut transport), b"hello\r\n");
    }

    #[test]
    fn write_replaces_inside_one_begin_operation_commit_transaction() {
        let mut transport = FakeTransport::new();
        let mut client = StorageClient::new();
        assert!(client.start(StorageCommand::Write { path: b"/a", data: b"xyz" }));
        let begin = ok(&mut client, &mut transport, 7, b"", false);
        assert_eq!(begin.operation, StorageApiOperation::Begin);
        let write = ok(&mut client, &mut transport, 0, b"", false);
        assert_eq!(write.operation, StorageApiOperation::Write);
        assert_eq!((write.transaction_id, write.flags), (7, STORAGE_API_FLAG_REPLACE));
        assert_eq!(write.data, b"xyz");
        let commit = ok(&mut client, &mut transport, 0, b"", false);
        assert_eq!((commit.operation, commit.transaction_id), (StorageApiOperation::Commit, 7));
        assert_eq!(output(&mut client, &mut transport), b"ok\r\n");
    }

    #[test]
    fn move_and_remove_report_ok() {
        let mut transport = FakeTransport::new();
        let mut client = StorageClient::new();
        assert!(client.start(StorageCommand::Move { from: b"a", to: b"b" }));
        ok(&mut client, &mut transport, 3, b"", false);
        let rename = ok(&mut client, &mut transport, 0, b"", false);
        assert_eq!(rename.operation, StorageApiOperation::Rename);
        ok(&mut client, &mut transport, 0, b"", false);
        assert_eq!(output(&mut client, &mut transport), b"ok\r\n");
    }

    #[test]
    fn list_joins_entries_with_crlf() {
        let mut transport = FakeTransport::new();
        let mut client = StorageClient::new();
        assert!(client.start(StorageCommand::List { path: b"/" }));
        ok(&mut client, &mut transport, 0, b"a", true);
        ok(&mut client, &mut transport, 0, b"b", false);
        assert_eq!(output(&mut client, &mut transport), b"a\r\nb\r\n");
    }

    #[test]
    fn mismatched_request_id_fails_as_stale() {
        let mut transport = FakeTransport::new();
        let mut client = StorageClient::new();
        assert!(client.start(StorageCommand::Cat { path: b"/a" }));
        assert!(client.drive(&mut transport));
        let wrong = request_id(&transport).wrapping_add(1);
        let reply =
            StorageApiResponse::encode(StorageApiStatus::Ok, wrong, 0, b"x", false).unwrap();
        transport.reply(Port::Storage, &reply);
        assert!(client.drive(&mut transport));
        assert_eq!(output(&mut client, &mut transport), b"stale transaction\r\n");
    }

    #[test]
    fn error_status_maps_to_the_same_text() {
        let mut transport = FakeTransport::new();
        let mut client = StorageClient::new();
        assert!(client.start(StorageCommand::Cat { path: b"/missing" }));
        exchange(&mut client, &mut transport, StorageApiStatus::NotFound, 0, b"", false);
        assert_eq!(output(&mut client, &mut transport), b"not found\r\n");

        assert!(client.start(StorageCommand::Touch { path: b"/x" }));
        exchange(&mut client, &mut transport, StorageApiStatus::Capacity, 0, b"", false);
        assert_eq!(output(&mut client, &mut transport), b"storage capacity exhausted\r\n");
    }

    #[test]
    fn failed_operation_aborts_its_transaction_then_reports_the_cause() {
        let mut transport = FakeTransport::new();
        let mut client = StorageClient::new();
        assert!(client.start(StorageCommand::Touch { path: b"/dup" }));
        ok(&mut client, &mut transport, 5, b"", false);
        let create =
            exchange(&mut client, &mut transport, StorageApiStatus::AlreadyExists, 0, b"", false);
        assert_eq!(create.operation, StorageApiOperation::CreateFile);
        assert!(client.active(), "the open transaction is aborted before the failure is shown");
        let abort = ok(&mut client, &mut transport, 0, b"", false);
        assert_eq!((abort.operation, abort.transaction_id), (StorageApiOperation::Abort, 5));
        assert_eq!(output(&mut client, &mut transport), b"already exists\r\n");
    }

    #[test]
    fn staged_write_streams_chunks_then_commits() {
        let mut transport = FakeTransport::new();
        let mut client = StorageClient::new();
        let data = [b'd'; 400];
        assert!(client.start(StorageCommand::TouchWrite { path: b"/big", data: &data }));
        let begin = ok(&mut client, &mut transport, 9, b"", false);
        assert_eq!(begin.operation, StorageApiOperation::StageWriteBegin);
        let mut offsets = Vec::new();
        let mut lengths = Vec::new();
        for _ in 0..3 {
            let chunk = ok(&mut client, &mut transport, 0, b"", false);
            assert_eq!(chunk.operation, StorageApiOperation::StageWriteChunk);
            assert_eq!(chunk.transaction_id, 9);
            offsets.push(chunk.offset);
            lengths.push(chunk.data.len());
        }
        assert_eq!((offsets, lengths), (vec![0, 192, 384], vec![192, 192, 16]));
        let commit = ok(&mut client, &mut transport, 0, b"", false);
        assert_eq!(commit.operation, StorageApiOperation::StageWriteCommit);
        assert_eq!(output(&mut client, &mut transport), b"ok\r\n");
    }

    #[test]
    fn staged_write_failure_aborts_the_stage() {
        let mut transport = FakeTransport::new();
        let mut client = StorageClient::new();
        assert!(client.start(StorageCommand::TouchWrite { path: b"/big", data: b"payload" }));
        ok(&mut client, &mut transport, 9, b"", false);
        let chunk =
            exchange(&mut client, &mut transport, StorageApiStatus::Capacity, 0, b"", false);
        assert_eq!(chunk.operation, StorageApiOperation::StageWriteChunk);
        let abort = ok(&mut client, &mut transport, 0, b"", false);
        assert_eq!(
            (abort.operation, abort.transaction_id),
            (StorageApiOperation::StageWriteAbort, 9)
        );
        assert_eq!(output(&mut client, &mut transport), b"storage capacity exhausted\r\n");
    }

    #[test]
    fn cancelling_a_staged_write_requests_a_stage_abort() {
        let mut transport = FakeTransport::new();
        let mut client = StorageClient::new();
        assert!(client.start(StorageCommand::TouchWrite { path: b"/big", data: b"payload" }));
        ok(&mut client, &mut transport, 9, b"", false);
        client.cancel();
        assert!(client.drive(&mut transport));
        let message: IpcBytes = transport.last_sent(Port::Storage);
        let request = StorageApiRequest::decode(&message).unwrap();
        assert_eq!(
            (request.operation, request.transaction_id),
            (StorageApiOperation::StageWriteAbort, 9)
        );
    }

    #[test]
    fn cancelling_before_any_transaction_ends_immediately() {
        let mut transport = FakeTransport::new();
        let mut client = StorageClient::new();
        assert!(client.start(StorageCommand::Touch { path: b"/a" }));
        client.cancel();
        assert_eq!(output(&mut client, &mut transport), b"command cancelled\r\n");
    }

    #[test]
    fn proof_abort_creates_then_aborts_and_reports_a_non_error_status() {
        let mut transport = FakeTransport::new();
        let mut client = StorageClient::new();
        assert!(client.start_proof_abort(b"/api-aborted"));
        ok(&mut client, &mut transport, 4, b"", false);
        let create = ok(&mut client, &mut transport, 0, b"", false);
        assert_eq!(create.operation, StorageApiOperation::CreateFile);
        let abort = ok(&mut client, &mut transport, 0, b"", false);
        assert_eq!(abort.operation, StorageApiOperation::Abort);
        assert!(client.done());
        assert_eq!(client.discard_result(), StorageApiStatus::Ok);
    }

    #[test]
    fn busy_client_and_oversized_data_are_refused() {
        let mut client = StorageClient::new();
        assert!(
            !client.start(StorageCommand::Write {
                path: b"/a",
                data: &[0; MAX_STORAGE_DATA_BYTES + 1]
            })
        );
        assert!(client.start(StorageCommand::Touch { path: b"/a" }));
        assert!(!client.start(StorageCommand::Touch { path: b"/b" }));
    }

    #[test]
    fn full_queue_waits_and_dead_peer_reports_the_cause() {
        let mut transport = FakeTransport::new();
        transport.fail_sends(Port::Storage, IpcStatus::Full);
        let mut client = StorageClient::new();
        assert!(client.start(StorageCommand::Touch { path: b"/a" }));
        assert!(!client.drive(&mut transport));
        assert!(client.active());

        let mut transport = FakeTransport::new();
        transport.fail_sends(Port::Storage, IpcStatus::Disconnected);
        let mut client = StorageClient::new();
        assert!(client.start(StorageCommand::Touch { path: b"/a" }));
        assert!(client.drive(&mut transport));
        assert_eq!(output(&mut client, &mut transport), b"storage service unavailable\r\n");
    }

    #[test]
    fn fetch_contents_marker_is_emitted_through_the_transport() {
        let mut transport = FakeTransport::new();
        let mut client = StorageClient::new();
        assert!(client.start(StorageCommand::Cat { path: b"/fetched" }));
        ok(&mut client, &mut transport, 0, b"LogOS-Fetch", false);
        output(&mut client, &mut transport);
        assert_eq!(transport.proof_lines, [b"LogOS vNext: fetch contents verified".to_vec()]);
    }

    #[test]
    fn storage_failures_preserve_actionable_causes() {
        assert_eq!(storage_ipc_error(IpcStatus::Disconnected), StorageApiStatus::Unavailable);
        assert_eq!(storage_ipc_error(IpcStatus::Unauthorized), StorageApiStatus::PermissionDenied);
        assert_eq!(storage_ipc_error(IpcStatus::Full), StorageApiStatus::Busy);
        assert_eq!(status_text(StorageApiStatus::Recovery), b"storage recovery required\r\n");
        assert_eq!(status_text(StorageApiStatus::ReadOnly), b"storage is read-only\r\n");
    }
}
