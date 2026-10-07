#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", no_main)]
#![cfg_attr(not(target_os = "none"), allow(dead_code, unused_imports, unused_variables))]

use core::{
    mem, ptr,
    sync::atomic::{AtomicU32, Ordering},
};

mod common;

#[cfg(feature = "storage-proof")]
use logos_abi::StorageApiStatus;
use logos_abi::{
    COMPLETION_FLAG_TRUNCATED, CompletionResponse, DeviceRequest, DeviceResponse, FlowControl,
    GuiSessionContext, IpcBytes, IpcStatus, MAX_COMPLETION_ITEM_BYTES, MessageKind,
};
#[cfg(all(feature = "qemu-proof", not(feature = "lockscreen-proof")))]
use logos_abi::{NetworkOperation, NetworkRequest, NetworkResult, NetworkState};

// logos-flow sizes its Storage client buffer without depending on the storage service.
const _: () = assert!(logos_flow::MAX_STORAGE_DATA_BYTES == logos_storage_service::MAX_FILE_BYTES);

const INPUT_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_BYTES,
    b"session",
    mem::size_of::<IpcBytes>(),
    logos_abi::IpcRights::Receive,
);
const OUTPUT_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_BYTES,
    b"session",
    mem::size_of::<IpcBytes>(),
    logos_abi::IpcRights::Send,
);
const STORAGE_SEND_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_BYTES,
    b"storage",
    mem::size_of::<IpcBytes>(),
    logos_abi::IpcRights::Send,
);
const STORAGE_RECEIVE_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_BYTES,
    b"storage",
    mem::size_of::<IpcBytes>(),
    logos_abi::IpcRights::Receive,
);
const NETWORK_SEND_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_BYTES,
    b"network",
    mem::size_of::<IpcBytes>(),
    logos_abi::IpcRights::Send,
);
const NETWORK_RECEIVE_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_BYTES,
    b"network",
    mem::size_of::<IpcBytes>(),
    logos_abi::IpcRights::Receive,
);
const FETCH_SEND_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_BYTES,
    b"fetch",
    mem::size_of::<IpcBytes>(),
    logos_abi::IpcRights::Send,
);
const FETCH_RECEIVE_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_BYTES,
    b"fetch",
    mem::size_of::<IpcBytes>(),
    logos_abi::IpcRights::Receive,
);
const DEVICE_SEND_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_DEVICE_REQUEST,
    b"device",
    mem::size_of::<DeviceRequest>(),
    logos_abi::IpcRights::Send,
);
const DEVICE_RECEIVE_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_DEVICE_RESPONSE,
    b"device",
    mem::size_of::<DeviceResponse>(),
    logos_abi::IpcRights::Receive,
);
const USER_SEND_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_BYTES,
    b"user",
    mem::size_of::<IpcBytes>(),
    logos_abi::IpcRights::Send,
);
const USER_RECEIVE_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_BYTES,
    b"user",
    mem::size_of::<IpcBytes>(),
    logos_abi::IpcRights::Receive,
);
const SHELL_CONTEXT_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_GUI_SESSION,
    b"shell",
    mem::size_of::<GuiSessionContext>(),
    logos_abi::IpcRights::Receive,
);

#[derive(Clone, Copy)]
struct IpcCapabilities {
    input: logos_abi::CapabilityHandle,
    output: logos_abi::CapabilityHandle,
    storage_send: logos_abi::CapabilityHandle,
    storage_receive: logos_abi::CapabilityHandle,
    network_send: logos_abi::CapabilityHandle,
    network_receive: logos_abi::CapabilityHandle,
    fetch_send: logos_abi::CapabilityHandle,
    fetch_receive: logos_abi::CapabilityHandle,
    device_send: logos_abi::CapabilityHandle,
    device_receive: logos_abi::CapabilityHandle,
    user_send: logos_abi::CapabilityHandle,
    user_receive: logos_abi::CapabilityHandle,
    shell_context: logos_abi::CapabilityHandle,
}

static mut IPC_CAPABILITIES: Option<IpcCapabilities> = None;

