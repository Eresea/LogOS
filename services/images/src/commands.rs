#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", no_main)]
#![cfg_attr(not(target_os = "none"), allow(dead_code, unused_imports, unused_variables))]

use core::{
    mem, ptr,
    sync::atomic::{AtomicU32, Ordering},
};

mod common;

use logos_abi::{
    COMPLETION_FLAG_TRUNCATED, CompletionRequest, CompletionResponse, CompletionStatus,
    IPC_FLAG_MORE, IpcBytes, IpcStatus, MAX_COMPLETION_ITEM_BYTES, MessageKind, NetworkOperation,
    NetworkRequest, NetworkResponse, NetworkResult, NetworkState, STORAGE_API_FLAG_REPLACE,
    StorageApiOperation, StorageApiRequest, StorageApiResponse, StorageApiStatus,
};

const INPUT_CAPABILITY: usize = common::capability_slot(
    logos_abi::ServiceId::Commands,
    logos_abi::IpcEndpointId::SessionToCommands,
    logos_abi::IpcRights::Receive,
);
const OUTPUT_CAPABILITY: usize = common::capability_slot(
    logos_abi::ServiceId::Commands,
    logos_abi::IpcEndpointId::CommandsToSession,
    logos_abi::IpcRights::Send,
);
const STORAGE_SEND_CAPABILITY: usize = common::capability_slot(
    logos_abi::ServiceId::Commands,
    logos_abi::IpcEndpointId::CommandsToStorage,
    logos_abi::IpcRights::Send,
);
const STORAGE_RECEIVE_CAPABILITY: usize = common::capability_slot(
    logos_abi::ServiceId::Commands,
    logos_abi::IpcEndpointId::StorageToCommands,
    logos_abi::IpcRights::Receive,
);
const NETWORK_SEND_CAPABILITY: usize = common::capability_slot(
    logos_abi::ServiceId::Commands,
    logos_abi::IpcEndpointId::CommandsToNetwork,
    logos_abi::IpcRights::Send,
);
const NETWORK_RECEIVE_CAPABILITY: usize = common::capability_slot(
    logos_abi::ServiceId::Commands,
    logos_abi::IpcEndpointId::NetworkToCommands,
    logos_abi::IpcRights::Receive,
);

static NEXT_MANAGER_REQUEST_ID: AtomicU32 = AtomicU32::new(1);
static NEXT_NETWORK_REQUEST_ID: AtomicU32 = AtomicU32::new(1);

