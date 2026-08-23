#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", no_main)]
#![cfg_attr(not(target_os = "none"), allow(dead_code, unused_imports, unused_variables))]

mod common;

use core::mem;
use logos_abi::{
    FetchBodyChunk, FetchControl, FetchPhase, FetchRequest, FetchResponse, FetchStatus, IpcBytes,
    IpcStatus, MessageKind, NetworkOperation, NetworkRequest, NetworkResponse, NetworkResult,
    StorageApiOperation, StorageApiRequest, StorageApiResponse, StorageApiStatus,
};
use logos_fetch::{ResponseParser, Url};

const FLOW_RECEIVE: common::CapabilitySpec = common::capability_contract(
    logos_abi::IPC_CONTRACT_BYTES,
    logos_abi::ServiceId::Flow.index() as u32,
    mem::size_of::<IpcBytes>(),
    logos_abi::IpcRights::Receive,
);
const FLOW_SEND: common::CapabilitySpec = common::capability_contract(
    logos_abi::IPC_CONTRACT_BYTES,
    logos_abi::ServiceId::Flow.index() as u32,
    mem::size_of::<IpcBytes>(),
    logos_abi::IpcRights::Send,
);
const STORAGE_SEND: common::CapabilitySpec = common::capability_contract(
    logos_abi::IPC_CONTRACT_BYTES,
    logos_abi::ServiceId::Storage.index() as u32,
    mem::size_of::<IpcBytes>(),
    logos_abi::IpcRights::Send,
);
const STORAGE_RECEIVE: common::CapabilitySpec = common::capability_contract(
    logos_abi::IPC_CONTRACT_BYTES,
    logos_abi::ServiceId::Storage.index() as u32,
    mem::size_of::<IpcBytes>(),
    logos_abi::IpcRights::Receive,
);
const NETWORK_SEND: common::CapabilitySpec = common::capability_contract(
    logos_abi::IPC_CONTRACT_BYTES,
    logos_abi::ServiceId::Network.index() as u32,
    mem::size_of::<IpcBytes>(),
    logos_abi::IpcRights::Send,
);
const NETWORK_RECEIVE: common::CapabilitySpec = common::capability_contract(
    logos_abi::IPC_CONTRACT_BYTES,
    logos_abi::ServiceId::Network.index() as u32,
    mem::size_of::<IpcBytes>(),
    logos_abi::IpcRights::Receive,
);

#[derive(Clone, Copy)]
struct IpcCapabilities {
    flow_receive: logos_abi::CapabilityHandle,
    flow_send: logos_abi::CapabilityHandle,
    storage_send: logos_abi::CapabilityHandle,
    storage_receive: logos_abi::CapabilityHandle,
    network_send: logos_abi::CapabilityHandle,
    network_receive: logos_abi::CapabilityHandle,
}

static mut IPC_CAPABILITIES: Option<IpcCapabilities> = None;

fn ipc_capabilities() -> IpcCapabilities {
    unsafe {
        (*core::ptr::addr_of!(IPC_CAPABILITIES)).unwrap_or(IpcCapabilities {
            flow_receive: logos_abi::CapabilityHandle::EMPTY,
            flow_send: logos_abi::CapabilityHandle::EMPTY,
            storage_send: logos_abi::CapabilityHandle::EMPTY,
            storage_receive: logos_abi::CapabilityHandle::EMPTY,
            network_send: logos_abi::CapabilityHandle::EMPTY,
            network_receive: logos_abi::CapabilityHandle::EMPTY,
        })
    }
}

const MAX_PATH_BYTES: usize = 160;
const POLL_TIMEOUT_TICKS: u32 = 30_000;

#[derive(Clone, Copy, Eq, PartialEq)]
enum PendingKind {
    Network,
    Storage,
    Body,
}

