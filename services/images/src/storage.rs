#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", no_main)]
#![cfg_attr(not(target_os = "none"), allow(dead_code, unused_imports, unused_variables))]

mod common;

use logos_abi::{
    IpcBytes, IpcCapability, IpcEndpointId, IpcStatus, MessageKind, PackageOperation,
    PackageRequest, PackageResponse, PackageStatus, StorageApiOperation, StorageApiRequest,
    StorageApiStatus, StorageOperation, StorageRequest, StorageResponse, StorageStatus,
};
use logos_storage::Block;
use logos_storage_service::{
    DurableNamespace, IpcBlockStore, KernelStorageIpc, NamespaceError, StorageApi, error_response,
};

const REQUEST_CAPABILITY: usize = common::capability_slot(
    logos_abi::ServiceId::Storage,
    logos_abi::IpcEndpointId::StorageToCore,
    logos_abi::IpcRights::Send,
);
const RESPONSE_CAPABILITY: usize = common::capability_slot(
    logos_abi::ServiceId::Storage,
    logos_abi::IpcEndpointId::CoreToStorage,
    logos_abi::IpcRights::Receive,
);
const FLOW_RECEIVE_CAPABILITY: usize = common::capability_slot(
    logos_abi::ServiceId::Storage,
    IpcEndpointId::FlowToStorage,
    logos_abi::IpcRights::Receive,
);
const FLOW_SEND_CAPABILITY: usize = common::capability_slot(
    logos_abi::ServiceId::Storage,
    IpcEndpointId::StorageToFlow,
    logos_abi::IpcRights::Send,
);
const FETCH_RECEIVE_CAPABILITY: usize = common::capability_slot(
    logos_abi::ServiceId::Storage,
    IpcEndpointId::FetchToStorage,
    logos_abi::IpcRights::Receive,
);
const FETCH_SEND_CAPABILITY: usize = common::capability_slot(
    logos_abi::ServiceId::Storage,
    IpcEndpointId::StorageToFetch,
    logos_abi::IpcRights::Send,
);
const PACKAGE_RECEIVE_CAPABILITY: usize = common::capability_slot(
    logos_abi::ServiceId::Storage,
    IpcEndpointId::CoreToStoragePackage,
    logos_abi::IpcRights::Receive,
);
const PACKAGE_SEND_CAPABILITY: usize = common::capability_slot(
    logos_abi::ServiceId::Storage,
    IpcEndpointId::StoragePackageToCore,
    logos_abi::IpcRights::Send,
);

#[derive(Clone, Copy, Eq, PartialEq)]
enum Client {
    Flow,
    Fetch,
}

fn send_response(client: Client, response: &IpcBytes) -> IpcStatus {
    match client {
        Client::Flow => common::ipc_send(FLOW_SEND_CAPABILITY, response),
        Client::Fetch => common::ipc_send(FETCH_SEND_CAPABILITY, response),
    }
}

struct StorageTransport {
    operation: Option<StorageOperation>,
}

impl StorageTransport {
    const fn new() -> Self {
        Self { operation: None }
    }

    fn heartbeat(&self) {
        common::heartbeat(logos_abi::ServiceId::Storage);
    }
}

impl KernelStorageIpc for StorageTransport {
    fn send(
        &mut self,
        capability: IpcCapability,
        request: StorageRequest,
        staging: &mut Block,
    ) -> IpcStatus {
        self.heartbeat();
        if common::capability(REQUEST_CAPABILITY) != Some(capability) {
            return IpcStatus::Unauthorized;
        }
        self.operation = Some(request.operation);
        if request.operation == StorageOperation::Write {
            unsafe { *(logos_abi::STORAGE_DATA_BASE as *mut Block) = *staging };
        }
        common::ipc_send(REQUEST_CAPABILITY, &request)
    }

