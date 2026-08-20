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
    DeviceOperation, DeviceRequest, DeviceResponse, DeviceStatus, FetchBodyChunk, FetchControl,
    FetchPhase, FetchRequest, FetchResponse, FetchStatus, FlowControl, IPC_FLAG_MORE, IpcBytes,
    IpcStatus, MAX_COMPLETION_ITEM_BYTES, MessageKind, NetworkOperation, NetworkRequest,
    NetworkResponse, NetworkResult, NetworkState, STORAGE_API_FLAG_REPLACE, StorageApiOperation,
    StorageApiRequest, StorageApiResponse, StorageApiStatus, UserOperation, UserRequest,
    UserResponse, UserStatus,
};

const INPUT_CAPABILITY: usize = common::capability_slot(
    logos_abi::ServiceId::Flow,
    logos_abi::IpcEndpointId::SessionToFlow,
    logos_abi::IpcRights::Receive,
);
const OUTPUT_CAPABILITY: usize = common::capability_slot(
    logos_abi::ServiceId::Flow,
    logos_abi::IpcEndpointId::FlowToSession,
    logos_abi::IpcRights::Send,
);
const STORAGE_SEND_CAPABILITY: usize = common::capability_slot(
    logos_abi::ServiceId::Flow,
    logos_abi::IpcEndpointId::FlowToStorage,
    logos_abi::IpcRights::Send,
);
const STORAGE_RECEIVE_CAPABILITY: usize = common::capability_slot(
    logos_abi::ServiceId::Flow,
    logos_abi::IpcEndpointId::StorageToFlow,
    logos_abi::IpcRights::Receive,
);
const NETWORK_SEND_CAPABILITY: usize = common::capability_slot(
    logos_abi::ServiceId::Flow,
    logos_abi::IpcEndpointId::FlowToNetwork,
    logos_abi::IpcRights::Send,
);
const NETWORK_RECEIVE_CAPABILITY: usize = common::capability_slot(
    logos_abi::ServiceId::Flow,
    logos_abi::IpcEndpointId::NetworkToFlow,
    logos_abi::IpcRights::Receive,
);
const FETCH_SEND_CAPABILITY: usize = common::capability_slot(
    logos_abi::ServiceId::Flow,
    logos_abi::IpcEndpointId::FlowToFetch,
    logos_abi::IpcRights::Send,
);
const FETCH_RECEIVE_CAPABILITY: usize = common::capability_slot(
    logos_abi::ServiceId::Flow,
    logos_abi::IpcEndpointId::FetchToFlow,
    logos_abi::IpcRights::Receive,
);
const DEVICE_SEND_CAPABILITY: usize = common::capability_slot(
    logos_abi::ServiceId::Flow,
    logos_abi::IpcEndpointId::FlowToDevice,
    logos_abi::IpcRights::Send,
);
const DEVICE_RECEIVE_CAPABILITY: usize = common::capability_slot(
    logos_abi::ServiceId::Flow,
    logos_abi::IpcEndpointId::DeviceToFlow,
    logos_abi::IpcRights::Receive,
);
const USER_SEND_CAPABILITY: usize = common::capability_slot(
    logos_abi::ServiceId::Flow,
    logos_abi::IpcEndpointId::FlowToUser,
    logos_abi::IpcRights::Send,
);
const USER_RECEIVE_CAPABILITY: usize = common::capability_slot(
    logos_abi::ServiceId::Flow,
    logos_abi::IpcEndpointId::UserToFlow,
    logos_abi::IpcRights::Receive,
);

static NEXT_MANAGER_REQUEST_ID: AtomicU32 = AtomicU32::new(1);
static NEXT_NETWORK_REQUEST_ID: AtomicU32 = AtomicU32::new(1);
static NEXT_DEVICE_REQUEST_ID: AtomicU32 = AtomicU32::new(1);
static NEXT_USER_REQUEST_ID: AtomicU32 = AtomicU32::new(1);

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