struct Operation {
    request_id: u32,
    url: Url,
    destination: [u8; MAX_PATH_BYTES],
    destination_len: usize,
    response_mode: bool,
    request_bytes: [u8; logos_abi::NETWORK_INLINE_PAYLOAD_BYTES],
    request_len: usize,
    parser: ResponseParser,
    phase: FetchPhase,
    network_handle: u32,
    network_generation: u16,
    network_epoch: u64,
    stage_handle: u64,
    staged_offset: usize,
    pending: Option<(PendingKind, u32)>,
    next_network_request: u32,
    next_storage_request: u32,
    started: u32,
    next_progress: usize,
    body_offset: usize,
    cancel: bool,
}

impl Operation {
    fn new(request: FetchRequest, now: u32) -> Result<Self, FetchStatus> {
        let url = Url::parse(request.url().ok_or(FetchStatus::Invalid)?)
            .map_err(|_| FetchStatus::Invalid)?;
        let destination = request.destination().ok_or(FetchStatus::Invalid)?;
        let response_mode = destination.is_empty();
        let needs_root = !response_mode && !destination.starts_with(b"/");
        let destination_len = destination.len() + usize::from(needs_root);
        if destination_len > MAX_PATH_BYTES {
            return Err(FetchStatus::Invalid);
        }
        let (request_bytes, request_len) = url.request().map_err(|_| FetchStatus::Invalid)?;
        let mut destination_copy = [0; MAX_PATH_BYTES];
        if needs_root {
            destination_copy[0] = b'/';
            destination_copy[1..destination_len].copy_from_slice(destination);
        } else {
            destination_copy[..destination_len].copy_from_slice(destination);
        }
        Ok(Self {
            request_id: request.request_id,
            url,
            destination: destination_copy,
            destination_len,
            response_mode,
            request_bytes,
            request_len,
            parser: ResponseParser::new(),
            phase: FetchPhase::Connect,
            network_handle: 0,
            network_generation: 0,
            network_epoch: 0,
            stage_handle: 0,
            staged_offset: 0,
            pending: None,
            next_network_request: 1,
            next_storage_request: 1,
            started: now,
            next_progress: 512,
            body_offset: 0,
            cancel: false,
        })
    }

    fn response(&self, status: FetchStatus) -> FetchResponse {
        FetchResponse::new(
            self.request_id,
            self.phase,
            status,
            self.parser.body().len() as u32,
            self.parser.content_length().map(|n| n as u32),
        )
        .with_response_status(self.parser.status())
    }
}

fn bytes_message<T: Copy>(kind: MessageKind, value: &T) -> Option<IpcBytes> {
    let bytes = unsafe {
        core::slice::from_raw_parts((value as *const T).cast::<u8>(), mem::size_of::<T>())
    };
    IpcBytes::from_bytes(kind, bytes)
}

fn send_progress(operation: &Operation, status: FetchStatus) {
    if let Some(message) = bytes_message(MessageKind::FetchResponse, &operation.response(status)) {
        let _ = common::ipc_send_handle(ipc_capabilities().flow_send, &message);
    }
}

fn send_body_chunk(operation: &mut Operation) -> Result<bool, FetchStatus> {
    let body = operation.parser.body();
    if operation.body_offset >= body.len() {
        return Ok(true);
    }
    let end = (operation.body_offset + logos_abi::FETCH_BODY_CHUNK_BYTES).min(body.len());
    let Some(chunk) = FetchBodyChunk::new(
        operation.request_id,
        operation.body_offset as u32,
        &body[operation.body_offset..end],
    ) else {
        return Err(FetchStatus::Malformed);
    };
    let Some(message) = bytes_message(MessageKind::FetchBodyChunk, &chunk) else {
        return Err(FetchStatus::Malformed);
    };
    match common::ipc_send_handle(ipc_capabilities().flow_send, &message) {
        IpcStatus::Ok => {
            operation.body_offset = end;
            Ok(operation.body_offset == body.len())
        }
        IpcStatus::Full => Ok(false),
        _ => Err(FetchStatus::Network),
    }
}

fn send_network(operation: &mut Operation, mut request: NetworkRequest) -> bool {
    request.request_id = operation.next_network_request;
    operation.next_network_request = operation.next_network_request.wrapping_add(1).max(1);
    let Some(message) = bytes_message(MessageKind::NetworkRequest, &request) else {
        return false;
    };
    if common::ipc_send_handle(ipc_capabilities().network_send, &message) != IpcStatus::Ok {
        return false;
    }
    operation.pending = Some((PendingKind::Network, request.request_id));
    true
}