    fn receive(
        &mut self,
        capability: IpcCapability,
        response: &mut StorageResponse,
        staging: &mut Block,
    ) -> IpcStatus {
        self.heartbeat();
        if common::capability(REQUEST_CAPABILITY) != Some(capability) {
            return IpcStatus::Unauthorized;
        }
        let status = common::ipc_receive(RESPONSE_CAPABILITY, response);
        if status == IpcStatus::Ok && self.operation == Some(StorageOperation::Read) {
            *staging = unsafe { *(logos_abi::STORAGE_DATA_BASE as *const Block) };
        }
        status
    }
}

fn new_store(capability: IpcCapability, blocks: u64) -> Option<IpcBlockStore<StorageTransport>> {
    IpcBlockStore::new_with_slot(
        StorageTransport::new(),
        capability,
        REQUEST_CAPABILITY as u16,
        capability.generation,
        capability.service_epoch,
        blocks,
    )
    .ok()
}

fn discover(capability: IpcCapability) -> Option<u64> {
    let request = StorageRequest::new(
        StorageOperation::Reopen,
        1,
        capability.generation,
        REQUEST_CAPABILITY as u16,
        capability.service_epoch,
        0,
        0,
        0,
        0,
    )?;
    let mut transport = StorageTransport::new();
    let mut staging = Block::zero();
    if transport.send(capability, request, &mut staging) != IpcStatus::Ok {
        return None;
    }
    let mut response = StorageResponse::new(1, StorageStatus::Invalid, 1, 0, 0, 0);
    if transport.receive(capability, &mut response, &mut staging) != IpcStatus::Ok
        || response.status != StorageStatus::Ok
    {
        return None;
    }
    (response.block_count > 2).then_some(response.block_count)
}

fn storage_error_status(error: NamespaceError) -> StorageApiStatus {
    if matches!(error, NamespaceError::Format(logos_storage::FormatError::UnsupportedVersion)) {
        StorageApiStatus::Unsupported
    } else {
        StorageApiStatus::Io
    }
}

fn package_status(error: NamespaceError) -> PackageStatus {
    match error {
        NamespaceError::Unsupported
        | NamespaceError::Package(logos_storage_service::PackageCatalogError::Unsupported) => {
            PackageStatus::Unsupported
        }
        NamespaceError::NotFound => PackageStatus::NotFound,
        NamespaceError::Stale
        | NamespaceError::Package(logos_storage_service::PackageCatalogError::Stale) => {
            PackageStatus::Stale
        }
        _ => PackageStatus::Io,
    }
}

fn handle_package_request<B: logos_storage::BlockStore>(
    request: PackageRequest,
    filesystem: &mut DurableNamespace<B>,
) -> PackageResponse {
    let Some(capability) = common::capability(PACKAGE_RECEIVE_CAPABILITY) else {
        return PackageResponse::new(request, PackageStatus::Invalid);
    };
    let service = match request.validate(
        PACKAGE_RECEIVE_CAPABILITY,
        capability.generation,
        capability.service_epoch,
    ) {
        Ok(service) => service,
        Err(status) => return PackageResponse::new(request, status),
    };
    match request.operation {
        PackageOperation::Lookup => match filesystem.lookup_package(service) {
            Ok(info) => PackageResponse::new(request, PackageStatus::Ok).with_package(
                info.handle.generation,
                info.bytes,
                info.package_version,
                info.crc32c,
            ),
            Err(error) => PackageResponse::new(request, package_status(error)),
        },
        PackageOperation::Read => {
            let handle = logos_storage_service::PackageHandle {
                service,
                generation: request.package_generation,
            };
            let mut block = logos_storage::Block::zero();
            match filesystem.read_package(
                handle,
                request.offset as usize,
                &mut block.as_bytes_mut()[..request.length as usize],
            ) {
                Ok(bytes) => {
                    unsafe { *(logos_abi::STORAGE_DATA_BASE as *mut logos_storage::Block) = block };
                    PackageResponse::new(request, PackageStatus::Ok).with_bytes(bytes as u16)
                }
                Err(error) => PackageResponse::new(request, package_status(error)),
            }
        }
    }
}

