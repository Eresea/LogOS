#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", no_main)]
#![cfg_attr(not(target_os = "none"), allow(dead_code, unused_imports, unused_variables))]

mod common;

use logos_abi::{
    IpcBytes, IpcEndpointId, IpcStatus, MessageKind, PackageOperation, PackageRequest,
    PackageResponse, PackageStatus, PackageTargetKind, StorageApiOperation, StorageApiStatus,
    StorageMapRequest, StorageMapResponse, StorageOperation, StorageRequest, StorageResponse,
    StorageStatus, USER_STORAGE_FLAG_BEGIN, USER_STORAGE_FLAG_END, UserStorageOperation,
    UserStorageRequest, UserStorageResponse, UserStorageStatus,
};
use logos_storage::Block;
use logos_storage_service::{
    DurableNamespaceV5, IpcBlockStore, KernelStorageIpc, NamespaceError, StorageApiV5,
    StorageCapability, error_response,
};
use logos_user::{USER_SNAPSHOT_BYTES, UserCatalog, UserCatalogStore};

const REQUEST_PROTOCOL_SLOT: u16 = 2;
const REQUEST_CAPABILITY: common::CapabilitySpec = common::capability_contract(
    logos_abi::IPC_CONTRACT_STORAGE_REQUEST,
    common::CORE_PEER_INDEX,
    core::mem::size_of::<StorageRequest>(),
    logos_abi::IpcRights::Send,
);
const RESPONSE_CAPABILITY: common::CapabilitySpec = common::capability_contract(
    logos_abi::IPC_CONTRACT_STORAGE_RESPONSE,
    common::CORE_PEER_INDEX,
    core::mem::size_of::<StorageResponse>(),
    logos_abi::IpcRights::Receive,
);
const FLOW_RECEIVE_CAPABILITY: common::CapabilitySpec = common::capability_contract(
    logos_abi::IPC_CONTRACT_BYTES,
    logos_abi::ServiceId::Flow.index() as u32,
    core::mem::size_of::<IpcBytes>(),
    logos_abi::IpcRights::Receive,
);
const FLOW_SEND_CAPABILITY: common::CapabilitySpec = common::capability_contract(
    logos_abi::IPC_CONTRACT_BYTES,
    logos_abi::ServiceId::Flow.index() as u32,
    core::mem::size_of::<IpcBytes>(),
    logos_abi::IpcRights::Send,
);
const FETCH_RECEIVE_CAPABILITY: common::CapabilitySpec = common::capability_contract(
    logos_abi::IPC_CONTRACT_BYTES,
    logos_abi::ServiceId::Fetch.index() as u32,
    core::mem::size_of::<IpcBytes>(),
    logos_abi::IpcRights::Receive,
);
const FETCH_SEND_CAPABILITY: common::CapabilitySpec = common::capability_contract(
    logos_abi::IPC_CONTRACT_BYTES,
    logos_abi::ServiceId::Fetch.index() as u32,
    core::mem::size_of::<IpcBytes>(),
    logos_abi::IpcRights::Send,
);
const PACKAGE_RECEIVE_CAPABILITY: common::CapabilitySpec = common::capability_contract(
    logos_abi::IPC_CONTRACT_PACKAGE_REQUEST,
    common::CORE_PEER_INDEX,
    core::mem::size_of::<PackageRequest>(),
    logos_abi::IpcRights::Receive,
);
const PACKAGE_SEND_CAPABILITY: common::CapabilitySpec = common::capability_contract(
    logos_abi::IPC_CONTRACT_PACKAGE_RESPONSE,
    common::CORE_PEER_INDEX,
    core::mem::size_of::<PackageResponse>(),
    logos_abi::IpcRights::Send,
);
const MAP_REQUEST_CAPABILITY: common::CapabilitySpec = common::capability_contract(
    logos_abi::IPC_CONTRACT_STORAGE_MAP_REQUEST,
    common::CORE_PEER_INDEX,
    core::mem::size_of::<StorageMapRequest>(),
    logos_abi::IpcRights::Send,
);
const MAP_RESPONSE_CAPABILITY: common::CapabilitySpec = common::capability_contract(
    logos_abi::IPC_CONTRACT_STORAGE_MAP_RESPONSE,
    common::CORE_PEER_INDEX,
    core::mem::size_of::<StorageMapResponse>(),
    logos_abi::IpcRights::Receive,
);
const USER_RECEIVE_CAPABILITY: common::CapabilitySpec = common::capability_contract(
    logos_abi::IPC_CONTRACT_BYTES,
    logos_abi::ServiceId::User.index() as u32,
    core::mem::size_of::<IpcBytes>(),
    logos_abi::IpcRights::Receive,
);
const USER_SEND_CAPABILITY: common::CapabilitySpec = common::capability_contract(
    logos_abi::IPC_CONTRACT_BYTES,
    logos_abi::ServiceId::User.index() as u32,
    core::mem::size_of::<IpcBytes>(),
    logos_abi::IpcRights::Send,
);