fn retry_network(operation: &mut Operation) -> bool {
    let mut request = match operation.phase {
        FetchPhase::SendRequest => NetworkRequest::new(NetworkOperation::TcpWrite, 1),
        FetchPhase::ReadResponse => NetworkRequest::new(NetworkOperation::TcpRead, 1),
        _ => return false,
    };
    request.handle = operation.network_handle;
    request.generation = operation.network_generation;
    request.service_epoch = operation.network_epoch;
    if operation.phase == FetchPhase::SendRequest {
        request.payload_len = operation.request_len as u16;
        request.payload[..operation.request_len]
            .copy_from_slice(&operation.request_bytes[..operation.request_len]);
    } else {
        request.payload_len = logos_abi::NETWORK_INLINE_PAYLOAD_BYTES as u16;
    }
    send_network(operation, request)
}

fn send_storage(operation: &mut Operation, message: Option<IpcBytes>, request_id: u32) -> bool {
    let Some(message) = message else {
        return false;
    };
    if common::ipc_send_handle(ipc_capabilities().storage_send, &message) != IpcStatus::Ok {
        return false;
    }
    operation.pending = Some((PendingKind::Storage, request_id));
    true
}

fn storage_request(
    operation: &mut Operation,
    action: StorageApiOperation,
    transaction: u64,
    offset: u32,
    path: &[u8],
    data: &[u8],
) -> (Option<IpcBytes>, u32) {
    let request_id = operation.next_storage_request;
    operation.next_storage_request = request_id.wrapping_add(1).max(1);
    (
        StorageApiRequest::encode(action, 0, request_id, transaction, offset, path, &[], data),
        request_id,
    )
}

fn cleanup(operation: &mut Operation, abort_stage: bool) {
    if let Some((PendingKind::Network, request_id)) = operation.pending {
        let cancel = NetworkRequest::new(NetworkOperation::Cancel, request_id);
        if let Some(message) = bytes_message(MessageKind::NetworkRequest, &cancel) {
            let _ = common::ipc_send_handle(ipc_capabilities().network_send, &message);
        }
    }
    if operation.network_handle != 0 {
        let mut close =
            NetworkRequest::new(NetworkOperation::Close, operation.next_network_request);
        operation.next_network_request = operation.next_network_request.wrapping_add(1).max(1);
        close.handle = operation.network_handle;
        close.generation = operation.network_generation;
        close.service_epoch = operation.network_epoch;
        if let Some(message) = bytes_message(MessageKind::NetworkRequest, &close) {
            let _ = common::ipc_send_handle(ipc_capabilities().network_send, &message);
        }
    }
    if abort_stage && operation.stage_handle != 0 {
        let request_id = operation.next_storage_request;
        operation.next_storage_request = operation.next_storage_request.wrapping_add(1).max(1);
        if let Some(message) = StorageApiRequest::encode(
            StorageApiOperation::StageWriteAbort,
            0,
            request_id,
            operation.stage_handle,
            0,
            &[],
            &[],
            &[],
        ) {
            let _ = common::ipc_send_handle(ipc_capabilities().storage_send, &message);
        }
    }
}

fn finish(operation: &mut Option<Operation>, status: FetchStatus) {
    if let Some(current) = operation.as_mut() {
        cleanup(current, status != FetchStatus::Ok);
        current.phase = match status {
            FetchStatus::Ok => FetchPhase::Complete,
            FetchStatus::Cancelled => FetchPhase::Cancelled,
            _ => FetchPhase::Failed,
        };
        send_progress(current, status);
    }
    *operation = None;
}