fn next_manager_request_id() -> u32 {
    loop {
        let current = NEXT_MANAGER_REQUEST_ID.load(Ordering::Relaxed);
        let next = current.wrapping_add(1).max(1);
        if NEXT_MANAGER_REQUEST_ID
            .compare_exchange(current, next, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
        {
            return current;
        }
    }
}

fn next_network_request_id() -> u32 {
    loop {
        let current = NEXT_NETWORK_REQUEST_ID.load(Ordering::Relaxed);
        let next = current.wrapping_add(1).max(1);
        if NEXT_NETWORK_REQUEST_ID
            .compare_exchange(current, next, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
        {
            return current;
        }
    }
}

struct NetworkClient;

impl NetworkClient {
    fn request(
        &mut self,
        operation: NetworkOperation,
        address: [u8; 4],
        port: u16,
    ) -> Result<NetworkResponse, IpcStatus> {
        let mut request = NetworkRequest::new(operation, next_network_request_id());
        request.address = address;
        request.port = port;
        if operation == NetworkOperation::IcmpPing {
            request.timeout_ticks = logos_abi::NETWORK_PING_TIMEOUT_TICKS;
        }
        self.request_message(request)
    }

    fn request_message(&mut self, request: NetworkRequest) -> Result<NetworkResponse, IpcStatus> {
        let request_bytes = unsafe {
            core::slice::from_raw_parts(
                (&request as *const NetworkRequest).cast::<u8>(),
                mem::size_of::<NetworkRequest>(),
            )
        };
        let message = IpcBytes::from_bytes(MessageKind::NetworkRequest, request_bytes)
            .ok_or(IpcStatus::Malformed)?;
        match common::ipc_send(NETWORK_SEND_CAPABILITY, &message) {
            IpcStatus::Ok => {}
            status => return Err(status),
        }
        for _ in 0..256 {
            let mut response = IpcBytes::empty(MessageKind::NetworkResponse);
            match common::ipc_receive(NETWORK_RECEIVE_CAPABILITY, &mut response) {
                IpcStatus::Ok => {
                    if response.kind != MessageKind::NetworkResponse
                        || response.len as usize != mem::size_of::<NetworkResponse>()
                    {
                        return Err(IpcStatus::Malformed);
                    }
                    let value: NetworkResponse =
                        unsafe { ptr::read_unaligned(response.bytes.as_ptr().cast()) };
                    return value.is_valid_for(request).then_some(value).ok_or(IpcStatus::Stale);
                }
                IpcStatus::Empty => common::wait(
                    common::ipc_read_event(logos_abi::IpcEndpointId::NetworkToCommands),
                    logos_abi::ServiceId::Commands,
                ),
                status => return Err(status),
            }
        }
        Err(IpcStatus::Empty)
    }
}

#[cfg(feature = "qemu-proof")]
fn manager_boot_probe() -> bool {
    let request_id = next_manager_request_id();
    let request = logos_abi::ManagerRequest::new(logos_abi::ManagerOperation::List, request_id);
    let mut response = logos_abi::ManagerResponse::new(
        logos_abi::ManagerOperation::List,
        logos_abi::ManagerStatus::Malformed,
        request_id,
    );
    if common::manager_call(&request, &mut response) != IpcStatus::Ok
        || response.status != logos_abi::ManagerStatus::Ok
        || response.record.slot != 0
        || &response.record.name[..usize::from(response.record.name_len)] != b"input"
    {
        return false;
    }
    let mut output = [0; logos_commands::MAX_OUTPUT_BYTES];
    let length = logos_commands::format_service_record(&response.record, &mut output);
    output[..length].starts_with(b"input ") && output[..length].ends_with(b"\r\n")
}

struct CompletionService {
    enabled: bool,
}

impl CompletionService {
    const fn new() -> Self {
        Self { enabled: true }
    }

    fn complete(&mut self, request: CompletionRequest) -> CompletionResponse {
        if !self.enabled || !request.is_valid() {
            return CompletionResponse::empty(request.request_id, CompletionStatus::Unavailable);
        }
        let Some(line) = request.line() else {
            return CompletionResponse::empty(request.request_id, CompletionStatus::Malformed);
        };
        let Ok(Some(context)) =
            logos_commands::completion_context(line, usize::from(request.cursor))
        else {
            return CompletionResponse::empty(request.request_id, CompletionStatus::NoMatch);
        };
        let mut response = CompletionResponse::empty(request.request_id, CompletionStatus::Ok);
        response.replace_start = context.replace_start as u8;
        response.replace_end = context.replace_end as u8;
        match context.target {
            logos_commands::CompletionTarget::Root => {
                for spec in logos_commands::COMMAND_SPECS {
                    if !spec.name.starts_with(context.prefix) {
                        continue;
                    }
                    let punctuation = match spec.kind {
                        logos_commands::CommandKind::Service => b"[\"".as_slice(),
                        logos_commands::CommandKind::Network => b".".as_slice(),
                        _ => b"()".as_slice(),
                    };
                    let mut candidate = [0; MAX_COMPLETION_ITEM_BYTES];
                    let Some(length) = copy_candidate(&mut candidate, spec.name, punctuation)
                    else {
                        response.flags |= COMPLETION_FLAG_TRUNCATED;
                        continue;
                    };
                    if !response.push_candidate(&candidate[..length]) {
                        response.flags |= COMPLETION_FLAG_TRUNCATED;
                        break;
                    }
                }
            }
            logos_commands::CompletionTarget::ServiceName => {
                if self.append_service_names(context.prefix, &mut response).is_err() {
                    response.status = CompletionStatus::Unavailable;
                }
            }
            logos_commands::CompletionTarget::ServiceMember => {
                for candidate in logos_commands::SERVICE_COMPLETION_MEMBERS {
                    if candidate.starts_with(context.prefix) && !response.push_candidate(candidate)
                    {
                        response.flags |= COMPLETION_FLAG_TRUNCATED;
                        break;
                    }
                }
            }
            logos_commands::CompletionTarget::NetworkMember => {
                for candidate in logos_commands::NETWORK_COMPLETION_MEMBERS {
                    if candidate.starts_with(context.prefix) && !response.push_candidate(candidate)
                    {
                        response.flags |= COMPLETION_FLAG_TRUNCATED;
                        break;
                    }
                }
            }
            logos_commands::CompletionTarget::InterfaceName => {
                if b"eth0".starts_with(context.prefix) && !response.push_candidate(b"eth0\"]") {
                    response.flags |= COMPLETION_FLAG_TRUNCATED;
                }
            }
        }
        if response.candidate_count == 0 && response.status == CompletionStatus::Ok {
            response.status = CompletionStatus::NoMatch;
        }
        response
    }

    fn append_service_names(
        &mut self,
        prefix: &[u8],
        response: &mut CompletionResponse,
    ) -> Result<(), ()> {
        let mut cursor = 0u8;
        for _ in 0..logos_abi::MAX_MANAGER_SERVICES {
            let request_id = next_manager_request_id();
            let mut request =
                logos_abi::ManagerRequest::new(logos_abi::ManagerOperation::List, request_id);
            request.cursor = cursor;
            let mut manager_response = logos_abi::ManagerResponse::new(
                logos_abi::ManagerOperation::List,
                logos_abi::ManagerStatus::Malformed,
                request_id,
            );
            if common::manager_call(&request, &mut manager_response) != IpcStatus::Ok
                || manager_response.status != logos_abi::ManagerStatus::Ok
            {
                return Err(());
            }
            let name_len = usize::from(manager_response.record.name_len)
                .min(manager_response.record.name.len());
            let name = &manager_response.record.name[..name_len];
            if name.starts_with(prefix) {
                let mut candidate = [0; MAX_COMPLETION_ITEM_BYTES];
                let Some(length) = copy_candidate(&mut candidate, name, b"\"]") else {
                    response.flags |= COMPLETION_FLAG_TRUNCATED;
                    return Ok(());
                };
                if !response.push_candidate(&candidate[..length]) {
                    response.flags |= COMPLETION_FLAG_TRUNCATED;
                    return Ok(());
                }
            }
            if manager_response.cursor == u8::MAX {
                break;
            }
            cursor = manager_response.cursor;
        }
        Ok(())
    }
}

fn copy_candidate(
    output: &mut [u8; MAX_COMPLETION_ITEM_BYTES],
    first: &[u8],
    second: &[u8],
) -> Option<usize> {
    let length = first.len().checked_add(second.len())?;
    if length > output.len() {
        return None;
    }
    output[..first.len()].copy_from_slice(first);
    output[first.len()..length].copy_from_slice(second);
    Some(length)
}

fn completion_request(message: &IpcBytes) -> Option<CompletionRequest> {
    (message.kind == MessageKind::CompletionRequest
        && message.len as usize == core::mem::size_of::<CompletionRequest>())
    .then(|| unsafe { ptr::read_unaligned(message.bytes.as_ptr().cast()) })
}

fn completion_message(response: CompletionResponse) -> IpcBytes {
    let bytes = unsafe {
        core::slice::from_raw_parts(
            (&response as *const CompletionResponse).cast::<u8>(),
            mem::size_of::<CompletionResponse>(),
        )
    };
    IpcBytes::from_bytes(MessageKind::CompletionResponse, bytes)
        .unwrap_or_else(|| IpcBytes::empty(MessageKind::CompletionResponse))
}

struct PendingOutput {
    bytes: [u8; logos_commands::MAX_OUTPUT_BYTES],
    len: usize,
    offset: usize,
    pending: bool,
}

impl PendingOutput {
    const fn new() -> Self {
        Self { bytes: [0; logos_commands::MAX_OUTPUT_BYTES], len: 0, offset: 0, pending: false }
    }

    fn stage(&mut self, bytes: &[u8]) {
        let count = bytes.len().min(self.bytes.len());
        self.bytes[..count].copy_from_slice(&bytes[..count]);
        self.len = count;
        self.offset = 0;
        self.pending = true;
    }

    fn flush(&mut self, capability_slot: usize) -> bool {
        let mut progressed = false;
        while self.offset < self.len {
            let end = (self.offset + logos_abi::MAX_IPC_BYTES).min(self.len);
            let Some(mut message) =
                IpcBytes::from_bytes(MessageKind::SessionOutput, &self.bytes[self.offset..end])
            else {
                break;
            };
            if end < self.len {
                message.flags = IPC_FLAG_MORE;
            }
            if common::ipc_send(capability_slot, &message) != IpcStatus::Ok {
                break;
            }
            self.offset = end;
            progressed = true;
        }
        if self.pending && self.offset == self.len {
            let message = IpcBytes::empty(MessageKind::SessionOutput);
            if self.len == 0 && common::ipc_send(capability_slot, &message) == IpcStatus::Ok {
                self.pending = false;
                progressed = true;
            }
        }
        if self.pending && self.offset == self.len && self.len != 0 {
            self.len = 0;
            self.offset = 0;
            self.pending = false;
        }
        progressed
    }
}

#[derive(Clone, Copy)]
enum StorageWork {
    List,
    Touch,
    Cat,
    Write,
    Remove,
    Move,
    #[cfg(feature = "storage-proof")]
    AbortProof,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum StoragePhase {
    Begin,
    Operation,
    Commit,
    Abort,
    Read,
    List,
    Idle,
}

struct StorageClient {
    work: StorageWork,
    phase: StoragePhase,
    busy: bool,
    done: bool,
    sent: bool,
    request_id: u32,
    transaction_id: u64,
    cursor: u32,
    path: [u8; logos_commands::MAX_COMMAND_BYTES],
    path_len: usize,
    secondary_path: [u8; logos_commands::MAX_COMMAND_BYTES],
    secondary_len: usize,
    data: [u8; logos_commands::MAX_COMMAND_BYTES],
    data_len: usize,
    result: [u8; logos_commands::MAX_OUTPUT_BYTES],
    result_len: usize,
    failure: StorageApiStatus,
    last_status: StorageApiStatus,
}

impl StorageClient {
    const fn new() -> Self {
        Self {
            work: StorageWork::List,
            phase: StoragePhase::Idle,
            busy: false,
            done: false,
            sent: false,
            request_id: 1,
            transaction_id: 0,
            cursor: 0,
            path: [0; logos_commands::MAX_COMMAND_BYTES],
            path_len: 0,
            secondary_path: [0; logos_commands::MAX_COMMAND_BYTES],
            secondary_len: 0,
            data: [0; logos_commands::MAX_COMMAND_BYTES],
            data_len: 0,
            result: [0; logos_commands::MAX_OUTPUT_BYTES],
            result_len: 0,
            failure: StorageApiStatus::Invalid,
            last_status: StorageApiStatus::Invalid,
        }
    }

    fn start(&mut self, command: logos_commands::StorageCommand<'_>) -> bool {
        let (work, phase, path, secondary, data) = match command {
            logos_commands::StorageCommand::List { path } => {
                (StorageWork::List, StoragePhase::List, path, &[][..], &[][..])
            }
            logos_commands::StorageCommand::Touch { path } => {
                (StorageWork::Touch, StoragePhase::Begin, path, &[][..], &[][..])
            }
            logos_commands::StorageCommand::Cat { path } => {
                (StorageWork::Cat, StoragePhase::Read, path, &[][..], &[][..])
            }
            logos_commands::StorageCommand::Write { path, data } => {
                (StorageWork::Write, StoragePhase::Begin, path, &[][..], data)
            }
            logos_commands::StorageCommand::Remove { path } => {
                (StorageWork::Remove, StoragePhase::Begin, path, &[][..], &[][..])
            }
            logos_commands::StorageCommand::Move { from, to } => {
                (StorageWork::Move, StoragePhase::Begin, from, to, &[][..])
            }
        };
        self.start_work(work, phase, path, secondary, data)
    }

    #[cfg(feature = "storage-proof")]
    fn start_proof_abort(&mut self, path: &[u8]) -> bool {
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
        let Some(path_len) =
            logos_commands::root_relative_path(path, &mut self.path).map(|path| path.len())
        else {
            return false;
        };
        let Some(secondary_len) =
            logos_commands::root_relative_path(secondary, &mut self.secondary_path)
                .map(|path| path.len())
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

    fn active(&self) -> bool {
        self.busy
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
                    StorageWork::Write => StorageApiOperation::Write,
                    StorageWork::Remove => StorageApiOperation::Remove,
                    StorageWork::Move => StorageApiOperation::Rename,
                    #[cfg(feature = "storage-proof")]
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

    fn drive(&mut self) -> bool {
        if !self.busy {
            return false;
        }
        if !self.sent {
            let Some(request) = self.request() else {
                self.fail(StorageApiStatus::Invalid);
                return true;
            };
            match common::ipc_send(STORAGE_SEND_CAPABILITY, &request) {
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
        match common::ipc_receive(STORAGE_RECEIVE_CAPABILITY, &mut message) {
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
        self.handle_response(response);
        true
    }

    fn handle_response(&mut self, response: StorageApiResponse<'_>) {
        if response.status != StorageApiStatus::Ok {
            if self.phase == StoragePhase::Operation && self.transaction_id != 0 {
                self.failure = response.status;
                self.phase = StoragePhase::Abort;
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

    #[cfg(feature = "storage-proof")]
    fn operation_aborts(&self) -> bool {
        matches!(self.work, StorageWork::AbortProof)
    }

    #[cfg(not(feature = "storage-proof"))]
    const fn operation_aborts(&self) -> bool {
        false
    }

    fn fail(&mut self, status: StorageApiStatus) {
        self.last_status = status;
        self.result_len = 0;
        self.append(status_text(status));
        self.phase = StoragePhase::Idle;
        self.busy = false;
        self.done = true;
    }

    fn succeed(&mut self) {
        self.last_status = StorageApiStatus::Ok;
        if matches!(
            self.work,
            StorageWork::Touch | StorageWork::Write | StorageWork::Remove | StorageWork::Move
        ) {
            self.append(b"ok\r\n");
        } else if matches!(self.work, StorageWork::Cat) {
            self.append(b"\r\n");
        }
        self.phase = StoragePhase::Idle;
        self.busy = false;
        self.done = true;
    }

    fn take_result(&mut self, pending: &mut PendingOutput) {
        if self.done {
            pending.stage(&self.result[..self.result_len]);
            self.done = false;
        }
    }

    #[cfg(feature = "storage-proof")]
    fn discard_result(&mut self) -> StorageApiStatus {
        self.done = false;
        self.last_status
    }

    #[cfg(feature = "storage-proof")]
    fn result_equals(&self, expected: &[u8]) -> bool {
        self.result[..self.result_len] == *expected
    }
}

#[cfg(feature = "storage-proof")]
struct StorageProof {
    step: u8,
    active: bool,
    recovery: bool,
}

#[cfg(feature = "storage-proof")]
impl StorageProof {
    const fn new() -> Self {
        Self { step: 0, active: false, recovery: false }
    }

    fn active(&self) -> bool {
        self.active
    }

    fn consume_result(&mut self, storage: &mut StorageClient) -> bool {
        if !storage.done || !self.active {
            return false;
        }
        let expected_content = match (self.recovery, self.step) {
            (false, 3) => Some(&b"durable-api\r\n"[..]),
            (true, 5) => Some(&b"recovered-api\r\n"[..]),
            _ => None,
        };
        let content_valid =
            expected_content.map_or(true, |expected| storage.result_equals(expected));
        let status = storage.discard_result();
        if self.step == 0 && status == StorageApiStatus::AlreadyExists {
            self.recovery = true;
        }
        let accepted = if self.recovery {
            match self.step {
                0 => status == StorageApiStatus::AlreadyExists,
                1 | 2 | 3 => status == StorageApiStatus::Ok,
                5 => status == StorageApiStatus::Ok && content_valid,
                4 | 6 | 7 => matches!(status, StorageApiStatus::Ok | StorageApiStatus::NotFound),
                _ => false,
            }
        } else {
            match self.step {
                0 | 1 | 2 => status == StorageApiStatus::Ok,
                3 => status == StorageApiStatus::Ok && content_valid,
                4 | 5 => status == StorageApiStatus::NotFound,
                _ => false,
            }
        };
        if accepted {
            self.step = self.step.saturating_add(1);
            self.active = false;
            if (!self.recovery && self.step > 5) || (self.recovery && self.step > 7) {
                self.step = u8::MAX;
            }
        } else {
            self.step = u8::MAX;
            self.active = false;
        }
        true
    }

    fn start_next(&mut self, storage: &mut StorageClient, pending: &PendingOutput) -> bool {
        if self.active
            || self.step == u8::MAX
            || storage.active()
            || storage.done
            || pending.pending
        {
            return false;
        }
        let started = match self.step {
            0 => storage.start(logos_commands::StorageCommand::Touch { path: b"/api-survivor" }),
            1 => storage.start(logos_commands::StorageCommand::Write {
                path: b"/api-survivor",
                data: if self.recovery { b"recovered-api" } else { b"durable-api" },
            }),
            2 => storage.start_proof_abort(b"/api-aborted"),
            3 if self.recovery => {
                storage.start(logos_commands::StorageCommand::Touch { path: b"/api-removed" })
            }
            3 => storage.start(logos_commands::StorageCommand::Cat { path: b"/api-survivor" }),
            4 if self.recovery => {
                storage.start(logos_commands::StorageCommand::Remove { path: b"/api-removed" })
            }
            4 => storage.start(logos_commands::StorageCommand::Cat { path: b"/api-aborted" }),
            5 if self.recovery => {
                storage.start(logos_commands::StorageCommand::Cat { path: b"/api-survivor" })
            }
            5 => storage.start(logos_commands::StorageCommand::Cat { path: b"/api-removed" }),
            6 => storage.start(logos_commands::StorageCommand::Cat { path: b"/api-aborted" }),
            7 => storage.start(logos_commands::StorageCommand::Cat { path: b"/api-removed" }),
            _ => false,
        };
        if started {
            self.active = true;
        }
        started
    }
}

fn storage_ipc_error(status: IpcStatus) -> StorageApiStatus {
    match status {
        IpcStatus::Stale => StorageApiStatus::Stale,
        IpcStatus::Malformed => StorageApiStatus::Invalid,
        IpcStatus::Disconnected | IpcStatus::Unauthorized => StorageApiStatus::Io,
        IpcStatus::Ok | IpcStatus::Full | IpcStatus::Empty => StorageApiStatus::Io,
    }
}

fn status_text(status: StorageApiStatus) -> &'static [u8] {
    match status {
        StorageApiStatus::NotFound => b"not found\r\n",
        StorageApiStatus::AlreadyExists => b"already exists\r\n",
        StorageApiStatus::Busy => b"storage busy\r\n",
        StorageApiStatus::Stale => b"stale transaction\r\n",
        StorageApiStatus::TooLarge => b"data too large\r\n",
        StorageApiStatus::NoTransaction => b"no transaction\r\n",
        _ => b"storage error\r\n",
    }
}

fn manager_error(status: logos_abi::ManagerStatus) -> &'static [u8] {
    match status {
        logos_abi::ManagerStatus::Unauthorized => b"service manager unauthorized\r\n",
        logos_abi::ManagerStatus::NotFound => b"service not found\r\n",
        logos_abi::ManagerStatus::Stale => b"stale service handle\r\n",
        logos_abi::ManagerStatus::InvalidState => b"invalid service state\r\n",
        logos_abi::ManagerStatus::Dependency => b"service dependency violation\r\n",
        logos_abi::ManagerStatus::Busy => b"service manager busy\r\n",
        logos_abi::ManagerStatus::Capacity => b"service manager capacity\r\n",
        logos_abi::ManagerStatus::Malformed => b"malformed service request\r\n",
        logos_abi::ManagerStatus::Unsupported => b"service operation unsupported\r\n",
        logos_abi::ManagerStatus::Ok | logos_abi::ManagerStatus::Accepted => {
            b"service manager error\r\n"
        }
    }
}

fn manager_record(name: &[u8]) -> Result<Option<logos_abi::ServiceManagerRecord>, IpcStatus> {
    let mut cursor = 0u8;
    for _ in 0..logos_abi::MAX_MANAGER_SERVICES {
        let request_id = next_manager_request_id();
        let mut request =
            logos_abi::ManagerRequest::new(logos_abi::ManagerOperation::List, request_id);
        request.cursor = cursor;
        let mut response = logos_abi::ManagerResponse::new(
            logos_abi::ManagerOperation::List,
            logos_abi::ManagerStatus::Malformed,
            request_id,
        );
        if common::manager_call(&request, &mut response) != IpcStatus::Ok
            || response.status != logos_abi::ManagerStatus::Ok
        {
            return Err(IpcStatus::Malformed);
        }
        let name_len = usize::from(response.record.name_len).min(response.record.name.len());
        if &response.record.name[..name_len] == name {
            return Ok(Some(response.record));
        }
        if response.cursor == u8::MAX {
            return Ok(None);
        }
        cursor = response.cursor;
    }
    Ok(None)
}

fn service_command(command: logos_commands::ServiceCommand<'_>, pending: &mut PendingOutput) {
    let (operation, name, list, property) = match command {
        logos_commands::ServiceCommand::List => (
            logos_abi::ManagerOperation::List,
            &[][..],
            true,
            logos_commands::ServiceProperty::Record,
        ),
        logos_commands::ServiceCommand::Lookup { name } => (
            logos_abi::ManagerOperation::Status,
            name,
            false,
            logos_commands::ServiceProperty::Record,
        ),
        logos_commands::ServiceCommand::Status { name } => (
            logos_abi::ManagerOperation::Status,
            name,
            false,
            logos_commands::ServiceProperty::Status,
        ),
        logos_commands::ServiceCommand::Name { name } => (
            logos_abi::ManagerOperation::Status,
            name,
            false,
            logos_commands::ServiceProperty::Name,
        ),
        logos_commands::ServiceCommand::Version { name } => (
            logos_abi::ManagerOperation::Status,
            name,
            false,
            logos_commands::ServiceProperty::Version,
        ),
        logos_commands::ServiceCommand::Start { name } => (
            logos_abi::ManagerOperation::Start,
            name,
            false,
            logos_commands::ServiceProperty::Record,
        ),
        logos_commands::ServiceCommand::Stop { name } => (
            logos_abi::ManagerOperation::Stop,
            name,
            false,
            logos_commands::ServiceProperty::Record,
        ),
        logos_commands::ServiceCommand::Restart { name } => (
            logos_abi::ManagerOperation::Restart,
            name,
            false,
            logos_commands::ServiceProperty::Record,
        ),
    };
    let target = if list {
        None
    } else {
        match manager_record(name) {
            Ok(Some(record)) => Some(record),
            Ok(None) => {
                pending.stage(b"service not found\r\n");
                return;
            }
            Err(_) => {
                pending.stage(b"service manager unavailable\r\n");
                return;
            }
        }
    };
    let mut output = [0; logos_commands::MAX_OUTPUT_BYTES];
    let mut output_len = 0;
    let mut cursor = 0u8;
    for _ in 0..logos_abi::MAX_MANAGER_SERVICES {
        let request_id = next_manager_request_id();
        let mut request = logos_abi::ManagerRequest::new(operation, request_id);
        request.cursor = cursor;
        if let Some(record) = target {
            request.slot = record.slot;
            request.generation = record.generation;
        }
        let mut response = logos_abi::ManagerResponse::new(
            operation,
            logos_abi::ManagerStatus::Malformed,
            request_id,
        );
        if common::manager_call(&request, &mut response) != IpcStatus::Ok {
            pending.stage(b"service manager unavailable\r\n");
            return;
        }
        if !matches!(
            response.status,
            logos_abi::ManagerStatus::Ok | logos_abi::ManagerStatus::Accepted
        ) {
            pending.stage(manager_error(response.status));
            return;
        }
        if !list || response.cursor != u8::MAX {
            let record = response.record;
            output_len += logos_commands::format_service_property(
                &record,
                if list { logos_commands::ServiceProperty::Record } else { property },
                &mut output[output_len..],
            );
        }
        if !list || response.cursor == u8::MAX {
            break;
        }
        cursor = response.cursor;
    }
    pending.stage(&output[..output_len]);
}

fn network_result_text(result: NetworkResult) -> &'static [u8] {
    match result {
        NetworkResult::Full => b"network queue full\r\n",
        NetworkResult::WouldBlock => b"network configuring\r\n",
        NetworkResult::Disabled => b"network disabled\r\n",
        NetworkResult::Unavailable => b"network unavailable\r\n",
        NetworkResult::Timeout => b"network timeout\r\n",
        NetworkResult::Stale => b"network restarting\r\n",
        NetworkResult::Refused => b"network refused\r\n",
        NetworkResult::Checksum => b"network checksum failure\r\n",
        NetworkResult::NotFound => b"network socket not found\r\n",
        NetworkResult::Invalid | NetworkResult::Unsupported => b"network request invalid\r\n",
        NetworkResult::Ok => b"ok\r\n",
    }
}

fn network_state_text(state: NetworkState) -> &'static [u8] {
    match state {
        NetworkState::Disabled => b"network disabled\r\n",
        NetworkState::Unavailable => b"network unavailable\r\n",
        NetworkState::Configuring => b"network configuring\r\n",
        NetworkState::Ready => b"network ready\r\n",
        NetworkState::Restarting => b"network restarting\r\n",
        NetworkState::Faulted => b"network unavailable\r\n",
    }
}

fn network_command(
    command: logos_commands::NetworkCommand<'_>,
    client: &mut NetworkClient,
    pending: &mut PendingOutput,
) {
    if let logos_commands::NetworkCommand::InterfaceStatus { name } = command {
        if name != b"eth0" {
            pending.stage(b"network interface not found\r\n");
            return;
        }
    }
    let (operation, address, port, success): (NetworkOperation, [u8; 4], u16, &[u8]) = match command
    {
        logos_commands::NetworkCommand::Status => (NetworkOperation::Status, [0; 4], 0, b""),
        logos_commands::NetworkCommand::InterfaceStatus { .. } => {
            (NetworkOperation::Status, [0; 4], 0, b"")
        }
        logos_commands::NetworkCommand::Ping { address } => {
            (NetworkOperation::IcmpPing, address, 0, b"ping ok\r\n")
        }
        logos_commands::NetworkCommand::TcpProbe { address, port } => {
            (NetworkOperation::TcpConnect, address, port, b"tcp probe ok\r\n")
        }
    };
    match client.request(operation, address, port) {
        Ok(response) if operation == NetworkOperation::Status => {
            pending.stage(network_state_text(response.state))
        }
        Ok(response) if response.result == NetworkResult::Ok => pending.stage(success),
        Ok(response) => pending.stage(network_result_text(response.result)),
        Err(IpcStatus::Stale | IpcStatus::Disconnected) => pending.stage(b"network restarting\r\n"),
        Err(IpcStatus::Unauthorized | IpcStatus::Empty) => {
            pending.stage(b"network unavailable\r\n")
        }
        Err(IpcStatus::Full) => pending.stage(b"network queue full\r\n"),
        Err(IpcStatus::Malformed | IpcStatus::Ok) => pending.stage(b"network request invalid\r\n"),
    }
}

#[cfg(feature = "qemu-proof")]
fn manager_restart_probe() -> bool {
    let Some(record) = manager_record(b"storage").ok().flatten() else {
        return false;
    };
    let request_id = next_manager_request_id();
    let mut request =
        logos_abi::ManagerRequest::new(logos_abi::ManagerOperation::Restart, request_id);
    request.slot = record.slot;
    request.generation = record.generation;
    let mut response = logos_abi::ManagerResponse::new(
        logos_abi::ManagerOperation::Restart,
        logos_abi::ManagerStatus::Malformed,
        request_id,
    );
    common::manager_call(&request, &mut response) == IpcStatus::Ok
        && response.status == logos_abi::ManagerStatus::Accepted
        && response.record.state == logos_abi::ManagerState::Stopping
}

#[cfg(feature = "qemu-proof")]
fn network_proof_probe(network: &mut NetworkClient) -> bool {
    for _ in 0..256 {
        let Ok(status) = network.request(NetworkOperation::Status, [0; 4], 0) else {
            return false;
        };
        if status.state == NetworkState::Disabled {
            return true;
        }
        if status.state == NetworkState::Ready {
            let Ok(tcp) = network.request(NetworkOperation::TcpConnect, [10, 0, 2, 2], 8080) else {
                return false;
            };
            if tcp.result != NetworkResult::Ok || !manager_restart_network() {
                return false;
            }
            for _ in 0..256 {
                let Ok(status) = network.request(NetworkOperation::Status, [0; 4], 0) else {
                    return false;
                };
                if status.state == NetworkState::Ready {
                    let mut listen =
                        NetworkRequest::new(NetworkOperation::TcpListen, next_network_request_id());
                    listen.port = 8081;
                    let Ok(listener) = network.request_message(listen) else {
                        return false;
                    };
                    if listener.result != NetworkResult::Ok {
                        return false;
                    }
                    let Ok(_) = network.request(NetworkOperation::IcmpPing, [10, 0, 2, 2], 0)
                    else {
                        return false;
                    };
                    let accepted = 'accept: {
                        for _ in 0..256 {
                            let mut accept = NetworkRequest::new(
                                NetworkOperation::TcpAccept,
                                next_network_request_id(),
                            );
                            accept.handle = listener.handle;
                            accept.generation = listener.generation;
                            accept.service_epoch = listener.service_epoch;
                            let Ok(response) = network.request_message(accept) else {
                                return false;
                            };
                            if response.result == NetworkResult::Ok {
                                break 'accept response;
                            }
                            if response.result != NetworkResult::WouldBlock {
                                return false;
                            }
                        }
                        return false;
                    };
                    let mut write =
                        NetworkRequest::new(NetworkOperation::TcpWrite, next_network_request_id());
                    write.handle = accepted.handle;
                    write.generation = accepted.generation;
                    write.service_epoch = accepted.service_epoch;
                    write.payload_page = 0;
                    write.payload_len = 1;
                    let mut write_completed = false;
                    for _ in 0..256 {
                        write.request_id = next_network_request_id();
                        let Ok(write_response) = network.request_message(write) else {
                            return false;
                        };
                        if write_response.result == NetworkResult::Ok {
                            write_completed = true;
                            break;
                        }
                        if write_response.result != NetworkResult::WouldBlock {
                            return false;
                        }
                    }
                    if !write_completed {
                        return false;
                    }
                    let mut read_completed = false;
                    for _ in 0..256 {
                        let mut read = NetworkRequest::new(
                            NetworkOperation::TcpRead,
                            next_network_request_id(),
                        );
                        read.handle = accepted.handle;
                        read.generation = accepted.generation;
                        read.service_epoch = accepted.service_epoch;
                        read.payload_page = 0;
                        read.payload_len = 128;
                        let Ok(read_response) = network.request_message(read) else {
                            return false;
                        };
                        if read_response.result == NetworkResult::Ok
                            && read_response.payload_len != 0
                        {
                            read_completed = true;
                            break;
                        }
                        if read_response.result != NetworkResult::WouldBlock {
                            return false;
                        }
                    }
                    if !read_completed {
                        return false;
                    }
                    let mut close =
                        NetworkRequest::new(NetworkOperation::Close, next_network_request_id());
                    close.handle = tcp.handle;
                    close.generation = tcp.generation;
                    close.service_epoch = tcp.service_epoch;
                    return network
                        .request_message(close)
                        .is_ok_and(|response| response.result == NetworkResult::Stale);
                }
                common::wait(0, logos_abi::ServiceId::Commands);
            }
            return false;
        }
        common::wait(0, logos_abi::ServiceId::Commands);
    }
    false
}

#[cfg(feature = "qemu-proof")]
fn manager_restart_network() -> bool {
    let Some(record) = manager_record(b"network").ok().flatten() else {
        return false;
    };
    let request_id = next_manager_request_id();
    let mut request =
        logos_abi::ManagerRequest::new(logos_abi::ManagerOperation::Restart, request_id);
    request.slot = record.slot;
    request.generation = record.generation;
    let mut response = logos_abi::ManagerResponse::new(
        logos_abi::ManagerOperation::Restart,
        logos_abi::ManagerStatus::Malformed,
        request_id,
    );
    common::manager_call(&request, &mut response) == IpcStatus::Ok
        && response.status == logos_abi::ManagerStatus::Accepted
}

#[cfg(feature = "qemu-proof")]
fn manager_command_probe(pending: &mut PendingOutput, network: &mut NetworkClient) -> bool {
    let Some(initial_storage) = manager_record(b"storage").ok().flatten() else {
        return false;
    };
    if initial_storage.generation != 1 {
        return network_proof_probe(network);
    }
    service_command(logos_commands::ServiceCommand::List, pending);
    let list = &pending.bytes[..pending.len];
    let list_valid = pending.pending
        && list
            .windows(b"storage running\r\n".len())
            .any(|window| window == b"storage running\r\n")
        && !list.windows(b"vacant".len()).any(|window| window == b"vacant");
    pending.len = 0;
    pending.offset = 0;
    pending.pending = false;
    if !list_valid {
        return false;
    }
    service_command(logos_commands::ServiceCommand::Stop { name: b"input" }, pending);
    let expected = b"service dependency violation\r\n";
    let dependency_valid = pending.pending
        && pending.len == expected.len()
        && pending.bytes[..pending.len] == *expected;
    pending.len = 0;
    pending.offset = 0;
    pending.pending = false;
    if !dependency_valid {
        return false;
    }
    network_proof_probe(network) && manager_restart_probe()
}

static mut COMMANDS: logos_commands::CommandService = logos_commands::CommandService::new();
static mut PENDING: PendingOutput = PendingOutput::new();
static mut STORAGE: StorageClient = StorageClient::new();
static mut NETWORK: NetworkClient = NetworkClient;
static mut COMPLETION: CompletionService = CompletionService::new();
static mut PENDING_COMPLETION: Option<IpcBytes> = None;

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    let commands = unsafe { &mut *core::ptr::addr_of_mut!(COMMANDS) };
    let pending = unsafe { &mut *core::ptr::addr_of_mut!(PENDING) };
    let storage = unsafe { &mut *core::ptr::addr_of_mut!(STORAGE) };
    let network = unsafe { &mut *core::ptr::addr_of_mut!(NETWORK) };
    let completion = unsafe { &mut *core::ptr::addr_of_mut!(COMPLETION) };
    let pending_completion = unsafe { &mut *core::ptr::addr_of_mut!(PENDING_COMPLETION) };
    #[cfg(feature = "qemu-proof")]
    while !manager_boot_probe() {
        common::wait(0, logos_abi::ServiceId::Commands);
    }
    #[cfg(feature = "qemu-proof")]
    if !manager_command_probe(pending, network) {
        common::idle();
    }
    #[cfg(feature = "storage-proof")]
    let mut proof = StorageProof::new();
    let mut heartbeat_ticks = 0u16;
    loop {
        common::heartbeat_tick(&mut heartbeat_ticks, logos_abi::ServiceId::Commands);
        let mut progressed = pending.flush(OUTPUT_CAPABILITY);
        if pending.pending {
            if !progressed {
                common::wait(
                    common::ipc_write_event(logos_abi::IpcEndpointId::CommandsToSession),
                    logos_abi::ServiceId::Commands,
                );
            }
            continue;
        }
        if let Some(message) = *pending_completion {
            match common::ipc_send(OUTPUT_CAPABILITY, &message) {
                IpcStatus::Ok | IpcStatus::Stale | IpcStatus::Disconnected => {
                    *pending_completion = None;
                    if message.kind == MessageKind::CompletionResponse {
                        progressed = true;
                    }
                }
                IpcStatus::Full => {}
                IpcStatus::Unauthorized | IpcStatus::Malformed | IpcStatus::Empty => {
                    *pending_completion = None;
                }
            }
            if pending_completion.is_some() {
                if !progressed {
                    common::wait(
                        common::ipc_write_event(logos_abi::IpcEndpointId::CommandsToSession),
                        logos_abi::ServiceId::Commands,
                    );
                }
                continue;
            }
        }
        if storage.active() {
            progressed |= storage.drive();
            if storage.done {
                #[cfg(feature = "storage-proof")]
                if !proof.active() {
                    storage.take_result(pending);
                }
                #[cfg(not(feature = "storage-proof"))]
                storage.take_result(pending);
                progressed = true;
            }
            if storage.active() {
                if !progressed {
                    common::wait(
                        common::ipc_read_event(logos_abi::IpcEndpointId::StorageToCommands)
                            | common::ipc_write_event(logos_abi::IpcEndpointId::CommandsToStorage)
                            | common::ipc_read_event(logos_abi::IpcEndpointId::SessionToCommands),
                        logos_abi::ServiceId::Commands,
                    );
                }
                continue;
            }
        }
        #[cfg(feature = "storage-proof")]
        if proof.consume_result(storage) {
            progressed = true;
        }
        #[cfg(feature = "storage-proof")]
        if proof.start_next(storage, pending) {
            progressed = true;
        }
        let mut message = IpcBytes::empty(MessageKind::SessionInput);
        if common::ipc_receive(INPUT_CAPABILITY, &mut message) == IpcStatus::Ok {
            progressed = true;
            if message.kind == MessageKind::CompletionRequest {
                if let Some(request) = completion_request(&message) {
                    *pending_completion = Some(completion_message(completion.complete(request)));
                    progressed = true;
                }
            } else if message.kind == MessageKind::SessionInput {
                if let Some(bytes) = message.as_bytes() {
                    match logos_commands::parse_service_command(bytes) {
                        Ok(Some(command)) => service_command(command, pending),
                        Err(logos_commands::ServiceCommandError::Usage) => {
                            pending.stage(
                                    b"usage: service[\"name\"].status | service[\"name\"].start() | service[\"name\"].stop() | service[\"name\"].restart()\r\n",
                                );
                        }
                        Ok(None) => match logos_commands::parse_network_command(bytes) {
                            Ok(Some(command)) => network_command(command, network, pending),
                            Err(logos_commands::NetworkCommandError::Usage) => {
                                pending.stage(
                                    b"usage: net.status | net.ping(\"address\") | net.interface[\"name\"].status\r\n",
                                );
                            }
                            Ok(None) => match logos_commands::parse_storage_command(bytes) {
                                Ok(Some(command)) => {
                                    if !storage.start(command) {
                                        pending.stage(b"storage request too large\r\n");
                                    }
                                }
                                Err(logos_commands::StorageCommandError::Usage) => {
                                    pending.stage(b"usage error\r\n");
                                }
                                Ok(None) => {
                                    let result = commands.execute(bytes);
                                    if result.action != logos_commands::CommandAction::None {
                                        let status = common::power(result.action as usize);
                                        if status != 0 {
                                            pending.stage(b"power action denied\r\n");
                                        }
                                    } else if result.clear_screen {
                                        pending.stage(b"\x1b[2J\x1b[H");
                                    } else {
                                        pending.stage(result.as_bytes());
                                    }
                                }
                            },
                        },
                    }
                }
            }
        }
        if !progressed {
            common::wait(
                common::ipc_read_event(logos_abi::IpcEndpointId::SessionToCommands)
                    | common::ipc_read_event(logos_abi::IpcEndpointId::StorageToCommands),
                logos_abi::ServiceId::Commands,
            );
        }
    }
}

#[cfg(target_os = "none")]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo<'_>) -> ! {
    common::idle()
}

#[cfg(not(target_os = "none"))]
fn main() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completion_provider_returns_targeted_static_candidates() {
        let mut provider = CompletionService::new();
        let root = provider.complete(CompletionRequest::new(1, b"he", 2).unwrap());
        assert_eq!(root.status, CompletionStatus::Ok);
        assert_eq!(root.candidate(0), Some(&b"help()"[..]));

        let member =
            provider.complete(CompletionRequest::new(2, b"service[\"storage\"].re", 21).unwrap());
        assert_eq!(member.candidate(0), Some(&b"restart()"[..]));

        let interface =
            provider.complete(CompletionRequest::new(3, b"net.interface[\"e", 16).unwrap());
        assert_eq!(interface.candidate(0), Some(&b"eth0\"]"[..]));
    }
}