static mut USER_CATALOG: UserCatalog = UserCatalog::new();
static mut USER_CATALOG_BUFFER: [u8; USER_SNAPSHOT_BYTES] = [0; USER_SNAPSHOT_BYTES];
static mut USER_CATALOG_LENGTH: usize = 0;
static mut USER_SAVE_BUFFER: [u8; USER_SNAPSHOT_BYTES] = [0; USER_SNAPSHOT_BYTES];
static mut USER_SAVE_LENGTH: usize = 0;
static mut USER_SAVE_ACTIVE: bool = false;

#[derive(Clone, Copy, Eq, PartialEq)]
enum Client {
    Flow,
    Fetch,
    User,
}

struct PendingMap {
    client: Client,
    request: IpcBytes,
    response: IpcBytes,
    handle: u64,
    map_request: StorageMapRequest,
    sent: bool,
    unmap: bool,
}

fn send_response(client: Client, response: &IpcBytes) -> IpcStatus {
    match client {
        Client::Flow => common::ipc_send(FLOW_SEND_CAPABILITY, response),
        Client::Fetch => common::ipc_send(FETCH_SEND_CAPABILITY, response),
        Client::User => common::ipc_send(USER_SEND_CAPABILITY, response),
    }
}

fn map_status(status: StorageStatus) -> StorageApiStatus {
    match status {
        StorageStatus::Ok => StorageApiStatus::Ok,
        StorageStatus::Stale => StorageApiStatus::Stale,
        StorageStatus::Full => StorageApiStatus::Capacity,
        StorageStatus::Invalid | StorageStatus::Unauthorized => StorageApiStatus::Invalid,
        StorageStatus::Unsupported => StorageApiStatus::Unsupported,
        StorageStatus::Io
        | StorageStatus::OutOfBounds
        | StorageStatus::ReadOnly
        | StorageStatus::Recovery => StorageApiStatus::Io,
    }
}

fn map_error_response(request: &IpcBytes, status: StorageApiStatus) -> Option<IpcBytes> {
    let request = logos_abi::StorageApiRequest::decode(request).ok()?;
    logos_abi::StorageApiResponse::encode_versioned(
        request.version,
        status,
        request.request_id,
        request.transaction_id,
        &[],
        false,
    )
}

fn mapped_response(response: &IpcBytes, mapping: StorageMapResponse) -> Option<IpcBytes> {
    let response = logos_abi::StorageApiResponse::decode(response).ok()?;
    let mut data = [0u8; logos_abi::STORAGE_API_MAP_DESCRIPTOR_BYTES];
    data[..8].copy_from_slice(&mapping.target_page.to_le_bytes());
    data[8] = mapping.pages;
    logos_abi::StorageApiResponse::encode_versioned(
        response.version,
        response.status,
        response.request_id,
        response.transaction_id,
        &data,
        response.more,
    )
}

fn response_handle(response: &IpcBytes) -> u64 {
    logos_abi::StorageApiResponse::decode(response).map_or(0, |response| response.transaction_id)
}