fn handle_network(operation: &mut Operation, response: NetworkResponse) -> Option<FetchStatus> {
    if response.request_id != operation.pending.map_or(0, |(_, id)| id) {
        return Some(FetchStatus::Stale);
    }
    operation.pending = None;
    if response.result == NetworkResult::WouldBlock {
        return if retry_network(operation) { None } else { Some(FetchStatus::Network) };
    }
    if response.result != NetworkResult::Ok {
        return Some(match response.result {
            NetworkResult::Timeout => FetchStatus::Timeout,
            NetworkResult::Stale => FetchStatus::Stale,
            _ => FetchStatus::Network,
        });
    }
    match operation.phase {
        FetchPhase::Connect => {
            operation.network_handle = response.handle;
            operation.network_generation = response.generation;
            operation.network_epoch = response.service_epoch;
            operation.phase = FetchPhase::SendRequest;
            send_progress(operation, FetchStatus::InProgress);
            let mut request = NetworkRequest::new(NetworkOperation::TcpWrite, 1);
            request.handle = operation.network_handle;
            request.generation = operation.network_generation;
            request.service_epoch = operation.network_epoch;
            request.payload_len = operation.request_len as u16;
            request.payload[..operation.request_len]
                .copy_from_slice(&operation.request_bytes[..operation.request_len]);
            if !send_network(operation, request) {
                return Some(FetchStatus::Network);
            }
        }
        FetchPhase::SendRequest => {
            operation.phase = FetchPhase::ReadResponse;
            send_progress(operation, FetchStatus::InProgress);
            let mut request = NetworkRequest::new(NetworkOperation::TcpRead, 1);
            request.handle = operation.network_handle;
            request.generation = operation.network_generation;
            request.service_epoch = operation.network_epoch;
            request.payload_len = logos_abi::NETWORK_INLINE_PAYLOAD_BYTES as u16;
            if !send_network(operation, request) {
                return Some(FetchStatus::Network);
            }
        }
        FetchPhase::ReadResponse => {
            if response.payload_len != 0 {
                let data = &response.payload[..usize::from(response.payload_len)];
                if let Err(error) = operation.parser.feed(data) {
                    return Some(if matches!(error, logos_fetch::ResponseError::TooLarge) {
                        FetchStatus::Oversized
                    } else {
                        FetchStatus::Malformed
                    });
                }
                if operation.parser.body().len() >= operation.next_progress {
                    operation.next_progress = operation.next_progress.saturating_add(512);
                    send_progress(operation, FetchStatus::InProgress);
                }
            }
            if operation.parser.complete() {
                if operation.response_mode {
                    operation.phase = FetchPhase::Complete;
                    match send_body_chunk(operation) {
                        Ok(true) => return Some(FetchStatus::Ok),
                        Ok(false) => operation.pending = Some((PendingKind::Body, 0)),
                        Err(status) => return Some(status),
                    }
                } else {
                    operation.phase = FetchPhase::StageStorage;
                    send_progress(operation, FetchStatus::InProgress);
                    let mut path = [0; MAX_PATH_BYTES];
                    let path_len = operation.destination_len;
                    path[..path_len].copy_from_slice(&operation.destination[..path_len]);
                    let (message, id) = storage_request(
                        operation,
                        StorageApiOperation::StageWriteBegin,
                        0,
                        0,
                        &path[..path_len],
                        &[],
                    );
                    if !send_storage(operation, message, id) {
                        return Some(FetchStatus::Storage);
                    }
                }
            } else {
                let mut request = NetworkRequest::new(NetworkOperation::TcpRead, 1);
                request.handle = operation.network_handle;
                request.generation = operation.network_generation;
                request.service_epoch = operation.network_epoch;
                request.payload_len = logos_abi::NETWORK_INLINE_PAYLOAD_BYTES as u16;
                if !send_network(operation, request) {
                    return Some(FetchStatus::Network);
                }
            }
        }
        _ => {}
    }
    None
}