fn ipc_capabilities() -> IpcCapabilities {
    unsafe {
        (*core::ptr::addr_of!(IPC_CAPABILITIES)).unwrap_or(IpcCapabilities {
            input: logos_abi::CapabilityHandle::EMPTY,
            output: logos_abi::CapabilityHandle::EMPTY,
            storage_send: logos_abi::CapabilityHandle::EMPTY,
            storage_receive: logos_abi::CapabilityHandle::EMPTY,
            network_send: logos_abi::CapabilityHandle::EMPTY,
            network_receive: logos_abi::CapabilityHandle::EMPTY,
            fetch_send: logos_abi::CapabilityHandle::EMPTY,
            fetch_receive: logos_abi::CapabilityHandle::EMPTY,
            device_send: logos_abi::CapabilityHandle::EMPTY,
            device_receive: logos_abi::CapabilityHandle::EMPTY,
            user_send: logos_abi::CapabilityHandle::EMPTY,
            user_receive: logos_abi::CapabilityHandle::EMPTY,
            shell_context: logos_abi::CapabilityHandle::EMPTY,
        })
    }
}

fn wait_for_ipc() {
    let capabilities = ipc_capabilities();
    common::wait_on_capabilities(&[
        capabilities.input,
        capabilities.output,
        capabilities.storage_send,
        capabilities.storage_receive,
        capabilities.network_send,
        capabilities.network_receive,
        capabilities.fetch_send,
        capabilities.fetch_receive,
        capabilities.device_send,
        capabilities.device_receive,
        capabilities.user_send,
        capabilities.user_receive,
        capabilities.shell_context,
    ]);
}

/// The real `logos_flow::Transport` adapter: maps a `Port` to the capability
/// handles discovered at startup and drives the `common::ipc_*` syscalls.
struct IpcTransport;

impl logos_flow::Transport for IpcTransport {
    fn send<T: Copy>(&mut self, port: logos_flow::Port, message: &T) -> IpcStatus {
        let capabilities = ipc_capabilities();
        let capability = match port {
            logos_flow::Port::Storage => capabilities.storage_send,
            logos_flow::Port::Network => capabilities.network_send,
            logos_flow::Port::Fetch => capabilities.fetch_send,
            logos_flow::Port::Device => capabilities.device_send,
            logos_flow::Port::User => capabilities.user_send,
            logos_flow::Port::Input => capabilities.input,
            logos_flow::Port::Output => capabilities.output,
        };
        common::ipc_send_handle(capability, message)
    }

    fn receive<T: Copy>(&mut self, port: logos_flow::Port, message: &mut T) -> IpcStatus {
        let capabilities = ipc_capabilities();
        let capability = match port {
            logos_flow::Port::Storage => capabilities.storage_receive,
            logos_flow::Port::Network => capabilities.network_receive,
            logos_flow::Port::Fetch => capabilities.fetch_receive,
            logos_flow::Port::Device => capabilities.device_receive,
            logos_flow::Port::User => capabilities.user_receive,
            logos_flow::Port::Input => capabilities.input,
            logos_flow::Port::Output => capabilities.output,
        };
        common::ipc_receive_handle(capability, message)
    }

    fn wait(&mut self) {
        wait_for_ipc();
    }

    fn next_request_id(&mut self, space: logos_flow::IdSpace) -> u32 {
        match space {
            logos_flow::IdSpace::Network => next_network_request_id(),
            logos_flow::IdSpace::Device => next_device_request_id(),
            logos_flow::IdSpace::User => next_user_request_id(),
        }
    }

    fn proof_line(&mut self, line: &[u8]) {
        #[cfg(feature = "fetch-proof")]
        common::proof_line(line);
        #[cfg(not(feature = "fetch-proof"))]
        let _ = line;
    }
}

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

#[cfg(all(feature = "qemu-proof", not(feature = "lockscreen-proof")))]
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
        || &response.record.name[..usize::from(response.record.name_len)] != b"input"
    {
        return false;
    }
    response.record.service.is_valid()
}