fn unmap_request(
    request_id: u32,
    client: Client,
    mapping: StorageMapResponse,
) -> StorageMapRequest {
    StorageMapRequest {
        operation: 2,
        flags: 0,
        reserved: 0,
        request_id,
        generation: mapping.generation,
        client: match client {
            Client::Flow => logos_abi::ServiceId::Flow as u16,
            Client::Fetch => logos_abi::ServiceId::Fetch as u16,
            Client::User => logos_abi::ServiceId::User as u16,
        },
        pages: 0,
        source_page: 0,
        target_page: mapping.target_page,
        window_generation: mapping.window_generation,
        reserved_tail: 0,
    }
}

struct StorageTransport {
    capability: StorageCapability,
    operation: Option<StorageOperation>,
}

impl StorageTransport {
    const fn new(capability: StorageCapability) -> Self {
        Self { capability, operation: None }
    }

    fn heartbeat(&self) {
        common::heartbeat(logos_abi::ServiceId::Storage);
    }
}

impl KernelStorageIpc for StorageTransport {
    fn send(
        &mut self,
        capability: StorageCapability,
        request: StorageRequest,
        staging: &mut Block,
    ) -> IpcStatus {
        self.heartbeat();
        if capability != self.capability {
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
        capability: StorageCapability,
        response: &mut StorageResponse,
        staging: &mut Block,
    ) -> IpcStatus {
        self.heartbeat();
        if capability != self.capability {
            return IpcStatus::Unauthorized;
        }
        let status = common::ipc_receive(RESPONSE_CAPABILITY, response);
        if status == IpcStatus::Ok && self.operation == Some(StorageOperation::Read) {
            *staging = unsafe { *(logos_abi::STORAGE_DATA_BASE as *const Block) };
        }
        status
    }
}

fn new_store(
    capability: StorageCapability,
    blocks: u64,
) -> Option<IpcBlockStore<StorageTransport>> {
    IpcBlockStore::new_with_slot(
        StorageTransport::new(capability),
        capability,
        REQUEST_PROTOCOL_SLOT,
        blocks,
    )
    .ok()
}

fn discover(capability: StorageCapability) -> Option<u64> {
    let request = StorageRequest::new(
        StorageOperation::Reopen,
        1,
        capability.generation,
        REQUEST_PROTOCOL_SLOT,
        capability.service_epoch,
        0,
        0,
        0,
        0,
    )?;
    let mut transport = StorageTransport::new(capability);
    let mut staging = Block::zero();
    if transport.send(capability, request, &mut staging) != IpcStatus::Ok {
        return None;
    }
    let mut response = StorageResponse::new(1, StorageStatus::Invalid, 1, 0, 0, 0);
    for _ in 0..64 {
        match transport.receive(capability, &mut response, &mut staging) {
            IpcStatus::Ok => {
                return (response.status == StorageStatus::Ok)
                    .then_some(response.block_count)
                    .filter(|blocks| *blocks > 2);
            }
            IpcStatus::Empty => common::wait(
                common::ipc_read_event(IpcEndpointId::CoreToStorage),
                logos_abi::ServiceId::Storage,
            ),
            _ => return None,
        }
    }
    None
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
        NamespaceError::Package(
            logos_storage_service::PackageCatalogError::VersionConflict
            | logos_storage_service::PackageCatalogError::MissingDependency
            | logos_storage_service::PackageCatalogError::DependencyConflict,
        ) => PackageStatus::Invalid,
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
    filesystem: &mut DurableNamespaceV5<B>,
) -> PackageResponse {
    let Ok(capability) = common::capability_handle(PACKAGE_RECEIVE_CAPABILITY) else {
        return PackageResponse::new(request, PackageStatus::Invalid);
    };
    let bootstrap = common::bootstrap_page();
    if !bootstrap.is_valid() {
        return PackageResponse::new(request, PackageStatus::Stale);
    }
    let target = match request.validate_target(
        capability,
        bootstrap.service_epoch as u16,
        bootstrap.service_epoch,
    ) {
        Ok(service) => service,
        Err(status) => return PackageResponse::new(request, status),
    };
    match request.operation {
        PackageOperation::Lookup => {
            let result = match target.kind {
                PackageTargetKind::Service => {
                    let Some(service) =
                        logos_abi::ServiceId::from_index(target.service.saturating_sub(1) as usize)
                    else {
                        return PackageResponse::new(request, PackageStatus::Invalid);
                    };
                    filesystem.lookup_package(service)
                }
                PackageTargetKind::Program => {
                    filesystem.lookup_package_name(&target.name[..target.name_len as usize])
                }
            };
            match result {
                Ok(info) => PackageResponse::new(request, PackageStatus::Ok).with_package(
                    info.handle.generation,
                    info.bytes,
                    info.package_version,
                    info.crc32c,
                ),
                Err(error) => PackageResponse::new(request, package_status(error)),
            }
        }
        PackageOperation::Read => {
            let target = match target.kind {
                PackageTargetKind::Service => {
                    logos_abi::ServiceId::from_index(target.service.saturating_sub(1) as usize)
                        .map(logos_storage_service::PackageKey::Service)
                }
                PackageTargetKind::Program => filesystem
                    .lookup_package_name(&target.name[..target.name_len as usize])
                    .ok()
                    .map(|info| info.handle.target),
            };
            let Some(target) = target else {
                return PackageResponse::new(request, PackageStatus::Invalid);
            };
            let handle = logos_storage_service::PackageHandle {
                target,
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
    let capability = common::capability_handle(PACKAGE_RECEIVE_CAPABILITY).ok()?;
    let bootstrap = common::bootstrap_page();
    let mut request = PackageRequest::new(
        PackageOperation::Lookup,
        logos_abi::ServiceId::Storage,
        1,
        bootstrap.service_epoch as u16,
        capability,
        bootstrap.service_epoch,
        0,
        0,
        0,
    )?;
    (common::ipc_receive(PACKAGE_RECEIVE_CAPABILITY, &mut request) == IpcStatus::Ok)
        .then_some(request)
}

fn ensure_user_catalog(
    filesystem: &mut DurableNamespaceV5<IpcBlockStore<StorageTransport>>,
) -> bool {
    let catalog = unsafe { &mut *core::ptr::addr_of_mut!(USER_CATALOG) };
    let buffer = unsafe { &mut *core::ptr::addr_of_mut!(USER_CATALOG_BUFFER) };
    match UserCatalogStore::load(filesystem, buffer) {
        Ok(length) if length != 0 => {
            if catalog.load_from(filesystem, buffer).is_err()
                || catalog.save_to(filesystem, buffer).is_err()
            {
                return false;
            }
            unsafe {
                USER_CATALOG_LENGTH = catalog.encode_snapshot(buffer).ok().unwrap_or(length);
            }
            true
        }
        Err(logos_user::UserError::NotFound) => {
            if catalog.save_to(filesystem, buffer).is_err() {
                return false;
            }
            unsafe {
                USER_CATALOG_LENGTH = catalog.encode_snapshot(buffer).ok().unwrap_or(0);
            }
            true
        }
        _ => false,
    }
}

fn user_storage_message(response: UserStorageResponse) -> IpcBytes {
    let bytes = unsafe {
        core::slice::from_raw_parts(
            (&response as *const UserStorageResponse).cast::<u8>(),
            core::mem::size_of::<UserStorageResponse>(),
        )
    };
    IpcBytes::from_bytes(MessageKind::UserStorageResponse, bytes)
        .unwrap_or_else(|| IpcBytes::empty(MessageKind::UserStorageResponse))
}

fn handle_user_storage_request(
    message: &IpcBytes,
    filesystem: &mut DurableNamespaceV5<IpcBlockStore<StorageTransport>>,
) -> Option<IpcBytes> {
    if message.kind != MessageKind::UserStorageRequest {
        return None;
    }
    let bytes = message.as_bytes()?;
    if bytes.len() != core::mem::size_of::<UserStorageRequest>() {
        return None;
    }
    let request = unsafe { core::ptr::read_unaligned(bytes.as_ptr().cast()) };
    let mut response = UserStorageResponse::invalid(request);
    if !request.is_valid() {
        return Some(user_storage_message(response));
    }
    match request.operation {
        UserStorageOperation::Load => {
            let length = unsafe { USER_CATALOG_LENGTH };
            let offset = request.offset as usize;
            let data_length = length.saturating_sub(offset).min(response.data.len());
            if offset >= length || data_length == 0 {
                response.status = UserStorageStatus::Invalid;
            } else {
                response.status = UserStorageStatus::Ok;
                response.total_len = length as u32;
                response.data_len = data_length as u16;
                let source = unsafe { &*core::ptr::addr_of!(USER_CATALOG_BUFFER) };
                response.data[..data_length].copy_from_slice(&source[offset..offset + data_length]);
            }
        }
        UserStorageOperation::Save => {
            let offset = request.offset as usize;
            let total = request.total_len as usize;
            let data_length = request.data_len as usize;
            let valid = total != 0
                && total <= USER_SNAPSHOT_BYTES
                && offset.checked_add(data_length).is_some_and(|end| end <= total)
                && ((request.flags & USER_STORAGE_FLAG_BEGIN != 0) == (offset == 0));
            if !valid {
                unsafe {
                    USER_SAVE_ACTIVE = false;
                }
            } else {
                unsafe {
                    if request.flags & USER_STORAGE_FLAG_BEGIN != 0 {
                        USER_SAVE_ACTIVE = true;
                        USER_SAVE_LENGTH = total;
                    }
                    let active = USER_SAVE_ACTIVE && USER_SAVE_LENGTH == total;
                    if active && (offset == 0 || offset < USER_SAVE_LENGTH) {
                        USER_SAVE_BUFFER[offset..offset + data_length]
                            .copy_from_slice(&request.data[..data_length]);
                    } else {
                        USER_SAVE_ACTIVE = false;
                    }
                    if USER_SAVE_ACTIVE && request.flags & USER_STORAGE_FLAG_END != 0 {
                        let snapshot = &USER_SAVE_BUFFER[..USER_SAVE_LENGTH];
                        if offset + data_length != USER_SAVE_LENGTH
                            || UserCatalogStore::save(filesystem, snapshot).is_err()
                            || filesystem.flush().is_err()
                        {
                            USER_SAVE_ACTIVE = false;
                        } else {
                            let target = &mut *core::ptr::addr_of_mut!(USER_CATALOG_BUFFER);
                            target[..USER_SAVE_LENGTH].copy_from_slice(snapshot);
                            USER_CATALOG_LENGTH = USER_SAVE_LENGTH;
                            USER_SAVE_ACTIVE = false;
                            response.status = UserStorageStatus::Ok;
                        }
                    } else if USER_SAVE_ACTIVE {
                        response.status = UserStorageStatus::Ok;
                    }
                }
            }
        }
    }
    Some(user_storage_message(response))
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
        common::heartbeat(logos_abi::ServiceId::Storage);
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
            let mut user_request = IpcBytes::empty(MessageKind::UserStorageRequest);
            if common::ipc_receive(USER_RECEIVE_CAPABILITY, &mut user_request) == IpcStatus::Ok {
                let mut response = UserStorageResponse {
                    operation: UserStorageOperation::Load,
                    status: UserStorageStatus::Io,
                    reserved: 0,
                    request_id: 1,
                    offset: 0,
                    total_len: 0,
                    data_len: 0,
                    data: [0; logos_abi::USER_STORAGE_CHUNK_BYTES],
                };
                if let Some(bytes) = user_request.as_bytes() {
                    if bytes.len() == core::mem::size_of::<UserStorageRequest>() {
                        let request = unsafe { core::ptr::read_unaligned(bytes.as_ptr().cast()) };
                        response = UserStorageResponse::invalid(request);
                    }
                }
                pending_response = Some((Client::User, user_storage_message(response)));
                progressed = true;
            } else {
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
        }
        if !progressed {
            common::wait(
                common::ipc_read_event(IpcEndpointId::FlowToStorage)
                    | common::ipc_write_event(IpcEndpointId::StorageToFlow)
                    | common::ipc_read_event(IpcEndpointId::FetchToStorage)
                    | common::ipc_write_event(IpcEndpointId::StorageToFetch)
                    | common::ipc_read_event(IpcEndpointId::UserToStorage)
                    | common::ipc_write_event(IpcEndpointId::StorageToUser)
                    | common::ipc_read_event(IpcEndpointId::CoreToStoragePackage)
                    | common::ipc_write_event(IpcEndpointId::StoragePackageToCore),
                logos_abi::ServiceId::Storage,
            );
        }
    }
}

fn run_filesystem(capability: StorageCapability, blocks: u64) -> ! {
    let Some(store) = new_store(capability, blocks) else {
        serve_storage_error(StorageApiStatus::Io);
    };
    let mut filesystem = match DurableNamespaceV5::open_v5(store) {
        Ok(filesystem) => filesystem,
        Err(NamespaceError::Format(logos_storage::FormatError::Unformatted)) => {
            let Some(store) = new_store(capability, blocks) else {
                serve_storage_error(StorageApiStatus::Io);
            };
            DurableNamespaceV5::format_v5(store)
                .unwrap_or_else(|error| serve_storage_error(storage_error_status(error)))
        }
        Err(NamespaceError::Format(logos_storage::FormatError::ProvisionedBlank)) => {
            let Some(store) = new_store(capability, blocks) else {
                serve_storage_error(StorageApiStatus::Io);
            };
            DurableNamespaceV5::format_v5_provisioned(store)
                .unwrap_or_else(|error| serve_storage_error(storage_error_status(error)))
        }
        Err(error) => serve_storage_error(storage_error_status(error)),
    };
    filesystem.flush().unwrap_or_else(|error| serve_storage_error(storage_error_status(error)));
    if !ensure_user_catalog(&mut filesystem) {
        serve_storage_error(StorageApiStatus::Io);
    }

    let mut api = StorageApiV5::new(filesystem);
    let mut pending_response: Option<(Client, IpcBytes)> = None;
    let mut pending_package: Option<PackageResponse> = None;
    let mut pending_map: Option<PendingMap> = None;
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
        if let Some(map) = pending_map.as_mut() {
            if !map.sent {
                match common::ipc_send(MAP_REQUEST_CAPABILITY, &map.map_request) {
                    IpcStatus::Ok => map.sent = true,
                    IpcStatus::Full => {}
                    _ => {
                        if !map.unmap {
                            api.cancel_map(map.handle);
                        }
                        let response = map_error_response(&map.request, StorageApiStatus::Io);
                        if let Some(response) = response {
                            pending_response = Some((map.client, response));
                        }
                        pending_map = None;
                    }
                }
                progressed = true;
            } else {
                let mut response = StorageMapResponse {
                    request_id: 0,
                    status: StorageStatus::Io,
                    reserved: [0; 3],
                    generation: 0,
                    target_page: 0,
                    pages: 0,
                    reserved_tail: [0; 7],
                    window_generation: 0,
                    reserved_end: [0; 4],
                };
                match common::ipc_receive(MAP_RESPONSE_CAPABILITY, &mut response) {
                    IpcStatus::Ok if response.request_id == map.map_request.request_id => {
                        if map.unmap {
                            if response.status == StorageStatus::Ok {
                                pending_response =
                                    api.handle(&map.request).map(|value| (map.client, value));
                            } else {
                                pending_response =
                                    map_error_response(&map.request, map_status(response.status))
                                        .map(|value| (map.client, value));
                            }
                        } else if response.status == StorageStatus::Ok
                            && api.complete_map(map.handle, response)
                        {
                            pending_response = mapped_response(&map.response, response)
                                .map(|value| (map.client, value));
                        } else {
                            api.cancel_map(map.handle);
                            pending_response =
                                map_error_response(&map.request, map_status(response.status))
                                    .map(|value| (map.client, value));
                        }
                        pending_map = None;
                    }
                    IpcStatus::Empty => {}
                    _ => {
                        if !map.unmap {
                            api.cancel_map(map.handle);
                        }
                        pending_response = map_error_response(&map.request, StorageApiStatus::Io)
                            .map(|value| (map.client, value));
                        pending_map = None;
                    }
                }
                progressed = true;
            }
        }
        if pending_response.is_none() && pending_package.is_none() {
            if let Some(request) = receive_package_request() {
                pending_package = Some(handle_package_request(request, api.namespace_mut()));
                progressed = true;
            }
        }
        if pending_response.is_none() && pending_package.is_none() && pending_map.is_none() {
            let mut user_request = IpcBytes::empty(MessageKind::UserStorageRequest);
            if common::ipc_receive(USER_RECEIVE_CAPABILITY, &mut user_request) == IpcStatus::Ok {
                pending_response = handle_user_storage_request(&user_request, api.namespace_mut())
                    .map(|response| (Client::User, response));
                progressed = true;
            } else {
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
                    let operation = logos_abi::StorageApiRequest::decode(&request).ok();
                    if let Some(operation) = operation {
                        if operation.operation == StorageApiOperation::UnmapRead {
                            if let Some(mapping) = api.map_release(operation.transaction_id) {
                                pending_map = Some(PendingMap {
                                    client,
                                    request,
                                    handle: operation.transaction_id,
                                    response: IpcBytes::empty(MessageKind::StorageResponse),
                                    map_request: unmap_request(
                                        operation.request_id,
                                        client,
                                        mapping,
                                    ),
                                    sent: false,
                                    unmap: true,
                                });
                            } else {
                                pending_response =
                                    api.handle(&request).map(|value| (client, value));
                            }
                        } else if let Some(response) = api.handle(&request) {
                            if operation.operation == StorageApiOperation::MapRead {
                                if let Some(map_request) = api.map_request(
                                    &response,
                                    capability.service_epoch,
                                    match client {
                                        Client::Flow => logos_abi::ServiceId::Flow,
                                        Client::Fetch => logos_abi::ServiceId::Fetch,
                                        Client::User => logos_abi::ServiceId::User,
                                    },
                                ) {
                                    pending_map = Some(PendingMap {
                                        client,
                                        request,
                                        handle: response_handle(&response),
                                        response,
                                        map_request,
                                        sent: false,
                                        unmap: false,
                                    });
                                } else {
                                    pending_response = Some((client, response));
                                }
                            } else {
                                pending_response = Some((client, response));
                            }
                        }
                    }
                }
                progressed = true;
            }
        }
        if !progressed {
            common::wait(
                common::ipc_read_event(IpcEndpointId::FlowToStorage)
                    | common::ipc_write_event(IpcEndpointId::StorageToFlow)
                    | common::ipc_read_event(IpcEndpointId::FetchToStorage)
                    | common::ipc_write_event(IpcEndpointId::StorageToFetch)
                    | common::ipc_read_event(IpcEndpointId::CoreToStorageMap)
                    | common::ipc_write_event(IpcEndpointId::StorageMapToCore)
                    | common::ipc_read_event(IpcEndpointId::UserToStorage)
                    | common::ipc_write_event(IpcEndpointId::StorageToUser)
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
    common::heartbeat(logos_abi::ServiceId::Storage);
    common::init_service_allocator();
    let Ok(capability_handle) = common::capability_handle(REQUEST_CAPABILITY) else {
        serve_storage_error(StorageApiStatus::Io)
    };
    let Ok(generation) = u16::try_from(capability_handle.generation()) else {
        serve_storage_error(StorageApiStatus::Io)
    };
    let Some(capability) = StorageCapability::new(
        capability_handle,
        generation,
        common::bootstrap_page().service_epoch,
    ) else {
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