fn handle_storage(
    operation: &mut Operation,
    response: StorageApiResponse<'_>,
) -> Option<FetchStatus> {
    if response.request_id != operation.pending.map_or(0, |(_, id)| id) {
        return Some(FetchStatus::Stale);
    }
    operation.pending = None;
    if response.status != StorageApiStatus::Ok {
        return Some(if response.status == StorageApiStatus::TooLarge {
            FetchStatus::Oversized
        } else {
            FetchStatus::Storage
        });
    }
    match operation.phase {
        FetchPhase::StageStorage if operation.stage_handle == 0 => {
            operation.stage_handle = response.transaction_id;
            if operation.parser.body().is_empty() {
                operation.phase = FetchPhase::Commit;
                send_progress(operation, FetchStatus::InProgress);
                let (message, id) = storage_request(
                    operation,
                    StorageApiOperation::StageWriteCommit,
                    operation.stage_handle,
                    0,
                    &[],
                    &[],
                );
                if !send_storage(operation, message, id) {
                    return Some(FetchStatus::Storage);
                }
            } else {
                let end = operation.parser.body().len().min(192);
                let mut chunk = [0; 192];
                chunk[..end].copy_from_slice(&operation.parser.body()[..end]);
                let handle = operation.stage_handle;
                let (message, id) = storage_request(
                    operation,
                    StorageApiOperation::StageWriteChunk,
                    handle,
                    0,
                    &[],
                    &chunk[..end],
                );
                operation.staged_offset = end;
                if !send_storage(operation, message, id) {
                    return Some(FetchStatus::Storage);
                }
            }
        }
        FetchPhase::StageStorage => {
            let body_len = operation.parser.body().len();
            if operation.staged_offset < body_len {
                let end = (operation.staged_offset + 192).min(body_len);
                let offset = operation.staged_offset;
                let mut chunk = [0; 192];
                chunk[..end - offset].copy_from_slice(&operation.parser.body()[offset..end]);
                let handle = operation.stage_handle;
                let (message, id) = storage_request(
                    operation,
                    StorageApiOperation::StageWriteChunk,
                    handle,
                    offset as u32,
                    &[],
                    &chunk[..end - offset],
                );
                operation.staged_offset = end;
                if !send_storage(operation, message, id) {
                    return Some(FetchStatus::Storage);
                }
            } else {
                operation.phase = FetchPhase::Commit;
                send_progress(operation, FetchStatus::InProgress);
                let (message, id) = storage_request(
                    operation,
                    StorageApiOperation::StageWriteCommit,
                    operation.stage_handle,
                    0,
                    &[],
                    &[],
                );
                if !send_storage(operation, message, id) {
                    return Some(FetchStatus::Storage);
                }
            }
        }
        FetchPhase::Commit => {
            operation.phase = FetchPhase::Complete;
            return Some(FetchStatus::Ok);
        }
        _ => {}
    }
    None
}