fn receive_package_request() -> Option<PackageRequest> {
    let mut request = PackageRequest::new(
        PackageOperation::Lookup,
        logos_abi::ServiceId::Storage,
        1,
        common::capability(PACKAGE_RECEIVE_CAPABILITY)
            .map_or(1, |capability| capability.generation),
        PACKAGE_RECEIVE_CAPABILITY as u16,
        common::capability(PACKAGE_RECEIVE_CAPABILITY)
            .map_or(1, |capability| capability.service_epoch),
        0,
        0,
        0,
    )
    .ok()?;
    (common::ipc_receive(PACKAGE_RECEIVE_CAPABILITY, &mut request) == IpcStatus::Ok)
        .then_some(request)
}

fn send_package_response(pending: &mut Option<PackageResponse>) -> bool {
    let Some(response) = *pending else {
        return false;
    };
    match common::ipc_send(PACKAGE_SEND_CAPABILITY, &response) {
        IpcStatus::Ok => {
            *pending = None;
            true
        }
        IpcStatus::Full => false,
        _ => {
            *pending = None;
            true
        }
    }
}

fn serve_storage_error(status: StorageApiStatus) -> ! {
    let mut pending_response: Option<(Client, IpcBytes)> = None;
    let mut pending_package: Option<PackageResponse> = None;
    let mut heartbeat_ticks = 0u16;
    loop {
        common::heartbeat_tick(&mut heartbeat_ticks, logos_abi::ServiceId::Storage);
        let mut progressed = false;
        if send_package_response(&mut pending_package) {
            progressed = true;
        }
        if let Some((client, response)) = pending_response {
            match send_response(client, &response) {
                IpcStatus::Ok => {
                    pending_response = None;
                    progressed = true;
                }
                IpcStatus::Full => {}
                _ => {
                    pending_response = None;
                    progressed = true;
                }
            }
        }
        if pending_response.is_none() && pending_package.is_none() {
            if let Some(request) = receive_package_request() {
                pending_package = Some(PackageResponse::new(request, PackageStatus::Io));
                progressed = true;
            }
        }
        if pending_response.is_none() && pending_package.is_none() {
            let mut request = IpcBytes::empty(MessageKind::StorageRequest);
            let mut client = Client::Flow;
            let status_from_client =
                match common::ipc_receive(FLOW_RECEIVE_CAPABILITY, &mut request) {
                    IpcStatus::Ok => IpcStatus::Ok,
                    _ => {
                        client = Client::Fetch;
                        common::ipc_receive(FETCH_RECEIVE_CAPABILITY, &mut request)
                    }
                };
            match status_from_client {
                IpcStatus::Ok => {
                    pending_response =
                        error_response(&request, status).map(|response| (client, response));
                    progressed = true;
                }
                IpcStatus::Empty => {}
                _ => progressed = true,
            }
        }
        if !progressed {
            common::wait(
                common::ipc_read_event(IpcEndpointId::FlowToStorage)
                    | common::ipc_write_event(IpcEndpointId::StorageToFlow)
                    | common::ipc_read_event(IpcEndpointId::FetchToStorage)
                    | common::ipc_write_event(IpcEndpointId::StorageToFetch)
                    | common::ipc_read_event(IpcEndpointId::CoreToStoragePackage)
                    | common::ipc_write_event(IpcEndpointId::StoragePackageToCore),
                logos_abi::ServiceId::Storage,
            );
        }
    }
}