fn next_device_request_id() -> u32 {
    loop {
        let current = NEXT_DEVICE_REQUEST_ID.load(Ordering::Relaxed);
        let next = current.wrapping_add(1).max(1);
        if NEXT_DEVICE_REQUEST_ID
            .compare_exchange(current, next, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
        {
            return current;
        }
    }
}

fn next_user_request_id() -> u32 {
    loop {
        let current = NEXT_USER_REQUEST_ID.load(Ordering::Relaxed);
        let next = current.wrapping_add(1).max(1);
        if NEXT_USER_REQUEST_ID
            .compare_exchange(current, next, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
        {
            return current;
        }
    }
}

struct NetworkClient {
    cancelled: bool,
}

impl NetworkClient {
    const fn new() -> Self {
        Self { cancelled: false }
    }

    fn take_cancelled(&mut self) -> bool {
        let cancelled = self.cancelled;
        self.cancelled = false;
        cancelled
    }
}

struct FetchClient {
    active: bool,
    request_id: u32,
    initial_progress: Option<FetchResponse>,
    cancel_pending: bool,
    response_mode: bool,
    foreground: bool,
    response_status: u16,
    response_ok: bool,
    body: [u8; logos_flow::interpreter::MAX_VALUE_BYTES],
    body_len: usize,
    promise_name: [u8; logos_flow::interpreter::MAX_VARIABLE_NAME_BYTES],
    promise_name_len: usize,
    callback_destination: [u8; logos_flow::MAX_FLOW_BYTES],
    callback_destination_len: usize,
}

impl FetchClient {
    const fn new() -> Self {
        Self {
            active: false,
            request_id: 0,
            initial_progress: None,
            cancel_pending: false,
            response_mode: false,
            foreground: true,
            response_status: 0,
            response_ok: false,
            body: [0; logos_flow::interpreter::MAX_VALUE_BYTES],
            body_len: 0,
            promise_name: [0; logos_flow::interpreter::MAX_VARIABLE_NAME_BYTES],
            promise_name_len: 0,
            callback_destination: [0; logos_flow::MAX_FLOW_BYTES],
            callback_destination_len: 0,
        }
    }

    fn active(&self) -> bool {
        self.active
    }

    fn start_with_mode(&mut self, url: &[u8], destination: &[u8], foreground: bool) -> bool {
        if self.active {
            return false;
        }
        let request_id = next_network_request_id();
        let Some(request) = FetchRequest::new(request_id, url, destination) else {
            return false;
        };
        let bytes = unsafe {
            core::slice::from_raw_parts(
                (&request as *const FetchRequest).cast::<u8>(),
                mem::size_of::<FetchRequest>(),
            )
        };
        let Some(message) = IpcBytes::from_bytes(MessageKind::FetchRequest, bytes) else {
            return false;
        };
        if common::ipc_send(FETCH_SEND_CAPABILITY, &message) != IpcStatus::Ok {
            return false;
        }
        self.active = true;
        self.request_id = request_id;
        self.cancel_pending = false;
        self.response_mode = destination.is_empty();
        self.foreground = foreground;
        self.response_status = 0;
        self.response_ok = false;
        self.body_len = 0;
        self.callback_destination_len = 0;
        self.initial_progress = Some(FetchResponse::new(
            request_id,
            FetchPhase::Connect,
            FetchStatus::InProgress,
            0,
            None,
        ));
        #[cfg(feature = "fetch-proof")]
        common::proof_line(b"LogOS vNext: Flow fetch started");
        true
    }

    fn start(&mut self, url: &[u8], destination: &[u8]) -> bool {
        self.start_with_mode(url, destination, true)
    }

    fn start_response(&mut self, url: &[u8]) -> bool {
        self.start_with_mode(url, &[], true)
    }

    fn start_response_background(&mut self, url: &[u8]) -> bool {
        self.start_with_mode(url, &[], false)
    }

    fn start_named_response(&mut self, url: &[u8], name: &[u8], foreground: bool) -> bool {
        if name.len() > self.promise_name.len() || !self.start_with_mode(url, &[], foreground) {
            return false;
        }
        self.promise_name[..name.len()].copy_from_slice(name);
        self.promise_name_len = name.len();
        true
    }

    fn start_to_file_mode(&mut self, url: &[u8], destination: &[u8], foreground: bool) -> bool {
        self.start_with_mode(url, destination, foreground)
    }

    fn start_response_to_file(&mut self, url: &[u8], destination: &[u8], foreground: bool) -> bool {
        if destination.is_empty() || destination.len() > self.callback_destination.len() {
            return false;
        }
        if !self.start_with_mode(url, &[], foreground) {
            return false;
        }
        self.callback_destination[..destination.len()].copy_from_slice(destination);
        self.callback_destination_len = destination.len();
        true
    }

    fn foreground(&self) -> bool {
        self.foreground
    }

    fn take_callback(&mut self) -> Option<(&[u8], &[u8])> {
        if self.callback_destination_len == 0 || self.active || !self.response_ok {
            return None;
        }
        Some((
            &self.callback_destination[..self.callback_destination_len],
            &self.body[..self.body_len],
        ))
    }

    fn clear_callback(&mut self) {
        self.callback_destination_len = 0;
        self.body_len = 0;
        self.response_ok = false;
    }

    fn active_promise_is(&self, name: &[u8]) -> bool {
        self.promise_name_len == name.len() && self.promise_name[..self.promise_name_len] == *name
    }

    fn resolve_promise(&mut self, flow: &mut logos_flow::FlowService) {
        if self.active || self.promise_name_len == 0 {
            return;
        }
        let name = &self.promise_name[..self.promise_name_len];
        if self.response_ok {
            let _ = flow.resolve_response_promise(
                name,
                self.response_status,
                &self.body[..self.body_len],
            );
        } else {
            let _ = flow.cancel_promise(name);
        }
        self.promise_name_len = 0;
    }

    fn cancel(&mut self) {
        if !self.active {
            return;
        }
        self.cancel_pending = true;
    }

    fn send_cancel(&mut self) -> IpcStatus {
        let control = FetchControl::cancel(self.request_id);
        let bytes = unsafe {
            core::slice::from_raw_parts(
                (&control as *const FetchControl).cast::<u8>(),
                mem::size_of::<FetchControl>(),
            )
        };
        if let Some(message) = IpcBytes::from_bytes(MessageKind::FetchControl, bytes) {
            return common::ipc_send(FETCH_SEND_CAPABILITY, &message);
        }
        IpcStatus::Malformed
    }

    fn drive(&mut self, pending: &mut PendingOutput) -> bool {
        if let Some(response) = self.initial_progress {
            match forward_fetch_progress(response) {
                IpcStatus::Ok => self.initial_progress = None,
                IpcStatus::Full => return false,
                _ => self.initial_progress = None,
            }
        }
        if self.cancel_pending {
            match self.send_cancel() {
                IpcStatus::Ok => self.cancel_pending = false,
                IpcStatus::Full => return false,
                _ => self.cancel_pending = false,
            }
        }
        let mut message = IpcBytes::empty(MessageKind::FetchResponse);
        match common::ipc_receive(FETCH_RECEIVE_CAPABILITY, &mut message) {
            IpcStatus::Empty => return false,
            IpcStatus::Ok => {}
            IpcStatus::Stale | IpcStatus::Disconnected | IpcStatus::Unauthorized => {
                self.active = false;
                pending.stage(b"fetch failed\r\n");
                return true;
            }
            IpcStatus::Full | IpcStatus::Malformed => {
                self.active = false;
                pending.stage(b"fetch failed\r\n");
                return true;
            }
        }
        if message.kind == MessageKind::FetchBodyChunk {
            if message.len as usize != mem::size_of::<FetchBodyChunk>() {
                self.active = false;
                pending.stage(b"fetch body malformed\r\n");
                return true;
            }
            let chunk: FetchBodyChunk =
                unsafe { ptr::read_unaligned(message.bytes.as_ptr().cast()) };
            let end = chunk.offset as usize + usize::from(chunk.len);
            if !self.response_mode
                || !chunk.is_valid()
                || chunk.request_id != self.request_id
                || chunk.offset as usize != self.body_len
                || end > self.body.len()
            {
                self.active = false;
                pending.stage(b"fetch body stale\r\n");
                return true;
            }
            self.body[self.body_len..end].copy_from_slice(&chunk.bytes[..usize::from(chunk.len)]);
            self.body_len = end;
            return true;
        }
        if message.len as usize != mem::size_of::<FetchResponse>() {
            self.active = false;
            pending.stage(b"fetch failed\r\n");
            return true;
        }
        let response: FetchResponse = unsafe { ptr::read_unaligned(message.bytes.as_ptr().cast()) };
        if !response.is_valid() || response.request_id != self.request_id {
            self.active = false;
            pending.stage(b"fetch failed\r\n");
            return true;
        }
        if matches!(
            response.phase,
            FetchPhase::Complete | FetchPhase::Failed | FetchPhase::Cancelled
        ) {
            self.response_status = response.response_status;
            self.response_ok = response.status == FetchStatus::Ok;
            self.active = false;
            self.cancel_pending = false;
            let message = match response.status {
                FetchStatus::Ok => {
                    #[cfg(feature = "fetch-proof")]
                    common::proof_line(b"LogOS vNext: Flow fetch complete");
                    b"fetch complete\r\n" as &[u8]
                }
                FetchStatus::Cancelled => {
                    #[cfg(feature = "fetch-proof")]
                    common::proof_line(b"LogOS vNext: Flow fetch cancelled");
                    b"fetch cancelled\r\n"
                }
                _ => {
                    #[cfg(feature = "fetch-proof")]
                    common::proof_line(b"LogOS vNext: Flow fetch failed");
                    b"fetch failed\r\n"
                }
            };
            if !(response.status == FetchStatus::Ok
                && !self.foreground
                && self.callback_destination_len == 0)
            {
                pending.stage(message);
            }
        } else {
            let _ = forward_fetch_progress(response);
        }
        true
    }
}

fn forward_fetch_progress(response: FetchResponse) -> IpcStatus {
    let bytes = unsafe {
        core::slice::from_raw_parts(
            (&response as *const FetchResponse).cast::<u8>(),
            mem::size_of::<FetchResponse>(),
        )
    };
    let Some(message) = IpcBytes::from_bytes(MessageKind::FlowProgress, bytes) else {
        return IpcStatus::Malformed;
    };
    common::ipc_send(OUTPUT_CAPABILITY, &message)
}

fn fetch_control(message: &IpcBytes, fetch: &mut FetchClient) -> bool {
    if message.kind != MessageKind::FlowControl
        || message.len as usize != mem::size_of::<FlowControl>()
    {
        return false;
    }
    let control: FlowControl = unsafe { ptr::read_unaligned(message.bytes.as_ptr().cast()) };
    if control.is_valid() && (control.request_id == 0 || control.request_id == fetch.request_id) {
        fetch.cancel();
        true
    } else {
        false
    }
}

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
        } else if operation == NetworkOperation::TcpConnect {
            request.timeout_ticks = logos_abi::NETWORK_TCP_CONNECT_TIMEOUT_TICKS;
        }
        self.request_message(request)
    }

    fn request_message(&mut self, request: NetworkRequest) -> Result<NetworkResponse, IpcStatus> {
        self.cancelled = false;
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
        let mut cancel_requested = false;
        let mut cancel_sent = false;
        for _ in 0..256 {
            if !cancel_requested {
                let mut control = IpcBytes::empty(MessageKind::FlowControl);
                if common::ipc_receive(INPUT_CAPABILITY, &mut control) == IpcStatus::Ok
                    && control.len as usize == mem::size_of::<FlowControl>()
                {
                    let value: FlowControl =
                        unsafe { ptr::read_unaligned(control.bytes.as_ptr().cast()) };
                    cancel_requested = value.is_valid()
                        && (value.request_id == 0 || value.request_id == request.request_id);
                }
            }
            if cancel_requested && !cancel_sent {
                let cancel = NetworkRequest::new(NetworkOperation::Cancel, request.request_id);
                let cancel_bytes = unsafe {
                    core::slice::from_raw_parts(
                        (&cancel as *const NetworkRequest).cast::<u8>(),
                        mem::size_of::<NetworkRequest>(),
                    )
                };
                let cancel_message =
                    IpcBytes::from_bytes(MessageKind::NetworkRequest, cancel_bytes)
                        .ok_or(IpcStatus::Malformed)?;
                match common::ipc_send(NETWORK_SEND_CAPABILITY, &cancel_message) {
                    IpcStatus::Ok => cancel_sent = true,
                    IpcStatus::Full => {
                        common::wait(
                            common::ipc_write_event(logos_abi::IpcEndpointId::FlowToNetwork),
                            logos_abi::ServiceId::Flow,
                        );
                        continue;
                    }
                    status => return Err(status),
                }
            }
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
                    if cancel_sent {
                        if value.operation == NetworkOperation::Cancel
                            && value.request_id == request.request_id
                        {
                            self.cancelled = true;
                            return Err(IpcStatus::Empty);
                        }
                        continue;
                    }
                    return value.is_valid_for(request).then_some(value).ok_or(IpcStatus::Stale);
                }
                IpcStatus::Empty => common::wait(
                    common::ipc_read_event(logos_abi::IpcEndpointId::NetworkToFlow)
                        | common::ipc_read_event(logos_abi::IpcEndpointId::SessionToFlow),
                    logos_abi::ServiceId::Flow,
                ),
                status => return Err(status),
            }
        }
        if cancel_sent {
            self.cancelled = true;
        }
        Err(IpcStatus::Empty)
    }

    fn close_tcp_response(&mut self, response: NetworkResponse) {
        if response.generation == 0 || response.service_epoch == 0 {
            return;
        }
        let mut close = NetworkRequest::new(NetworkOperation::Close, next_network_request_id());
        close.handle = response.handle;
        close.generation = response.generation;
        close.service_epoch = response.service_epoch;
        let _ = self.request_message(close);
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
    let mut output = [0; logos_flow::MAX_OUTPUT_BYTES];
    let length = logos_flow::format_service_record(&response.record, &mut output);
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
            let mut response =
                CompletionResponse::empty(request.request_id, CompletionStatus::Unavailable);
            response.line_revision = request.line_revision;
            return response;
        }
        let Some(line) = request.line() else {
            let mut response =
                CompletionResponse::empty(request.request_id, CompletionStatus::Malformed);
            response.line_revision = request.line_revision;
            return response;
        };
        let Ok(Some(context)) = logos_flow::completion_context(line, usize::from(request.cursor))
        else {
            let mut response =
                CompletionResponse::empty(request.request_id, CompletionStatus::NoMatch);
            response.line_revision = request.line_revision;
            return response;
        };
        let mut response = CompletionResponse::empty(request.request_id, CompletionStatus::Ok);
        response.line_revision = request.line_revision;
        response.replace_start = context.replace_start as u8;
        response.replace_end = context.replace_end as u8;
        match context.target {
            logos_flow::CompletionTarget::Root => {
                if b"help".starts_with(context.prefix)
                    && !response.push_candidate_with_cursor(
                        b"help()",
                        logos_flow::completion_cursor_offset(b"help()"),
                    )
                {
                    response.flags |= COMPLETION_FLAG_TRUNCATED;
                }
                if b"clear".starts_with(context.prefix)
                    && !response.push_candidate_with_cursor(
                        b"clear()",
                        logos_flow::completion_cursor_offset(b"clear()"),
                    )
                {
                    response.flags |= COMPLETION_FLAG_TRUNCATED;
                }
                if b"echo".starts_with(context.prefix)
                    && !response.push_candidate_with_cursor(
                        b"echo(\"\")",
                        logos_flow::completion_cursor_offset(b"echo(\"\")"),
                    )
                {
                    response.flags |= COMPLETION_FLAG_TRUNCATED;
                }
                for spec in logos_flow::FLOW_SPECS {
                    if !spec.name.starts_with(context.prefix) {
                        continue;
                    }
                    let punctuation = match spec.kind {
                        logos_flow::FlowKind::Filesystem => b".".as_slice(),
                        logos_flow::FlowKind::Service => b"[\"".as_slice(),
                        logos_flow::FlowKind::Network => b".".as_slice(),
                        logos_flow::FlowKind::System => b".".as_slice(),
                        logos_flow::FlowKind::Package => b".".as_slice(),
                        logos_flow::FlowKind::Program => b".".as_slice(),
                        logos_flow::FlowKind::Device => b".".as_slice(),
                    };
                    let mut candidate = [0; MAX_COMPLETION_ITEM_BYTES];
                    let Some(length) = copy_candidate(&mut candidate, spec.name, punctuation)
                    else {
                        response.flags |= COMPLETION_FLAG_TRUNCATED;
                        continue;
                    };
                    if !response.push_candidate_with_cursor(
                        &candidate[..length],
                        logos_flow::completion_cursor_offset(&candidate[..length]),
                    ) {
                        response.flags |= COMPLETION_FLAG_TRUNCATED;
                        break;
                    }
                }
            }
            logos_flow::CompletionTarget::ServiceName => {
                if self.append_service_names(context.prefix, &mut response).is_err() {
                    response.status = CompletionStatus::Unavailable;
                }
            }
            logos_flow::CompletionTarget::ServiceMember => {
                for candidate in logos_flow::SERVICE_COMPLETION_MEMBERS {
                    if candidate.starts_with(context.prefix)
                        && !response.push_candidate_with_cursor(
                            candidate,
                            logos_flow::completion_cursor_offset(candidate),
                        )
                    {
                        response.flags |= COMPLETION_FLAG_TRUNCATED;
                        break;
                    }
                }
            }
            logos_flow::CompletionTarget::NetworkMember => {
                for candidate in logos_flow::NETWORK_COMPLETION_MEMBERS {
                    if candidate.starts_with(context.prefix)
                        && !response.push_candidate_with_cursor(
                            candidate,
                            logos_flow::completion_cursor_offset(candidate),
                        )
                    {
                        response.flags |= COMPLETION_FLAG_TRUNCATED;
                        break;
                    }
                }
            }
            logos_flow::CompletionTarget::SystemMember => {
                for candidate in logos_flow::SYSTEM_COMPLETION_MEMBERS {
                    if candidate.starts_with(context.prefix)
                        && !response.push_candidate_with_cursor(
                            candidate,
                            logos_flow::completion_cursor_offset(candidate),
                        )
                    {
                        response.flags |= COMPLETION_FLAG_TRUNCATED;
                        break;
                    }
                }
            }
            logos_flow::CompletionTarget::FilesystemMember => {
                for candidate in logos_flow::FILESYSTEM_COMPLETION_MEMBERS {
                    if candidate.starts_with(context.prefix)
                        && !response.push_candidate_with_cursor(
                            candidate,
                            logos_flow::completion_cursor_offset(candidate),
                        )
                    {
                        response.flags |= COMPLETION_FLAG_TRUNCATED;
                        break;
                    }
                }
            }
            logos_flow::CompletionTarget::PackageMember => {
                for candidate in logos_flow::PACKAGE_COMPLETION_MEMBERS {
                    if candidate.starts_with(context.prefix)
                        && !response.push_candidate_with_cursor(
                            candidate,
                            logos_flow::completion_cursor_offset(candidate),
                        )
                    {
                        response.flags |= COMPLETION_FLAG_TRUNCATED;
                        break;
                    }
                }
            }
            logos_flow::CompletionTarget::ProgramMember => {
                for candidate in logos_flow::PROGRAM_COMPLETION_MEMBERS {
                    if candidate.starts_with(context.prefix)
                        && !response.push_candidate_with_cursor(
                            candidate,
                            logos_flow::completion_cursor_offset(candidate),
                        )
                    {
                        response.flags |= COMPLETION_FLAG_TRUNCATED;
                        break;
                    }
                }
            }
            logos_flow::CompletionTarget::DeviceMember => {
                for candidate in logos_flow::DEVICE_COMPLETION_MEMBERS {
                    if candidate.starts_with(context.prefix)
                        && !response.push_candidate_with_cursor(
                            candidate,
                            logos_flow::completion_cursor_offset(candidate),
                        )
                    {
                        response.flags |= COMPLETION_FLAG_TRUNCATED;
                        break;
                    }
                }
            }
            logos_flow::CompletionTarget::FileHandleOpen
            | logos_flow::CompletionTarget::FileHandleOpenMember
            | logos_flow::CompletionTarget::FileHandleTouch
            | logos_flow::CompletionTarget::FileHandleTouchMember => {
                let candidates = match context.target {
                    logos_flow::CompletionTarget::FileHandleOpen => {
                        &logos_flow::FILE_OPEN_COMPLETION_MEMBERS
                    }
                    logos_flow::CompletionTarget::FileHandleOpenMember => {
                        &logos_flow::FILE_OPEN_MEMBER_COMPLETION
                    }
                    logos_flow::CompletionTarget::FileHandleTouch => {
                        &logos_flow::FILE_TOUCH_COMPLETION_MEMBERS
                    }
                    logos_flow::CompletionTarget::FileHandleTouchMember => {
                        &logos_flow::FILE_TOUCH_MEMBER_COMPLETION
                    }
                    _ => unreachable!(),
                };
                for candidate in candidates {
                    if candidate.starts_with(context.prefix)
                        && !response.push_candidate_with_cursor(
                            candidate,
                            logos_flow::completion_cursor_offset(candidate),
                        )
                    {
                        response.flags |= COMPLETION_FLAG_TRUNCATED;
                        break;
                    }
                }
            }
            logos_flow::CompletionTarget::InterfaceName => {
                if b"eth0".starts_with(context.prefix)
                    && !response.push_candidate_with_cursor(
                        b"eth0\"]",
                        logos_flow::completion_cursor_offset(b"eth0\"]"),
                    )
                {
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

fn trim_flow_input(bytes: &[u8]) -> &[u8] {
    let mut start = 0;
    while start < bytes.len() && bytes[start].is_ascii_whitespace() {
        start += 1;
    }
    &bytes[start..]
}

fn flow_is_foreground(bytes: &[u8]) -> bool {
    trim_flow_input(bytes).starts_with(b"await ")
}

struct PendingOutput {
    bytes: [u8; logos_flow::MAX_OUTPUT_BYTES],
    len: usize,
    offset: usize,
    pending: bool,
}

impl PendingOutput {
    const fn new() -> Self {
        Self { bytes: [0; logos_flow::MAX_OUTPUT_BYTES], len: 0, offset: 0, pending: false }
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
    TouchWrite,
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
    StageBegin,
    StageChunk,
    StageCommit,
    StageAbort,
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
    path: [u8; logos_flow::MAX_FLOW_BYTES],
    path_len: usize,
    secondary_path: [u8; logos_flow::MAX_FLOW_BYTES],
    secondary_len: usize,
    data: [u8; logos_storage_service::MAX_FILE_BYTES],
    data_len: usize,
    result: [u8; logos_flow::MAX_OUTPUT_BYTES],
    result_len: usize,
    failure: StorageApiStatus,
    last_status: StorageApiStatus,
    cancelled: bool,
}

#[derive(Clone, Copy)]
enum PackageWork {
    List,
    Info,
    Install,
}

struct PackageClient {
    work: Option<PackageWork>,
    name: [u8; logos_flow::MAX_FLOW_BYTES],
    name_len: usize,
    active: bool,
    done: bool,
    sent: bool,
    request_id: u32,
    cursor: u32,
    result: [u8; logos_flow::MAX_OUTPUT_BYTES],
    result_len: usize,
    cancelled: bool,
}

impl PackageClient {
    const fn new() -> Self {
        Self {
            work: None,
            name: [0; logos_flow::MAX_FLOW_BYTES],
            name_len: 0,
            active: false,
            done: false,
            sent: false,
            request_id: 1,
            cursor: 0,
            result: [0; logos_flow::MAX_OUTPUT_BYTES],
            result_len: 0,
            cancelled: false,
        }
    }

    fn start(&mut self, command: logos_flow::PackageCommand<'_>) -> bool {
        if self.active || self.done {
            return false;
        }
        self.name_len = 0;
        self.result_len = 0;
        self.cursor = 0;
        self.sent = false;
        self.cancelled = false;
        self.work = match command {
            logos_flow::PackageCommand::List => Some(PackageWork::List),
            logos_flow::PackageCommand::Info { name } => {
                if name.is_empty() || name.len() > self.name.len() {
                    return false;
                }
                self.name[..name.len()].copy_from_slice(name);
                self.name_len = name.len();
                Some(PackageWork::Info)
            }
            logos_flow::PackageCommand::Install { path } => {
                let Some(path) = logos_flow::root_relative_path(path, &mut self.name) else {
                    return false;
                };
                self.name_len = path.len();
                Some(PackageWork::Install)
            }
        };
        self.active = true;
        true
    }

    fn active(&self) -> bool {
        self.active
    }

    fn cancel(&mut self) {
        if self.active {
            self.cancelled = true;
        }
    }

    fn request(&self) -> Option<IpcBytes> {
        let (operation, offset, path) = match self.work {
            Some(PackageWork::List) => (StorageApiOperation::PackageList, self.cursor, &[][..]),
            Some(PackageWork::Info) => {
                (StorageApiOperation::PackageInfo, 0, &self.name[..self.name_len])
            }
            Some(PackageWork::Install) => {
                (StorageApiOperation::PackageInstall, 0, &self.name[..self.name_len])
            }
            None => return None,
        };
        StorageApiRequest::encode(operation, 0, self.request_id, 0, offset, path, &[], &[])
    }

    fn drive(&mut self) -> bool {
        if !self.active {
            return false;
        }
        if self.cancelled && !self.sent {
            self.fail(StorageApiStatus::Unsupported);
            return true;
        }
        if !self.sent {
            let Some(request) = self.request() else {
                self.fail(StorageApiStatus::Invalid);
                return true;
            };
            match common::ipc_send(STORAGE_SEND_CAPABILITY, &request) {
                IpcStatus::Ok => self.sent = true,
                IpcStatus::Full => return false,
                status => {
                    self.fail(storage_ipc_error(status));
                    return true;
                }
            }
            return true;
        }
        let mut message = IpcBytes::empty(MessageKind::StorageResponse);
        match common::ipc_receive(STORAGE_RECEIVE_CAPABILITY, &mut message) {
            IpcStatus::Ok => self.sent = false,
            IpcStatus::Empty => return false,
            status => {
                self.fail(storage_ipc_error(status));
                return true;
            }
        }
        let Ok(response) = StorageApiResponse::decode(&message) else {
            self.fail(StorageApiStatus::Invalid);
            return true;
        };
        self.handle_response(response);
        true
    }

    fn handle_response(&mut self, response: StorageApiResponse<'_>) {
        if response.request_id != self.request_id {
            if self.cancelled {
                self.sent = true;
            } else {
                self.fail(StorageApiStatus::Stale);
            }
            return;
        }
        if self.cancelled {
            self.fail(StorageApiStatus::Unsupported);
            return;
        }
        if response.status != StorageApiStatus::Ok {
            self.fail(response.status);
            return;
        }
        self.append(response.data);
        if response.more {
            if response.data.is_empty() || self.result_len == self.result.len() {
                self.fail(StorageApiStatus::Invalid);
            } else {
                self.cursor = self.cursor.saturating_add(1);
                self.request_id = self.request_id.wrapping_add(1).max(1);
            }
        } else {
            self.succeed();
        }
    }

    fn append(&mut self, bytes: &[u8]) {
        let amount = bytes.len().min(self.result.len().saturating_sub(self.result_len));
        self.result[self.result_len..self.result_len + amount].copy_from_slice(&bytes[..amount]);
        self.result_len += amount;
    }

    fn fail(&mut self, status: StorageApiStatus) {
        self.result_len = 0;
        if self.cancelled {
            self.append(b"command cancelled\r\n");
            self.cancelled = false;
        } else {
            self.append(status_text(status));
        }
        self.active = false;
        self.done = true;
    }

    fn succeed(&mut self) {
        self.active = false;
        self.done = true;
    }

    fn take_result(&mut self, pending: &mut PendingOutput) {
        if self.done {
            pending.stage(&self.result[..self.result_len]);
            self.done = false;
        }
    }
}

struct DeviceClient {
    active: bool,
    done: bool,
    sent: bool,
    request_id: u32,
    result: [u8; logos_flow::MAX_OUTPUT_BYTES],
    result_len: usize,
}

impl DeviceClient {
    const fn new() -> Self {
        Self {
            active: false,
            done: false,
            sent: false,
            request_id: 1,
            result: [0; logos_flow::MAX_OUTPUT_BYTES],
            result_len: 0,
        }
    }

    fn start(&mut self, command: logos_flow::DeviceCommand) -> bool {
        if self.active || self.done || command != logos_flow::DeviceCommand::List {
            return false;
        }
        self.active = true;
        self.sent = false;
        self.result_len = 0;
        self.request_id = next_device_request_id();
        true
    }

    fn active(&self) -> bool {
        self.active
    }

    fn drive(&mut self) -> bool {
        if !self.active {
            return false;
        }
        let request = DeviceRequest::new(DeviceOperation::List, self.request_id);
        if !self.sent {
            match common::ipc_send(DEVICE_SEND_CAPABILITY, &request) {
                IpcStatus::Ok => {
                    self.sent = true;
                }
                IpcStatus::Full => return false,
                _ => self.fail(b"device manager unavailable\r\n"),
            }
            return true;
        }
        let mut response = DeviceResponse::new(request, DeviceStatus::Invalid, 1, 1);
        match common::ipc_receive(DEVICE_RECEIVE_CAPABILITY, &mut response) {
            IpcStatus::Ok => {}
            IpcStatus::Empty => return false,
            _ => {
                self.fail(b"device manager unavailable\r\n");
                return true;
            }
        }
        if !response.is_valid_for(request) {
            self.fail(b"device inventory malformed\r\n");
            return true;
        }
        if response.status != DeviceStatus::Ok {
            self.fail(b"device inventory unavailable\r\n");
            return true;
        }
        let mut manager = logos_device::DeviceManager::new();
        if manager.publish(response).is_err() {
            self.fail(b"device inventory malformed\r\n");
            return true;
        }
        self.result_len = manager.format_list(&mut self.result);
        self.active = false;
        self.done = true;
        true
    }

    fn fail(&mut self, message: &[u8]) {
        self.result_len = message.len().min(self.result.len());
        self.result[..self.result_len].copy_from_slice(&message[..self.result_len]);
        self.active = false;
        self.done = true;
    }

    fn take_result(&mut self, pending: &mut PendingOutput) {
        if self.done {
            pending.stage(&self.result[..self.result_len]);
            self.done = false;
        }
    }
}

struct UserClient {
    active: bool,
    done: bool,
    sent: bool,
    request: UserRequest,
    session: logos_abi::SessionHandle,
    user: logos_abi::UserId,
    capability: logos_abi::CapabilityHandle,
    root: logos_abi::NamespaceRoot,
    rights: logos_abi::NamespaceRights,
    result: [u8; logos_flow::MAX_OUTPUT_BYTES],
    result_len: usize,
}

impl UserClient {
    const fn new() -> Self {
        Self {
            active: false,
            done: false,
            sent: false,
            request: UserRequest::new(UserOperation::Login, 1),
            session: logos_abi::SessionHandle::EMPTY,
            user: logos_abi::UserId::EMPTY,
            capability: logos_abi::CapabilityHandle::EMPTY,
            root: logos_abi::NamespaceRoot::EMPTY,
            rights: logos_abi::NamespaceRights::NONE,
            result: [0; logos_flow::MAX_OUTPUT_BYTES],
            result_len: 0,
        }
    }

    fn start(&mut self, command: logos_flow::UserCommand<'_>) -> bool {
        if self.active || self.done {
            return false;
        }
        let operation = match command {
            logos_flow::UserCommand::Claim { .. } => UserOperation::Claim,
            logos_flow::UserCommand::Create { .. } => UserOperation::Create,
            logos_flow::UserCommand::Login { .. } => UserOperation::Login,
            logos_flow::UserCommand::Logout => UserOperation::Logout,
            logos_flow::UserCommand::Rename { .. } => UserOperation::Rename,
            logos_flow::UserCommand::SetPassword { .. } => UserOperation::SetPassword,
            logos_flow::UserCommand::Derive { .. } => UserOperation::Derive,
            logos_flow::UserCommand::RevokeCapability => UserOperation::RevokeCapability,
        };
        let mut request = UserRequest::new(operation, next_user_request_id());
        request.session = self.session;
        request.user = self.user;
        request.capability = self.capability;
        request.root = self.root;
        request.rights = self.rights;
        match command {
            logos_flow::UserCommand::Claim { name, password }
            | logos_flow::UserCommand::Create { name, password }
            | logos_flow::UserCommand::Login { name, password } => {
                if !request.set_name(name) || !request.set_password(password) {
                    return false;
                }
            }
            logos_flow::UserCommand::Rename { name } => {
                if !request.set_name(name) {
                    return false;
                }
            }
            logos_flow::UserCommand::SetPassword { password } => {
                if !request.set_password(password) {
                    return false;
                }
            }
            logos_flow::UserCommand::Logout | logos_flow::UserCommand::RevokeCapability => {}
            logos_flow::UserCommand::Derive { rights } => {
                request.rights = rights;
            }
        }
        self.request = request;
        self.active = true;
        self.sent = false;
        self.result_len = 0;
        true
    }

    fn active(&self) -> bool {
        self.active
    }

    fn drive(&mut self) -> bool {
        if !self.active {
            return false;
        }
        if !self.sent {
            let bytes = unsafe {
                core::slice::from_raw_parts(
                    (&self.request as *const UserRequest).cast::<u8>(),
                    core::mem::size_of::<UserRequest>(),
                )
            };
            let Some(message) = IpcBytes::from_bytes(MessageKind::UserRequest, bytes) else {
                self.fail(b"user request too large\r\n");
                return true;
            };
            match common::ipc_send(USER_SEND_CAPABILITY, &message) {
                IpcStatus::Ok => self.sent = true,
                IpcStatus::Full => return false,
                _ => self.fail(b"user service unavailable\r\n"),
            }
            return true;
        }
        let mut message = IpcBytes::empty(MessageKind::UserResponse);
        match common::ipc_receive(USER_RECEIVE_CAPABILITY, &mut message) {
            IpcStatus::Ok => {}
            IpcStatus::Empty => return false,
            _ => {
                self.fail(b"user service unavailable\r\n");
                return true;
            }
        }
        let Some(bytes) = message.as_bytes() else {
            self.fail(b"user response malformed\r\n");
            return true;
        };
        if bytes.len() != core::mem::size_of::<UserResponse>() {
            self.fail(b"user response malformed\r\n");
            return true;
        }
        let response: UserResponse = unsafe { core::ptr::read_unaligned(bytes.as_ptr().cast()) };
        if !response.is_valid_for(self.request) {
            self.fail(b"user response stale\r\n");
            return true;
        }
        if response.status == UserStatus::Ok {
            if matches!(self.request.operation, UserOperation::Claim | UserOperation::Login) {
                self.session = response.session;
                self.user = response.user;
                self.capability = response.capability;
                self.root = response.root;
                self.rights = response.rights;
            } else if self.request.operation == UserOperation::Logout {
                self.session = logos_abi::SessionHandle::EMPTY;
                self.user = logos_abi::UserId::EMPTY;
                self.capability = logos_abi::CapabilityHandle::EMPTY;
                self.root = logos_abi::NamespaceRoot::EMPTY;
                self.rights = logos_abi::NamespaceRights::NONE;
            } else if self.request.operation == UserOperation::RevokeCapability {
                self.capability = logos_abi::CapabilityHandle::EMPTY;
            } else if self.request.operation == UserOperation::Derive {
                self.capability = response.capability;
                self.root = response.root;
                self.rights = response.rights;
            }
            self.finish(b"user: ok\r\n");
        } else {
            self.finish(user_status_text(response.status));
        }
        true
    }

    fn finish(&mut self, message: &[u8]) {
        self.result_len = message.len().min(self.result.len());
        self.result[..self.result_len].copy_from_slice(&message[..self.result_len]);
        self.active = false;
        self.done = true;
    }

    fn fail(&mut self, message: &[u8]) {
        self.finish(message);
    }

    fn take_result(&mut self, pending: &mut PendingOutput) {
        if self.done {
            pending.stage(&self.result[..self.result_len]);
            self.done = false;
        }
    }
}

fn user_status_text(status: UserStatus) -> &'static [u8] {
    match status {
        UserStatus::Unclaimed => b"user: system is unclaimed\r\n",
        UserStatus::AlreadyClaimed => b"user: already claimed\r\n",
        UserStatus::NotFound => b"user: user not found\r\n",
        UserStatus::Unauthorized => b"user: unauthorized\r\n",
        UserStatus::BadCredentials => b"user: bad credentials\r\n",
        UserStatus::Stale => b"user: stale handle\r\n",
        UserStatus::Revoked => b"user: revoked\r\n",
        UserStatus::Capacity => b"user: capacity exhausted\r\n",
        UserStatus::Corrupt | UserStatus::Invalid => b"user: invalid\r\n",
        UserStatus::Ok => b"user: ok\r\n",
    }
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
            path: [0; logos_flow::MAX_FLOW_BYTES],
            path_len: 0,
            secondary_path: [0; logos_flow::MAX_FLOW_BYTES],
            secondary_len: 0,
            data: [0; logos_storage_service::MAX_FILE_BYTES],
            data_len: 0,
            result: [0; logos_flow::MAX_OUTPUT_BYTES],
            result_len: 0,
            failure: StorageApiStatus::Invalid,
            last_status: StorageApiStatus::Invalid,
            cancelled: false,
        }
    }

    fn start(&mut self, command: logos_flow::StorageCommand<'_>) -> bool {
        let (work, phase, path, secondary, data) = match command {
            logos_flow::StorageCommand::List { path } => {
                (StorageWork::List, StoragePhase::List, path, &[][..], &[][..])
            }
            logos_flow::StorageCommand::Touch { path } => {
                (StorageWork::Touch, StoragePhase::Begin, path, &[][..], &[][..])
            }
            logos_flow::StorageCommand::Cat { path } => {
                (StorageWork::Cat, StoragePhase::Read, path, &[][..], &[][..])
            }
            logos_flow::StorageCommand::Write { path, data } => {
                (StorageWork::Write, StoragePhase::Begin, path, &[][..], data)
            }
            logos_flow::StorageCommand::TouchWrite { path, data } => {
                (StorageWork::TouchWrite, StoragePhase::StageBegin, path, &[][..], data)
            }
            logos_flow::StorageCommand::WriteVariables { .. } => return false,
            logos_flow::StorageCommand::Remove { path } => {
                (StorageWork::Remove, StoragePhase::Begin, path, &[][..], &[][..])
            }
            logos_flow::StorageCommand::Move { from, to } => {
                (StorageWork::Move, StoragePhase::Begin, from, to, &[][..])
            }
        };
        self.start_work(work, phase, path, secondary, data)
    }

    fn start_touch_write(&mut self, path: &[u8], data: &[u8]) -> bool {
        self.start_work(StorageWork::TouchWrite, StoragePhase::StageBegin, path, &[], data)
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
        self.cancelled = false;
        let Some(path_len) =
            logos_flow::root_relative_path(path, &mut self.path).map(|path| path.len())
        else {
            return false;
        };
        let Some(secondary_len) =
            logos_flow::root_relative_path(secondary, &mut self.secondary_path)
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

    fn cancel(&mut self) {
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

    fn take_result(&mut self, pending: &mut PendingOutput) {
        if self.done {
            #[cfg(feature = "fetch-proof")]
            if self.result[..self.result_len] == *b"LogOS-Fetch\r\n" {
                common::proof_line(b"LogOS vNext: fetch contents verified");
            }
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
    shutdown_attempted: bool,
}

#[cfg(feature = "storage-proof")]
impl StorageProof {
    const fn new() -> Self {
        Self { step: 0, active: false, recovery: false, shutdown_attempted: false }
    }

    fn active(&self) -> bool {
        self.active
    }

    fn consume_result(&mut self, storage: &mut StorageClient) -> bool {
        if !storage.done || !self.active {
            return false;
        }
        let expected_content = match (self.recovery, self.step) {
            (false, 4) => Some(&b"replacement-api\r\n"[..]),
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
                0 | 1 | 2 | 3 => status == StorageApiStatus::Ok,
                4 => status == StorageApiStatus::Ok && content_valid,
                5 | 6 => status == StorageApiStatus::NotFound,
                _ => false,
            }
        };
        if accepted {
            self.step = self.step.saturating_add(1);
            self.active = false;
            if (!self.recovery && self.step > 6) || (self.recovery && self.step > 7) {
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
            0 => storage.start(logos_flow::StorageCommand::Touch { path: b"/api-survivor" }),
            1 => storage.start(logos_flow::StorageCommand::Write {
                path: b"/api-survivor",
                data: if self.recovery { b"recovered-api" } else { b"durable-api" },
            }),
            2 if self.recovery => storage.start_proof_abort(b"/api-aborted"),
            2 => storage.start(logos_flow::StorageCommand::Write {
                path: b"/api-survivor",
                data: b"replacement-api",
            }),
            3 if self.recovery => {
                storage.start(logos_flow::StorageCommand::Touch { path: b"/api-removed" })
            }
            3 => storage.start_proof_abort(b"/api-aborted"),
            4 if self.recovery => {
                storage.start(logos_flow::StorageCommand::Remove { path: b"/api-removed" })
            }
            4 => storage.start(logos_flow::StorageCommand::Cat { path: b"/api-survivor" }),
            5 if self.recovery => {
                storage.start(logos_flow::StorageCommand::Cat { path: b"/api-survivor" })
            }
            5 => storage.start(logos_flow::StorageCommand::Cat { path: b"/api-aborted" }),
            6 if self.recovery => {
                storage.start(logos_flow::StorageCommand::Cat { path: b"/api-aborted" })
            }
            6 => storage.start(logos_flow::StorageCommand::Cat { path: b"/api-removed" }),
            7 => storage.start(logos_flow::StorageCommand::Cat { path: b"/api-removed" }),
            _ => false,
        };
        if started {
            self.active = true;
        }
        started
    }

    fn request_shutdown(&mut self) -> bool {
        if self.step != u8::MAX || self.shutdown_attempted {
            return false;
        }
        self.shutdown_attempted = true;
        common::power(logos_flow::FlowAction::Shutdown as usize) != 0
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
        StorageApiStatus::Invalid => b"invalid storage request\r\n",
        StorageApiStatus::NotFound => b"not found\r\n",
        StorageApiStatus::AlreadyExists => b"already exists\r\n",
        StorageApiStatus::Busy => b"storage busy\r\n",
        StorageApiStatus::Capacity => b"storage capacity exhausted\r\n",
        StorageApiStatus::Io => b"storage I/O error\r\n",
        StorageApiStatus::Unsupported => b"storage unsupported\r\n",
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

fn program_command(command: logos_flow::ProgramCommand<'_>, pending: &mut PendingOutput) {
    let (operation, name) = match command {
        logos_flow::ProgramCommand::Start { name } => {
            (logos_abi::ManagerOperation::ProgramStart, name)
        }
        logos_flow::ProgramCommand::Status { name } => {
            (logos_abi::ManagerOperation::ProgramStatus, name)
        }
        logos_flow::ProgramCommand::Stop { name } => {
            (logos_abi::ManagerOperation::ProgramStop, name)
        }
    };
    let request_id = next_manager_request_id();
    let Some(request) =
        logos_abi::ManagerRequest::new(operation, request_id).with_program_name(name)
    else {
        pending.stage(b"program name is too long\r\n");
        return;
    };
    let mut response =
        logos_abi::ManagerResponse::new(operation, logos_abi::ManagerStatus::Malformed, request_id);
    if common::manager_call(&request, &mut response) != IpcStatus::Ok {
        pending.stage(b"program manager unavailable\r\n");
    } else if !matches!(
        response.status,
        logos_abi::ManagerStatus::Ok | logos_abi::ManagerStatus::Accepted
    ) {
        pending.stage(manager_error(response.status));
    } else {
        let mut output = [0; logos_flow::MAX_OUTPUT_BYTES];
        let length = logos_flow::format_service_record(&response.record, &mut output);
        pending.stage(&output[..length]);
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

fn service_command(command: logos_flow::ServiceCommand<'_>, pending: &mut PendingOutput) {
    let (operation, name, list, property) = match command {
        logos_flow::ServiceCommand::List => {
            (logos_abi::ManagerOperation::List, &[][..], true, logos_flow::ServiceProperty::Record)
        }
        logos_flow::ServiceCommand::Lookup { name } => {
            (logos_abi::ManagerOperation::Status, name, false, logos_flow::ServiceProperty::Record)
        }
        logos_flow::ServiceCommand::Status { name } => {
            (logos_abi::ManagerOperation::Status, name, false, logos_flow::ServiceProperty::Status)
        }
        logos_flow::ServiceCommand::Name { name } => {
            (logos_abi::ManagerOperation::Status, name, false, logos_flow::ServiceProperty::Name)
        }
        logos_flow::ServiceCommand::Version { name } => {
            (logos_abi::ManagerOperation::Status, name, false, logos_flow::ServiceProperty::Version)
        }
        logos_flow::ServiceCommand::Start { name } => {
            (logos_abi::ManagerOperation::Start, name, false, logos_flow::ServiceProperty::Record)
        }
        logos_flow::ServiceCommand::Stop { name } => {
            (logos_abi::ManagerOperation::Stop, name, false, logos_flow::ServiceProperty::Record)
        }
        logos_flow::ServiceCommand::Restart { name } => {
            (logos_abi::ManagerOperation::Restart, name, false, logos_flow::ServiceProperty::Record)
        }
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
    let mut output = [0; logos_flow::MAX_OUTPUT_BYTES];
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
            output_len += logos_flow::format_service_property(
                &record,
                if list { logos_flow::ServiceProperty::Record } else { property },
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
        NetworkResult::Cancelled => b"network cancelled\r\n",
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
    command: logos_flow::NetworkCommand<'_>,
    client: &mut NetworkClient,
    fetch: &mut FetchClient,
    pending: &mut PendingOutput,
) {
    if let logos_flow::NetworkCommand::Fetch { url, destination } = command {
        if !fetch.start(url, destination) {
            pending.stage(if fetch.active() {
                b"fetch already active\r\n"
            } else {
                b"fetch request too large\r\n"
            });
        }
        return;
    }
    if let logos_flow::NetworkCommand::InterfaceStatus { name } = command {
        if name != b"eth0" {
            pending.stage(b"network interface not found\r\n");
            return;
        }
    }
    let (operation, address, port, success): (NetworkOperation, [u8; 4], u16, &[u8]) = match command
    {
        logos_flow::NetworkCommand::Status => (NetworkOperation::Status, [0; 4], 0, b""),
        logos_flow::NetworkCommand::InterfaceStatus { .. } => {
            (NetworkOperation::Status, [0; 4], 0, b"")
        }
        logos_flow::NetworkCommand::Ping { address } => {
            (NetworkOperation::IcmpPing, address, 0, b"ping ok\r\n")
        }
        logos_flow::NetworkCommand::TcpProbe { address, port } => {
            (NetworkOperation::TcpConnect, address, port, b"tcp probe ok\r\n")
        }
        logos_flow::NetworkCommand::Fetch { .. } => unreachable!(),
    };
    match client.request(operation, address, port) {
        Ok(response) if operation == NetworkOperation::Status => {
            pending.stage(network_state_text(response.state))
        }
        Ok(response) if operation == NetworkOperation::TcpConnect => {
            client.close_tcp_response(response);
            if response.result == NetworkResult::Ok {
                pending.stage(success);
            } else {
                pending.stage(network_result_text(response.result));
            }
        }
        Ok(response) if response.result == NetworkResult::Ok => pending.stage(success),
        Ok(response) => pending.stage(network_result_text(response.result)),
        Err(IpcStatus::Stale | IpcStatus::Disconnected) => pending.stage(b"network restarting\r\n"),
        Err(IpcStatus::Empty) if client.take_cancelled() => pending.stage(b"network cancelled\r\n"),
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
            if tcp.result != NetworkResult::Ok {
                return false;
            }
            if !manager_restart_network() {
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
                    write.payload_len = 1;
                    write.payload[0] = 0x4e;
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
                        read.payload_len = logos_abi::NETWORK_INLINE_PAYLOAD_BYTES as u16;
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
                common::wait(0, logos_abi::ServiceId::Flow);
            }
            return false;
        }
        common::wait(0, logos_abi::ServiceId::Flow);
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
    if cfg!(feature = "fetch-proof") {
        let _ = (pending, network, initial_storage);
        return true;
    }
    if initial_storage.generation != 1 {
        return network_proof_probe(network);
    }
    service_command(logos_flow::ServiceCommand::List, pending);
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
    service_command(logos_flow::ServiceCommand::Stop { name: b"input" }, pending);
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

static mut FLOW: logos_flow::FlowService = logos_flow::FlowService::new();
static mut PENDING: PendingOutput = PendingOutput::new();
static mut STORAGE: StorageClient = StorageClient::new();
static mut PACKAGE: PackageClient = PackageClient::new();
static mut DEVICE: DeviceClient = DeviceClient::new();
static mut USER: UserClient = UserClient::new();
static mut NETWORK: NetworkClient = NetworkClient::new();
static mut FETCH: FetchClient = FetchClient::new();
static mut COMPLETION: CompletionService = CompletionService::new();
static mut PENDING_COMPLETION: Option<IpcBytes> = None;

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    let flow = unsafe { &mut *core::ptr::addr_of_mut!(FLOW) };
    let pending = unsafe { &mut *core::ptr::addr_of_mut!(PENDING) };
    let storage = unsafe { &mut *core::ptr::addr_of_mut!(STORAGE) };
    let package = unsafe { &mut *core::ptr::addr_of_mut!(PACKAGE) };
    let device = unsafe { &mut *core::ptr::addr_of_mut!(DEVICE) };
    let user = unsafe { &mut *core::ptr::addr_of_mut!(USER) };
    let network = unsafe { &mut *core::ptr::addr_of_mut!(NETWORK) };
    let fetch = unsafe { &mut *core::ptr::addr_of_mut!(FETCH) };
    let completion = unsafe { &mut *core::ptr::addr_of_mut!(COMPLETION) };
    let pending_completion = unsafe { &mut *core::ptr::addr_of_mut!(PENDING_COMPLETION) };
    #[cfg(feature = "qemu-proof")]
    while !manager_boot_probe() {
        common::wait(0, logos_abi::ServiceId::Flow);
    }
    #[cfg(feature = "qemu-proof")]
    if !manager_command_probe(pending, network) {
        common::idle();
    }
    #[cfg(feature = "storage-proof")]
    let mut proof = StorageProof::new();
    let mut heartbeat_ticks = 0u16;
    loop {
        common::heartbeat_tick(&mut heartbeat_ticks, logos_abi::ServiceId::Flow);
        let mut progressed = pending.flush(OUTPUT_CAPABILITY);
        if storage.active()
            || package.active()
            || device.active()
            || user.active()
            || (fetch.active() && fetch.foreground())
        {
            let mut control = IpcBytes::empty(MessageKind::FlowControl);
            if common::ipc_receive(INPUT_CAPABILITY, &mut control) == IpcStatus::Ok {
                if fetch.active() {
                    progressed |= fetch_control(&control, fetch);
                } else if control.kind == MessageKind::FlowControl
                    && control.len as usize == mem::size_of::<FlowControl>()
                {
                    let value: FlowControl =
                        unsafe { ptr::read_unaligned(control.bytes.as_ptr().cast()) };
                    if value.is_valid() {
                        if storage.active() {
                            storage.cancel();
                        } else if package.active() {
                            package.cancel();
                        }
                        progressed = true;
                    }
                }
            }
        }
        if fetch.active() && fetch.cancel_pending {
            match fetch.send_cancel() {
                IpcStatus::Ok => fetch.cancel_pending = false,
                IpcStatus::Full => {
                    common::wait(
                        common::ipc_write_event(logos_abi::IpcEndpointId::FlowToFetch),
                        logos_abi::ServiceId::Flow,
                    );
                    continue;
                }
                _ => fetch.cancel_pending = false,
            }
        }
        if pending.pending {
            if !progressed {
                common::wait(
                    common::ipc_write_event(logos_abi::IpcEndpointId::FlowToSession),
                    logos_abi::ServiceId::Flow,
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
                        common::ipc_write_event(logos_abi::IpcEndpointId::FlowToSession),
                        logos_abi::ServiceId::Flow,
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
                        common::ipc_read_event(logos_abi::IpcEndpointId::StorageToFlow)
                            | common::ipc_write_event(logos_abi::IpcEndpointId::FlowToStorage)
                            | common::ipc_read_event(logos_abi::IpcEndpointId::SessionToFlow),
                        logos_abi::ServiceId::Flow,
                    );
                }
                continue;
            }
        }
        if package.active() {
            progressed |= package.drive();
            if package.done {
                package.take_result(pending);
                progressed = true;
            }
            if package.active() {
                if !progressed {
                    common::wait(
                        common::ipc_read_event(logos_abi::IpcEndpointId::StorageToFlow)
                            | common::ipc_write_event(logos_abi::IpcEndpointId::FlowToStorage)
                            | common::ipc_read_event(logos_abi::IpcEndpointId::SessionToFlow),
                        logos_abi::ServiceId::Flow,
                    );
                }
                continue;
            }
        }
        if device.active() {
            progressed |= device.drive();
            if device.done {
                device.take_result(pending);
                progressed = true;
            }
            if device.active() {
                if !progressed {
                    common::wait(
                        common::ipc_read_event(logos_abi::IpcEndpointId::DeviceToFlow)
                            | common::ipc_write_event(logos_abi::IpcEndpointId::FlowToDevice)
                            | common::ipc_read_event(logos_abi::IpcEndpointId::SessionToFlow),
                        logos_abi::ServiceId::Flow,
                    );
                }
                continue;
            }
        }
        if user.active() {
            progressed |= user.drive();
            if user.done {
                user.take_result(pending);
                progressed = true;
            }
            if user.active() {
                if !progressed {
                    common::wait(
                        common::ipc_read_event(logos_abi::IpcEndpointId::UserToFlow)
                            | common::ipc_write_event(logos_abi::IpcEndpointId::FlowToUser)
                            | common::ipc_read_event(logos_abi::IpcEndpointId::SessionToFlow),
                        logos_abi::ServiceId::Flow,
                    );
                }
                continue;
            }
        }
        if fetch.active() {
            progressed |= fetch.drive(pending);
            if !fetch.active() {
                fetch.resolve_promise(flow);
                if let Some((destination, body)) = fetch.take_callback() {
                    if storage.start_touch_write(destination, body) {
                        fetch.clear_callback();
                        progressed = true;
                    } else {
                        fetch.clear_callback();
                        pending.stage(b"flow: response publication failed\r\n");
                    }
                }
            }
            if fetch.active() && fetch.foreground() {
                if !progressed {
                    common::wait(
                        common::ipc_read_event(logos_abi::IpcEndpointId::FetchToFlow)
                            | common::ipc_write_event(logos_abi::IpcEndpointId::FlowToFetch)
                            | common::ipc_write_event(logos_abi::IpcEndpointId::FlowToSession)
                            | common::ipc_read_event(logos_abi::IpcEndpointId::SessionToFlow),
                        logos_abi::ServiceId::Flow,
                    );
                }
                continue;
            }
            if fetch.active() {
                progressed = true;
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
        #[cfg(feature = "storage-proof")]
        if proof.request_shutdown() {
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
                    match flow.operation(bytes) {
                        Ok(Some(logos_flow::FlowOperation::Help { topic })) => {
                            let mut output = [0; logos_flow::MAX_OUTPUT_BYTES];
                            let length = logos_flow::format_help(topic, &mut output);
                            pending.stage(&output[..length]);
                        }
                        Ok(Some(logos_flow::FlowOperation::Clear)) => {
                            pending.stage(b"\x1b[2J\x1b[H");
                        }
                        Ok(Some(logos_flow::FlowOperation::Echo { text })) => {
                            pending.stage(text);
                        }
                        Ok(Some(logos_flow::FlowOperation::EchoVariable { name })) => {
                            let mut output = [0; logos_flow::MAX_OUTPUT_BYTES];
                            if let Some(length) = flow.copy_string_variable(name, &mut output) {
                                pending.stage(&output[..length]);
                            } else {
                                pending.stage(b"flow: string variable is unavailable\r\n");
                            }
                        }
                        Ok(Some(logos_flow::FlowOperation::Service(command))) => {
                            service_command(command, pending)
                        }
                        Ok(Some(logos_flow::FlowOperation::Network(command))) => {
                            network_command(command, network, fetch, pending)
                        }
                        Ok(Some(logos_flow::FlowOperation::Storage(command))) => match command {
                            logos_flow::StorageCommand::WriteVariables {
                                path,
                                data,
                                path_is_variable,
                                data_is_variable,
                                create,
                            } => {
                                let mut resolved_path = [0; logos_flow::MAX_FLOW_BYTES];
                                let mut resolved_data = [0; logos_storage_service::MAX_FILE_BYTES];
                                let path = if path_is_variable {
                                    let Some(length) =
                                        flow.copy_string_variable(path, &mut resolved_path)
                                    else {
                                        pending.stage(b"flow: string variable is unavailable\r\n");
                                        continue;
                                    };
                                    &resolved_path[..length]
                                } else {
                                    path
                                };
                                let data = if data_is_variable {
                                    let Some(length) = flow.copy_value(data, &mut resolved_data)
                                    else {
                                        pending.stage(b"flow: value is unavailable\r\n");
                                        continue;
                                    };
                                    &resolved_data[..length]
                                } else {
                                    data
                                };
                                let accepted = if create {
                                    storage.start_touch_write(path, data)
                                } else {
                                    storage.start(logos_flow::StorageCommand::Write { path, data })
                                };
                                if !accepted {
                                    pending.stage(b"storage request too large\r\n");
                                }
                            }
                            command => {
                                if !storage.start(command) {
                                    pending.stage(b"storage request too large\r\n");
                                }
                            }
                        },
                        Ok(Some(logos_flow::FlowOperation::Package(command))) => {
                            if !package.start(command) {
                                pending.stage(b"package request too large\r\n");
                            }
                        }
                        Ok(Some(logos_flow::FlowOperation::Device(command))) => {
                            if !device.start(command) {
                                pending.stage(b"device request busy or too large\r\n");
                            }
                        }
                        Ok(Some(logos_flow::FlowOperation::User(command))) => {
                            if !user.start(command) {
                                pending.stage(b"user request busy or too large\r\n");
                            }
                        }
                        Ok(Some(logos_flow::FlowOperation::Program(command))) => {
                            program_command(command, pending);
                        }
                        Ok(Some(logos_flow::FlowOperation::System(operation))) => match operation {
                            logos_flow::SystemOperation::Version => {
                                pending.stage(b"LogOS vNext 0.1.0\r\n")
                            }
                            logos_flow::SystemOperation::Uname => pending.stage(b"LogOS\r\n"),
                            logos_flow::SystemOperation::Shutdown => {
                                if common::power(logos_flow::FlowAction::Shutdown as usize) != 0 {
                                    pending.stage(b"power action denied\r\n");
                                }
                            }
                            logos_flow::SystemOperation::Reboot => {
                                if common::power(logos_flow::FlowAction::Reboot as usize) != 0 {
                                    pending.stage(b"power action denied\r\n");
                                }
                            }
                        },
                        Ok(Some(logos_flow::FlowOperation::CancelPromise { name })) => {
                            let foreground = flow_is_foreground(bytes);
                            let active = fetch.active_promise_is(name);
                            let cancelled = flow.cancel_promise(name);
                            if active {
                                fetch.cancel();
                            }
                            if !foreground {
                                pending.stage(if cancelled || active {
                                    &[]
                                } else {
                                    b"flow: promise is not active\r\n"
                                });
                            }
                        }
                        Ok(Some(logos_flow::FlowOperation::FetchResponse { url })) => {
                            let foreground = flow_is_foreground(bytes);
                            if !(if foreground {
                                fetch.start_response(url)
                            } else {
                                fetch.start_response_background(url)
                            }) {
                                pending.stage(b"fetch request busy or too large\r\n");
                            } else if !foreground {
                                pending.stage(&[]);
                            }
                        }
                        Ok(Some(logos_flow::FlowOperation::FetchResponseVariable {
                            name,
                            url,
                            url_is_variable,
                        })) => {
                            let mut resolved = [0; logos_flow::MAX_FLOW_BYTES];
                            let foreground = flow_is_foreground(bytes);
                            let (resolved_url, resolved_len) = if url_is_variable {
                                let Some(length) = flow.copy_string_variable(url, &mut resolved)
                                else {
                                    pending.stage(b"flow: string variable is unavailable\r\n");
                                    continue;
                                };
                                (&resolved[..], length)
                            } else {
                                (url, url.len())
                            };
                            let started = if name.is_empty() {
                                if foreground {
                                    fetch.start_response(&resolved_url[..resolved_len])
                                } else {
                                    fetch.start_response_background(&resolved_url[..resolved_len])
                                }
                            } else {
                                fetch.start_named_response(
                                    &resolved_url[..resolved_len],
                                    name,
                                    foreground,
                                )
                            };
                            if !started {
                                if !name.is_empty() {
                                    let _ = flow.cancel_promise(name);
                                }
                                pending.stage(b"fetch request busy or too large\r\n");
                            } else if !foreground {
                                pending.stage(&[]);
                            }
                        }
                        Ok(Some(logos_flow::FlowOperation::FetchResponseToFile {
                            url,
                            destination,
                        })) => {
                            let foreground = flow_is_foreground(bytes);
                            if !fetch.start_to_file_mode(url, destination, foreground) {
                                pending.stage(b"fetch request busy or too large\r\n");
                            } else if !foreground {
                                pending.stage(&[]);
                            }
                        }
                        Ok(Some(logos_flow::FlowOperation::FetchResponseToFileVariables {
                            url,
                            destination,
                        })) => {
                            let mut resolved_url = [0; logos_flow::MAX_FLOW_BYTES];
                            let mut resolved_destination = [0; logos_flow::MAX_FLOW_BYTES];
                            let foreground = flow_is_foreground(bytes);
                            let Some(url_len) = flow.copy_string_variable(url, &mut resolved_url)
                            else {
                                pending.stage(b"flow: string variable is unavailable\r\n");
                                continue;
                            };
                            let Some(destination_len) =
                                flow.copy_string_variable(destination, &mut resolved_destination)
                            else {
                                pending.stage(b"flow: string variable is unavailable\r\n");
                                continue;
                            };
                            if !fetch.start_to_file_mode(
                                &resolved_url[..url_len],
                                &resolved_destination[..destination_len],
                                foreground,
                            ) {
                                pending.stage(b"fetch request busy or too large\r\n");
                            } else if !foreground {
                                pending.stage(&[]);
                            }
                        }
                        Ok(Some(logos_flow::FlowOperation::WriteResponse { url, destination })) => {
                            if !fetch.start_response_to_file(
                                url,
                                destination,
                                flow_is_foreground(bytes),
                            ) {
                                pending.stage(b"fetch request busy or too large\r\n");
                            }
                        }
                        Ok(Some(logos_flow::FlowOperation::WriteResponsePromise {
                            name,
                            destination,
                            destination_is_variable,
                        })) => {
                            let mut body = [0; logos_flow::interpreter::MAX_VALUE_BYTES];
                            if destination_is_variable {
                                let mut resolved = [0; logos_flow::MAX_FLOW_BYTES];
                                let Some(length) =
                                    flow.copy_string_variable(destination, &mut resolved)
                                else {
                                    pending.stage(b"flow: string variable is unavailable\r\n");
                                    continue;
                                };
                                let Some((_, body_len)) =
                                    flow.copy_response_promise(name, &mut body)
                                else {
                                    pending.stage(b"flow: promise is not ready\r\n");
                                    continue;
                                };
                                if storage.start_touch_write(&resolved[..length], &body[..body_len])
                                {
                                    let _ = flow.take_promise(name);
                                    continue;
                                }
                                pending.stage(b"flow: response publication failed\r\n");
                            } else {
                                let Some((_, body_len)) =
                                    flow.copy_response_promise(name, &mut body)
                                else {
                                    pending.stage(b"flow: promise is not ready\r\n");
                                    continue;
                                };
                                if storage.start_touch_write(destination, &body[..body_len]) {
                                    let _ = flow.take_promise(name);
                                    continue;
                                }
                                pending.stage(b"flow: response publication failed\r\n");
                            }
                        }
                        Ok(Some(logos_flow::FlowOperation::WriteResponseVariables {
                            url,
                            destination,
                            url_is_variable,
                            destination_is_variable,
                        })) => {
                            let mut resolved_url = [0; logos_flow::MAX_FLOW_BYTES];
                            let mut resolved_destination = [0; logos_flow::MAX_FLOW_BYTES];
                            let url = if url_is_variable {
                                let Some(length) =
                                    flow.copy_string_variable(url, &mut resolved_url)
                                else {
                                    pending.stage(b"flow: string variable is unavailable\r\n");
                                    continue;
                                };
                                &resolved_url[..length]
                            } else {
                                url
                            };
                            let destination = if destination_is_variable {
                                let Some(length) = flow
                                    .copy_string_variable(destination, &mut resolved_destination)
                                else {
                                    pending.stage(b"flow: string variable is unavailable\r\n");
                                    continue;
                                };
                                &resolved_destination[..length]
                            } else {
                                destination
                            };
                            if !fetch.start_response_to_file(
                                url,
                                destination,
                                flow_is_foreground(bytes),
                            ) {
                                pending.stage(b"fetch request busy or too large\r\n");
                            }
                        }
                        Ok(Some(logos_flow::FlowOperation::AwaitPromise { name })) => {
                            match flow.promise_state(name) {
                                Some(logos_flow::PromiseState::Pending)
                                    if fetch.active_promise_is(name) =>
                                {
                                    fetch.foreground = true;
                                }
                                Some(logos_flow::PromiseState::Ready) => {
                                    let _ = flow.take_promise(name);
                                    pending.stage(b"fetch complete\r\n");
                                }
                                _ => pending.stage(b"flow: promise is not active\r\n"),
                            }
                        }
                        Ok(None) => {
                            pending.stage(b"flow: operation not found\r\n");
                        }
                        Err(error) => {
                            let mut diagnostic = [0; logos_flow::MAX_OUTPUT_BYTES];
                            let length = logos_flow::format_flow_diagnostic(error, &mut diagnostic);
                            pending.stage(&diagnostic[..length]);
                        }
                    }
                }
            }
        }
        if !progressed {
            common::wait(
                common::ipc_read_event(logos_abi::IpcEndpointId::SessionToFlow)
                    | common::ipc_read_event(logos_abi::IpcEndpointId::StorageToFlow)
                    | common::ipc_read_event(logos_abi::IpcEndpointId::FetchToFlow),
                logos_abi::ServiceId::Flow,
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
        let root = provider.complete(CompletionRequest::new(1, b"f", 1).unwrap());
        assert_eq!(root.status, CompletionStatus::Ok);
        assert_eq!(root.candidate(0), Some(&b"fs."[..]));

        let help = provider.complete(CompletionRequest::new(4, b"hel", 3).unwrap());
        assert_eq!(help.candidate(0), Some(&b"help()"[..]));

        let repeated_help = provider.complete(CompletionRequest::new(10, b"help()", 4).unwrap());
        assert_eq!(repeated_help.status, CompletionStatus::NoMatch);

        let clear = provider.complete(CompletionRequest::new(8, b"cle", 3).unwrap());
        assert_eq!(clear.candidate(0), Some(&b"clear()"[..]));

        let echo = provider.complete(CompletionRequest::new(9, b"ech", 3).unwrap());
        assert_eq!(echo.candidate(0), Some(&b"echo(\"\")"[..]));
        assert_eq!(echo.cursor_offsets[0], 6);

        let fs = provider.complete(CompletionRequest::new(5, b"fs.l", 4).unwrap());
        assert_eq!(fs.candidate(0), Some(&b"list()"[..]));
        assert_eq!(fs.cursor_offsets[0], 6);

        let fs_touch = provider.complete(CompletionRequest::new(7, b"fs.t", 4).unwrap());
        assert_eq!(fs_touch.candidate(0), Some(&b"touch(\"\").create()"[..]));
        assert_eq!(fs_touch.cursor_offsets[0], 7);

        let fs_move = provider.complete(CompletionRequest::new(13, b"fs.mo", 5).unwrap());
        assert_eq!(fs_move.candidate(0), Some(&b"move(\"\", \"\")"[..]));
        assert_eq!(fs_move.cursor_offsets[0], 6);

        let network = provider.complete(CompletionRequest::new(14, b"net.", 4).unwrap());
        assert_eq!(network.candidate(1), Some(&b"ping(\"\")"[..]));
        assert_eq!(network.cursor_offsets[1], 6);
        assert_eq!(network.candidate(2), Some(&b"tcp-probe(\"\", 0)"[..]));
        assert_eq!(network.cursor_offsets[2], 11);
        assert_eq!(network.candidate(3), Some(&b"fetch(\"\")"[..]));
        assert_eq!(network.cursor_offsets[3], 7);

        let sys = provider.complete(CompletionRequest::new(6, b"sys.v", 5).unwrap());
        assert_eq!(sys.candidate(0), Some(&b"version()"[..]));

        let member =
            provider.complete(CompletionRequest::new(2, b"service[\"storage\"].re", 21).unwrap());
        assert_eq!(member.candidate(0), Some(&b"restart()"[..]));

        let file_handle =
            provider.complete(CompletionRequest::new(11, b"fs.open(\"test\").", 16).unwrap());
        assert_eq!(file_handle.candidate_count, 2);
        assert_eq!(file_handle.candidate(0), Some(&b"read()"[..]));
        assert_eq!(file_handle.candidate(1), Some(&b"write(\"\")"[..]));
        assert_eq!(file_handle.cursor_offsets[0], 6);
        assert_eq!(file_handle.cursor_offsets[1], 7);

        let packages = provider.complete(CompletionRequest::new(15, b"pkg.", 4).unwrap());
        assert_eq!(packages.cursor_offsets[0], 6);
        assert_eq!(packages.cursor_offsets[1], 6);
        assert_eq!(packages.cursor_offsets[2], 9);

        let filtered_file_handle =
            provider.complete(CompletionRequest::new(12, b"fs.open(\"test\").re", 18).unwrap());
        assert_eq!(filtered_file_handle.candidate_count, 1);
        assert_eq!(filtered_file_handle.candidate(0), Some(&b"read()"[..]));

        let interface =
            provider.complete(CompletionRequest::new(3, b"net.interface[\"e", 16).unwrap());
        assert_eq!(interface.candidate(0), Some(&b"eth0\"]"[..]));
    }

    #[test]
    fn cancelled_package_request_drains_its_response() {
        let mut client = PackageClient::new();
        assert!(client.start(logos_flow::PackageCommand::List));
        client.sent = true;
        client.cancel();
        let message = StorageApiResponse::encode(
            StorageApiStatus::Ok,
            client.request_id,
            0,
            b"storage 1.0.0\r\n",
            false,
        )
        .unwrap();
        client.handle_response(StorageApiResponse::decode(&message).unwrap());
        assert!(!client.active);
        assert!(client.done);
        assert_eq!(&client.result[..client.result_len], b"command cancelled\r\n");
    }
}