#[cfg(target_os = "none")]
#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    common::init_service_allocator();
    let capabilities = IpcCapabilities {
        flow_receive: match common::capability_handle(FLOW_RECEIVE) {
            Ok(capability) => capability,
            Err(_) => common::idle(),
        },
        flow_send: match common::capability_handle(FLOW_SEND) {
            Ok(capability) => capability,
            Err(_) => common::idle(),
        },
        storage_send: match common::capability_handle(STORAGE_SEND) {
            Ok(capability) => capability,
            Err(_) => common::idle(),
        },
        storage_receive: match common::capability_handle(STORAGE_RECEIVE) {
            Ok(capability) => capability,
            Err(_) => common::idle(),
        },
        network_send: match common::capability_handle(NETWORK_SEND) {
            Ok(capability) => capability,
            Err(_) => common::idle(),
        },
        network_receive: match common::capability_handle(NETWORK_RECEIVE) {
            Ok(capability) => capability,
            Err(_) => common::idle(),
        },
    };
    unsafe { *core::ptr::addr_of_mut!(IPC_CAPABILITIES) = Some(capabilities) };
    let mut operation: Option<Operation> = None;
    let mut ticks = 0u16;
    let mut elapsed = 0u32;
    loop {
        common::heartbeat_tick(&mut ticks);
        elapsed = elapsed.saturating_add(1);
        let mut control = IpcBytes::empty(MessageKind::FetchControl);
        if common::ipc_receive_handle(ipc_capabilities().flow_receive, &mut control)
            == IpcStatus::Ok
            && control.len as usize == mem::size_of::<FetchControl>()
        {
            let value: FetchControl =
                unsafe { core::ptr::read_unaligned(control.bytes.as_ptr().cast()) };
            if value.is_valid()
                && operation.as_ref().is_some_and(|current| current.request_id == value.request_id)
            {
                if let Some(current) = operation.as_mut() {
                    current.cancel = true;
                }
            }
        }
        if let Some(current) = operation.as_ref() {
            let timed_out = elapsed.wrapping_sub(current.started) >= POLL_TIMEOUT_TICKS;
            let cancelled = current.cancel;
            if cancelled || timed_out {
                finish(
                    &mut operation,
                    if cancelled { FetchStatus::Cancelled } else { FetchStatus::Timeout },
                );
                continue;
            }
        }
        if operation.is_none() {
            let mut message = IpcBytes::empty(MessageKind::FetchRequest);
            if common::ipc_receive_handle(ipc_capabilities().flow_receive, &mut message)
                == IpcStatus::Ok
                && message.len as usize == mem::size_of::<FetchRequest>()
            {
                let request: FetchRequest =
                    unsafe { core::ptr::read_unaligned(message.bytes.as_ptr().cast()) };
                if request.is_valid() {
                    match Operation::new(request, elapsed) {
                        Ok(mut current) => {
                            send_progress(&current, FetchStatus::InProgress);
                            let mut connect = NetworkRequest::new(NetworkOperation::TcpConnect, 1);
                            connect.timeout_ticks = logos_abi::NETWORK_TCP_CONNECT_TIMEOUT_TICKS;
                            connect.address = current.url.address;
                            connect.port = current.url.port;
                            if send_network(&mut current, connect) {
                                operation = Some(current);
                            }
                        }
                        Err(status) => {
                            let response = FetchResponse::new(
                                request.request_id,
                                FetchPhase::Failed,
                                status,
                                0,
                                None,
                            );
                            if let Some(message) =
                                bytes_message(MessageKind::FetchResponse, &response)
                            {
                                let _ =
                                    common::ipc_send_handle(ipc_capabilities().flow_send, &message);
                            }
                        }
                    }
                }
            }
        }
        if let Some(current) = operation.as_mut() {
            if let Some((kind, _)) = current.pending {
                if kind == PendingKind::Network {
                    let mut message = IpcBytes::empty(MessageKind::NetworkResponse);
                    if common::ipc_receive_handle(ipc_capabilities().network_receive, &mut message)
                        == IpcStatus::Ok
                        && message.len as usize == mem::size_of::<NetworkResponse>()
                    {
                        let response: NetworkResponse =
                            unsafe { core::ptr::read_unaligned(message.bytes.as_ptr().cast()) };
                        if let Some(status) = handle_network(current, response) {
                            finish(&mut operation, status);
                        }
                    }
                } else if kind == PendingKind::Storage {
                    let mut message = IpcBytes::empty(MessageKind::StorageResponse);
                    if common::ipc_receive_handle(ipc_capabilities().storage_receive, &mut message)
                        == IpcStatus::Ok
                    {
                        if let Ok(response) = StorageApiResponse::decode(&message) {
                            if let Some(status) = handle_storage(current, response) {
                                finish(&mut operation, status);
                            }
                        } else {
                            finish(&mut operation, FetchStatus::Storage);
                        }
                    }
                } else {
                    current.pending = None;
                    match send_body_chunk(current) {
                        Ok(true) => finish(&mut operation, FetchStatus::Ok),
                        Ok(false) => current.pending = Some((PendingKind::Body, 0)),
                        Err(status) => finish(&mut operation, status),
                    }
                }
            }
        }
        common::wait_on_capabilities(&[
            ipc_capabilities().flow_receive,
            ipc_capabilities().network_receive,
            ipc_capabilities().storage_receive,
        ]);
    }
}

#[cfg(target_os = "none")]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo<'_>) -> ! {
    common::idle()
}

#[cfg(not(target_os = "none"))]
fn main() {}