fn run_filesystem(capability: IpcCapability, blocks: u64) -> ! {
    let Some(store) = new_store(capability, blocks) else {
        serve_storage_error(StorageApiStatus::Io);
    };
    let mut filesystem = match DurableNamespace::open(store) {
        Ok(filesystem) => filesystem,
        Err(NamespaceError::Format(logos_storage::FormatError::Unformatted)) => {
            let Some(store) = new_store(capability, blocks) else {
                serve_storage_error(StorageApiStatus::Io);
            };
            DurableNamespace::format(store)
                .unwrap_or_else(|error| serve_storage_error(storage_error_status(error)))
        }
        Err(NamespaceError::Format(logos_storage::FormatError::ProvisionedBlank)) => {
            let Some(store) = new_store(capability, blocks) else {
                serve_storage_error(StorageApiStatus::Io);
            };
            DurableNamespace::format_provisioned(store)
                .unwrap_or_else(|error| serve_storage_error(storage_error_status(error)))
        }
        Err(error) => serve_storage_error(storage_error_status(error)),
    };
    filesystem.flush().unwrap_or_else(|error| serve_storage_error(storage_error_status(error)));

    let mut api = StorageApi::new(filesystem);
    let mut pending_response: Option<(Client, IpcBytes)> = None;
    let mut pending_package: Option<PackageResponse> = None;
    let mut heartbeat_ticks = 0u16;
    loop {
        common::heartbeat_tick(&mut heartbeat_ticks, logos_abi::ServiceId::Storage);
        let mut progressed = false;
        if send_package_response(&mut pending_package) {
            progressed = true;
        }
        if let Some((client, response)) = pending_response {
            if send_response(client, &response) == IpcStatus::Ok {
                pending_response = None;
                progressed = true;
            }
        }
        if pending_response.is_none() && pending_package.is_none() {
            if let Some(request) = receive_package_request() {
                pending_package = Some(handle_package_request(request, api.namespace_mut()));
                progressed = true;
            }
        }
        if pending_response.is_none() && pending_package.is_none() {
            let mut request = IpcBytes::empty(MessageKind::StorageRequest);
            let mut client = Client::Flow;
            let received = match common::ipc_receive(FLOW_RECEIVE_CAPABILITY, &mut request) {
                IpcStatus::Ok => IpcStatus::Ok,
                _ => {
                    client = Client::Fetch;
                    common::ipc_receive(FETCH_RECEIVE_CAPABILITY, &mut request)
                }
            };
            if received == IpcStatus::Ok {
                let stage_from_flow = client == Client::Flow
                    && StorageApiRequest::decode(&request).is_ok_and(|request| {
                        matches!(
                            request.operation,
                            StorageApiOperation::StageWriteBegin
                                | StorageApiOperation::StageWriteChunk
                                | StorageApiOperation::StageWriteCommit
                                | StorageApiOperation::StageWriteAbort
                        )
                    });
                pending_response = if stage_from_flow {
                    error_response(&request, StorageApiStatus::Invalid)
                } else {
                    api.handle(&request)
                }
                .map(|response| (client, response));
                progressed = true;
            }
        }
        if !progressed {
            common::wait(
                common::ipc_read_event(IpcEndpointId::FlowToStorage)
                    | common::ipc_write_event(IpcEndpointId::StorageToFlow)
                    | common::ipc_read_event(IpcEndpointId::FetchToStorage)
                    | common::ipc_write_event(IpcEndpointId::StorageToFetch)
                    | common::ipc_read_event(IpcEndpointId::CoreToStoragePackage)
                    | common::ipc_write_event(IpcEndpointId::StoragePackageToCore),
                logos_abi::ServiceId::Storage,
            );
        }
    }
}

/// The storage image owns the durable format and namespace. It discovers the
/// whole raw volume, reopens valid media, or formats only blank media.
#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    let Some(capability) = common::capability(REQUEST_CAPABILITY) else {
        serve_storage_error(StorageApiStatus::Io)
    };
    let Some(blocks) = discover(capability) else { serve_storage_error(StorageApiStatus::Io) };
    run_filesystem(capability, blocks)
}

#[cfg(target_os = "none")]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo<'_>) -> ! {
    common::idle()
}

#[cfg(not(target_os = "none"))]
fn main() {}