/// Completion's only live data: the service manager's names starting with `prefix`.
#[allow(clippy::result_unit_err)]
fn append_service_names(prefix: &[u8], response: &mut CompletionResponse) -> Result<(), ()> {
    let mut cursor = 0u64;
    loop {
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
        let name_len =
            usize::from(manager_response.record.name_len).min(manager_response.record.name.len());
        let name = &manager_response.record.name[..name_len];
        if name.starts_with(prefix) {
            let mut candidate = [0; MAX_COMPLETION_ITEM_BYTES];
            let Some(length) = logos_flow::copy_candidate(&mut candidate, name, b"\"]") else {
                response.flags |= COMPLETION_FLAG_TRUNCATED;
                return Ok(());
            };
            if !response.push_candidate(&candidate[..length]) {
                response.flags |= COMPLETION_FLAG_TRUNCATED;
                return Ok(());
            }
        }
        if manager_response.cursor == u64::MAX {
            break;
        }
        if manager_response.cursor <= cursor {
            return Err(());
        }
        cursor = manager_response.cursor;
    }
    Ok(())
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

    fn consume_result(&mut self, storage: &mut logos_flow::StorageClient) -> bool {
        if !storage.done() || !self.active {
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

    fn start_next(
        &mut self,
        storage: &mut logos_flow::StorageClient,
        pending: &logos_flow::PendingOutput,
    ) -> bool {
        if self.active
            || self.step == u8::MAX
            || storage.active()
            || storage.done()
            || pending.is_pending()
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
        if self.step != u8::MAX {
            return false;
        }
        // The kernel refuses until its own proof passed, so keep asking.
        common::power(logos_flow::FlowAction::Shutdown as usize) != 0
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

fn program_command(
    command: logos_flow::ProgramCommand<'_>,
    pending: &mut logos_flow::PendingOutput,
) {
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
    let mut cursor = 0u64;
    loop {
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
        if response.cursor == u64::MAX {
            return Ok(None);
        }
        if response.cursor <= cursor {
            return Err(IpcStatus::Malformed);
        }
        cursor = response.cursor;
    }
}

fn service_command(
    command: logos_flow::ServiceCommand<'_>,
    pending: &mut logos_flow::PendingOutput,
) {
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
    let mut cursor = 0u64;
    loop {
        let request_id = next_manager_request_id();
        let mut request = logos_abi::ManagerRequest::new(operation, request_id);
        request.cursor = cursor;
        if let Some(record) = target {
            request.service = record.service;
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
        if !list || response.cursor != u64::MAX {
            let record = response.record;
            output_len += logos_flow::format_service_property(
                &record,
                if list { logos_flow::ServiceProperty::Record } else { property },
                &mut output[output_len..],
            );
        }
        if !list || response.cursor == u64::MAX {
            break;
        }
        if response.cursor <= cursor {
            pending.stage(b"service manager malformed response\r\n");
            return;
        }
        cursor = response.cursor;
    }
    pending.stage(&output[..output_len]);
}

#[cfg(all(feature = "qemu-proof", not(feature = "lockscreen-proof")))]
fn manager_restart_probe() -> bool {
    let Some(record) = manager_record(b"storage").ok().flatten() else {
        return false;
    };
    let request_id = next_manager_request_id();
    let mut request =
        logos_abi::ManagerRequest::new(logos_abi::ManagerOperation::Restart, request_id);
    request.service = record.service;
    let mut response = logos_abi::ManagerResponse::new(
        logos_abi::ManagerOperation::Restart,
        logos_abi::ManagerStatus::Malformed,
        request_id,
    );
    common::manager_call(&request, &mut response) == IpcStatus::Ok
        && response.status == logos_abi::ManagerStatus::Accepted
        && response.record.state == logos_abi::ManagerState::Stopping
}

#[cfg(all(feature = "qemu-proof", not(feature = "lockscreen-proof")))]
fn network_proof_probe(
    network: &mut logos_flow::NetworkClient,
    transport: &mut IpcTransport,
) -> bool {
    for _ in 0..256 {
        let Ok(status) = network.request(transport, NetworkOperation::Status, [0; 4], 0) else {
            return false;
        };
        if status.state == NetworkState::Disabled {
            return true;
        }
        if status.state == NetworkState::Ready {
            let Ok(tcp) =
                network.request(transport, NetworkOperation::TcpConnect, [10, 0, 2, 2], 8080)
            else {
                return false;
            };
            if tcp.result != NetworkResult::Ok {
                return false;
            }
            if !manager_restart_network() {
                return false;
            }
            for _ in 0..256 {
                let Ok(status) = network.request(transport, NetworkOperation::Status, [0; 4], 0)
                else {
                    return false;
                };
                if status.state == NetworkState::Ready {
                    let mut listen =
                        NetworkRequest::new(NetworkOperation::TcpListen, next_network_request_id());
                    listen.port = 8081;
                    let Ok(listener) = network.request_message(transport, listen) else {
                        return false;
                    };
                    if listener.result != NetworkResult::Ok {
                        return false;
                    }
                    let Ok(_) =
                        network.request(transport, NetworkOperation::IcmpPing, [10, 0, 2, 2], 0)
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
                            let Ok(response) = network.request_message(transport, accept) else {
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
                        let Ok(write_response) = network.request_message(transport, write) else {
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
                        let Ok(read_response) = network.request_message(transport, read) else {
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
                        .request_message(transport, close)
                        .is_ok_and(|response| response.result == NetworkResult::Stale);
                }
                common::sleep();
            }
            return false;
        }
        common::sleep();
    }
    false
}

#[cfg(all(feature = "qemu-proof", not(feature = "lockscreen-proof")))]
fn manager_restart_network() -> bool {
    let Some(record) = manager_record(b"network").ok().flatten() else {
        return false;
    };
    let request_id = next_manager_request_id();
    let mut request =
        logos_abi::ManagerRequest::new(logos_abi::ManagerOperation::Restart, request_id);
    request.service = record.service;
    let mut response = logos_abi::ManagerResponse::new(
        logos_abi::ManagerOperation::Restart,
        logos_abi::ManagerStatus::Malformed,
        request_id,
    );
    common::manager_call(&request, &mut response) == IpcStatus::Ok
        && response.status == logos_abi::ManagerStatus::Accepted
}

#[cfg(all(feature = "qemu-proof", not(feature = "lockscreen-proof")))]
fn manager_command_probe(
    pending: &mut logos_flow::PendingOutput,
    network: &mut logos_flow::NetworkClient,
    transport: &mut IpcTransport,
) -> bool {
    let Some(initial_storage) = manager_record(b"storage").ok().flatten() else {
        return false;
    };
    if cfg!(feature = "fetch-proof") {
        let _ = (pending, network, initial_storage);
        return true;
    }
    if initial_storage.service.generation() != 1 || initial_storage.restarts != 0 {
        return network_proof_probe(network, transport);
    }
    service_command(logos_flow::ServiceCommand::List, pending);
    let list = pending.staged();
    let list_valid = pending.is_pending()
        && list
            .windows(b"storage running\r\n".len())
            .any(|window| window == b"storage running\r\n")
        && !list.windows(b"vacant".len()).any(|window| window == b"vacant");
    pending.discard();
    if !list_valid {
        return false;
    }
    service_command(logos_flow::ServiceCommand::Stop { name: b"input" }, pending);
    let expected = b"service dependency violation\r\n";
    let dependency_valid = pending.is_pending() && pending.staged() == *expected;
    pending.discard();
    if !dependency_valid {
        return false;
    }
    network_proof_probe(network, transport) && manager_restart_probe()
}

/// One interpreter (variables) per Terminal session (T3c, #98). The seven
/// clients below stay singletons: Session (`services/images/src/session.rs`)
/// only ever lets one session's command reach Flow at a time, so they never
/// need to be duplicated, only tagged by whichever session currently owns
/// them (`ACTIVE_SESSION`).
const EMPTY_FLOW_SERVICE: logos_flow::FlowService = logos_flow::FlowService::new();
static mut FLOWS: [logos_flow::FlowService; logos_abi::MAX_SHELL_SESSIONS] =
    [EMPTY_FLOW_SERVICE; logos_abi::MAX_SHELL_SESSIONS];
/// The session whose `SessionInput`/`CompletionRequest` is currently being
/// served; every reply Flow sends out is tagged with it until the next
/// `SessionInput`/`CompletionRequest` picks a new one.
static mut ACTIVE_SESSION: u8 = 0;
static mut PENDING: logos_flow::PendingOutput = logos_flow::PendingOutput::new();
static mut STORAGE: logos_flow::StorageClient = logos_flow::StorageClient::new();
static mut PACKAGE: logos_flow::PackageClient = logos_flow::PackageClient::new();
static mut DEVICE: logos_flow::DeviceClient = logos_flow::DeviceClient::new();
static mut USER: logos_flow::UserClient = logos_flow::UserClient::new();
static mut NETWORK: logos_flow::NetworkClient = logos_flow::NetworkClient::new();
static mut FETCH: logos_flow::FetchClient = logos_flow::FetchClient::new();
static mut COMPLETION: logos_flow::CompletionService = logos_flow::CompletionService::new();
static mut PENDING_COMPLETION: Option<IpcBytes> = None;

fn required_capability(spec: common::CapabilitySpec) -> logos_abi::CapabilityHandle {
    common::capability_handle(spec).unwrap_or_else(|_| common::idle())
}

fn active_session() -> u8 {
    unsafe { *core::ptr::addr_of!(ACTIVE_SESSION) }
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    common::init_service_allocator();
    let capabilities = IpcCapabilities {
        input: required_capability(INPUT_CAPABILITY),
        output: required_capability(OUTPUT_CAPABILITY),
        storage_send: required_capability(STORAGE_SEND_CAPABILITY),
        storage_receive: required_capability(STORAGE_RECEIVE_CAPABILITY),
        network_send: required_capability(NETWORK_SEND_CAPABILITY),
        network_receive: required_capability(NETWORK_RECEIVE_CAPABILITY),
        fetch_send: required_capability(FETCH_SEND_CAPABILITY),
        fetch_receive: required_capability(FETCH_RECEIVE_CAPABILITY),
        device_send: required_capability(DEVICE_SEND_CAPABILITY),
        device_receive: required_capability(DEVICE_RECEIVE_CAPABILITY),
        user_send: required_capability(USER_SEND_CAPABILITY),
        user_receive: required_capability(USER_RECEIVE_CAPABILITY),
        shell_context: required_capability(SHELL_CONTEXT_CAPABILITY),
    };
    unsafe { *core::ptr::addr_of_mut!(IPC_CAPABILITIES) = Some(capabilities) };
    let flows = unsafe { &mut *core::ptr::addr_of_mut!(FLOWS) };
    let pending = unsafe { &mut *core::ptr::addr_of_mut!(PENDING) };
    let storage = unsafe { &mut *core::ptr::addr_of_mut!(STORAGE) };
    let package = unsafe { &mut *core::ptr::addr_of_mut!(PACKAGE) };
    let device = unsafe { &mut *core::ptr::addr_of_mut!(DEVICE) };
    let user = unsafe { &mut *core::ptr::addr_of_mut!(USER) };
    let network = unsafe { &mut *core::ptr::addr_of_mut!(NETWORK) };
    let fetch = unsafe { &mut *core::ptr::addr_of_mut!(FETCH) };
    let completion = unsafe { &mut *core::ptr::addr_of_mut!(COMPLETION) };
    let pending_completion = unsafe { &mut *core::ptr::addr_of_mut!(PENDING_COMPLETION) };
    let mut transport = IpcTransport;
    let mut shell_context = GuiSessionContext::EMPTY;
    #[cfg(all(feature = "qemu-proof", not(feature = "lockscreen-proof")))]
    while !manager_boot_probe() {
        common::sleep();
    }
    #[cfg(all(feature = "qemu-proof", not(feature = "lockscreen-proof")))]
    if !manager_command_probe(pending, network, &mut transport) {
        // The storage proof exercises the normal Flow->Storage command path;
        // a failed optional manager/network preflight must not strand Flow
        // before that workload can run.
        #[cfg(not(feature = "storage-proof"))]
        common::idle();
    }
    #[cfg(feature = "storage-proof")]
    let mut proof = StorageProof::new();
    let mut heartbeat_ticks = 0u16;
    loop {
        if pending.is_pending()
            || pending_completion.is_some()
            || storage.active()
            || package.active()
            || device.active()
            || user.active()
            || fetch.active()
        {
            common::heartbeat();
        } else {
            common::heartbeat_tick(&mut heartbeat_ticks);
        }
        let mut progressed = pending.flush(&mut transport, active_session());
        while common::ipc_receive_handle(ipc_capabilities().shell_context, &mut shell_context)
            == IpcStatus::Ok
        {
            user.adopt_context(shell_context);
            progressed = true;
        }
        if storage.active()
            || package.active()
            || device.active()
            || user.active()
            || (fetch.active() && fetch.foreground())
        {
            let mut control = IpcBytes::empty(MessageKind::FlowControl);
            if common::ipc_receive_handle(ipc_capabilities().input, &mut control) == IpcStatus::Ok {
                if fetch.active() {
                    progressed |= fetch.handle_control(&control, active_session());
                } else if control.kind == MessageKind::FlowControl
                    && control.len as usize == mem::size_of::<FlowControl>()
                {
                    let value: FlowControl =
                        unsafe { ptr::read_unaligned(control.bytes.as_ptr().cast()) };
                    if value.is_valid() && value.session == active_session() {
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
        if fetch.active()
            && fetch.cancel_pending()
            && fetch.send_cancel(&mut transport) == IpcStatus::Full
        {
            wait_for_ipc();
            continue;
        }
        if pending.is_pending() {
            if !progressed {
                wait_for_ipc();
            }
            continue;
        }
        if let Some(message) = *pending_completion {
            match common::ipc_send_handle(ipc_capabilities().output, &message) {
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
                    wait_for_ipc();
                }
                continue;
            }
        }
        if storage.active() {
            progressed |= storage.drive(&mut transport);
            if storage.done() {
                #[cfg(feature = "storage-proof")]
                if !proof.active() {
                    storage.take_result(&mut transport, pending);
                }
                #[cfg(not(feature = "storage-proof"))]
                storage.take_result(&mut transport, pending);
                progressed = true;
            }
            if storage.active() {
                if !progressed {
                    wait_for_ipc();
                }
                continue;
            }
        }
        if package.active() {
            progressed |= package.drive(&mut transport);
            if package.done() {
                package.take_result(pending);
                progressed = true;
            }
            if package.active() {
                if !progressed {
                    wait_for_ipc();
                }
                continue;
            }
        }
        if device.active() {
            progressed |= device.drive(&mut transport);
            if device.done() {
                device.take_result(pending);
                progressed = true;
            }
            if device.active() {
                if !progressed {
                    wait_for_ipc();
                }
                continue;
            }
        }
        if user.active() {
            progressed |= user.drive(&mut transport);
            if user.done() {
                user.take_result(pending);
                progressed = true;
            }
            if user.active() {
                if !progressed {
                    wait_for_ipc();
                }
                continue;
            }
        }
        if fetch.active() {
            progressed |= fetch.drive(&mut transport, pending, active_session());
            if !fetch.active() {
                fetch.resolve_promise(&mut flows[active_session() as usize]);
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
                    wait_for_ipc();
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
        if common::ipc_receive_handle(ipc_capabilities().input, &mut message) == IpcStatus::Ok {
            progressed = true;
            // Every reply below (`pending`, `pending_completion`, fetch
            // progress) is tagged with this until the next `SessionInput`/
            // `CompletionRequest` changes it (T3c, #98): Session never
            // lets a second session's request reach Flow before this
            // one's reply comes back, so it can't change mid-command.
            unsafe { *core::ptr::addr_of_mut!(ACTIVE_SESSION) = message.session() };
            if message.kind == MessageKind::CompletionRequest {
                if let Some(request) = logos_flow::completion_request(&message) {
                    *pending_completion = Some(
                        logos_flow::completion_message(
                            completion.complete(request, append_service_names),
                        )
                        .with_session(message.session()),
                    );
                    progressed = true;
                }
            } else if message.kind == MessageKind::SessionInput {
                if let Some(bytes) = message.as_bytes() {
                    let flow = &mut flows[message.session() as usize];
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
                            logos_flow::network_command(
                                command,
                                network,
                                &mut transport,
                                fetch,
                                pending,
                            )
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
                            if !device.start(&mut transport, command) {
                                pending.stage(b"device request busy or too large\r\n");
                            }
                        }
                        Ok(Some(logos_flow::FlowOperation::User(command))) => {
                            if !user.start(&mut transport, command) {
                                pending.stage(b"user request busy or too large\r\n");
                            }
                        }
                        Ok(Some(logos_flow::FlowOperation::Program(command))) => {
                            program_command(command, pending);
                        }
                        Ok(Some(logos_flow::FlowOperation::System(operation))) => match operation {
                            logos_flow::SystemOperation::Version => {
                                let version = logos_abi::LOGOS_VERSION;
                                let mut line = [0u8; logos_abi::LOGOS_VERSION.len() + 2];
                                line[..version.len()].copy_from_slice(version);
                                line[version.len()..].copy_from_slice(b"\r\n");
                                pending.stage(&line)
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
                            let foreground = logos_flow::flow_is_foreground(bytes);
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
                            let foreground = logos_flow::flow_is_foreground(bytes);
                            if !(if foreground {
                                fetch.start_response(&mut transport, url)
                            } else {
                                fetch.start_response_background(&mut transport, url)
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
                            let foreground = logos_flow::flow_is_foreground(bytes);
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
                                    fetch.start_response(
                                        &mut transport,
                                        &resolved_url[..resolved_len],
                                    )
                                } else {
                                    fetch.start_response_background(
                                        &mut transport,
                                        &resolved_url[..resolved_len],
                                    )
                                }
                            } else {
                                fetch.start_named_response(
                                    &mut transport,
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
                            let foreground = logos_flow::flow_is_foreground(bytes);
                            if !fetch.start_to_file_mode(
                                &mut transport,
                                url,
                                destination,
                                foreground,
                            ) {
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
                            let foreground = logos_flow::flow_is_foreground(bytes);
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
                                &mut transport,
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
                                &mut transport,
                                url,
                                destination,
                                logos_flow::flow_is_foreground(bytes),
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
                                &mut transport,
                                url,
                                destination,
                                logos_flow::flow_is_foreground(bytes),
                            ) {
                                pending.stage(b"fetch request busy or too large\r\n");
                            }
                        }
                        Ok(Some(logos_flow::FlowOperation::AwaitPromise { name })) => {
                            match flow.promise_state(name) {
                                Some(logos_flow::PromiseState::Pending)
                                    if fetch.active_promise_is(name) =>
                                {
                                    fetch.set_foreground();
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
            wait_for_ipc();
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
    fn each_sessions_flow_service_keeps_its_own_variables() {
        // T3c (#98): `FLOWS` holds one `FlowService` per Terminal session
        // so a `var` assignment in one tab can never leak into another's.
        let mut flows = [logos_flow::FlowService::new(), logos_flow::FlowService::new()];
        assert!(flows[0].validate(br#"var text = "session-a""#).is_ok());
        assert!(flows[1].validate(br#"var text = "session-b""#).is_ok());
        let mut output = [0u8; 32];
        let length = flows[0].copy_string_variable(b"text", &mut output).unwrap();
        assert_eq!(&output[..length], b"session-a");
        let length = flows[1].copy_string_variable(b"text", &mut output).unwrap();
        assert_eq!(&output[..length], b"session-b");
    }
}
