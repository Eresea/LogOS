//! Post-UEFI service image and address-space ownership.

use alloc::vec::Vec;
use core::{
    mem::MaybeUninit,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};

use logos_abi::{ServiceHandle, ServiceId};

use crate::memory::{ExclusionKind, MemoryExclusion, OwnerId};
use crate::{
    frame_pool::{FrameAddress, FramePool},
    loader::{LoadError, LoadedImage},
    page_table::{IdentityPageTableMemory, PageTableBuilder, PageTableError, PageTableMemory},
    process::{
        AddressSpaceRoot, MappingFlags, ProcessError, ProcessHandle, UserLaunch, VirtualMapping,
    },
    runtime_ipc::RuntimeIpcRegistry,
    runtime_services::ServiceImageSource,
    service_images::SERVICE_IMAGES,
    service_ipc::IpcError,
    service_loader::ServiceImageBundle,
    service_manager::{ManagerAction, ManagerDecision, ProgramManager},
    service_startup::ServiceStartup,
    supervisor::LiveSupervisor,
};

const SERVICE_COUNT: usize = SERVICE_IMAGES.len();
const MAX_PROGRAMS: usize = crate::service_manager::MAX_PROGRAM_SLOTS;
const MAX_ACTIVE_PAGE_TABLE_FRAMES: usize = 4096;
const CORE_SERVICE_HANDLE_INDEX: u32 = u32::MAX;
// Package activation remains an internal hook until package-manager policy exists.
#[allow(dead_code)]
const PACKAGE_EXCHANGE_POLLS: usize = 1024;

struct ServiceHeapState {
    frames: Vec<FrameAddress>,
    quota_pages: usize,
}

impl ServiceHeapState {
    const fn empty() -> Self {
        Self { frames: Vec::new(), quota_pages: 0 }
    }
}

fn dynamic_endpoint_peer(
    endpoint: logos_abi::IpcEndpointId,
    producer: bool,
    service_handles: &[logos_abi::ServiceHandle; SERVICE_COUNT],
    generation: u32,
) -> Result<logos_abi::ServiceHandle, ServiceRuntimeError> {
    let core = || {
        logos_abi::ServiceHandle::new(CORE_SERVICE_HANDLE_INDEX, generation)
            .ok_or(ServiceRuntimeError::StaleGeneration)
    };
    match endpoint {
        logos_abi::IpcEndpointId::StorageToCore
        | logos_abi::IpcEndpointId::NetworkToCore
        | logos_abi::IpcEndpointId::DeviceToCore
        | logos_abi::IpcEndpointId::StoragePackageToCore
        | logos_abi::IpcEndpointId::StorageMapToCore => {
            if producer {
                service_handles
                    .get(endpoint.producer().index())
                    .copied()
                    .filter(|handle| handle.is_valid())
                    .ok_or(ServiceRuntimeError::StaleGeneration)
            } else {
                core()
            }
        }
        logos_abi::IpcEndpointId::CoreToStorage
        | logos_abi::IpcEndpointId::CoreToNetwork
        | logos_abi::IpcEndpointId::CoreToDevice
        | logos_abi::IpcEndpointId::CoreToStoragePackage
        | logos_abi::IpcEndpointId::CoreToStorageMap => {
            if producer {
                core()
            } else {
                service_handles
                    .get(endpoint.consumer().index())
                    .copied()
                    .filter(|handle| handle.is_valid())
                    .ok_or(ServiceRuntimeError::StaleGeneration)
            }
        }
        logos_abi::IpcEndpointId::FetchToStorage if !producer => service_handles
            [ServiceId::Storage.index()]
        .is_valid()
        .then_some(service_handles[ServiceId::Storage.index()])
        .ok_or(ServiceRuntimeError::StaleGeneration),
        logos_abi::IpcEndpointId::FetchToNetwork if !producer => service_handles
            [ServiceId::Network.index()]
        .is_valid()
        .then_some(service_handles[ServiceId::Network.index()])
        .ok_or(ServiceRuntimeError::StaleGeneration),
        _ => service_handles
            .get((if producer { endpoint.producer() } else { endpoint.consumer() }).index())
            .copied()
            .filter(|handle| handle.is_valid())
            .ok_or(ServiceRuntimeError::StaleGeneration),
    }
}

fn dynamic_core_handle(generation: u32) -> Result<logos_abi::ServiceHandle, ServiceRuntimeError> {
    logos_abi::ServiceHandle::new(CORE_SERVICE_HANDLE_INDEX, generation)
        .ok_or(ServiceRuntimeError::StaleGeneration)
}

fn builtin_service_for_handle(
    handles: &[logos_abi::ServiceHandle; SERVICE_COUNT],
    handle: logos_abi::ServiceHandle,
) -> Option<ServiceId> {
    SERVICE_IMAGES
        .iter()
        .find_map(|spec| (handles[spec.service().index()] == handle).then_some(spec.service()))
}

fn event_status(error: crate::runtime_events::EventError) -> logos_abi::EventStatus {
    match error {
        crate::runtime_events::EventError::Stale => logos_abi::EventStatus::Stale,
        crate::runtime_events::EventError::Unauthorized => logos_abi::EventStatus::Unauthorized,
        crate::runtime_events::EventError::Capacity => logos_abi::EventStatus::Capacity,
        crate::runtime_events::EventError::Duplicate => logos_abi::EventStatus::Duplicate,
        crate::runtime_events::EventError::NotMember => logos_abi::EventStatus::NotMember,
        crate::runtime_events::EventError::InvalidDeadline => {
            logos_abi::EventStatus::InvalidDeadline
        }
        crate::runtime_events::EventError::Busy => logos_abi::EventStatus::Busy,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServiceRuntimeError {
    Resources,
    Image,
    Load(LoadError),
    Populate(LoadError),
    PageTableRoot(PageTableError),
    PageTableMap(PageTableError),
    Process(ProcessError),
    Startup(crate::service_startup::StartupError),
    Ipc(IpcError),
    IpcPrivateMapping(PageTableError),
    IpcPrivateProcess(ProcessError),
    Framebuffer(PageTableError),
    FramebufferProcess(ProcessError),
    FramebufferConfig(PageTableError),
    FramebufferConfigProcess(ProcessError),
    Keyboard(PageTableError),
    KeyboardProcess(ProcessError),
    TaskCapacity,
    TaskAddressSpace,
    TaskLaunch,
    TaskStop,
    RestartLimit,
    StaleGeneration,
}

pub struct ServiceRuntime {
    frame_pool: FramePool,
    images: [LoadedImage; SERVICE_COUNT],
    tables: [MaybeUninit<PageTableBuilder>; SERVICE_COUNT],
    table_ready: [bool; SERVICE_COUNT],
    processes: crate::process::ProcessTable,
    launches: [Option<(ProcessHandle, UserLaunch)>; SERVICE_COUNT],
    startup: ServiceStartup,
    dynamic_ipc: Option<RuntimeIpcRegistry>,
    dynamic_services: Option<crate::runtime_services::RuntimeServiceRegistry>,
    service_handles: [logos_abi::ServiceHandle; SERVICE_COUNT],
    dynamic_events: Option<crate::runtime_events::RuntimeEventRegistry>,
    ipc_staging_frames: [Option<FrameAddress>; SERVICE_COUNT],
    service_bootstrap_frames: [u64; SERVICE_COUNT],
    bootstrap_control: [logos_abi::CapabilityHandle; SERVICE_COUNT],
    bootstrap_directory: [logos_abi::CapabilityHandle; SERVICE_COUNT],
    bootstrap_heap: [logos_abi::CapabilityHandle; SERVICE_COUNT],
    keyboard_event: logos_abi::EventHandle,
    service_heaps: [ServiceHeapState; SERVICE_COUNT],
    user_kdf_workspace: [u64; logos_abi::USER_KDF_WORKSPACE_PAGES],
    storage_data_frames: [Option<FrameAddress>; logos_abi::STORAGE_DATA_PAGES],
    network_config: logos_abi::NetworkConfig,
    network_config_frame: Option<FrameAddress>,
    network_packet_frames: [Option<FrameAddress>; logos_abi::NETWORK_PACKET_PAGE_COUNT],
    framebuffer_config_frame: Option<FrameAddress>,
    keyboard_frame: Option<FrameAddress>,
    tasks: [Option<crate::TaskHandle>; SERVICE_COUNT],
    heartbeat_ticks: [AtomicU64; SERVICE_COUNT],
    supervisor: LiveSupervisor,
    manager: ProgramManager,
    pending_restart: Option<Vec<ServiceHandle>>,
    storage_map_windows: [[Option<crate::storage_ipc::StorageMapWindow>;
        crate::storage_ipc::STORAGE_MAP_WINDOWS_PER_CLIENT];
        crate::storage_ipc::STORAGE_MAP_CLIENTS],
    ipc_generation: u16,
    service_epoch: u64,
    storage_response: Option<logos_abi::StorageResponse>,
    device_response: Option<logos_abi::DeviceResponse>,
    storage_map_response: Option<logos_abi::StorageMapResponse>,
    package_request: Option<logos_abi::PackageRequest>,
    package_capability: logos_abi::CapabilityHandle,
    package_response: Option<logos_abi::PackageResponse>,
    #[allow(dead_code)]
    package_next_request: u32,
    prepared_packages: [Option<PreparedServiceImage>; SERVICE_COUNT],
    active_packages: [Option<ActivePackageImage>; SERVICE_COUNT],
    programs: [ProgramRuntime; MAX_PROGRAMS],
    pending_program_start: Option<(usize, logos_abi::ServiceManagerRecord)>,
    network_packet_response: Option<logos_abi::NetworkPacketDescriptor>,
    network_packet_sequence: u32,
    suppressed_heartbeats: [AtomicBool; SERVICE_COUNT],
    frame_pool_ready: bool,
    #[cfg(feature = "storage-proof")]
    storage_proof: crate::storage_proof::StorageProofObserver,
}

struct PreparedServiceImage {
    service: ServiceId,
    plan: crate::process::ElfLoadPlan,
    image: LoadedImage,
}

#[derive(Clone, Copy)]
struct ActivePackageImage {
    service: ServiceId,
    plan: crate::process::ElfLoadPlan,
}

struct ProgramRuntime {
    manager_slot: u8,
    generation: u32,
    name: [u8; logos_abi::MAX_PACKAGE_NAME_BYTES],
    name_len: u8,
    process: Option<ProcessHandle>,
    task: Option<crate::TaskHandle>,
    image: LoadedImage,
    table: MaybeUninit<PageTableBuilder>,
    table_ready: bool,
}

impl ProgramRuntime {
    const fn empty() -> Self {
        Self {
            manager_slot: u8::MAX,
            generation: 0,
            name: [0; logos_abi::MAX_PACKAGE_NAME_BYTES],
            name_len: 0,
            process: None,
            task: None,
            image: LoadedImage::empty(),
            table: MaybeUninit::uninit(),
            table_ready: false,
        }
    }
}

#[allow(dead_code)]
struct RuntimePackageReader<'a, 'b> {
    runtime: &'a mut ServiceRuntime,
    runtime_guard: &'b mut crate::arch::ServiceRuntimeGuard,
    target: logos_abi::PackageTarget,
    generation: u32,
    base: usize,
    bytes: usize,
    cached_block: Option<usize>,
    cached_len: usize,
    cache: [u8; logos_abi::PACKAGE_TRANSFER_BYTES],
}

#[allow(dead_code)]
impl<'a, 'b> RuntimePackageReader<'a, 'b> {
    fn new(
        runtime: &'a mut ServiceRuntime,
        runtime_guard: &'b mut crate::arch::ServiceRuntimeGuard,
        target: logos_abi::PackageTarget,
        generation: u32,
        base: usize,
        bytes: usize,
    ) -> Self {
        Self {
            runtime,
            runtime_guard,
            target,
            generation,
            base,
            bytes,
            cached_block: None,
            cached_len: 0,
            cache: [0; logos_abi::PACKAGE_TRANSFER_BYTES],
        }
    }

    fn read_cached(&mut self, offset: usize, output: &mut [u8]) -> Result<usize, ProcessError> {
        let end = offset.checked_add(output.len()).ok_or(ProcessError::ReadFailure)?;
        if end > self.bytes {
            return Err(ProcessError::ReadFailure);
        }
        let mut copied = 0;
        while copied < output.len() {
            let absolute = self
                .base
                .checked_add(offset)
                .and_then(|value| value.checked_add(copied))
                .ok_or(ProcessError::ReadFailure)?;
            let block =
                absolute / logos_abi::PACKAGE_TRANSFER_BYTES * logos_abi::PACKAGE_TRANSFER_BYTES;
            if self.cached_block != Some(block) {
                let package_end =
                    self.base.checked_add(self.bytes).ok_or(ProcessError::ReadFailure)?;
                let block_end = core::cmp::min(
                    block
                        .checked_add(logos_abi::PACKAGE_TRANSFER_BYTES)
                        .ok_or(ProcessError::ReadFailure)?,
                    package_end,
                );
                if block >= block_end {
                    return Err(ProcessError::ReadFailure);
                }
                let amount = block_end - block;
                let request = self.runtime.next_package_request_target(
                    logos_abi::PackageOperation::Read,
                    self.target,
                    self.generation,
                    block,
                    amount,
                )?;
                let response = self.runtime.package_exchange(
                    request,
                    &mut self.cache[..amount],
                    self.runtime_guard,
                )?;
                if response.bytes as usize != amount {
                    return Err(ProcessError::ReadFailure);
                }
                self.cached_block = Some(block);
                self.cached_len = amount;
            }
            let cache_offset = absolute.checked_sub(block).ok_or(ProcessError::ReadFailure)?;
            if cache_offset >= self.cached_len {
                return Err(ProcessError::ReadFailure);
            }
            let amount = core::cmp::min(output.len() - copied, self.cached_len - cache_offset);
            output[copied..copied + amount]
                .copy_from_slice(&self.cache[cache_offset..cache_offset + amount]);
            copied += amount;
        }
        Ok(copied)
    }
}

impl crate::process::ImageReader for RuntimePackageReader<'_, '_> {
    fn len(&self) -> usize {
        self.bytes
    }

    fn read(&mut self, offset: usize, output: &mut [u8]) -> Result<usize, ProcessError> {
        self.read_cached(offset, output)
    }
}

impl logos_package::PackageReader for RuntimePackageReader<'_, '_> {
    fn len(&self) -> usize {
        self.bytes
    }

    fn read(
        &mut self,
        offset: usize,
        output: &mut [u8],
    ) -> Result<usize, logos_package::PackageError> {
        self.read_cached(offset, output).map_err(|_| logos_package::PackageError::Reader)
    }
}

struct ServiceRestartGate;

impl ServiceRestartGate {
    fn acquire() -> Self {
        crate::arch::begin_service_restart();
        Self
    }
}

impl Drop for ServiceRestartGate {
    fn drop(&mut self) {
        crate::arch::end_service_restart();
    }
}

impl ServiceRuntime {
    pub const fn new() -> Self {
        Self {
            frame_pool: FramePool::empty(),
            images: [const { LoadedImage::empty() }; SERVICE_COUNT],
            tables: [const { MaybeUninit::uninit() }; SERVICE_COUNT],
            table_ready: [false; SERVICE_COUNT],
            processes: crate::process::ProcessTable::new(),
            launches: [None; SERVICE_COUNT],
            startup: ServiceStartup::new(),
            dynamic_ipc: None,
            dynamic_services: None,
            service_handles: [logos_abi::ServiceHandle::EMPTY; SERVICE_COUNT],
            dynamic_events: None,
            ipc_staging_frames: [None; SERVICE_COUNT],
            service_bootstrap_frames: [0; SERVICE_COUNT],
            bootstrap_control: [logos_abi::CapabilityHandle::EMPTY; SERVICE_COUNT],
            bootstrap_directory: [logos_abi::CapabilityHandle::EMPTY; SERVICE_COUNT],
            bootstrap_heap: [logos_abi::CapabilityHandle::EMPTY; SERVICE_COUNT],
            keyboard_event: logos_abi::EventHandle::EMPTY,
            service_heaps: [const { ServiceHeapState::empty() }; SERVICE_COUNT],
            user_kdf_workspace: [0; logos_abi::USER_KDF_WORKSPACE_PAGES],
            storage_data_frames: [None; logos_abi::STORAGE_DATA_PAGES],
            network_config: logos_abi::NetworkConfig::disabled(),
            network_config_frame: None,
            network_packet_frames: [None; logos_abi::NETWORK_PACKET_PAGE_COUNT],
            framebuffer_config_frame: None,
            keyboard_frame: None,
            tasks: [None; SERVICE_COUNT],
            heartbeat_ticks: [const { AtomicU64::new(0) }; SERVICE_COUNT],
            supervisor: LiveSupervisor::new(),
            manager: ProgramManager::new(),
            pending_restart: None,
            storage_map_windows: [[None; crate::storage_ipc::STORAGE_MAP_WINDOWS_PER_CLIENT];
                crate::storage_ipc::STORAGE_MAP_CLIENTS],
            ipc_generation: 1,
            service_epoch: 1,
            storage_response: None,
            device_response: None,
            storage_map_response: None,
            package_request: None,
            package_capability: logos_abi::CapabilityHandle::EMPTY,
            package_response: None,
            package_next_request: 1,
            prepared_packages: [const { None }; SERVICE_COUNT],
            active_packages: [None; SERVICE_COUNT],
            programs: [const { ProgramRuntime::empty() }; MAX_PROGRAMS],
            pending_program_start: None,
            network_packet_response: None,
            network_packet_sequence: 1,
            suppressed_heartbeats: [const { AtomicBool::new(false) }; SERVICE_COUNT],
            frame_pool_ready: false,
            #[cfg(feature = "storage-proof")]
            storage_proof: crate::storage_proof::StorageProofObserver::new(),
        }
    }

    pub fn start(&mut self, bundle: &ServiceImageBundle) -> Result<(), ServiceRuntimeError> {
        let result = self.start_inner(bundle);
        if let Err(error) = result {
            let reclaim_result = self.reclaim_resources();
            self.reclaim_prepared_packages();
            reclaim_result?;
            return Err(error);
        }
        Ok(())
    }

    pub fn configure_network(&mut self, config: logos_abi::NetworkConfig) {
        self.network_config = config;
    }

    fn runtime_service_handle(
        &self,
        service: ServiceId,
    ) -> Result<logos_abi::ServiceHandle, ServiceRuntimeError> {
        let handle =
            self.service_handles.get(service.index()).copied().filter(|handle| handle.is_valid());
        handle.ok_or(ServiceRuntimeError::StaleGeneration)
    }

    fn initialize_dynamic_services(&mut self) -> Result<(), ServiceRuntimeError> {
        let mut registry = crate::runtime_services::RuntimeServiceRegistry::new_with_generation(
            (self.service_epoch as u32).max(1),
        );
        let mut handles = [logos_abi::ServiceHandle::EMPTY; SERVICE_COUNT];
        for spec in SERVICE_IMAGES {
            handles[spec.service().index()] = registry
                .register_with_quota(
                    spec.name(),
                    b"builtin",
                    &[],
                    self.service_heaps[spec.service().index()].quota_pages,
                )
                .map_err(|_| ServiceRuntimeError::Resources)?;
            registry
                .set_manager_rights(
                    handles[spec.service().index()],
                    if spec.service() == ServiceId::Flow {
                        logos_abi::ManagerRights::ALL
                    } else {
                        logos_abi::ManagerRights::NONE
                    },
                )
                .map_err(|_| ServiceRuntimeError::Resources)?;
        }
        for spec in SERVICE_IMAGES {
            let mut dependencies = Vec::new();
            dependencies.try_reserve(SERVICE_COUNT).map_err(|_| ServiceRuntimeError::Resources)?;
            for dependency in SERVICE_IMAGES {
                if crate::service_images::service_dependencies(spec.service())
                    & (1u16 << dependency.service().index())
                    != 0
                {
                    dependencies.push(handles[dependency.service().index()]);
                }
            }
            registry
                .set_dependencies(handles[spec.service().index()], &dependencies)
                .map_err(|_| ServiceRuntimeError::Resources)?;
        }
        if !self.network_config.is_enabled() {
            registry
                .disable(handles[ServiceId::Network.index()])
                .map_err(|_| ServiceRuntimeError::Resources)?;
            registry
                .disable(handles[ServiceId::Fetch.index()])
                .map_err(|_| ServiceRuntimeError::Resources)?;
        }
        for service in crate::service_images::SERVICE_START_ORDER {
            if !self.network_config.is_enabled()
                && (service == ServiceId::Network || service == ServiceId::Fetch)
            {
                continue;
            }
            registry.start(handles[service.index()]).map_err(|_| ServiceRuntimeError::Resources)?;
        }
        self.service_handles = handles;
        self.dynamic_services = Some(registry);
        Ok(())
    }

    fn initialize_dynamic_ipc(&mut self) -> Result<(), ServiceRuntimeError> {
        let generation = (self.service_epoch as u32).max(1);
        let mut registry = RuntimeIpcRegistry::new_with_generation_and_budget(
            generation,
            self.frame_pool.available(),
        );
        let mut events =
            crate::runtime_events::RuntimeEventRegistry::new_with_generation(generation);
        let mut package_capability = logos_abi::CapabilityHandle::EMPTY;
        for raw in 0..logos_abi::IPC_ENDPOINT_COUNT {
            let endpoint_id = logos_abi::IpcEndpointId::from_index(raw)
                .ok_or(ServiceRuntimeError::Ipc(IpcError::InvalidIdentity))?;
            let producer =
                dynamic_endpoint_peer(endpoint_id, true, &self.service_handles, generation)?;
            let consumer =
                dynamic_endpoint_peer(endpoint_id, false, &self.service_handles, generation)?;
            let message_bytes = logos_abi::ipc_message_size(raw)
                .ok_or(ServiceRuntimeError::Ipc(IpcError::InvalidIdentity))?;
            let contract_id = logos_abi::ipc_contract_id(raw)
                .ok_or(ServiceRuntimeError::Ipc(IpcError::InvalidIdentity))?;
            let queue_capacity = bootstrap_queue_capacity(raw);
            let endpoint = registry
                .create_endpoint(
                    producer,
                    consumer,
                    contract_id,
                    message_bytes,
                    queue_capacity,
                    self.service_epoch,
                    &mut events,
                )
                .map_err(|_| ServiceRuntimeError::Ipc(IpcError::Capacity))?;
            if endpoint_id == logos_abi::IpcEndpointId::CoreToNetwork {
                let (read_event, _) = registry
                    .endpoint_events(endpoint)
                    .map_err(|_| ServiceRuntimeError::Ipc(IpcError::InvalidIdentity))?;
                crate::runtime_events::bind_hardware_event(
                    crate::runtime_events::HardwareEventSource::Network,
                    read_event,
                );
            }

            let core = dynamic_core_handle(generation)?;
            if producer == core || consumer == core {
                let rights = if producer == core {
                    logos_abi::IpcRights::Send
                } else {
                    logos_abi::IpcRights::Receive
                };
                registry
                    .grant(core, endpoint, rights)
                    .map_err(|_| ServiceRuntimeError::Ipc(IpcError::Capacity))?;
            }

            for spec in SERVICE_IMAGES {
                let service = spec.service();
                let owner = self.runtime_service_handle(service)?;
                for rights in [logos_abi::IpcRights::Send, logos_abi::IpcRights::Receive] {
                    let owns_endpoint = match rights {
                        logos_abi::IpcRights::Send => producer == owner,
                        logos_abi::IpcRights::Receive => consumer == owner,
                    };
                    if !owns_endpoint {
                        continue;
                    }
                    let grant = registry
                        .grant(owner, endpoint, rights)
                        .map_err(|_| ServiceRuntimeError::Ipc(IpcError::Capacity))?;
                    if service == ServiceId::Storage
                        && endpoint_id == logos_abi::IpcEndpointId::CoreToStoragePackage
                        && rights == logos_abi::IpcRights::Receive
                    {
                        package_capability = grant;
                    }
                }
            }
        }
        let input = self.runtime_service_handle(ServiceId::Input)?;
        let keyboard_event =
            events.create_event(input).map_err(|_| ServiceRuntimeError::Ipc(IpcError::Capacity))?;
        crate::runtime_events::bind_hardware_event(
            crate::runtime_events::HardwareEventSource::Keyboard,
            keyboard_event,
        );
        self.keyboard_event = keyboard_event;
        for spec in SERVICE_IMAGES {
            let service = spec.service();
            let owner = self.runtime_service_handle(service)?;
            let (ipc_endpoints, capabilities) = registry.ownership_counts(owner);
            let events_owned = events.ownership_count(owner) + events.event_set_count(owner);
            if let Some(services) = self.dynamic_services.as_mut() {
                services
                    .set_runtime_counts(owner, ipc_endpoints, capabilities, events_owned)
                    .map_err(|_| ServiceRuntimeError::Resources)?;
            }
        }
        self.dynamic_ipc = Some(registry);
        self.dynamic_events = Some(events);
        self.package_capability = package_capability;
        #[cfg(feature = "qemu-proof")]
        crate::proof::dynamic_ipc_ready();
        Ok(())
    }

    fn start_inner(&mut self, bundle: &ServiceImageBundle) -> Result<(), ServiceRuntimeError> {
        let resources = crate::arch::boot_resources().ok_or(ServiceRuntimeError::Resources)?;
        if !self.frame_pool_ready {
            let metadata_reservation =
                resources.frame_metadata().ok_or(ServiceRuntimeError::Resources)?;
            let metadata = crate::memory::FrameMetadataRegion::new(
                metadata_reservation.base(),
                metadata_reservation
                    .pages()
                    .checked_mul(crate::boot_resources::PAGE_SIZE)
                    .ok_or(ServiceRuntimeError::Resources)?,
            )
            .ok_or(ServiceRuntimeError::Resources)?;
            let metadata_exclusion = MemoryExclusion::new(
                metadata_reservation.base(),
                metadata_reservation.pages(),
                ExclusionKind::Reserved,
            )
            .ok_or(ServiceRuntimeError::Resources)?;
            if let Some(framebuffer) = resources.framebuffer() {
                let pages = framebuffer
                    .bytes()
                    .checked_add(crate::boot_resources::PAGE_SIZE - 1)
                    .ok_or(ServiceRuntimeError::Resources)?
                    / crate::boot_resources::PAGE_SIZE;
                let exclusion =
                    MemoryExclusion::new(framebuffer.base(), pages, ExclusionKind::Framebuffer)
                        .ok_or(ServiceRuntimeError::Resources)?;
                self.frame_pool
                    .initialize_with_metadata(
                        resources.memory_map(),
                        &[metadata_exclusion, exclusion],
                        metadata,
                    )
                    .map_err(|_| ServiceRuntimeError::Resources)?;
            } else {
                self.frame_pool
                    .initialize_with_metadata(
                        resources.memory_map(),
                        &[metadata_exclusion],
                        metadata,
                    )
                    .map_err(|_| ServiceRuntimeError::Resources)?;
            }
            if !reserve_active_page_tables(&mut self.frame_pool, crate::arch::current_cr3()) {
                return Err(ServiceRuntimeError::Resources);
            }
            self.frame_pool.reserve(FrameAddress::from_raw(0x8000));
            crate::arch::reserve_kernel_frames(&mut self.frame_pool);
            self.frame_pool_ready = true;
        }

        if !crate::memory::kernel_global_allocator_bound() {
            crate::memory::bind_kernel_global_allocator(self.frame_pool.allocator())
                .map_err(|_| ServiceRuntimeError::Resources)?;
        }

        for (index, spec) in SERVICE_IMAGES.iter().enumerate() {
            let service = spec.service();
            let stack_pages = match service {
                ServiceId::Storage => crate::process::STORAGE_STACK_PAGES,
                ServiceId::Network => crate::process::NETWORK_STACK_PAGES,
                ServiceId::Flow => crate::process::FLOW_STACK_PAGES,
                _ => crate::process::USER_STACK_PAGES,
            };
            let uses_prepared = self.prepared_packages[index]
                .as_ref()
                .is_some_and(|prepared| prepared.service == service);
            let mut memory = IdentityPageTableMemory;
            let (plan, mut loaded) = if uses_prepared {
                let prepared =
                    self.prepared_packages[index].take().ok_or(ServiceRuntimeError::Image)?;
                (prepared.plan, prepared.image)
            } else {
                let image = unsafe { bundle.image(service) }.ok_or(ServiceRuntimeError::Image)?;
                let plan = spec.validate_image(image).map_err(|_| ServiceRuntimeError::Image)?;
                let mut loaded =
                    LoadedImage::load_with_stack_pages(plan, &mut self.frame_pool, stack_pages)
                        .map_err(ServiceRuntimeError::Load)?;
                if let Err(error) = loaded.populate(plan, image, &mut memory) {
                    loaded.reclaim(&mut self.frame_pool);
                    return Err(ServiceRuntimeError::Populate(error));
                }
                (plan, loaded)
            };
            let mut tables = match if uses_prepared {
                PageTableBuilder::new_for_owner(
                    &mut self.frame_pool,
                    &mut memory,
                    OwnerId::service(service),
                )
            } else {
                PageTableBuilder::new(&mut self.frame_pool, &mut memory)
            } {
                Ok(tables) => tables,
                Err(error) => {
                    loaded.reclaim(&mut self.frame_pool);
                    return Err(ServiceRuntimeError::PageTableRoot(error));
                }
            };
            if let Err(error) = tables.map_image(&loaded, &mut self.frame_pool, &mut memory) {
                tables.reclaim(&mut self.frame_pool, &mut memory);
                loaded.reclaim(&mut self.frame_pool);
                return Err(ServiceRuntimeError::PageTableMap(error));
            }
            let process = match self.processes.start_plan(plan) {
                Ok(process) => process,
                Err(error) => {
                    tables.reclaim(&mut self.frame_pool, &mut memory);
                    loaded.reclaim(&mut self.frame_pool);
                    return Err(ServiceRuntimeError::Process(error));
                }
            };
            let Some(root) = AddressSpaceRoot::new(tables.root().raw() as usize) else {
                let _ = self.processes.exit(process, 1);
                let _ = self.processes.reclaim(process);
                tables.reclaim(&mut self.frame_pool, &mut memory);
                loaded.reclaim(&mut self.frame_pool);
                return Err(ServiceRuntimeError::Process(ProcessError::AddressSpace));
            };
            if let Err(error) = self.processes.bind_address_space_root(process, root) {
                let _ = self.processes.exit(process, 1);
                let _ = self.processes.reclaim(process);
                tables.reclaim(&mut self.frame_pool, &mut memory);
                loaded.reclaim(&mut self.frame_pool);
                return Err(ServiceRuntimeError::Process(error));
            }
            if let Err(error) = map_loaded_pages(&mut self.processes, process, &loaded) {
                let _ = self.processes.exit(process, 1);
                let _ = self.processes.reclaim(process);
                tables.reclaim(&mut self.frame_pool, &mut memory);
                loaded.reclaim(&mut self.frame_pool);
                return Err(ServiceRuntimeError::Process(error));
            }
            self.map_service_heap(service, process, &mut tables)?;
            if service == ServiceId::User {
                let pages = logos_abi::USER_KDF_WORKSPACE_PAGES;
                for page in 0..pages {
                    let frame = self
                        .frame_pool
                        .allocate_for(OwnerId::service(service))
                        .map_err(|_| ServiceRuntimeError::Resources)?;
                    self.user_kdf_workspace[page] = frame.raw();
                    memory.clear(frame).map_err(ServiceRuntimeError::IpcPrivateMapping)?;
                    tables
                        .map_raw_page(
                            logos_abi::USER_KDF_WORKSPACE_BASE + page * crate::loader::PAGE_SIZE,
                            frame,
                            MappingFlags::DATA,
                            &mut self.frame_pool,
                            &mut memory,
                        )
                        .map_err(ServiceRuntimeError::IpcPrivateMapping)?;
                }
                let mapping = VirtualMapping::new_service_workspace(
                    logos_abi::USER_KDF_WORKSPACE_BASE,
                    self.user_kdf_workspace[0] as usize,
                    pages,
                    MappingFlags::DATA,
                )
                .ok_or(ServiceRuntimeError::IpcPrivateProcess(ProcessError::AddressSpace))?;
                self.processes
                    .map(process, mapping)
                    .map_err(ServiceRuntimeError::IpcPrivateProcess)?;
            }
            let launch =
                match self.processes.user_launch(process, loaded.entry(), loaded.stack_top()) {
                    Ok(launch) => launch,
                    Err(error) => {
                        let _ = self.processes.exit(process, 1);
                        let _ = self.processes.reclaim(process);
                        tables.reclaim(&mut self.frame_pool, &mut memory);
                        loaded.reclaim(&mut self.frame_pool);
                        return Err(ServiceRuntimeError::Process(error));
                    }
                };
            self.launches[index] = Some((process, launch));
            self.images[index] = loaded;
            self.tables[index].write(tables);
            self.table_ready[index] = true;
        }
        self.initialize_dynamic_services()?;
        self.initialize_dynamic_ipc()?;
        self.publish_keyboard_event()?;
        let mut memory = IdentityPageTableMemory;
        for spec in SERVICE_IMAGES {
            let service = spec.service();
            let index = service.index();
            let Some((process, _)) = self.launch(service) else {
                return Err(ServiceRuntimeError::IpcPrivateProcess(ProcessError::InvalidHandle));
            };
            let staging = self.frame_pool.allocate().map_err(|_| ServiceRuntimeError::Resources)?;
            self.ipc_staging_frames[index] = Some(staging);
            memory.clear(staging).map_err(ServiceRuntimeError::IpcPrivateMapping)?;
            self.map_ipc_private_page(
                process,
                staging,
                logos_abi::IPC_STAGING_BASE,
                MappingFlags::DATA,
            )?;
            if service == ServiceId::Storage {
                for page in 0..logos_abi::STORAGE_DATA_PAGES {
                    let data =
                        self.frame_pool.allocate().map_err(|_| ServiceRuntimeError::Resources)?;
                    self.storage_data_frames[page] = Some(data);
                    memory.clear(data).map_err(ServiceRuntimeError::IpcPrivateMapping)?;
                    let address = logos_abi::STORAGE_DATA_BASE
                        .checked_add(page * crate::loader::PAGE_SIZE)
                        .ok_or(ServiceRuntimeError::IpcPrivateMapping(
                            PageTableError::InvalidVirtualAddress,
                        ))?;
                    self.map_ipc_private_page(process, data, address, MappingFlags::DATA)?;
                }
            }
            if service == ServiceId::Network && self.network_config.is_enabled() {
                let config =
                    self.frame_pool.allocate().map_err(|_| ServiceRuntimeError::Resources)?;
                self.network_config_frame = Some(config);
                memory.clear(config).map_err(ServiceRuntimeError::IpcPrivateMapping)?;
                unsafe {
                    (config.raw() as usize as *mut logos_abi::NetworkConfig)
                        .write(self.network_config);
                }
                self.map_ipc_private_page(
                    process,
                    config,
                    logos_abi::NETWORK_CONFIG_BASE,
                    MappingFlags::READ_ONLY_DATA,
                )?;
                for page in 0..logos_abi::NETWORK_PACKET_PAGE_COUNT {
                    let packet =
                        self.frame_pool.allocate().map_err(|_| ServiceRuntimeError::Resources)?;
                    self.network_packet_frames[page] = Some(packet);
                    memory.clear(packet).map_err(ServiceRuntimeError::IpcPrivateMapping)?;
                    let address = logos_abi::NETWORK_PACKET_BASE
                        .checked_add(page * crate::loader::PAGE_SIZE)
                        .ok_or(ServiceRuntimeError::IpcPrivateMapping(
                            PageTableError::InvalidVirtualAddress,
                        ))?;
                    self.map_ipc_private_page(process, packet, address, MappingFlags::DATA)?;
                }
            }
        }
        let framebuffer = resources.framebuffer().ok_or(ServiceRuntimeError::Resources)?;
        self.map_framebuffer(framebuffer)?;
        let framebuffer_config_frame =
            self.frame_pool.allocate().map_err(|_| ServiceRuntimeError::Resources)?;
        self.framebuffer_config_frame = Some(framebuffer_config_frame);
        memory.clear(framebuffer_config_frame).map_err(ServiceRuntimeError::FramebufferConfig)?;
        initialize_framebuffer_config(framebuffer_config_frame, framebuffer);
        self.map_framebuffer_config(framebuffer_config_frame)?;
        let keyboard_frame =
            self.frame_pool.allocate().map_err(|_| ServiceRuntimeError::Resources)?;
        self.keyboard_frame = Some(keyboard_frame);
        memory.clear(keyboard_frame).map_err(ServiceRuntimeError::Keyboard)?;
        self.map_keyboard_ring(keyboard_frame)?;
        crate::arch::publish_keyboard_ring(keyboard_frame.raw() as usize);
        self.startup.mark_launch_ready();
        Ok(())
    }

    fn publish_keyboard_event(&mut self) -> Result<(), ServiceRuntimeError> {
        let frame = self
            .service_bootstrap_frames
            .get(ServiceId::Input.index())
            .copied()
            .filter(|raw| *raw != 0)
            .map(FrameAddress::from_raw)
            .ok_or(ServiceRuntimeError::IpcPrivateMapping(PageTableError::InvalidMapping))?;
        let mut page = unsafe {
            core::ptr::read_unaligned(frame.raw() as usize as *const logos_abi::ServiceBootstrapPage)
        };
        page.keyboard_event = self.keyboard_event;
        unsafe {
            core::ptr::write_unaligned(
                frame.raw() as usize as *mut logos_abi::ServiceBootstrapPage,
                page,
            );
        }
        Ok(())
    }

    pub fn image(&self, service: ServiceId) -> Option<&LoadedImage> {
        let index = service.index();
        if self.table_ready[index] { Some(&self.images[index]) } else { None }
    }

    #[cfg(feature = "package-proof")]
    pub(crate) fn package_frame_accounting_valid(&self) -> bool {
        SERVICE_IMAGES.iter().all(|spec| {
            let service = spec.service();
            let Some(image) = self.image(service) else {
                return false;
            };
            let expected = match self.dynamic_services.as_ref().and_then(|registry| {
                registry.image_source(self.runtime_service_handle(service).ok()?).ok()
            }) {
                Some(ServiceImageSource::FilesystemPackage) => {
                    image.page_count() as u32
                        + unsafe { self.tables[service.index()].assume_init_ref().table_count() }
                            as u32
                }
                Some(ServiceImageSource::Builtin) => 0,
                None => return false,
            };
            self.frame_pool.manager().owner_live(crate::memory::OwnerId::service(service))
                == expected
        })
    }

    pub fn root(&self, service: ServiceId) -> Option<usize> {
        let index = service.index();
        if !self.table_ready[index] {
            return None;
        }
        // SAFETY: `table_ready` is set only after the corresponding builder is
        // initialized and remains true for the runtime lifetime.
        Some(unsafe { self.tables[index].assume_init_ref().root().raw() as usize })
    }

    pub fn launch(&self, service: ServiceId) -> Option<(ProcessHandle, UserLaunch)> {
        self.launches[service.index()]
    }

    pub fn all_launch_ready(&self) -> bool {
        self.startup.all_launch_ready()
    }

    #[allow(dead_code)]
    pub(crate) fn keyboard_frame(&self) -> Option<FrameAddress> {
        self.keyboard_frame
    }

    pub(crate) fn keyboard_ring_address(&self) -> Option<usize> {
        self.keyboard_frame().map(|frame| frame.raw() as usize)
    }

    #[allow(dead_code)]
    pub(crate) fn framebuffer_config_frame(&self) -> Option<FrameAddress> {
        self.framebuffer_config_frame
    }

    fn map_framebuffer(
        &mut self,
        framebuffer: crate::boot_resources::FramebufferInfo,
    ) -> Result<(), ServiceRuntimeError> {
        let bytes = framebuffer
            .bytes()
            .checked_add(crate::boot_resources::PAGE_SIZE - 1)
            .ok_or(ServiceRuntimeError::Framebuffer(PageTableError::InvalidMapping))?;
        let pages = usize::try_from(bytes / crate::boot_resources::PAGE_SIZE)
            .map_err(|_| ServiceRuntimeError::Framebuffer(PageTableError::InvalidMapping))?;
        let service = ServiceId::Display;
        let index = service.index();
        let Some((process, _)) = self.launch(service) else {
            return Err(ServiceRuntimeError::FramebufferProcess(ProcessError::InvalidHandle));
        };
        let mut memory = IdentityPageTableMemory;
        let tables = unsafe { self.tables[index].assume_init_mut() };
        for page in 0..pages {
            let offset = (page as u64)
                .checked_mul(crate::boot_resources::PAGE_SIZE)
                .ok_or(ServiceRuntimeError::Framebuffer(PageTableError::InvalidMapping))?;
            let physical = framebuffer
                .base()
                .checked_add(offset)
                .ok_or(ServiceRuntimeError::Framebuffer(PageTableError::InvalidMapping))?;
            tables
                .map_raw_page(
                    logos_abi::DISPLAY_FRAMEBUFFER_BASE + page * crate::loader::PAGE_SIZE,
                    FrameAddress::from_raw(physical),
                    MappingFlags::DATA,
                    &mut self.frame_pool,
                    &mut memory,
                )
                .map_err(ServiceRuntimeError::Framebuffer)?;
        }
        let mapping = VirtualMapping::new_device(
            logos_abi::DISPLAY_FRAMEBUFFER_BASE,
            framebuffer.base() as usize,
            pages,
            MappingFlags::DATA,
        )
        .ok_or(ServiceRuntimeError::FramebufferProcess(ProcessError::AddressSpace))?;
        self.processes.map(process, mapping).map_err(ServiceRuntimeError::FramebufferProcess)
    }

    fn map_keyboard_ring(&mut self, frame: FrameAddress) -> Result<(), ServiceRuntimeError> {
        let service = ServiceId::Input;
        let index = service.index();
        let Some((process, _)) = self.launch(service) else {
            return Err(ServiceRuntimeError::KeyboardProcess(ProcessError::InvalidHandle));
        };
        let mut memory = IdentityPageTableMemory;
        let tables = unsafe { self.tables[index].assume_init_mut() };
        tables
            .map_raw_page(
                logos_abi::INPUT_KEYBOARD_RING_BASE,
                frame,
                MappingFlags::DATA,
                &mut self.frame_pool,
                &mut memory,
            )
            .map_err(ServiceRuntimeError::Keyboard)?;
        let mapping = VirtualMapping::new(
            logos_abi::INPUT_KEYBOARD_RING_BASE,
            frame.raw() as usize,
            1,
            MappingFlags::DATA,
        )
        .ok_or(ServiceRuntimeError::KeyboardProcess(ProcessError::AddressSpace))?;
        self.processes.map(process, mapping).map_err(ServiceRuntimeError::KeyboardProcess)
    }

    fn map_ipc_private_page(
        &mut self,
        process: ProcessHandle,
        frame: FrameAddress,
        virtual_address: usize,
        flags: MappingFlags,
    ) -> Result<(), ServiceRuntimeError> {
        let index = SERVICE_IMAGES
            .iter()
            .position(|spec| {
                self.launch(spec.service()).is_some_and(|(handle, _)| handle == process)
            })
            .ok_or(ServiceRuntimeError::IpcPrivateProcess(ProcessError::InvalidHandle))?;
        let mut memory = IdentityPageTableMemory;
        unsafe { self.tables[index].assume_init_mut() }
            .map_raw_page(virtual_address, frame, flags, &mut self.frame_pool, &mut memory)
            .map_err(ServiceRuntimeError::IpcPrivateMapping)?;
        let mapping = VirtualMapping::new(virtual_address, frame.raw() as usize, 1, flags)
            .ok_or(ServiceRuntimeError::IpcPrivateProcess(ProcessError::AddressSpace))?;
        self.processes.map(process, mapping).map_err(ServiceRuntimeError::IpcPrivateProcess)
    }

    fn map_service_heap(
        &mut self,
        service: ServiceId,
        process: ProcessHandle,
        tables: &mut PageTableBuilder,
    ) -> Result<(), ServiceRuntimeError> {
        let index = service.index();
        let mut memory = IdentityPageTableMemory;
        let heap_quota_pages = logos_abi::SERVICE_HEAP_MAX_PAGES;
        let initial_heap_pages = logos_abi::SERVICE_HEAP_INITIAL_PAGES;
        self.service_heaps[index].quota_pages = heap_quota_pages;
        self.service_heaps[index].frames.clear();
        if self.service_heaps[index].frames.try_reserve_exact(initial_heap_pages).is_err() {
            return Err(ServiceRuntimeError::Resources);
        }
        let bootstrap = self
            .frame_pool
            .allocate_for(OwnerId::service(service))
            .map_err(|_| ServiceRuntimeError::Resources)?;
        self.service_bootstrap_frames[index] = bootstrap.raw();
        memory.clear(bootstrap).map_err(ServiceRuntimeError::IpcPrivateMapping)?;
        let generation = (self.service_epoch as u32).max(1);
        let control = logos_abi::CapabilityHandle::new(0, generation)
            .ok_or(ServiceRuntimeError::Resources)?;
        let directory = logos_abi::CapabilityHandle::new(1, generation)
            .ok_or(ServiceRuntimeError::Resources)?;
        let heap = logos_abi::CapabilityHandle::new(2, generation)
            .ok_or(ServiceRuntimeError::Resources)?;
        self.bootstrap_control[index] = control;
        self.bootstrap_directory[index] = directory;
        self.bootstrap_heap[index] = heap;
        let page = logos_abi::ServiceBootstrapPage {
            abi_version: logos_abi::RUNTIME_ABI_VERSION,
            flags: 0,
            service_epoch: self.service_epoch,
            service: logos_abi::ServiceHandle::new(index as u32, generation)
                .ok_or(ServiceRuntimeError::Resources)?,
            control,
            directory,
            heap,
            keyboard_event: if service == ServiceId::Input {
                self.keyboard_event
            } else {
                logos_abi::EventHandle::EMPTY
            },
            heap_base: logos_abi::SERVICE_HEAP_BASE as u64,
            heap_pages: initial_heap_pages as u32,
            heap_quota_pages: heap_quota_pages as u32,
        };
        unsafe { core::ptr::write_unaligned(bootstrap.raw() as usize as *mut _, page) };
        tables
            .map_raw_page(
                logos_abi::SERVICE_BOOTSTRAP_BASE,
                bootstrap,
                MappingFlags::READ_ONLY_DATA,
                &mut self.frame_pool,
                &mut memory,
            )
            .map_err(ServiceRuntimeError::IpcPrivateMapping)?;
        let bootstrap_mapping = VirtualMapping::new(
            logos_abi::SERVICE_BOOTSTRAP_BASE,
            bootstrap.raw() as usize,
            1,
            MappingFlags::READ_ONLY_DATA,
        )
        .ok_or(ServiceRuntimeError::IpcPrivateProcess(ProcessError::AddressSpace))?;
        self.processes
            .map(process, bootstrap_mapping)
            .map_err(ServiceRuntimeError::IpcPrivateProcess)?;

        for heap_page in 0..initial_heap_pages {
            let frame = self
                .frame_pool
                .allocate_for(OwnerId::service(service))
                .map_err(|_| ServiceRuntimeError::Resources)?;
            self.service_heaps[index].frames.push(frame);
            memory.clear(frame).map_err(ServiceRuntimeError::IpcPrivateMapping)?;
            let address = logos_abi::SERVICE_HEAP_BASE + heap_page * crate::loader::PAGE_SIZE;
            tables
                .map_raw_page(address, frame, MappingFlags::DATA, &mut self.frame_pool, &mut memory)
                .map_err(ServiceRuntimeError::IpcPrivateMapping)?;
        }
        let heap_physical = self.service_heaps[index]
            .frames
            .first()
            .map(|frame| frame.raw() as usize)
            .ok_or(ServiceRuntimeError::Resources)?;
        let heap_mapping = VirtualMapping::new_service_heap(
            logos_abi::SERVICE_HEAP_BASE,
            heap_physical,
            initial_heap_pages,
            heap_quota_pages,
            MappingFlags::DATA,
        )
        .ok_or(ServiceRuntimeError::IpcPrivateProcess(ProcessError::AddressSpace))?;
        self.processes.map(process, heap_mapping).map_err(ServiceRuntimeError::IpcPrivateProcess)
    }

    pub(crate) fn grow_service_heap(
        &mut self,
        process: ProcessHandle,
        capability_raw: u64,
        pages: usize,
    ) -> logos_abi::IpcStatus {
        if pages != 1 {
            return logos_abi::IpcStatus::Malformed;
        }
        let Some(service) = self.service_for_process(process) else {
            return logos_abi::IpcStatus::Unauthorized;
        };
        if self.dynamic_service_state(service)
            != Some(crate::runtime_services::ServiceState::Running)
        {
            return logos_abi::IpcStatus::Stale;
        }
        let expected = self.bootstrap_heap[service.index()];
        if !expected.is_valid() {
            return logos_abi::IpcStatus::Stale;
        }
        if capability_raw != expected.raw() {
            return logos_abi::IpcStatus::Unauthorized;
        }
        let index = service.index();
        let page = self.service_heaps[index].frames.len();
        let quota_pages = self
            .dynamic_services
            .as_ref()
            .and_then(|registry| {
                registry.heap_quota_pages(self.runtime_service_handle(service).ok()?).ok()
            })
            .unwrap_or(self.service_heaps[index].quota_pages);
        if page >= quota_pages {
            return logos_abi::IpcStatus::Full;
        }
        if self.service_heaps[index].frames.try_reserve(1).is_err() {
            return logos_abi::IpcStatus::Full;
        }
        let frame = match self.frame_pool.allocate_for(OwnerId::service(service)) {
            Ok(frame) => frame,
            Err(_) => return logos_abi::IpcStatus::Full,
        };
        let mut memory = IdentityPageTableMemory;
        if memory.clear(frame).is_err() {
            let _ = self.frame_pool.release(frame);
            return logos_abi::IpcStatus::Full;
        }
        let address = logos_abi::SERVICE_HEAP_BASE + page * crate::loader::PAGE_SIZE;
        let tables = unsafe { self.tables[index].assume_init_mut() };
        if tables
            .map_raw_page(address, frame, MappingFlags::DATA, &mut self.frame_pool, &mut memory)
            .is_err()
        {
            let _ = self.frame_pool.release(frame);
            return logos_abi::IpcStatus::Full;
        }
        self.service_heaps[index].frames.push(frame);
        logos_abi::IpcStatus::Ok
    }

    pub(crate) fn shrink_service_heap(
        &mut self,
        process: ProcessHandle,
        capability_raw: u64,
        pages: usize,
    ) -> logos_abi::IpcStatus {
        if pages != 1 {
            return logos_abi::IpcStatus::Malformed;
        }
        let Some(service) = self.service_for_process(process) else {
            return logos_abi::IpcStatus::Unauthorized;
        };
        if self.dynamic_service_state(service)
            != Some(crate::runtime_services::ServiceState::Running)
        {
            return logos_abi::IpcStatus::Stale;
        }
        let expected = self.bootstrap_heap[service.index()];
        if !expected.is_valid() {
            return logos_abi::IpcStatus::Stale;
        }
        if capability_raw != expected.raw() {
            return logos_abi::IpcStatus::Unauthorized;
        }
        let index = service.index();
        let page = self.service_heaps[index].frames.len();
        if page <= logos_abi::SERVICE_HEAP_INITIAL_PAGES {
            return logos_abi::IpcStatus::Full;
        }
        let address = logos_abi::SERVICE_HEAP_BASE + (page - 1) * crate::loader::PAGE_SIZE;
        let tables = unsafe { self.tables[index].assume_init_mut() };
        let mut memory = IdentityPageTableMemory;
        let frame = match tables.unmap_page(address, &mut memory) {
            Ok(frame) => frame,
            Err(_) => return logos_abi::IpcStatus::Full,
        };
        if self.frame_pool.release(frame).is_err() {
            crate::arch_fatal(b"LogOS vNext: service heap release");
        }
        let _ = self.service_heaps[index].frames.pop();
        logos_abi::IpcStatus::Ok
    }

    fn map_framebuffer_config(&mut self, frame: FrameAddress) -> Result<(), ServiceRuntimeError> {
        let service = ServiceId::Display;
        let index = service.index();
        let Some((process, _)) = self.launch(service) else {
            return Err(ServiceRuntimeError::FramebufferConfigProcess(ProcessError::InvalidHandle));
        };
        let mut memory = IdentityPageTableMemory;
        let tables = unsafe { self.tables[index].assume_init_mut() };
        tables
            .map_raw_page(
                logos_abi::DISPLAY_CONFIG_BASE,
                frame,
                MappingFlags::READ_ONLY_DATA,
                &mut self.frame_pool,
                &mut memory,
            )
            .map_err(ServiceRuntimeError::FramebufferConfig)?;
        let mapping = VirtualMapping::new(
            logos_abi::DISPLAY_CONFIG_BASE,
            frame.raw() as usize,
            1,
            MappingFlags::READ_ONLY_DATA,
        )
        .ok_or(ServiceRuntimeError::FramebufferConfigProcess(ProcessError::AddressSpace))?;
        self.processes.map(process, mapping).map_err(ServiceRuntimeError::FramebufferConfigProcess)
    }

    pub fn start_tasks(&mut self) -> Result<(), ServiceRuntimeError> {
        if self.network_config.is_enabled() {
            self.queue_network_link();
        }
        for service in crate::service_images::SERVICE_START_ORDER {
            if (service == ServiceId::Network || service == ServiceId::Fetch)
                && !self.network_config.is_enabled()
            {
                continue;
            }
            self.start_service_task(service)?;
            self.startup.start(service).map_err(ServiceRuntimeError::Startup)?;
        }
        self.supervisor.clear_startup_grace();
        Ok(())
    }

    fn queue_network_link(&mut self) {
        let mut link = logos_abi::NetworkPacketDescriptor::new(
            logos_abi::NetworkPacketOperation::LinkState,
            0,
            self.network_packet_sequence,
        );
        link.generation = self.ipc_generation;
        link.service_epoch = self.service_epoch;
        if let Some(mac) = crate::arch::network_mac() {
            link.mac = mac;
        } else {
            link.result = logos_abi::NetworkResult::NotFound;
        }
        self.network_packet_sequence = self.network_packet_sequence.wrapping_add(1).max(1);
        self.network_packet_response = Some(link);
    }

    fn start_service_task(&mut self, service: ServiceId) -> Result<(), ServiceRuntimeError> {
        let index = service.index();
        if self.tasks[index].is_some() {
            return Ok(());
        }
        let Some((process, launch)) = self.launch(service) else {
            return Err(ServiceRuntimeError::TaskLaunch);
        };
        let task =
            crate::SCHEDULER.spawn_user(service_task_entry, process, launch).map_err(|error| {
                match error {
                    crate::SpawnError::Capacity => ServiceRuntimeError::TaskCapacity,
                    crate::SpawnError::AddressSpace => ServiceRuntimeError::TaskAddressSpace,
                    crate::SpawnError::UserLaunch => ServiceRuntimeError::TaskLaunch,
                }
            })?;
        self.tasks[index] = Some(task);
        let now = crate::current_ticks();
        self.heartbeat_ticks[index].store(now, Ordering::Release);
        self.supervisor.register(service, now);
        self.sync_dynamic_service_running(service);
        Ok(())
    }

    fn sync_dynamic_service_running(&mut self, service: ServiceId) {
        let Ok(handle) = self.runtime_service_handle(service) else { return };
        let Some((process, launch)) = self.launch(service) else { return };
        let Some(task) = self.tasks[service.index()] else { return };
        let heap_pages = self.service_heaps[service.index()].frames.len();
        if let Some(registry) = self.dynamic_services.as_mut() {
            let _ = registry.start(handle);
            let _ = registry.set_runtime_ownership(
                handle,
                process.raw(),
                launch.address_space_root().raw() as u64,
                task.raw(),
                heap_pages,
            );
        }
    }

    fn sync_dynamic_service_stopped(&mut self, service: ServiceId) {
        let Ok(handle) = self.runtime_service_handle(service) else { return };
        if let Some(registry) = self.dynamic_services.as_mut() {
            let _ = registry.stop(handle);
        }
    }

    fn sync_dynamic_service_failed(&mut self, service: ServiceId) {
        let Ok(handle) = self.runtime_service_handle(service) else { return };
        if let Some(registry) = self.dynamic_services.as_mut() {
            let _ = registry.fail(handle);
        }
    }

    fn sync_dynamic_service_stopping(&mut self, service: ServiceId) {
        let Ok(handle) = self.runtime_service_handle(service) else { return };
        if let Some(registry) = self.dynamic_services.as_mut() {
            let _ = registry.mark_stopping(handle);
        }
    }

    fn refresh_dynamic_service_counts(&mut self, service: ServiceId) {
        let Ok(owner) = self.runtime_service_handle(service) else { return };
        let (ipc_endpoints, capabilities) = self
            .dynamic_ipc
            .as_ref()
            .map(|registry| registry.ownership_counts(owner))
            .unwrap_or((0, 0));
        let events = self
            .dynamic_events
            .as_ref()
            .map(|registry| registry.ownership_count(owner) + registry.event_set_count(owner))
            .unwrap_or(0);
        if let Some(registry) = self.dynamic_services.as_mut() {
            let _ = registry.set_runtime_counts(owner, ipc_endpoints, capabilities, events);
        }
    }

    fn dynamic_service_state(
        &self,
        service: ServiceId,
    ) -> Option<crate::runtime_services::ServiceState> {
        let handle = self.runtime_service_handle(service).ok()?;
        self.dynamic_services.as_ref()?.state(handle).ok()
    }

    fn abort_dynamic_service_lifecycle(
        &mut self,
        operation: logos_abi::ManagerOperation,
        service: ServiceId,
    ) {
        let Ok(handle) = self.runtime_service_handle(service) else { return };
        if let Some(registry) = self.dynamic_services.as_mut() {
            let _ = registry.abort_lifecycle(operation, handle);
        }
    }

    fn uses_package_image(&self, service: ServiceId) -> bool {
        self.dynamic_services.as_ref().and_then(|registry| {
            registry.image_source(self.runtime_service_handle(service).ok()?).ok()
        }) == Some(ServiceImageSource::FilesystemPackage)
    }

    /// Reset the bounded image-owned memory and private staging before a
    /// stopped predeclared service is started again. Package-backed services
    /// require the graph restart path until durable reloading is available.
    fn reset_service_image(&mut self, service: ServiceId) -> Result<(), ServiceRuntimeError> {
        if self.uses_package_image(service) {
            return Err(ServiceRuntimeError::Image);
        }
        let Some(bundle) = crate::arch::service_images() else {
            return Err(ServiceRuntimeError::Resources);
        };
        let Some(spec) = SERVICE_IMAGES.iter().find(|spec| spec.service() == service) else {
            return Err(ServiceRuntimeError::Image);
        };
        let Some(image) = (unsafe { bundle.image(service) }) else {
            return Err(ServiceRuntimeError::Image);
        };
        let plan = spec.validate_image(image).map_err(|_| ServiceRuntimeError::Image)?;
        let mut memory = IdentityPageTableMemory;
        self.images[service.index()]
            .populate(plan, image, &mut memory)
            .map_err(ServiceRuntimeError::Populate)?;
        if let Some(frame) = self.ipc_staging_frames[service.index()] {
            memory.clear(frame).map_err(ServiceRuntimeError::IpcPrivateMapping)?;
        }
        if service == ServiceId::Storage {
            self.storage_response = None;
            self.storage_map_response = None;
            self.package_request = None;
            self.package_response = None;
            for frame in self.storage_data_frames.iter().flatten().copied() {
                memory.clear(frame).map_err(ServiceRuntimeError::IpcPrivateMapping)?;
            }
        }
        if service == ServiceId::Device {
            self.device_response = None;
        }
        if service == ServiceId::Network {
            if let Some(frame) = self.network_config_frame {
                memory.clear(frame).map_err(ServiceRuntimeError::IpcPrivateMapping)?;
                unsafe {
                    (frame.raw() as usize as *mut logos_abi::NetworkConfig)
                        .write(self.network_config);
                }
            }
            // Packet pages are Core-owned VirtIO DMA buffers. A targeted
            // Network restart must not clear them while the device can still
            // own a descriptor; the new service instance reinitializes its
            // protocol state without touching the transport ring.
        }
        Ok(())
    }

    fn request_stop_service(&mut self, service: ServiceId) -> Result<bool, ServiceRuntimeError> {
        let stop_requested = self.request_stop_task(service)?;
        if stop_requested {
            self.sync_dynamic_service_stopping(service);
        }
        Ok(stop_requested)
    }

    #[allow(dead_code)]
    fn next_package_request(
        &mut self,
        operation: logos_abi::PackageOperation,
        service: ServiceId,
        package_generation: u32,
        offset: usize,
        length: usize,
    ) -> Result<logos_abi::PackageRequest, ProcessError> {
        self.next_package_request_target(
            operation,
            logos_abi::PackageTarget::service(service),
            package_generation,
            offset,
            length,
        )
    }

    fn next_package_request_target(
        &mut self,
        operation: logos_abi::PackageOperation,
        target: logos_abi::PackageTarget,
        package_generation: u32,
        offset: usize,
        length: usize,
    ) -> Result<logos_abi::PackageRequest, ProcessError> {
        let request_id = self.package_next_request;
        self.package_next_request = self.package_next_request.wrapping_add(1).max(1);
        let offset = u32::try_from(offset).map_err(|_| ProcessError::InvalidImage)?;
        let length = u16::try_from(length).map_err(|_| ProcessError::InvalidImage)?;
        let request = match target.kind {
            logos_abi::PackageTargetKind::Service => logos_abi::PackageRequest::new(
                operation,
                ServiceId::from_index(target.service.saturating_sub(1) as usize)
                    .ok_or(ProcessError::InvalidImage)?,
                request_id,
                self.ipc_generation,
                self.package_capability,
                self.service_epoch,
                package_generation,
                offset,
                length,
            ),
            logos_abi::PackageTargetKind::Program => logos_abi::PackageRequest::new_program(
                operation,
                &target.name[..target.name_len as usize],
                request_id,
                self.ipc_generation,
                self.package_capability,
                self.service_epoch,
                package_generation,
                offset,
                length,
            ),
        };
        request.ok_or(ProcessError::InvalidImage)
    }

    #[inline]
    fn package_request_slot(&self) -> Option<logos_abi::PackageRequest> {
        unsafe { core::ptr::read_volatile(core::ptr::addr_of!(self.package_request)) }
    }

    #[inline]
    #[allow(dead_code)]
    fn set_package_request_slot(&mut self, request: Option<logos_abi::PackageRequest>) {
        unsafe { core::ptr::write_volatile(core::ptr::addr_of_mut!(self.package_request), request) }
    }

    #[inline]
    #[allow(dead_code)]
    fn take_package_response_slot(&mut self) -> Option<logos_abi::PackageResponse> {
        let response =
            unsafe { core::ptr::read_volatile(core::ptr::addr_of!(self.package_response)) };
        unsafe { core::ptr::write_volatile(core::ptr::addr_of_mut!(self.package_response), None) };
        response
    }

    #[inline]
    fn package_response_slot(&self) -> Option<logos_abi::PackageResponse> {
        unsafe { core::ptr::read_volatile(core::ptr::addr_of!(self.package_response)) }
    }

    #[inline]
    fn set_package_response_slot(&mut self, response: Option<logos_abi::PackageResponse>) {
        unsafe {
            core::ptr::write_volatile(core::ptr::addr_of_mut!(self.package_response), response)
        }
    }

    #[allow(dead_code)]
    fn package_exchange(
        &mut self,
        request: logos_abi::PackageRequest,
        output: &mut [u8],
        runtime_guard: &mut crate::arch::ServiceRuntimeGuard,
    ) -> Result<logos_abi::PackageResponse, ProcessError> {
        if self.package_request_slot().is_some() {
            return Err(ProcessError::ReadFailure);
        }
        let _ = self.take_package_response_slot();
        self.set_package_request_slot(Some(request));
        let Some(package_endpoint) = self
            .dynamic_endpoint(
                None,
                Some(ServiceId::Storage),
                logos_abi::IPC_CONTRACT_PACKAGE_REQUEST,
            )
            .ok()
        else {
            self.set_package_request_slot(None);
            return Err(ProcessError::ReadFailure);
        };
        let request_capability = self
            .core_capability(package_endpoint, logos_abi::IpcRights::Send)
            .map_err(|_| ProcessError::ReadFailure)?;
        if request_capability.is_valid() {
            let Some(staging_frame) = self.ipc_staging_frames[ServiceId::Storage.index()] else {
                self.set_package_request_slot(None);
                return Err(ProcessError::ReadFailure);
            };
            unsafe {
                core::ptr::write_unaligned(
                    staging_frame.raw() as usize as *mut logos_abi::PackageRequest,
                    request,
                );
            }
            let bytes = unsafe {
                core::slice::from_raw_parts(
                    staging_frame.raw() as usize as *const u8,
                    core::mem::size_of::<logos_abi::PackageRequest>(),
                )
            };
            let core = match dynamic_core_handle((self.service_epoch as u32).max(1)) {
                Ok(core) => core,
                Err(_) => {
                    self.set_package_request_slot(None);
                    return Err(ProcessError::ReadFailure);
                }
            };
            let status = self.send_dynamic(core, request_capability, bytes);
            if status != logos_abi::IpcStatus::Ok {
                self.set_package_request_slot(None);
                return Err(ProcessError::ReadFailure);
            }
        }
        for _ in 0..PACKAGE_EXCHANGE_POLLS {
            if let Some(response) = self.take_package_response_slot() {
                if response.validate_for(request, self.ipc_generation, self.service_epoch).is_err()
                {
                    continue;
                }
                self.set_package_request_slot(None);
                if response.status != logos_abi::PackageStatus::Ok {
                    return Err(match response.status {
                        logos_abi::PackageStatus::NotFound
                        | logos_abi::PackageStatus::Stale
                        | logos_abi::PackageStatus::Unsupported
                        | logos_abi::PackageStatus::Invalid => ProcessError::InvalidImage,
                        _ => ProcessError::ReadFailure,
                    });
                }
                if request.operation == logos_abi::PackageOperation::Read {
                    let amount = response.bytes as usize;
                    if amount > output.len() {
                        return Err(ProcessError::ReadFailure);
                    }
                    let Some(frame) = self.storage_data_frames[0] else {
                        return Err(ProcessError::ReadFailure);
                    };
                    unsafe {
                        output[..amount].copy_from_slice(core::slice::from_raw_parts(
                            frame.raw() as usize as *const u8,
                            amount,
                        ));
                    }
                }
                return Ok(response);
            }
            runtime_guard.pause();
            crate::arch::yield_current();
            runtime_guard.resume();
        }
        self.set_package_request_slot(None);
        Err(ProcessError::ReadFailure)
    }

    #[allow(dead_code)]
    pub(crate) fn activate_package(
        &mut self,
        service: ServiceId,
        runtime_guard: &mut crate::arch::ServiceRuntimeGuard,
    ) -> Result<(), ServiceRuntimeError> {
        if self.prepared_packages[service.index()].is_some() {
            return Err(ServiceRuntimeError::Image);
        }
        let bundle = crate::arch::service_images().ok_or(ServiceRuntimeError::Resources)?;
        let request = self
            .next_package_request(logos_abi::PackageOperation::Lookup, service, 0, 0, 0)
            .map_err(ServiceRuntimeError::Process)?;
        let response = self
            .package_exchange(request, &mut [], runtime_guard)
            .map_err(ServiceRuntimeError::Process)?;
        let package_bytes = response.package_bytes as usize;
        let package_generation = response.package_generation;
        if package_bytes < logos_package::PACKAGE_HEADER_BYTES {
            return Err(ServiceRuntimeError::Image);
        }

        let (payload_offset, payload_length) = {
            let mut raw_reader = RuntimePackageReader::new(
                self,
                runtime_guard,
                logos_abi::PackageTarget::service(service),
                package_generation,
                0,
                package_bytes,
            );
            let mut package_scratch = [0; crate::loader::PAGE_SIZE];
            let mut prefix = [0; 10];
            logos_package::PackageReader::read(&mut raw_reader, 0, &mut prefix)
                .map_err(|_| ServiceRuntimeError::Image)?;
            let format_version = u16::from_le_bytes([prefix[8], prefix[9]]);
            if format_version == logos_package::PACKAGE_FORMAT_VERSION_V2 {
                let header =
                    logos_package::validate_package_v2(&mut raw_reader, &mut package_scratch)
                        .map_err(|_| ServiceRuntimeError::Image)?;
                if header.manifest.kind != logos_package::PackageKind::Service
                    || header.manifest.target != logos_package::PackageTarget::Service(service)
                {
                    return Err(ServiceRuntimeError::Image);
                }
                (logos_package::PACKAGE_HEADER_V2_BYTES, header.payload_length as usize)
            } else {
                let header = logos_package::validate_package(
                    &mut raw_reader,
                    service,
                    logos_abi::ABI_VERSION,
                    &mut package_scratch,
                )
                .map_err(|_| ServiceRuntimeError::Image)?;
                (logos_package::PACKAGE_HEADER_BYTES, header.payload_length as usize)
            }
        };
        let plan = {
            let mut payload_reader = RuntimePackageReader::new(
                self,
                runtime_guard,
                logos_abi::PackageTarget::service(service),
                package_generation,
                payload_offset,
                payload_length,
            );
            crate::process::ElfLoadPlan::parse_reader(&mut payload_reader)
                .map_err(|_| ServiceRuntimeError::Image)?
        };
        let stack_pages = match service {
            ServiceId::Storage => crate::process::STORAGE_STACK_PAGES,
            ServiceId::Network => crate::process::NETWORK_STACK_PAGES,
            ServiceId::Flow => crate::process::FLOW_STACK_PAGES,
            _ => crate::process::USER_STACK_PAGES,
        };
        let owner = crate::memory::OwnerId::service(service);
        let mut image = LoadedImage::load_with_stack_pages_for_owner(
            plan,
            &mut self.frame_pool,
            stack_pages,
            owner,
        )
        .map_err(ServiceRuntimeError::Load)?;
        let mut payload_reader = RuntimePackageReader::new(
            self,
            runtime_guard,
            logos_abi::PackageTarget::service(service),
            package_generation,
            payload_offset,
            payload_length,
        );
        let mut scratch = [0; crate::loader::PAGE_SIZE];
        let mut memory = IdentityPageTableMemory;
        if let Err(error) =
            image.populate_reader(plan, &mut payload_reader, &mut scratch, &mut memory)
        {
            image.reclaim(&mut self.frame_pool);
            return Err(ServiceRuntimeError::Populate(error));
        }
        self.prepared_packages[service.index()] =
            Some(PreparedServiceImage { service, plan, image });
        if let Err(error) = self.restart(bundle, runtime_guard) {
            self.reclaim_prepared_packages();
            return Err(error);
        }
        let handle = self.runtime_service_handle(service)?;
        self.dynamic_services
            .as_mut()
            .ok_or(ServiceRuntimeError::Resources)?
            .set_image(handle, b"package")
            .map_err(|_| ServiceRuntimeError::Resources)?;
        let service_handle = self.runtime_service_handle(service)?;
        self.dynamic_services
            .as_mut()
            .ok_or(ServiceRuntimeError::Resources)?
            .set_image_source(service_handle, ServiceImageSource::FilesystemPackage)
            .map_err(|_| ServiceRuntimeError::Resources)?;
        self.active_packages[service.index()] = Some(ActivePackageImage { service, plan });
        Ok(())
    }

    #[cfg(feature = "package-proof")]
    pub(crate) fn restart_for_package_proof(
        &mut self,
        runtime_guard: &mut crate::arch::ServiceRuntimeGuard,
    ) -> Result<(), ServiceRuntimeError> {
        let bundle = crate::arch::service_images().ok_or(ServiceRuntimeError::Resources)?;
        self.restart(bundle, runtime_guard)
    }

    fn retain_active_package_images(&mut self) -> Result<(), ServiceRuntimeError> {
        for index in 0..SERVICE_COUNT {
            let Some(active) = self.active_packages[index] else {
                continue;
            };
            if self.prepared_packages[index].is_some() {
                continue;
            }
            if !self.table_ready[index] {
                return Err(ServiceRuntimeError::Image);
            }
            let image = core::mem::replace(&mut self.images[index], LoadedImage::empty());
            self.prepared_packages[index] =
                Some(PreparedServiceImage { service: active.service, plan: active.plan, image });
        }
        Ok(())
    }

    fn request_stop_task(&mut self, service: ServiceId) -> Result<bool, ServiceRuntimeError> {
        let index = service.index();
        let Some(task) = self.tasks[index] else {
            return Err(ServiceRuntimeError::TaskStop);
        };
        if crate::SCHEDULER.request_stop(task) {
            return Ok(true);
        }
        if crate::SCHEDULER.state(task).is_none() {
            self.tasks[index] = None;
            self.supervisor.unregister(service);
            self.sync_dynamic_service_stopped(service);
            return Ok(false);
        }
        Err(ServiceRuntimeError::TaskStop)
    }

    pub(crate) fn record_heartbeat(
        &mut self,
        service: ServiceId,
        process: ProcessHandle,
        now: u64,
    ) -> bool {
        let index = service.index();
        if self.launch(service).is_none_or(|(current, _)| current != process) {
            return false;
        }
        if !self.suppressed_heartbeats[index].load(Ordering::Acquire) {
            self.heartbeat_ticks[index].store(now, Ordering::Release);
            if let Ok(handle) = self.runtime_service_handle(service) {
                if let Some(registry) = self.dynamic_services.as_mut() {
                    let _ = registry.set_heartbeat(handle, now);
                }
            }
        }
        true
    }

    pub(crate) fn owns_service_process(&self, service: ServiceId, process: ProcessHandle) -> bool {
        self.launch(service).is_some_and(|(current, _)| current == process)
    }

    #[cfg(feature = "qemu-proof")]
    pub(crate) fn suppress_heartbeat(&self, service: ServiceId) {
        self.suppressed_heartbeats[service.index()].store(true, Ordering::Release);
    }

    pub(crate) fn heartbeat_tick(&self, service: ServiceId) -> u64 {
        self.heartbeat_ticks[service.index()].load(Ordering::Acquire)
    }

    fn dynamic_device_request(
        &mut self,
        service: ServiceId,
        caller: logos_abi::ServiceHandle,
        capability: logos_abi::CapabilityHandle,
    ) -> crate::service_ipc::IpcOutcome {
        let Some(staging_frame) = self.ipc_staging_frames[service.index()] else {
            return crate::service_ipc::IpcOutcome {
                status: logos_abi::IpcStatus::Unauthorized,
                notified: false,
            };
        };
        let request_bytes = core::mem::size_of::<logos_abi::DeviceRequest>();
        let request = unsafe {
            core::slice::from_raw_parts(staging_frame.raw() as usize as *const u8, request_bytes)
        };
        let request_status = self.send_dynamic(caller, capability, request);
        if request_status != logos_abi::IpcStatus::Ok {
            return crate::service_ipc::IpcOutcome { status: request_status, notified: false };
        }

        let core = match dynamic_core_handle((self.service_epoch as u32).max(1)) {
            Ok(core) => core,
            Err(_) => {
                return crate::service_ipc::IpcOutcome {
                    status: logos_abi::IpcStatus::Stale,
                    notified: false,
                };
            }
        };
        let Some(request_endpoint) = self
            .dynamic_endpoint(Some(ServiceId::Device), None, logos_abi::IPC_CONTRACT_DEVICE_REQUEST)
            .ok()
        else {
            return crate::service_ipc::IpcOutcome {
                status: logos_abi::IpcStatus::Disconnected,
                notified: false,
            };
        };
        let core_capability =
            match self.core_capability(request_endpoint, logos_abi::IpcRights::Receive) {
                Ok(capability) => capability,
                Err(status) => return crate::service_ipc::IpcOutcome { status, notified: false },
            };
        let request = unsafe {
            core::slice::from_raw_parts_mut(staging_frame.raw() as usize as *mut u8, request_bytes)
        };
        let receive_status = self.receive_dynamic(core, core_capability, request);
        if receive_status != logos_abi::IpcStatus::Ok {
            return crate::service_ipc::IpcOutcome { status: receive_status, notified: false };
        }
        let wire = unsafe {
            core::slice::from_raw_parts(staging_frame.raw() as usize as *const u8, request_bytes)
        };
        let request = unsafe {
            core::ptr::read_unaligned(
                staging_frame.raw() as usize as *const logos_abi::DeviceRequest
            )
        };
        if !logos_abi::DeviceRequest::wire_enums_valid(wire)
            || crate::device_ipc::validate_dynamic_request(
                request,
                self.ipc_generation,
                self.service_epoch,
            )
            .is_err()
        {
            return crate::service_ipc::IpcOutcome {
                status: logos_abi::IpcStatus::Malformed,
                notified: false,
            };
        }
        let response = match request.operation {
            logos_abi::DeviceOperation::List => {
                crate::arch::device_list_response(request, self.ipc_generation, self.service_epoch)
            }
        };
        unsafe {
            core::ptr::write_unaligned(
                staging_frame.raw() as usize as *mut logos_abi::DeviceResponse,
                response,
            );
        }
        let Some(response_endpoint) = self
            .dynamic_endpoint(
                None,
                Some(ServiceId::Device),
                logos_abi::IPC_CONTRACT_DEVICE_RESPONSE,
            )
            .ok()
        else {
            return crate::service_ipc::IpcOutcome {
                status: logos_abi::IpcStatus::Disconnected,
                notified: false,
            };
        };
        let response_capability =
            match self.core_capability(response_endpoint, logos_abi::IpcRights::Send) {
                Ok(capability) => capability,
                Err(status) => return crate::service_ipc::IpcOutcome { status, notified: false },
            };
        let response_bytes = unsafe {
            core::slice::from_raw_parts(
                staging_frame.raw() as usize as *const u8,
                core::mem::size_of::<logos_abi::DeviceResponse>(),
            )
        };
        let response_status = self.send_dynamic(core, response_capability, response_bytes);
        crate::service_ipc::IpcOutcome {
            status: response_status,
            notified: response_status == logos_abi::IpcStatus::Ok,
        }
    }

    fn dynamic_core_request(
        &mut self,
        service: ServiceId,
        caller: logos_abi::ServiceHandle,
        capability: logos_abi::CapabilityHandle,
        endpoint: logos_abi::EndpointHandle,
        message_bytes: usize,
    ) -> logos_abi::IpcStatus {
        let Some(staging_frame) = self.ipc_staging_frames[service.index()] else {
            return logos_abi::IpcStatus::Unauthorized;
        };
        let request = unsafe {
            core::slice::from_raw_parts(staging_frame.raw() as usize as *const u8, message_bytes)
        };
        let status = self.send_dynamic(caller, capability, request);
        if status != logos_abi::IpcStatus::Ok {
            return status;
        }
        let core = match dynamic_core_handle((self.service_epoch as u32).max(1)) {
            Ok(core) => core,
            Err(_) => return logos_abi::IpcStatus::Stale,
        };
        let core_capability = match self.core_capability(endpoint, logos_abi::IpcRights::Receive) {
            Ok(capability) => capability,
            Err(status) => return status,
        };
        let request = unsafe {
            core::slice::from_raw_parts_mut(staging_frame.raw() as usize as *mut u8, message_bytes)
        };
        self.receive_dynamic(core, core_capability, request)
    }

    fn queue_dynamic_core_response(
        &mut self,
        service: ServiceId,
        endpoint: logos_abi::EndpointHandle,
        message_bytes: usize,
    ) -> logos_abi::IpcStatus {
        let Some(staging_frame) = self.ipc_staging_frames[service.index()] else {
            return logos_abi::IpcStatus::Unauthorized;
        };
        let core = match dynamic_core_handle((self.service_epoch as u32).max(1)) {
            Ok(core) => core,
            Err(_) => return logos_abi::IpcStatus::Stale,
        };
        let core_capability = match self.core_capability(endpoint, logos_abi::IpcRights::Send) {
            Ok(capability) => capability,
            Err(status) => return status,
        };
        let bytes = unsafe {
            core::slice::from_raw_parts(staging_frame.raw() as usize as *const u8, message_bytes)
        };
        self.send_dynamic(core, core_capability, bytes)
    }

    fn receive_dynamic(
        &mut self,
        caller: logos_abi::ServiceHandle,
        capability: logos_abi::CapabilityHandle,
        bytes: &mut [u8],
    ) -> logos_abi::IpcStatus {
        let event = self.dynamic_ipc.as_ref().and_then(|registry| {
            registry
                .capability_endpoint(caller, capability, logos_abi::IpcRights::Receive)
                .ok()
                .and_then(|(endpoint, _)| registry.endpoint_events(endpoint).ok())
                .map(|(_, write)| write)
        });
        let status = match (self.dynamic_ipc.as_mut(), self.dynamic_events.as_mut()) {
            (Some(registry), Some(events)) => registry.receive(caller, capability, bytes, events),
            _ => logos_abi::IpcStatus::Disconnected,
        };
        if status == logos_abi::IpcStatus::Ok {
            if let Some(event) = event {
                self.signal_dynamic_event_waiters(event);
            }
        }
        status
    }

    fn core_capability(
        &self,
        endpoint: logos_abi::EndpointHandle,
        rights: logos_abi::IpcRights,
    ) -> Result<logos_abi::CapabilityHandle, logos_abi::IpcStatus> {
        let core = dynamic_core_handle((self.service_epoch as u32).max(1))
            .map_err(|_| logos_abi::IpcStatus::Stale)?;
        self.dynamic_ipc
            .as_ref()
            .ok_or(logos_abi::IpcStatus::Disconnected)?
            .capability_for(core, endpoint, rights)
    }

    fn dynamic_endpoint(
        &self,
        producer: Option<ServiceId>,
        consumer: Option<ServiceId>,
        contract_id: u16,
    ) -> Result<logos_abi::EndpointHandle, logos_abi::IpcStatus> {
        let core = dynamic_core_handle((self.service_epoch as u32).max(1))
            .map_err(|_| logos_abi::IpcStatus::Stale)?;
        let producer = match producer {
            Some(service) => {
                self.runtime_service_handle(service).map_err(|_| logos_abi::IpcStatus::Stale)?
            }
            None => core,
        };
        let consumer = match consumer {
            Some(service) => {
                self.runtime_service_handle(service).map_err(|_| logos_abi::IpcStatus::Stale)?
            }
            None => core,
        };
        self.dynamic_ipc.as_ref().ok_or(logos_abi::IpcStatus::Disconnected)?.find_endpoint(
            producer,
            consumer,
            contract_id,
        )
    }

    fn send_dynamic(
        &mut self,
        caller: logos_abi::ServiceHandle,
        capability: logos_abi::CapabilityHandle,
        bytes: &[u8],
    ) -> logos_abi::IpcStatus {
        let event = self.dynamic_ipc.as_ref().and_then(|registry| {
            registry
                .capability_endpoint(caller, capability, logos_abi::IpcRights::Send)
                .ok()
                .and_then(|(endpoint, _)| registry.endpoint_events(endpoint).ok())
                .map(|(read, _)| read)
        });
        let status = match (self.dynamic_ipc.as_mut(), self.dynamic_events.as_mut()) {
            (Some(registry), Some(events)) => registry.send(caller, capability, bytes, events),
            _ => logos_abi::IpcStatus::Disconnected,
        };
        if status == logos_abi::IpcStatus::Ok {
            if let Some(event) = event {
                self.signal_dynamic_event_waiters(event);
            }
        }
        status
    }

    fn dynamic_contract_matches(
        &self,
        endpoint: logos_abi::EndpointHandle,
        producer: Option<ServiceId>,
        consumer: Option<ServiceId>,
        contract_id: u16,
    ) -> bool {
        self.dynamic_endpoint(producer, consumer, contract_id).ok() == Some(endpoint)
    }

    fn endpoint_requires_core_dispatch(&self, endpoint: logos_abi::EndpointHandle) -> bool {
        [
            (Some(ServiceId::Storage), None, logos_abi::IPC_CONTRACT_STORAGE_REQUEST),
            (None, Some(ServiceId::Storage), logos_abi::IPC_CONTRACT_STORAGE_RESPONSE),
            (Some(ServiceId::Network), None, logos_abi::IPC_CONTRACT_PACKET),
            (None, Some(ServiceId::Network), logos_abi::IPC_CONTRACT_PACKET),
            (Some(ServiceId::Device), None, logos_abi::IPC_CONTRACT_DEVICE_REQUEST),
            (None, Some(ServiceId::Device), logos_abi::IPC_CONTRACT_DEVICE_RESPONSE),
            (None, Some(ServiceId::Storage), logos_abi::IPC_CONTRACT_PACKAGE_REQUEST),
            (Some(ServiceId::Storage), None, logos_abi::IPC_CONTRACT_PACKAGE_RESPONSE),
            (Some(ServiceId::Storage), None, logos_abi::IPC_CONTRACT_STORAGE_MAP_REQUEST),
            (None, Some(ServiceId::Storage), logos_abi::IPC_CONTRACT_STORAGE_MAP_RESPONSE),
        ]
        .into_iter()
        .any(|(producer, consumer, contract)| {
            self.dynamic_contract_matches(endpoint, producer, consumer, contract)
        })
    }

    pub(crate) fn ipc_send(
        &mut self,
        process: ProcessHandle,
        capability_raw: u64,
        length: usize,
    ) -> crate::service_ipc::IpcOutcome {
        let Some(service) = self.service_for_process(process) else {
            return crate::service_ipc::IpcOutcome {
                status: logos_abi::IpcStatus::Unauthorized,
                notified: false,
            };
        };
        if self.dynamic_service_state(service)
            != Some(crate::runtime_services::ServiceState::Running)
        {
            return crate::service_ipc::IpcOutcome {
                status: logos_abi::IpcStatus::Disconnected,
                notified: false,
            };
        }
        if logos_abi::CapabilityHandle::from_raw(capability_raw).is_none() {
            return crate::service_ipc::IpcOutcome {
                status: logos_abi::IpcStatus::Unauthorized,
                notified: false,
            };
        }
        let dynamic_capability = logos_abi::CapabilityHandle::from_raw(capability_raw)
            .ok_or(logos_abi::IpcStatus::Unauthorized);
        let dynamic_capability = match dynamic_capability {
            Ok(capability) => capability,
            Err(status) => {
                return crate::service_ipc::IpcOutcome { status, notified: false };
            }
        };
        let caller = match self.runtime_service_handle(service) {
            Ok(caller) => caller,
            Err(_) => {
                return crate::service_ipc::IpcOutcome {
                    status: logos_abi::IpcStatus::Stale,
                    notified: false,
                };
            }
        };
        let (endpoint, expected_bytes) = match self
            .dynamic_ipc
            .as_ref()
            .ok_or(logos_abi::IpcStatus::Disconnected)
            .and_then(|registry| {
                registry.capability_endpoint(caller, dynamic_capability, logos_abi::IpcRights::Send)
            }) {
            Ok(resolved) => resolved,
            Err(status) => return crate::service_ipc::IpcOutcome { status, notified: false },
        };
        if self.dynamic_contract_matches(
            endpoint,
            Some(ServiceId::Device),
            None,
            logos_abi::IPC_CONTRACT_DEVICE_REQUEST,
        ) {
            if length != expected_bytes {
                return crate::service_ipc::IpcOutcome {
                    status: logos_abi::IpcStatus::Malformed,
                    notified: false,
                };
            }
            return self.dynamic_device_request(service, caller, dynamic_capability);
        }
        let disabled_flow_network = !self.network_config.is_enabled()
            && self.dynamic_contract_matches(
                endpoint,
                Some(ServiceId::Flow),
                Some(ServiceId::Network),
                logos_abi::IPC_CONTRACT_BYTES,
            );
        if !self.endpoint_requires_core_dispatch(endpoint) && !disabled_flow_network {
            if length != expected_bytes {
                return crate::service_ipc::IpcOutcome {
                    status: logos_abi::IpcStatus::Malformed,
                    notified: false,
                };
            }
            let Some(staging_frame) = self.ipc_staging_frames[service.index()] else {
                return crate::service_ipc::IpcOutcome {
                    status: logos_abi::IpcStatus::Unauthorized,
                    notified: false,
                };
            };
            let bytes = unsafe {
                core::slice::from_raw_parts(staging_frame.raw() as usize as *const u8, length)
            };
            #[cfg(feature = "storage-proof")]
            if service == ServiceId::Flow
                && self.dynamic_contract_matches(
                    endpoint,
                    Some(ServiceId::Flow),
                    Some(ServiceId::Storage),
                    logos_abi::IPC_CONTRACT_BYTES,
                )
            {
                self.storage_proof.observe_request(bytes);
            }
            let status = self.send_dynamic(caller, dynamic_capability, bytes);
            return crate::service_ipc::IpcOutcome {
                status,
                notified: status == logos_abi::IpcStatus::Ok,
            };
        }
        if self.endpoint_requires_core_dispatch(endpoint) {
            let status = self.dynamic_core_request(
                service,
                caller,
                dynamic_capability,
                endpoint,
                expected_bytes,
            );
            if status != logos_abi::IpcStatus::Ok {
                return crate::service_ipc::IpcOutcome { status, notified: false };
            }
        }
        if length != expected_bytes {
            return crate::service_ipc::IpcOutcome {
                status: logos_abi::IpcStatus::Malformed,
                notified: false,
            };
        }
        let Some(staging_frame) = self.ipc_staging_frames[service.index()] else {
            return crate::service_ipc::IpcOutcome {
                status: logos_abi::IpcStatus::Unauthorized,
                notified: false,
            };
        };
        if service == ServiceId::Network
            && self.dynamic_contract_matches(
                endpoint,
                Some(ServiceId::Network),
                None,
                logos_abi::IPC_CONTRACT_PACKET,
            )
        {
            if length != core::mem::size_of::<logos_abi::NetworkPacketDescriptor>() {
                return crate::service_ipc::IpcOutcome {
                    status: logos_abi::IpcStatus::Malformed,
                    notified: false,
                };
            }
            let wire = unsafe {
                core::slice::from_raw_parts(
                    staging_frame.raw() as usize as *const u8,
                    core::mem::size_of::<logos_abi::NetworkPacketDescriptor>(),
                )
            };
            if !logos_abi::NetworkPacketDescriptor::wire_enums_valid(wire) {
                return crate::service_ipc::IpcOutcome {
                    status: logos_abi::IpcStatus::Malformed,
                    notified: false,
                };
            }
            let descriptor: logos_abi::NetworkPacketDescriptor =
                unsafe { core::ptr::read_unaligned(staging_frame.raw() as usize as *const _) };
            if !descriptor.is_valid() {
                return crate::service_ipc::IpcOutcome {
                    status: logos_abi::IpcStatus::Malformed,
                    notified: false,
                };
            }
            if descriptor.generation != 0 && descriptor.generation != self.ipc_generation {
                return crate::service_ipc::IpcOutcome {
                    status: logos_abi::IpcStatus::Stale,
                    notified: false,
                };
            }
            if descriptor.service_epoch != 0 && descriptor.service_epoch != self.service_epoch {
                return crate::service_ipc::IpcOutcome {
                    status: logos_abi::IpcStatus::Stale,
                    notified: false,
                };
            }
            let status = match descriptor.operation {
                logos_abi::NetworkPacketOperation::SubmitTx => {
                    if descriptor.page < logos_abi::NETWORK_RX_PACKET_PAGES as u16 {
                        logos_abi::IpcStatus::Malformed
                    } else {
                        let Some(frame) = self.network_packet_frames[descriptor.page as usize]
                        else {
                            return crate::service_ipc::IpcOutcome {
                                status: logos_abi::IpcStatus::Unauthorized,
                                notified: false,
                            };
                        };
                        if crate::arch::submit_network_frame(
                            frame.raw() as usize,
                            descriptor.length as usize,
                        ) {
                            #[cfg(feature = "qemu-proof")]
                            crate::arch_proof_line(b"LogOS vNext: network tx submitted");
                            logos_abi::IpcStatus::Ok
                        } else {
                            logos_abi::IpcStatus::Full
                        }
                    }
                }
                logos_abi::NetworkPacketOperation::Reset => {
                    crate::arch::reset_network_device();
                    logos_abi::IpcStatus::Ok
                }
                logos_abi::NetworkPacketOperation::RecycleRx
                | logos_abi::NetworkPacketOperation::LinkState => logos_abi::IpcStatus::Ok,
            };
            return crate::service_ipc::IpcOutcome { status, notified: false };
        }
        if service == ServiceId::Flow
            && self.dynamic_contract_matches(
                endpoint,
                Some(ServiceId::Flow),
                Some(ServiceId::Network),
                logos_abi::IPC_CONTRACT_BYTES,
            )
            && self.dynamic_service_state(ServiceId::Network)
                == Some(crate::runtime_services::ServiceState::Disabled)
        {
            if length != core::mem::size_of::<logos_abi::IpcBytes>() {
                return crate::service_ipc::IpcOutcome {
                    status: logos_abi::IpcStatus::Malformed,
                    notified: false,
                };
            }
            let wire = unsafe {
                core::slice::from_raw_parts(
                    staging_frame.raw() as usize as *const u8,
                    core::mem::size_of::<logos_abi::IpcBytes>(),
                )
            };
            if !logos_abi::IpcBytes::wire_enums_valid(wire) {
                return crate::service_ipc::IpcOutcome {
                    status: logos_abi::IpcStatus::Malformed,
                    notified: false,
                };
            }
            let request_message = unsafe {
                core::ptr::read_unaligned(staging_frame.raw() as usize as *const logos_abi::IpcBytes)
            };
            if request_message.kind != logos_abi::MessageKind::NetworkRequest
                || request_message.len as usize != core::mem::size_of::<logos_abi::NetworkRequest>()
            {
                return crate::service_ipc::IpcOutcome {
                    status: logos_abi::IpcStatus::Malformed,
                    notified: false,
                };
            }
            if !logos_abi::NetworkRequest::wire_enums_valid(
                &request_message.bytes[..core::mem::size_of::<logos_abi::NetworkRequest>()],
            ) {
                return crate::service_ipc::IpcOutcome {
                    status: logos_abi::IpcStatus::Malformed,
                    notified: false,
                };
            }
            let request: logos_abi::NetworkRequest =
                unsafe { core::ptr::read_unaligned(request_message.bytes.as_ptr().cast()) };
            let response = logos_abi::NetworkResponse::new(
                request.operation,
                logos_abi::NetworkResult::Disabled,
                logos_abi::NetworkState::Disabled,
                request.request_id,
            );
            let response_bytes = unsafe {
                core::slice::from_raw_parts(
                    (&response as *const logos_abi::NetworkResponse).cast::<u8>(),
                    core::mem::size_of::<logos_abi::NetworkResponse>(),
                )
            };
            let response_message = logos_abi::IpcBytes::from_bytes(
                logos_abi::MessageKind::NetworkResponse,
                response_bytes,
            )
            .ok_or(crate::service_ipc::IpcError::Capacity)
            .map_err(ServiceRuntimeError::Ipc)
            .unwrap_or_else(|_| {
                logos_abi::IpcBytes::empty(logos_abi::MessageKind::NetworkResponse)
            });
            unsafe {
                core::ptr::write_unaligned(
                    staging_frame.raw() as usize as *mut logos_abi::IpcBytes,
                    response_message,
                );
            }
            let bytes = unsafe {
                core::slice::from_raw_parts(
                    staging_frame.raw() as usize as *const u8,
                    core::mem::size_of::<logos_abi::IpcBytes>(),
                )
            };
            let network = match self.runtime_service_handle(ServiceId::Network) {
                Ok(network) => network,
                Err(_) => {
                    return crate::service_ipc::IpcOutcome {
                        status: logos_abi::IpcStatus::Stale,
                        notified: false,
                    };
                }
            };
            let Some(endpoint) = self
                .dynamic_endpoint(
                    Some(ServiceId::Network),
                    Some(ServiceId::Flow),
                    logos_abi::IPC_CONTRACT_PACKET,
                )
                .ok()
            else {
                return crate::service_ipc::IpcOutcome {
                    status: logos_abi::IpcStatus::Disconnected,
                    notified: false,
                };
            };
            let Some(response_capability) = self.dynamic_ipc.as_ref().and_then(|registry| {
                registry.capability_for(network, endpoint, logos_abi::IpcRights::Send).ok()
            }) else {
                return crate::service_ipc::IpcOutcome {
                    status: logos_abi::IpcStatus::Unauthorized,
                    notified: false,
                };
            };
            let status = self.send_dynamic(network, response_capability, bytes);
            return crate::service_ipc::IpcOutcome {
                status,
                notified: status == logos_abi::IpcStatus::Ok,
            };
        }
        if service == ServiceId::Storage
            && self.dynamic_contract_matches(
                endpoint,
                Some(ServiceId::Storage),
                None,
                logos_abi::IPC_CONTRACT_STORAGE_MAP_REQUEST,
            )
        {
            if length != core::mem::size_of::<logos_abi::StorageMapRequest>() {
                return crate::service_ipc::IpcOutcome {
                    status: logos_abi::IpcStatus::Malformed,
                    notified: false,
                };
            }
            if self.storage_map_response.is_some() {
                return crate::service_ipc::IpcOutcome {
                    status: logos_abi::IpcStatus::Full,
                    notified: false,
                };
            }
            let request = unsafe {
                core::ptr::read_unaligned(
                    staging_frame.raw() as usize as *const logos_abi::StorageMapRequest
                )
            };
            let status = if request.operation == crate::storage_ipc::STORAGE_MAP_OPERATION {
                if request.request_id == 0
                    || request.reserved != 0
                    || request.reserved_tail != 0
                    || request.pages > u8::MAX as u16
                {
                    Err(logos_abi::StorageStatus::Invalid)
                } else {
                    crate::storage_ipc::validate_map_descriptor(
                        request.generation,
                        request.client,
                        request.source_page,
                        request.pages as u8,
                        request.flags,
                        self.service_epoch,
                    )
                }
            } else if request.operation == crate::storage_ipc::STORAGE_UNMAP_OPERATION {
                if request.request_id == 0
                    || request.generation != self.service_epoch
                    || request.reserved != 0
                    || request.reserved_tail != 0
                    || request.pages != 0
                    || request.source_page != 0
                {
                    Err(logos_abi::StorageStatus::Stale)
                } else if crate::storage_ipc::storage_map_client_slot(request.client).is_none()
                    || request.flags != 0
                    || request.target_page == 0
                    || request.window_generation == 0
                {
                    Err(logos_abi::StorageStatus::Unauthorized)
                } else {
                    Ok(())
                }
            } else {
                Err(logos_abi::StorageStatus::Invalid)
            };
            let response = match status {
                Ok(()) => {
                    if request.operation == crate::storage_ipc::STORAGE_UNMAP_OPERATION {
                        let status = self
                            .unmap_storage_window(crate::storage_ipc::StorageMapRelease {
                                generation: request.generation,
                                client: request.client,
                                target_page: request.target_page,
                                window_generation: request.window_generation,
                            })
                            .map_or(logos_abi::StorageStatus::Io, |_| logos_abi::StorageStatus::Ok);
                        logos_abi::StorageMapResponse {
                            request_id: request.request_id,
                            status,
                            reserved: [0; 3],
                            generation: request.generation,
                            target_page: 0,
                            pages: 0,
                            reserved_tail: [0; 7],
                            window_generation: 0,
                            reserved_end: [0; 4],
                        }
                    } else {
                        let mut descriptor = [0u8; logos_abi::STORAGE_API_MAP_DESCRIPTOR_BYTES];
                        descriptor[..8].copy_from_slice(&request.source_page.to_le_bytes());
                        descriptor[8] = request.pages as u8;
                        match self.map_storage_descriptor(
                            request.generation,
                            request.client,
                            &descriptor,
                        ) {
                            Ok(mapped) => logos_abi::StorageMapResponse {
                                request_id: request.request_id,
                                status: logos_abi::StorageStatus::Ok,
                                reserved: [0; 3],
                                generation: mapped.generation,
                                target_page: mapped.target_page,
                                pages: mapped.pages,
                                reserved_tail: [0; 7],
                                window_generation: mapped.window_generation,
                                reserved_end: [0; 4],
                            },
                            Err(error) => logos_abi::StorageMapResponse {
                                request_id: request.request_id,
                                status: if error == PageTableError::Capacity {
                                    logos_abi::StorageStatus::Full
                                } else {
                                    logos_abi::StorageStatus::Io
                                },
                                reserved: [0; 3],
                                generation: request.generation,
                                target_page: 0,
                                pages: 0,
                                reserved_tail: [0; 7],
                                window_generation: 0,
                                reserved_end: [0; 4],
                            },
                        }
                    }
                }
                Err(status) => logos_abi::StorageMapResponse {
                    request_id: request.request_id,
                    status,
                    reserved: [0; 3],
                    generation: request.generation,
                    target_page: 0,
                    pages: 0,
                    reserved_tail: [0; 7],
                    window_generation: 0,
                    reserved_end: [0; 4],
                },
            };
            let Some(response_endpoint) = self
                .dynamic_endpoint(
                    None,
                    Some(ServiceId::Storage),
                    logos_abi::IPC_CONTRACT_STORAGE_MAP_RESPONSE,
                )
                .ok()
            else {
                return crate::service_ipc::IpcOutcome {
                    status: logos_abi::IpcStatus::Disconnected,
                    notified: false,
                };
            };
            if let Err(status) = self.core_capability(response_endpoint, logos_abi::IpcRights::Send)
            {
                return crate::service_ipc::IpcOutcome { status, notified: false };
            }
            unsafe {
                core::ptr::write_unaligned(
                    staging_frame.raw() as usize as *mut logos_abi::StorageMapResponse,
                    response,
                );
            }
            let status = self.queue_dynamic_core_response(
                service,
                response_endpoint,
                core::mem::size_of::<logos_abi::StorageMapResponse>(),
            );
            return crate::service_ipc::IpcOutcome {
                status,
                notified: status == logos_abi::IpcStatus::Ok,
            };
        }
        if service == ServiceId::Storage
            && self.dynamic_contract_matches(
                endpoint,
                Some(ServiceId::Storage),
                None,
                logos_abi::IPC_CONTRACT_STORAGE_REQUEST,
            )
        {
            if length != core::mem::size_of::<logos_abi::StorageRequest>() {
                return crate::service_ipc::IpcOutcome {
                    status: logos_abi::IpcStatus::Malformed,
                    notified: false,
                };
            }
            if self.storage_response.is_some() {
                return crate::service_ipc::IpcOutcome {
                    status: logos_abi::IpcStatus::Full,
                    notified: false,
                };
            }
            let bytes = staging_frame.raw() as usize as *const u8;
            let operation = unsafe { *bytes };
            if !(1..=9).contains(&operation) {
                return crate::service_ipc::IpcOutcome {
                    status: logos_abi::IpcStatus::Malformed,
                    notified: false,
                };
            }
            let request = unsafe { core::ptr::read_unaligned(bytes.cast()) };
            let request = match crate::storage_ipc::validate_request(
                request,
                self.ipc_generation,
                self.service_epoch,
            ) {
                Ok(()) => request,
                Err(logos_abi::StorageStatus::Unauthorized) => {
                    return crate::service_ipc::IpcOutcome {
                        status: logos_abi::IpcStatus::Unauthorized,
                        notified: false,
                    };
                }
                Err(logos_abi::StorageStatus::Stale) => {
                    return crate::service_ipc::IpcOutcome {
                        status: logos_abi::IpcStatus::Stale,
                        notified: false,
                    };
                }
                Err(_) => {
                    return crate::service_ipc::IpcOutcome {
                        status: logos_abi::IpcStatus::Malformed,
                        notified: false,
                    };
                }
            };
            let response = match request.operation {
                logos_abi::StorageOperation::Reopen => match crate::arch::storage_block_count() {
                    Ok(block_count) => logos_abi::StorageResponse::new(
                        request.request_id,
                        logos_abi::StorageStatus::Ok,
                        request.generation,
                        0,
                        0,
                        request.transaction_id,
                    )
                    .with_block_count(block_count),
                    Err(status) => logos_abi::StorageResponse::new(
                        request.request_id,
                        status,
                        request.generation,
                        0,
                        0,
                        request.transaction_id,
                    ),
                },
                logos_abi::StorageOperation::Read | logos_abi::StorageOperation::Write => {
                    if request.blocks != 1
                        || request.payload_bytes as usize != logos_storage::BLOCK_BYTES
                    {
                        logos_abi::StorageResponse::new(
                            request.request_id,
                            logos_abi::StorageStatus::Invalid,
                            request.generation,
                            0,
                            0,
                            request.transaction_id,
                        )
                    } else if let Some(data) = self.storage_data_frames[0] {
                        let status =
                            match crate::arch::transfer_storage_block(request, data.raw() as usize)
                            {
                                Ok(()) => logos_abi::StorageStatus::Ok,
                                Err(status) => status,
                            };
                        logos_abi::StorageResponse::new(
                            request.request_id,
                            status,
                            request.generation,
                            if status == logos_abi::StorageStatus::Ok { 1 } else { 0 },
                            if status == logos_abi::StorageStatus::Ok {
                                logos_storage::BLOCK_BYTES as u16
                            } else {
                                0
                            },
                            request.transaction_id,
                        )
                    } else {
                        logos_abi::StorageResponse::new(
                            request.request_id,
                            logos_abi::StorageStatus::Io,
                            request.generation,
                            0,
                            0,
                            request.transaction_id,
                        )
                    }
                }
                logos_abi::StorageOperation::Flush => {
                    let status = crate::arch::flush_storage_device()
                        .map_or_else(|status| status, |_| logos_abi::StorageStatus::Ok);
                    logos_abi::StorageResponse::new(
                        request.request_id,
                        status,
                        request.generation,
                        0,
                        0,
                        request.transaction_id,
                    )
                }
                _ => crate::storage_ipc::unsupported_response(request),
            };
            let Some(response_endpoint) = self
                .dynamic_endpoint(
                    None,
                    Some(ServiceId::Storage),
                    logos_abi::IPC_CONTRACT_STORAGE_RESPONSE,
                )
                .ok()
            else {
                return crate::service_ipc::IpcOutcome {
                    status: logos_abi::IpcStatus::Disconnected,
                    notified: false,
                };
            };
            if let Err(status) = self.core_capability(response_endpoint, logos_abi::IpcRights::Send)
            {
                return crate::service_ipc::IpcOutcome { status, notified: false };
            }
            unsafe {
                core::ptr::write_unaligned(
                    staging_frame.raw() as usize as *mut logos_abi::StorageResponse,
                    response,
                );
            }
            let status = self.queue_dynamic_core_response(
                service,
                response_endpoint,
                core::mem::size_of::<logos_abi::StorageResponse>(),
            );
            return crate::service_ipc::IpcOutcome {
                status,
                notified: status == logos_abi::IpcStatus::Ok,
            };
        }
        if service == ServiceId::Device
            && self.dynamic_contract_matches(
                endpoint,
                Some(ServiceId::Device),
                None,
                logos_abi::IPC_CONTRACT_DEVICE_REQUEST,
            )
        {
            if length != core::mem::size_of::<logos_abi::DeviceRequest>() {
                return crate::service_ipc::IpcOutcome {
                    status: logos_abi::IpcStatus::Malformed,
                    notified: false,
                };
            }
            if self.device_response.is_some() {
                return crate::service_ipc::IpcOutcome {
                    status: logos_abi::IpcStatus::Full,
                    notified: false,
                };
            }
            let wire = unsafe {
                core::slice::from_raw_parts(
                    staging_frame.raw() as usize as *const u8,
                    core::mem::size_of::<logos_abi::DeviceRequest>(),
                )
            };
            if !logos_abi::DeviceRequest::wire_enums_valid(wire) {
                return crate::service_ipc::IpcOutcome {
                    status: logos_abi::IpcStatus::Malformed,
                    notified: false,
                };
            }
            let request = unsafe {
                core::ptr::read_unaligned(
                    staging_frame.raw() as usize as *const logos_abi::DeviceRequest
                )
            };
            if crate::device_ipc::validate_request(request, self.ipc_generation, self.service_epoch)
                .is_err()
            {
                return crate::service_ipc::IpcOutcome {
                    status: logos_abi::IpcStatus::Malformed,
                    notified: false,
                };
            }
            let response = match request.operation {
                logos_abi::DeviceOperation::List => crate::arch::device_list_response(
                    request,
                    self.ipc_generation,
                    self.service_epoch,
                ),
            };
            self.device_response = Some(response);
            return crate::service_ipc::IpcOutcome {
                status: logos_abi::IpcStatus::Ok,
                notified: true,
            };
        }
        if service == ServiceId::Storage
            && self.dynamic_contract_matches(
                endpoint,
                Some(ServiceId::Storage),
                None,
                logos_abi::IPC_CONTRACT_PACKAGE_RESPONSE,
            )
        {
            if length != core::mem::size_of::<logos_abi::PackageResponse>() {
                return crate::service_ipc::IpcOutcome {
                    status: logos_abi::IpcStatus::Malformed,
                    notified: false,
                };
            }
            if self.package_response_slot().is_some() {
                return crate::service_ipc::IpcOutcome {
                    status: logos_abi::IpcStatus::Full,
                    notified: false,
                };
            }
            let wire = unsafe {
                core::slice::from_raw_parts(
                    staging_frame.raw() as usize as *const u8,
                    core::mem::size_of::<logos_abi::PackageResponse>(),
                )
            };
            if !logos_abi::PackageResponse::wire_enums_valid(wire) {
                return crate::service_ipc::IpcOutcome {
                    status: logos_abi::IpcStatus::Malformed,
                    notified: false,
                };
            }
            let response = unsafe {
                core::ptr::read_unaligned(
                    staging_frame.raw() as usize as *const logos_abi::PackageResponse
                )
            };
            let Some(request) = self.package_request_slot() else {
                return crate::service_ipc::IpcOutcome {
                    status: logos_abi::IpcStatus::Malformed,
                    notified: false,
                };
            };
            if response.validate_for(request, self.ipc_generation, self.service_epoch).is_err() {
                return crate::service_ipc::IpcOutcome {
                    status: logos_abi::IpcStatus::Malformed,
                    notified: false,
                };
            }
            self.set_package_response_slot(Some(response));
            return crate::service_ipc::IpcOutcome {
                status: logos_abi::IpcStatus::Ok,
                notified: true,
            };
        }
        let Some(staging_frame) = self.ipc_staging_frames[service.index()] else {
            return crate::service_ipc::IpcOutcome {
                status: logos_abi::IpcStatus::Unauthorized,
                notified: false,
            };
        };
        let bytes = unsafe {
            core::slice::from_raw_parts(staging_frame.raw() as usize as *const u8, length)
        };
        #[cfg(feature = "storage-proof")]
        if service == ServiceId::Flow
            && self.dynamic_contract_matches(
                endpoint,
                Some(ServiceId::Flow),
                Some(ServiceId::Storage),
                logos_abi::IPC_CONTRACT_BYTES,
            )
        {
            self.storage_proof.observe_request(bytes);
        }
        let status = self.send_dynamic(caller, dynamic_capability, bytes);
        crate::service_ipc::IpcOutcome { status, notified: status == logos_abi::IpcStatus::Ok }
    }

    pub(crate) fn ipc_receive(
        &mut self,
        process: ProcessHandle,
        capability_raw: u64,
    ) -> crate::service_ipc::IpcOutcome {
        let Some(service) = self.service_for_process(process) else {
            return crate::service_ipc::IpcOutcome {
                status: logos_abi::IpcStatus::Unauthorized,
                notified: false,
            };
        };
        if self.dynamic_service_state(service)
            != Some(crate::runtime_services::ServiceState::Running)
        {
            return crate::service_ipc::IpcOutcome {
                status: logos_abi::IpcStatus::Disconnected,
                notified: false,
            };
        }
        if logos_abi::CapabilityHandle::from_raw(capability_raw).is_none() {
            return crate::service_ipc::IpcOutcome {
                status: logos_abi::IpcStatus::Unauthorized,
                notified: false,
            };
        }
        let Some(dynamic_capability) = logos_abi::CapabilityHandle::from_raw(capability_raw) else {
            return crate::service_ipc::IpcOutcome {
                status: logos_abi::IpcStatus::Unauthorized,
                notified: false,
            };
        };
        {
            let caller = match self.runtime_service_handle(service) {
                Ok(caller) => caller,
                Err(_) => {
                    return crate::service_ipc::IpcOutcome {
                        status: logos_abi::IpcStatus::Stale,
                        notified: false,
                    };
                }
            };
            let (endpoint, expected_bytes) = match self
                .dynamic_ipc
                .as_ref()
                .ok_or(logos_abi::IpcStatus::Disconnected)
                .and_then(|registry| {
                    registry.capability_endpoint(
                        caller,
                        dynamic_capability,
                        logos_abi::IpcRights::Receive,
                    )
                }) {
                Ok(resolved) => resolved,
                Err(status) => return crate::service_ipc::IpcOutcome { status, notified: false },
            };
            if self.dynamic_contract_matches(
                endpoint,
                None,
                Some(ServiceId::Device),
                logos_abi::IPC_CONTRACT_DEVICE_RESPONSE,
            ) {
                let Some(staging_frame) = self.ipc_staging_frames[service.index()] else {
                    return crate::service_ipc::IpcOutcome {
                        status: logos_abi::IpcStatus::Unauthorized,
                        notified: false,
                    };
                };
                let bytes = unsafe {
                    core::slice::from_raw_parts_mut(
                        staging_frame.raw() as usize as *mut u8,
                        expected_bytes,
                    )
                };
                let status = self.receive_dynamic(caller, dynamic_capability, bytes);
                return crate::service_ipc::IpcOutcome {
                    status,
                    notified: status == logos_abi::IpcStatus::Ok,
                };
            }
            if self.dynamic_contract_matches(
                endpoint,
                None,
                Some(ServiceId::Storage),
                logos_abi::IPC_CONTRACT_STORAGE_RESPONSE,
            ) {
                let Some(staging_frame) = self.ipc_staging_frames[service.index()] else {
                    return crate::service_ipc::IpcOutcome {
                        status: logos_abi::IpcStatus::Unauthorized,
                        notified: false,
                    };
                };
                let bytes = unsafe {
                    core::slice::from_raw_parts_mut(
                        staging_frame.raw() as usize as *mut u8,
                        expected_bytes,
                    )
                };
                let status = self.receive_dynamic(caller, dynamic_capability, bytes);
                return crate::service_ipc::IpcOutcome {
                    status,
                    notified: status == logos_abi::IpcStatus::Ok,
                };
            }
            if self.dynamic_contract_matches(
                endpoint,
                None,
                Some(ServiceId::Storage),
                logos_abi::IPC_CONTRACT_STORAGE_MAP_RESPONSE,
            ) {
                let Some(staging_frame) = self.ipc_staging_frames[service.index()] else {
                    return crate::service_ipc::IpcOutcome {
                        status: logos_abi::IpcStatus::Unauthorized,
                        notified: false,
                    };
                };
                let bytes = unsafe {
                    core::slice::from_raw_parts_mut(
                        staging_frame.raw() as usize as *mut u8,
                        expected_bytes,
                    )
                };
                let status = self.receive_dynamic(caller, dynamic_capability, bytes);
                return crate::service_ipc::IpcOutcome {
                    status,
                    notified: status == logos_abi::IpcStatus::Ok,
                };
            }
            if self.dynamic_contract_matches(
                endpoint,
                None,
                Some(ServiceId::Storage),
                logos_abi::IPC_CONTRACT_PACKAGE_REQUEST,
            ) {
                let Some(staging_frame) = self.ipc_staging_frames[service.index()] else {
                    return crate::service_ipc::IpcOutcome {
                        status: logos_abi::IpcStatus::Unauthorized,
                        notified: false,
                    };
                };
                let bytes = unsafe {
                    core::slice::from_raw_parts_mut(
                        staging_frame.raw() as usize as *mut u8,
                        expected_bytes,
                    )
                };
                let status = self.receive_dynamic(caller, dynamic_capability, bytes);
                return crate::service_ipc::IpcOutcome {
                    status,
                    notified: status == logos_abi::IpcStatus::Ok,
                };
            }
            if self.dynamic_contract_matches(
                endpoint,
                None,
                Some(ServiceId::Network),
                logos_abi::IPC_CONTRACT_PACKET,
            ) {
                let Some(staging_frame) = self.ipc_staging_frames[service.index()] else {
                    return crate::service_ipc::IpcOutcome {
                        status: logos_abi::IpcStatus::Unauthorized,
                        notified: false,
                    };
                };
                let response = if let Some(response) = self.network_packet_response.take() {
                    #[cfg(feature = "qemu-proof")]
                    crate::arch_proof_line(b"LogOS vNext: network link delivered");
                    response
                } else {
                    let Some(frame) = self.network_packet_frames.first().and_then(|frame| *frame)
                    else {
                        return crate::service_ipc::IpcOutcome {
                            status: logos_abi::IpcStatus::Unauthorized,
                            notified: false,
                        };
                    };
                    let Some(length) = crate::arch::take_network_frame(frame.raw() as usize) else {
                        return crate::service_ipc::IpcOutcome {
                            status: logos_abi::IpcStatus::Empty,
                            notified: false,
                        };
                    };
                    let mut response = logos_abi::NetworkPacketDescriptor::new(
                        logos_abi::NetworkPacketOperation::RecycleRx,
                        0,
                        self.network_packet_sequence,
                    );
                    response.length = length as u16;
                    response.generation = self.ipc_generation;
                    response.service_epoch = self.service_epoch;
                    self.network_packet_sequence =
                        self.network_packet_sequence.wrapping_add(1).max(1);
                    response
                };
                unsafe {
                    core::ptr::write_unaligned(
                        staging_frame.raw() as usize as *mut logos_abi::NetworkPacketDescriptor,
                        response,
                    );
                }
                let status = self.queue_dynamic_core_response(
                    service,
                    endpoint,
                    core::mem::size_of::<logos_abi::NetworkPacketDescriptor>(),
                );
                if status != logos_abi::IpcStatus::Ok {
                    self.network_packet_response = Some(response);
                    return crate::service_ipc::IpcOutcome { status, notified: false };
                }
                let bytes = unsafe {
                    core::slice::from_raw_parts_mut(
                        staging_frame.raw() as usize as *mut u8,
                        expected_bytes,
                    )
                };
                let status = self.receive_dynamic(caller, dynamic_capability, bytes);
                return crate::service_ipc::IpcOutcome {
                    status,
                    notified: status == logos_abi::IpcStatus::Ok,
                };
            }
            {
                let Some(staging_frame) = self.ipc_staging_frames[service.index()] else {
                    return crate::service_ipc::IpcOutcome {
                        status: logos_abi::IpcStatus::Unauthorized,
                        notified: false,
                    };
                };
                let bytes = unsafe {
                    core::slice::from_raw_parts_mut(
                        staging_frame.raw() as usize as *mut u8,
                        expected_bytes,
                    )
                };
                let status = self.receive_dynamic(caller, dynamic_capability, bytes);
                if status == logos_abi::IpcStatus::Ok {
                    #[cfg(feature = "qemu-proof")]
                    if service == ServiceId::Flow
                        && self.dynamic_contract_matches(
                            endpoint,
                            Some(ServiceId::Network),
                            Some(ServiceId::Flow),
                            logos_abi::IPC_CONTRACT_BYTES,
                        )
                    {
                        if !logos_abi::IpcBytes::wire_enums_valid(bytes) {
                            return crate::service_ipc::IpcOutcome {
                                status: logos_abi::IpcStatus::Malformed,
                                notified: false,
                            };
                        }
                        let message = unsafe {
                            core::ptr::read_unaligned(bytes.as_ptr().cast::<logos_abi::IpcBytes>())
                        };
                        if message.kind == logos_abi::MessageKind::NetworkResponse
                            && message.len as usize
                                == core::mem::size_of::<logos_abi::NetworkResponse>()
                        {
                            if !logos_abi::NetworkResponse::wire_enums_valid(
                                &message.bytes
                                    [..core::mem::size_of::<logos_abi::NetworkResponse>()],
                            ) {
                                return crate::service_ipc::IpcOutcome {
                                    status: logos_abi::IpcStatus::Malformed,
                                    notified: false,
                                };
                            }
                            let response = unsafe {
                                core::ptr::read_unaligned(
                                    message.bytes.as_ptr().cast::<logos_abi::NetworkResponse>(),
                                )
                            };
                            if response.operation == logos_abi::NetworkOperation::TcpConnect
                                && response.result == logos_abi::NetworkResult::Ok
                            {
                                crate::proof::network_tcp_completed();
                            }
                            if response.operation == logos_abi::NetworkOperation::Close
                                && response.result == logos_abi::NetworkResult::Stale
                            {
                                crate::proof::network_stale_rejected();
                            }
                        }
                    }
                    #[cfg(feature = "storage-proof")]
                    if service == ServiceId::Flow
                        && self.dynamic_contract_matches(
                            endpoint,
                            Some(ServiceId::Storage),
                            Some(ServiceId::Flow),
                            logos_abi::IPC_CONTRACT_BYTES,
                        )
                    {
                        self.storage_proof.observe_response(bytes);
                    }
                }
                crate::service_ipc::IpcOutcome {
                    status,
                    notified: status == logos_abi::IpcStatus::Ok,
                }
            }
        }
    }

    pub(crate) fn directory_call(
        &mut self,
        process: ProcessHandle,
        capability_raw: u64,
        length: usize,
    ) -> logos_abi::DirectoryStatus {
        let Some(service) = self.service_for_process(process) else {
            return logos_abi::DirectoryStatus::Unauthorized;
        };
        if self.dynamic_service_state(service)
            != Some(crate::runtime_services::ServiceState::Running)
        {
            return logos_abi::DirectoryStatus::Stale;
        }
        if length != core::mem::size_of::<logos_abi::DirectoryRequest>() {
            return logos_abi::DirectoryStatus::Malformed;
        }
        let Some(staging_frame) = self.ipc_staging_frames[service.index()] else {
            return logos_abi::DirectoryStatus::Unauthorized;
        };
        let wire = unsafe {
            core::slice::from_raw_parts(
                staging_frame.raw() as usize as *const u8,
                core::mem::size_of::<logos_abi::DirectoryRequest>(),
            )
        };
        if !logos_abi::DirectoryRequest::wire_enums_valid(wire) {
            return logos_abi::DirectoryStatus::Malformed;
        }
        let request = unsafe {
            core::ptr::read_volatile(
                staging_frame.raw() as usize as *const logos_abi::DirectoryRequest
            )
        };
        if !request.is_valid() {
            return logos_abi::DirectoryStatus::Malformed;
        }
        let directory_generation = self.bootstrap_directory[service.index()].generation();
        if directory_generation == 0 {
            return logos_abi::DirectoryStatus::Stale;
        }
        let Some(service_handle) =
            logos_abi::ServiceHandle::new(service.index() as u32, directory_generation)
        else {
            return logos_abi::DirectoryStatus::Stale;
        };
        if request.subject != logos_abi::ServiceHandle::EMPTY && request.subject != service_handle {
            return logos_abi::DirectoryStatus::Unauthorized;
        }
        let Some(directory) = logos_abi::CapabilityHandle::from_raw(capability_raw) else {
            return logos_abi::DirectoryStatus::Unauthorized;
        };
        let expected_directory = self.bootstrap_directory[service.index()];
        if !expected_directory.is_valid() {
            return logos_abi::DirectoryStatus::Stale;
        }
        if directory.generation() != directory_generation {
            return logos_abi::DirectoryStatus::Stale;
        }
        if directory != expected_directory {
            return logos_abi::DirectoryStatus::Unauthorized;
        }
        let mut dynamic_request = request;
        dynamic_request.subject = service_handle;
        let mut response = logos_abi::DirectoryResponse::empty(
            request.operation,
            logos_abi::DirectoryStatus::Malformed,
            request.request_id,
        );
        let status = match request.operation {
            logos_abi::DirectoryOperation::Capabilities => match self.dynamic_ipc.as_ref() {
                Some(registry) => registry.directory(dynamic_request, &mut response),
                None => logos_abi::DirectoryStatus::Stale,
            },
            logos_abi::DirectoryOperation::Endpoints => match self.dynamic_ipc.as_ref() {
                Some(registry) => registry.directory_endpoints(dynamic_request, &mut response),
                None => logos_abi::DirectoryStatus::Stale,
            },
            logos_abi::DirectoryOperation::Services => match self.dynamic_services.as_ref() {
                Some(registry) => registry.list(request.cursor, &mut response, request.request_id),
                None => logos_abi::DirectoryStatus::Stale,
            },
        };
        #[cfg(feature = "qemu-proof")]
        if status == logos_abi::DirectoryStatus::Ok {
            crate::proof::dynamic_directory_used();
        }
        core::sync::atomic::fence(Ordering::Release);
        unsafe {
            core::ptr::write_volatile(
                staging_frame.raw() as usize as *mut logos_abi::DirectoryResponse,
                response,
            );
        }
        status
    }

    pub(crate) fn manager_call(
        &mut self,
        process: ProcessHandle,
        capability_raw: u64,
        length: usize,
    ) -> logos_abi::IpcStatus {
        let Some(service) = self.service_for_process(process) else {
            return logos_abi::IpcStatus::Unauthorized;
        };
        if self.dynamic_service_state(service)
            != Some(crate::runtime_services::ServiceState::Running)
        {
            return logos_abi::IpcStatus::Stale;
        }
        let control = self.bootstrap_control[service.index()];
        if !control.is_valid() {
            return logos_abi::IpcStatus::Stale;
        }
        if capability_raw != control.raw() {
            return logos_abi::IpcStatus::Unauthorized;
        }
        if length != core::mem::size_of::<logos_abi::ManagerRequest>() {
            return logos_abi::IpcStatus::Malformed;
        }
        let Ok(service_handle) = self.runtime_service_handle(service) else {
            return logos_abi::IpcStatus::Stale;
        };
        let rights = self
            .dynamic_services
            .as_ref()
            .and_then(|registry| registry.manager_rights(service_handle).ok())
            .unwrap_or(logos_abi::ManagerRights::NONE);
        if rights == logos_abi::ManagerRights::NONE {
            return logos_abi::IpcStatus::Stale;
        }
        let Some(staging_frame) = self.ipc_staging_frames[service.index()] else {
            return logos_abi::IpcStatus::Unauthorized;
        };
        let bytes = staging_frame.raw() as usize as *const u8;
        let operation =
            unsafe { *bytes.add(core::mem::offset_of!(logos_abi::ManagerRequest, operation)) };
        if logos_abi::ManagerOperation::from_raw(operation).is_none() {
            return logos_abi::IpcStatus::Malformed;
        }
        let wire = unsafe {
            core::slice::from_raw_parts(bytes, core::mem::size_of::<logos_abi::ManagerRequest>())
        };
        if !logos_abi::ManagerRequest::wire_enums_valid(wire) {
            return logos_abi::IpcStatus::Malformed;
        }
        let request =
            unsafe { core::ptr::read_unaligned(bytes.cast::<logos_abi::ManagerRequest>()) };
        if matches!(
            request.operation,
            logos_abi::ManagerOperation::List
                | logos_abi::ManagerOperation::Status
                | logos_abi::ManagerOperation::Start
                | logos_abi::ManagerOperation::Stop
                | logos_abi::ManagerOperation::Restart
        ) && !crate::runtime_services::service_request_shape_valid(request)
        {
            let response = logos_abi::ManagerResponse::new(
                request.operation,
                logos_abi::ManagerStatus::Malformed,
                request.request_id,
            );
            unsafe {
                core::ptr::write_unaligned(
                    staging_frame.raw() as usize as *mut logos_abi::ManagerResponse,
                    response,
                );
            }
            return logos_abi::IpcStatus::Ok;
        }
        if matches!(
            request.operation,
            logos_abi::ManagerOperation::List | logos_abi::ManagerOperation::Status
        ) {
            let response = self
                .dynamic_services
                .as_ref()
                .map(|registry| registry.manager_request(request))
                .unwrap_or_else(|| {
                    logos_abi::ManagerResponse::new(
                        request.operation,
                        logos_abi::ManagerStatus::Stale,
                        request.request_id,
                    )
                });
            #[cfg(feature = "qemu-proof")]
            if matches!(
                response.status,
                logos_abi::ManagerStatus::Ok | logos_abi::ManagerStatus::Stale
            ) {
                crate::proof::dynamic_manager_used();
            }
            unsafe {
                core::ptr::write_unaligned(
                    staging_frame.raw() as usize as *mut logos_abi::ManagerResponse,
                    response,
                );
            }
            return logos_abi::IpcStatus::Ok;
        }
        let service_lifecycle = matches!(
            request.operation,
            logos_abi::ManagerOperation::Start
                | logos_abi::ManagerOperation::Stop
                | logos_abi::ManagerOperation::Restart
        );
        let mut decision = if service_lifecycle {
            let Some(registry) = self.dynamic_services.as_mut() else {
                let response = logos_abi::ManagerResponse::new(
                    request.operation,
                    logos_abi::ManagerStatus::Stale,
                    request.request_id,
                );
                unsafe {
                    core::ptr::write_unaligned(
                        staging_frame.raw() as usize as *mut logos_abi::ManagerResponse,
                        response,
                    );
                }
                return logos_abi::IpcStatus::Ok;
            };
            let action = match registry.begin_lifecycle_action(request.operation, request.service) {
                Ok(action) => action,
                Err(status) => {
                    let response = logos_abi::ManagerResponse::new(
                        request.operation,
                        status,
                        request.request_id,
                    );
                    unsafe {
                        core::ptr::write_unaligned(
                            staging_frame.raw() as usize as *mut logos_abi::ManagerResponse,
                            response,
                        );
                    }
                    return logos_abi::IpcStatus::Ok;
                }
            };
            let action = match action {
                crate::runtime_services::RuntimeLifecycleAction::Start(handle) => {
                    builtin_service_for_handle(&self.service_handles, handle)
                        .map(ManagerAction::Start)
                        .or_else(|| {
                            let _ =
                                registry.abort_lifecycle_members(core::slice::from_ref(&handle));
                            None
                        })
                }
                crate::runtime_services::RuntimeLifecycleAction::Stop(handle) => {
                    builtin_service_for_handle(&self.service_handles, handle)
                        .map(ManagerAction::Stop)
                        .or_else(|| {
                            let _ =
                                registry.abort_lifecycle_members(core::slice::from_ref(&handle));
                            None
                        })
                }
                crate::runtime_services::RuntimeLifecycleAction::Restart(handles) => {
                    let mut services = Vec::new();
                    if services.try_reserve(handles.len()).is_err() {
                        let _ = registry.abort_lifecycle_members(&handles);
                        None
                    } else {
                        let mut valid = true;
                        for handle in &handles {
                            let Some(service) =
                                builtin_service_for_handle(&self.service_handles, *handle)
                            else {
                                valid = false;
                                break;
                            };
                            services.push(service);
                        }
                        if valid {
                            Some(ManagerAction::Restart(services))
                        } else {
                            let _ = registry.abort_lifecycle_members(&handles);
                            None
                        }
                    }
                }
            };
            let Some(action) = action else {
                let _ = registry.abort_lifecycle(request.operation, request.service);
                let response = logos_abi::ManagerResponse::new(
                    request.operation,
                    logos_abi::ManagerStatus::Unsupported,
                    request.request_id,
                );
                unsafe {
                    core::ptr::write_unaligned(
                        staging_frame.raw() as usize as *mut logos_abi::ManagerResponse,
                        response,
                    );
                }
                return logos_abi::IpcStatus::Ok;
            };
            let mut status_request = logos_abi::ManagerRequest::new(
                logos_abi::ManagerOperation::Status,
                request.request_id,
            );
            status_request.service = request.service;
            let mut response = registry.manager_request(status_request);
            response.operation = request.operation;
            response.status = logos_abi::ManagerStatus::Accepted;
            response.request_id = request.request_id;
            ManagerDecision { response, action }
        } else {
            self.manager.request(request, rights)
        };
        match decision.action {
            ManagerAction::None => {}
            ManagerAction::Start(service) => {
                if self.uses_package_image(service) {
                    if self.pending_restart.is_some() {
                        self.abort_dynamic_service_lifecycle(
                            logos_abi::ManagerOperation::Start,
                            service,
                        );
                        decision.response.status = logos_abi::ManagerStatus::Busy;
                        self.refresh_manager_response_record(&mut decision.response);
                    } else {
                        let Ok(handle) = self.runtime_service_handle(service) else {
                            self.abort_dynamic_service_lifecycle(
                                logos_abi::ManagerOperation::Start,
                                service,
                            );
                            decision.response.status = logos_abi::ManagerStatus::Stale;
                            self.refresh_manager_response_record(&mut decision.response);
                            return logos_abi::IpcStatus::Ok;
                        };
                        self.pending_restart = Some(alloc::vec![handle]);
                    }
                } else if self.reset_service_image(service).is_err()
                    || self.start_service_task(service).is_err()
                {
                    self.abort_dynamic_service_lifecycle(
                        logos_abi::ManagerOperation::Start,
                        service,
                    );
                    self.sync_dynamic_service_failed(service);
                    decision.response.status = logos_abi::ManagerStatus::Capacity;
                    self.refresh_manager_response_record(&mut decision.response);
                }
            }
            ManagerAction::Stop(service) => match self.request_stop_service(service) {
                Ok(true) => {}
                Ok(false) => {
                    decision.response.status = logos_abi::ManagerStatus::Ok;
                    self.refresh_manager_response_record(&mut decision.response);
                }
                Err(_) => {
                    self.abort_dynamic_service_lifecycle(
                        logos_abi::ManagerOperation::Stop,
                        service,
                    );
                    self.sync_dynamic_service_failed(service);
                    decision.response.status = logos_abi::ManagerStatus::Busy;
                    self.refresh_manager_response_record(&mut decision.response);
                }
            },
            ManagerAction::Restart(services) => {
                if self.pending_restart.is_some()
                    || services.iter().any(|service| {
                        self.tasks[service.index()]
                            .is_none_or(|task| crate::SCHEDULER.state(task).is_none())
                    })
                {
                    self.abort_dynamic_service_lifecycle(
                        logos_abi::ManagerOperation::Restart,
                        services[0],
                    );
                    decision.response.status = logos_abi::ManagerStatus::Busy;
                } else {
                    let mut handles = Vec::new();
                    if handles.try_reserve(services.len()).is_err() {
                        self.abort_dynamic_service_lifecycle(
                            logos_abi::ManagerOperation::Restart,
                            services[0],
                        );
                        decision.response.status = logos_abi::ManagerStatus::Capacity;
                    } else {
                        for service in &services {
                            let Ok(handle) = self.runtime_service_handle(*service) else {
                                self.abort_dynamic_service_lifecycle(
                                    logos_abi::ManagerOperation::Restart,
                                    services[0],
                                );
                                decision.response.status = logos_abi::ManagerStatus::Stale;
                                handles.clear();
                                break;
                            };
                            handles.push(handle);
                        }
                    }
                    if handles.len() == services.len() {
                        let mut admitted = 0;
                        for service in &services {
                            if self.request_stop_task(*service).is_err() {
                                self.abort_dynamic_service_lifecycle(
                                    logos_abi::ManagerOperation::Restart,
                                    services[0],
                                );
                                decision.response.status = logos_abi::ManagerStatus::Busy;
                                break;
                            }
                            admitted += 1;
                        }
                        if admitted == services.len() {
                            self.refresh_manager_response_record(&mut decision.response);
                            self.pending_restart = Some(handles);
                        }
                    }
                    self.refresh_manager_response_record(&mut decision.response);
                }
            }
            ManagerAction::ProgramStart(slot) => {
                if self.pending_program_start.is_some() {
                    decision.response.status = logos_abi::ManagerStatus::Busy;
                } else {
                    self.pending_program_start = Some((slot, decision.response.record));
                }
            }
            ManagerAction::ProgramStop(slot) => {
                if self.request_stop_program(slot).is_err() {
                    decision.response.status = logos_abi::ManagerStatus::Busy;
                }
            }
        }
        if service_lifecycle {
            if let Some(registry) = self.dynamic_services.as_ref() {
                let mut status_request = logos_abi::ManagerRequest::new(
                    logos_abi::ManagerOperation::Status,
                    request.request_id,
                );
                status_request.service = request.service;
                let status_response = registry.manager_request(status_request);
                if status_response.status == logos_abi::ManagerStatus::Ok {
                    decision.response.record = status_response.record;
                }
            }
        }
        if service_lifecycle
            && !matches!(
                decision.response.status,
                logos_abi::ManagerStatus::Malformed
                    | logos_abi::ManagerStatus::Unauthorized
                    | logos_abi::ManagerStatus::Stale
                    | logos_abi::ManagerStatus::Unsupported
            )
        {
            decision.response.record.service = request.service;
        }
        unsafe {
            core::ptr::write_unaligned(
                staging_frame.raw() as usize as *mut logos_abi::ManagerResponse,
                decision.response,
            );
        }
        logos_abi::IpcStatus::Ok
    }

    pub(crate) fn event_call(
        &mut self,
        process: ProcessHandle,
        length: usize,
    ) -> logos_abi::EventStatus {
        let Some(service) = self.service_for_process(process) else {
            return logos_abi::EventStatus::Unauthorized;
        };
        if self.dynamic_service_state(service)
            != Some(crate::runtime_services::ServiceState::Running)
        {
            return logos_abi::EventStatus::Stale;
        }
        if length != core::mem::size_of::<logos_abi::EventRequest>() {
            return logos_abi::EventStatus::Malformed;
        }
        let Some(staging_frame) = self.ipc_staging_frames[service.index()] else {
            return logos_abi::EventStatus::Unauthorized;
        };
        let wire = unsafe {
            core::slice::from_raw_parts(
                staging_frame.raw() as usize as *const u8,
                core::mem::size_of::<logos_abi::EventRequest>(),
            )
        };
        if !logos_abi::EventRequest::wire_enums_valid(wire) {
            return logos_abi::EventStatus::Malformed;
        }
        let request = unsafe {
            core::ptr::read_unaligned(staging_frame.raw() as usize as *const logos_abi::EventRequest)
        };
        if !request.is_valid() {
            return logos_abi::EventStatus::Malformed;
        }
        let owner = match self.runtime_service_handle(service) {
            Ok(owner) => owner,
            Err(_) => return logos_abi::EventStatus::Stale,
        };
        let events = self.dynamic_events.get_or_insert_with(|| {
            crate::runtime_events::RuntimeEventRegistry::new_with_generation(
                (self.service_epoch as u32).max(1),
            )
        });
        let mut response =
            logos_abi::EventResponse::empty(logos_abi::EventStatus::Malformed, request.request_id);
        let status = match request.operation {
            logos_abi::EventOperation::Create => match events.create_event(owner) {
                Ok(event) => {
                    response.event = event;
                    logos_abi::EventStatus::Ok
                }
                Err(error) => event_status(error),
            },
            logos_abi::EventOperation::Destroy => events
                .destroy_event(owner, request.event)
                .map_or_else(event_status, |_| logos_abi::EventStatus::Ok),
            logos_abi::EventOperation::CreateSet => match events.create_set(owner) {
                Ok(set) => {
                    response.event_set = set;
                    logos_abi::EventStatus::Ok
                }
                Err(error) => event_status(error),
            },
            logos_abi::EventOperation::Add => events
                .add(owner, request.event_set, request.event)
                .map_or_else(event_status, |_| logos_abi::EventStatus::Ok),
            logos_abi::EventOperation::Remove => events
                .remove(owner, request.event_set, request.event)
                .map_or_else(event_status, |_| logos_abi::EventStatus::Ok),
            logos_abi::EventOperation::DestroySet => events
                .destroy_set(owner, request.event_set)
                .map_or_else(event_status, |_| logos_abi::EventStatus::Ok),
            logos_abi::EventOperation::Cancel => events
                .cancel_wait(owner, request.event_set)
                .map_or_else(event_status, |_| logos_abi::EventStatus::Ok),
            logos_abi::EventOperation::Wait => match events.wait_any(
                owner,
                request.event_set,
                crate::arch::current_ticks(),
                (request.deadline != u64::MAX).then_some(request.deadline),
            ) {
                Ok(crate::runtime_events::EventWait::Ready(event)) => {
                    response.event = event;
                    logos_abi::EventStatus::Ready
                }
                Ok(crate::runtime_events::EventWait::Pending) => {
                    if self
                        .prepare_dynamic_event_wait(
                            service,
                            owner,
                            request.event_set,
                            request.deadline,
                        )
                        .is_err()
                    {
                        logos_abi::EventStatus::Stale
                    } else {
                        logos_abi::EventStatus::Pending
                    }
                }
                Ok(crate::runtime_events::EventWait::Timeout) => logos_abi::EventStatus::Timeout,
                Err(error) => event_status(error),
            },
            logos_abi::EventOperation::Signal => {
                events.signal(owner, request.event).map_or_else(event_status, |_| {
                    self.signal_dynamic_event_waiters(request.event);
                    logos_abi::EventStatus::Ok
                })
            }
        };
        response.status = status;
        self.refresh_dynamic_service_counts(service);
        unsafe {
            core::ptr::write_unaligned(
                staging_frame.raw() as usize as *mut logos_abi::EventResponse,
                response,
            );
        }
        status
    }

    fn refresh_manager_response_record(&self, response: &mut logos_abi::ManagerResponse) {
        let Some(registry) = self.dynamic_services.as_ref() else {
            return;
        };
        let mut request = logos_abi::ManagerRequest::new(
            logos_abi::ManagerOperation::Status,
            response.request_id,
        );
        request.service = response.record.service;
        let status = registry.manager_request(request);
        if status.status == logos_abi::ManagerStatus::Ok {
            response.record = status.record;
        }
    }

    fn prepare_dynamic_event_wait(
        &self,
        service: ServiceId,
        owner: logos_abi::ServiceHandle,
        set: logos_abi::EventSetHandle,
        deadline: u64,
    ) -> Result<(), logos_abi::EventStatus> {
        let events = self.dynamic_events.as_ref().ok_or(logos_abi::EventStatus::Stale)?;
        let _ = events.members(owner, set).map_err(event_status)?;
        let Some(task) = self.tasks[service.index()] else {
            return Err(logos_abi::EventStatus::Stale);
        };
        let should_block = crate::arch::prepare_service_event_set_wait(task, set, deadline)
            .ok_or(logos_abi::EventStatus::Stale)?;
        if should_block && events.has_ready_event(owner, set).map_err(event_status)? {
            crate::arch::signal_event_set(set);
        }
        if should_block {
            #[cfg(feature = "qemu-proof")]
            crate::proof::dynamic_event_blocked();
        }
        Ok(())
    }

    fn signal_dynamic_event_waiters(&self, event: logos_abi::EventHandle) {
        let Some(events) = self.dynamic_events.as_ref() else { return };
        let Some(registry) = self.dynamic_services.as_ref() else { return };
        events.for_each_waiter(event, |set, owner| {
            if registry.validate_lifecycle_handle(owner).is_err() {
                return;
            }
            let Some(service) = builtin_service_for_handle(&self.service_handles, owner) else {
                return;
            };
            if self.tasks[service.index()].is_none() {
                return;
            }
            crate::arch::signal_event_set(set);
        });
    }

    #[cfg(feature = "qemu-proof")]
    pub(crate) fn manager_proof(
        &mut self,
        request: logos_abi::ManagerRequest,
    ) -> Option<logos_abi::ManagerResponse> {
        let process = self.launch(ServiceId::Flow)?.0;
        let frame = self.ipc_staging_frames[ServiceId::Flow.index()]?;
        unsafe {
            core::ptr::write_unaligned(
                frame.raw() as usize as *mut logos_abi::ManagerRequest,
                request,
            );
        }
        if self.manager_call(
            process,
            self.bootstrap_control[ServiceId::Flow.index()].raw(),
            core::mem::size_of::<logos_abi::ManagerRequest>(),
        ) != logos_abi::IpcStatus::Ok
        {
            return None;
        }
        Some(unsafe {
            core::ptr::read_unaligned(frame.raw() as usize as *const logos_abi::ManagerResponse)
        })
    }

    #[cfg(feature = "qemu-proof")]
    pub(crate) fn event_proof(&mut self) -> bool {
        let process = match self.launch(ServiceId::Flow) {
            Some((process, _)) => process,
            None => return false,
        };
        let event = self
            .dynamic_ipc
            .as_ref()
            .and_then(|registry| {
                let endpoint = self
                    .dynamic_endpoint(
                        Some(ServiceId::Fetch),
                        Some(ServiceId::Flow),
                        logos_abi::IPC_CONTRACT_BYTES,
                    )
                    .ok()?;
                registry.endpoint_events(endpoint).ok()
            })
            .map(|(read, _)| read);
        let Some(event) = event else { return false };
        let create = logos_abi::EventRequest::new(logos_abi::EventOperation::CreateSet, 0x5001);
        let Some((status, response)) = self.event_proof_call(process, create) else {
            return false;
        };
        if status != logos_abi::EventStatus::Ok || !response.event_set.is_valid() {
            return false;
        }
        let set = response.event_set;
        let mut add = logos_abi::EventRequest::new(logos_abi::EventOperation::Add, 0x5002);
        add.event_set = set;
        add.event = event;
        if self
            .event_proof_call(process, add)
            .is_none_or(|(status, _)| status != logos_abi::EventStatus::Ok)
        {
            return false;
        }
        let mut signal = logos_abi::EventRequest::new(logos_abi::EventOperation::Signal, 0x5003);
        signal.event = event;
        if self
            .event_proof_call(process, signal)
            .is_none_or(|(status, _)| status != logos_abi::EventStatus::Ok)
        {
            return false;
        }
        let mut wait = logos_abi::EventRequest::new(logos_abi::EventOperation::Wait, 0x5004);
        wait.event_set = set;
        if self.event_proof_call(process, wait).is_none_or(|(status, response)| {
            status != logos_abi::EventStatus::Ready || response.event != event
        }) {
            return false;
        }
        let mut destroy =
            logos_abi::EventRequest::new(logos_abi::EventOperation::DestroySet, 0x5005);
        destroy.event_set = set;
        if self
            .event_proof_call(process, destroy)
            .is_none_or(|(status, _)| status != logos_abi::EventStatus::Ok)
        {
            return false;
        }
        let stale = self
            .event_proof_call(process, destroy)
            .is_some_and(|(status, _)| status == logos_abi::EventStatus::Stale);
        if stale {
            crate::proof::dynamic_event_used();
        }
        stale
    }

    #[cfg(feature = "qemu-proof")]
    fn event_proof_call(
        &mut self,
        process: ProcessHandle,
        request: logos_abi::EventRequest,
    ) -> Option<(logos_abi::EventStatus, logos_abi::EventResponse)> {
        let service = self.service_for_process(process)?;
        let frame = self.ipc_staging_frames[service.index()]?;
        unsafe {
            core::ptr::write_unaligned(
                frame.raw() as usize as *mut logos_abi::EventRequest,
                request,
            );
        }
        let status = self.event_call(process, core::mem::size_of::<logos_abi::EventRequest>());
        let response = unsafe {
            core::ptr::read_unaligned(frame.raw() as usize as *const logos_abi::EventResponse)
        };
        response.is_valid_for(request).then_some((status, response))
    }

    #[cfg(feature = "qemu-proof")]
    pub(crate) fn manager_restart_ready(&self, service: ServiceId) -> bool {
        let Some(registry) = self.dynamic_services.as_ref() else {
            return false;
        };
        let Ok(handle) = self.runtime_service_handle(service) else {
            return false;
        };
        let mut request = logos_abi::ManagerRequest::new(logos_abi::ManagerOperation::Status, 1);
        request.service = handle;
        let response = registry.manager_request(request);
        response.is_valid_for(request)
            && response.record.state == logos_abi::ManagerState::Running
            && response.record.restarts != 0
    }

    pub(crate) fn service_for_process(&self, process: ProcessHandle) -> Option<ServiceId> {
        SERVICE_IMAGES.iter().find_map(|spec| {
            self.launch(spec.service())
                .is_some_and(|(current, _)| current == process)
                .then_some(spec.service())
        })
    }

    #[cfg(feature = "qemu-proof")]
    pub(crate) fn hostile_ipc_layout_valid(&self) -> bool {
        for spec in SERVICE_IMAGES {
            let service = spec.service();
            let Some((process, _)) = self.launch(service) else {
                return false;
            };
            let mut staging = false;
            for mapping_index in 0..crate::process::MAX_MAPPINGS_PER_ADDRESS_SPACE {
                let Some(mapping) = self.processes.mapping(process, mapping_index) else {
                    continue;
                };
                let address = mapping.virtual_address();
                let Some(mapping_bytes) = mapping.pages().checked_mul(crate::loader::PAGE_SIZE)
                else {
                    return false;
                };
                let Some(_mapping_end) = address.checked_add(mapping_bytes) else {
                    return false;
                };
                if address == logos_abi::IPC_STAGING_BASE {
                    staging = mapping.flags() == MappingFlags::DATA;
                }
            }
            if !staging {
                return false;
            }
        }
        true
    }

    pub(crate) fn fault_process(
        &mut self,
        process: ProcessHandle,
        vector: u8,
    ) -> Result<(), crate::process::ProcessError> {
        self.processes.fault(process, vector)
    }

    pub(crate) fn exit_process(
        &mut self,
        process: ProcessHandle,
        code: u8,
    ) -> Result<(), ProcessError> {
        self.processes.exit(process, code)
    }

    /// Remove a tracked process mapping from its page table and process table.
    /// The operation is bounded by the mapping's fixed page count.
    #[allow(dead_code)]
    pub(crate) fn unmap_process_mapping(
        &mut self,
        process: ProcessHandle,
        mapping_index: usize,
    ) -> Result<VirtualMapping, ServiceRuntimeError> {
        let mapping = self
            .processes
            .mapping(process, mapping_index)
            .ok_or(ServiceRuntimeError::Process(ProcessError::AddressSpace))?;
        let service = self
            .service_for_process(process)
            .ok_or(ServiceRuntimeError::Process(ProcessError::InvalidHandle))?;
        let index = service.index();
        if !self.table_ready[index] {
            return Err(ServiceRuntimeError::Process(ProcessError::AddressSpace));
        }
        let tables = unsafe { self.tables[index].assume_init_mut() };
        let mut memory = IdentityPageTableMemory;
        for page in 0..mapping.pages() {
            let address = mapping
                .virtual_address()
                .checked_add(page * crate::loader::PAGE_SIZE)
                .ok_or(ServiceRuntimeError::PageTableMap(PageTableError::InvalidMapping))?;
            tables.unmap_page(address, &mut memory).map_err(ServiceRuntimeError::PageTableMap)?;
        }
        self.processes.unmap(process, mapping_index).map_err(ServiceRuntimeError::Process)
    }

    /// Map Storage-owned cache pages read-only into an authorized Flow/Fetch window.
    pub(crate) fn map_storage_window(
        &mut self,
        request: crate::storage_ipc::StorageMapRequest,
        cache_start: u64,
    ) -> Result<crate::storage_ipc::StorageMapResponse, PageTableError> {
        let client_slot = crate::storage_ipc::storage_map_client_slot(request.client)
            .ok_or(PageTableError::InvalidMapping)?;
        let windows = self.storage_map_windows[client_slot];
        crate::storage_ipc::validate_map_request(
            request,
            self.service_epoch,
            request.client,
            cache_start,
            &windows,
        )
        .map_err(|_| PageTableError::InvalidMapping)?;
        let window_slot =
            windows.iter().position(Option::is_none).ok_or(PageTableError::Capacity)?;
        let service = match client_slot {
            0 => ServiceId::Flow,
            1 => ServiceId::Fetch,
            _ => return Err(PageTableError::InvalidMapping),
        };
        let process = self
            .launch(service)
            .map(|(process, _)| process)
            .ok_or(PageTableError::InvalidMapping)?;
        let source_slot =
            request.source_page.checked_sub(cache_start).ok_or(PageTableError::InvalidFrame)?
                as usize;
        let last_slot =
            source_slot.checked_add(request.pages as usize).ok_or(PageTableError::InvalidFrame)?;
        if last_slot > logos_abi::STORAGE_CACHE_PAGES {
            return Err(PageTableError::InvalidFrame);
        }
        if !self.table_ready[service.index()] {
            return Err(PageTableError::InvalidMapping);
        }
        let tables = unsafe { self.tables[service.index()].assume_init_mut() };
        let mut memory = IdentityPageTableMemory;
        for page in 0..request.pages as usize {
            let virtual_address = request.target_page as usize + page * crate::loader::PAGE_SIZE;
            let frame = self.storage_data_frames[1 + source_slot + page]
                .ok_or(PageTableError::InvalidFrame)?;
            if let Err(error) = tables.map_raw_page(
                virtual_address,
                frame,
                MappingFlags::READ_ONLY_DATA,
                &mut self.frame_pool,
                &mut memory,
            ) {
                for rollback in 0..page {
                    let address =
                        request.target_page as usize + rollback * crate::loader::PAGE_SIZE;
                    let _ = tables.unmap_page(address, &mut memory);
                }
                for index in 0..crate::process::MAX_MAPPINGS_PER_ADDRESS_SPACE {
                    if self.processes.mapping(process, index).is_some_and(|mapping| {
                        mapping.virtual_address() >= request.target_page as usize
                            && mapping.virtual_address()
                                < request.target_page as usize
                                    + request.pages as usize * crate::loader::PAGE_SIZE
                    }) {
                        let _ = self.processes.unmap(process, index);
                    }
                }
                return Err(error);
            }
            let mapping = match VirtualMapping::new(
                virtual_address,
                frame.raw() as usize,
                1,
                MappingFlags::READ_ONLY_DATA,
            ) {
                Some(mapping) => mapping,
                None => {
                    let _ = tables.unmap_page(virtual_address, &mut memory);
                    for rollback in 0..page {
                        let address =
                            request.target_page as usize + rollback * crate::loader::PAGE_SIZE;
                        let _ = tables.unmap_page(address, &mut memory);
                    }
                    return Err(PageTableError::InvalidMapping);
                }
            };
            if let Err(error) = self.processes.map(process, mapping) {
                let _ = tables.unmap_page(virtual_address, &mut memory);
                for rollback in 0..page {
                    let address =
                        request.target_page as usize + rollback * crate::loader::PAGE_SIZE;
                    let _ = tables.unmap_page(address, &mut memory);
                }
                for index in 0..crate::process::MAX_MAPPINGS_PER_ADDRESS_SPACE {
                    if self.processes.mapping(process, index).is_some_and(|mapping| {
                        mapping.virtual_address() >= request.target_page as usize
                            && mapping.virtual_address()
                                < request.target_page as usize
                                    + request.pages as usize * crate::loader::PAGE_SIZE
                    }) {
                        let _ = self.processes.unmap(process, index);
                    }
                }
                return Err(match error {
                    ProcessError::Capacity => PageTableError::Capacity,
                    _ => PageTableError::InvalidMapping,
                });
            }
        }
        self.storage_map_windows[client_slot][window_slot] =
            Some(crate::storage_ipc::StorageMapWindow {
                target_page: request.target_page,
                pages: request.pages,
                generation: request.window_generation,
            });
        Ok(crate::storage_ipc::StorageMapResponse::accepted(request))
    }

    /// Select a Core-owned target window for a Storage map descriptor.
    pub(crate) fn map_storage_descriptor(
        &mut self,
        generation: u64,
        client: u16,
        descriptor: &[u8],
    ) -> Result<crate::storage_ipc::StorageMapResponse, PageTableError> {
        let client_slot = crate::storage_ipc::storage_map_client_slot(client)
            .ok_or(PageTableError::InvalidMapping)?;
        let window_slot = self.storage_map_windows[client_slot]
            .iter()
            .position(Option::is_none)
            .ok_or(PageTableError::Capacity)?;
        let target_page = crate::storage_ipc::storage_map_target(client_slot, window_slot)
            .ok_or(PageTableError::InvalidMapping)?;
        let request = crate::storage_ipc::map_request_from_descriptor(
            generation,
            client,
            target_page,
            (window_slot as u32).saturating_add(1),
            descriptor,
        )
        .ok_or(PageTableError::InvalidMapping)?;
        self.map_storage_window(request, crate::storage_ipc::STORAGE_CACHE_START)
    }

    /// Unmap a previously granted Storage window and reject stale generations.
    pub(crate) fn unmap_storage_window(
        &mut self,
        request: crate::storage_ipc::StorageMapRelease,
    ) -> Result<(), PageTableError> {
        let client_slot = crate::storage_ipc::storage_map_client_slot(request.client)
            .ok_or(PageTableError::InvalidMapping)?;
        if request.generation != self.service_epoch || request.window_generation == 0 {
            return Err(PageTableError::InvalidMapping);
        }
        let Some((window_slot, window)) =
            self.storage_map_windows[client_slot].iter().enumerate().find_map(|(slot, window)| {
                window.and_then(|window| {
                    (window.target_page == request.target_page
                        && window.generation == request.window_generation)
                        .then_some((slot, window))
                })
            })
        else {
            return Err(PageTableError::InvalidMapping);
        };
        let service = if client_slot == 0 { ServiceId::Flow } else { ServiceId::Fetch };
        let process = self
            .launch(service)
            .map(|(process, _)| process)
            .ok_or(PageTableError::InvalidMapping)?;
        for page in 0..window.pages as usize {
            let virtual_address = window.target_page as usize + page * crate::loader::PAGE_SIZE;
            let mapping_index = (0..crate::process::MAX_MAPPINGS_PER_ADDRESS_SPACE)
                .find(|index| {
                    self.processes.mapping(process, *index).is_some_and(|mapping| {
                        mapping.virtual_address() == virtual_address
                            && mapping.pages() == 1
                            && mapping.flags() == MappingFlags::READ_ONLY_DATA
                    })
                })
                .ok_or(PageTableError::InvalidMapping)?;
            self.unmap_process_mapping(process, mapping_index)
                .map_err(|_| PageTableError::InvalidMapping)?;
        }
        self.storage_map_windows[client_slot][window_slot] = None;
        Ok(())
    }

    /// Release a Core-selected window returned by `map_storage_descriptor`.
    #[allow(dead_code)]
    pub(crate) fn unmap_storage_response(
        &mut self,
        client: u16,
        response: crate::storage_ipc::StorageMapResponse,
    ) -> Result<(), PageTableError> {
        self.unmap_storage_window(crate::storage_ipc::StorageMapRelease {
            generation: response.generation,
            client,
            target_page: response.target_page,
            window_generation: response.window_generation,
        })
    }

    /// Stop every service task at a scheduler boundary before reclaiming any
    /// process frames or page-table roots.
    fn restart(
        &mut self,
        bundle: &ServiceImageBundle,
        runtime_guard: &mut crate::arch::ServiceRuntimeGuard,
    ) -> Result<(), ServiceRuntimeError> {
        let _restart_gate = ServiceRestartGate::acquire();
        crate::arch::begin_service_runtime_transition();
        self.ipc_generation = self.ipc_generation.wrapping_add(1).max(1);
        self.service_epoch = self.service_epoch.wrapping_add(1).max(1);
        self.network_config.service_epoch =
            self.network_config.service_epoch.wrapping_add(1).max(1);
        if !self.supervisor.prepare_restart() {
            return Err(ServiceRuntimeError::RestartLimit);
        }
        self.stop_tasks(runtime_guard)?;
        self.retain_active_package_images()?;
        crate::arch::prepare_task_address_space(0);
        crate::arch::restart_critical_section(|| {
            crate::arch::disable_keyboard_irq();
            crate::arch::reset_events();
            self.reclaim_resources()?;
            self.start(bundle)?;
            for suppressed in &self.suppressed_heartbeats {
                suppressed.store(false, Ordering::Release);
            }
            let old_service_epoch = self.service_epoch.wrapping_sub(1).max(1);
            let stale_rejected = self.dynamic_ipc.is_some()
                && self.dynamic_ipc.as_ref().is_some_and(|registry| {
                    registry.all_endpoint_generations_differ(old_service_epoch as u32)
                });
            if !stale_rejected {
                return Err(ServiceRuntimeError::StaleGeneration);
            }
            let result = self.start_tasks();
            if result.is_ok() {
                #[cfg(feature = "qemu-proof")]
                crate::proof::network_restart_completed();
                crate::arch::enable_keyboard_irq();
                crate::arch::finish_service_runtime_transition();
            }
            result
        })
    }

    fn restart_network(
        &mut self,
        bundle: &ServiceImageBundle,
        runtime_guard: &mut crate::arch::ServiceRuntimeGuard,
    ) -> Result<(), ServiceRuntimeError> {
        // Reuse the complete runtime teardown so no old Network or Fetch
        // process, heap, IPC, or event resources remain reachable.
        self.restart(bundle, runtime_guard)
    }

    pub fn supervise(
        &mut self,
        bundle: &ServiceImageBundle,
        now: u64,
        runtime_guard: &mut crate::arch::ServiceRuntimeGuard,
    ) -> Result<bool, ServiceRuntimeError> {
        if let Some((slot, record)) = self.pending_program_start.take() {
            if self.start_program(slot, record, runtime_guard).is_err() {
                let _ = self.manager.mark_program_failed(slot, record.program_generation);
            }
            return Ok(true);
        }
        for slot in 0..MAX_PROGRAMS {
            let Some(task) = self.programs[slot].task else { continue };
            if crate::SCHEDULER.state(task) != Some(crate::TaskState::Completed) {
                continue;
            }
            if !crate::SCHEDULER.reclaim_completed(task) {
                return Err(ServiceRuntimeError::TaskStop);
            }
            let generation = self.programs[slot].generation;
            let Some(process) = self.programs[slot].process.take() else {
                let _ = self.manager.mark_program_stopped(slot, generation);
                continue;
            };
            let state = self
                .processes
                .state(process)
                .unwrap_or(crate::process::ProcessState::Faulted(0xff));
            let forced_stop = matches!(state, crate::process::ProcessState::Running);
            if forced_stop {
                let _ = self.processes.exit(process, 0xff);
            }
            let terminal = self
                .processes
                .state(process)
                .unwrap_or(crate::process::ProcessState::Faulted(0xff));
            let _ = self.processes.reclaim(process);
            if matches!(terminal, crate::process::ProcessState::Exited(_)) && !forced_stop {
                let _ = self.manager.mark_program_terminal(
                    slot,
                    generation,
                    logos_abi::ManagerState::Exited,
                );
            } else if matches!(terminal, crate::process::ProcessState::Faulted(_)) {
                let _ = self.manager.mark_program_terminal(
                    slot,
                    generation,
                    logos_abi::ManagerState::Faulted,
                );
            } else {
                let _ = self.manager.mark_program_stopped(slot, generation);
            }
            if self.programs[slot].table_ready {
                let mut memory = IdentityPageTableMemory;
                unsafe { self.programs[slot].table.assume_init_mut() }
                    .reclaim(&mut self.frame_pool, &mut memory);
                self.programs[slot].table_ready = false;
            }
            self.programs[slot].image.reclaim(&mut self.frame_pool);
            self.programs[slot].task = None;
            self.programs[slot].manager_slot = u8::MAX;
        }
        for spec in SERVICE_IMAGES {
            let service = spec.service();
            let index = service.index();
            let Some(task) = self.tasks[index] else {
                continue;
            };
            if crate::SCHEDULER.state(task) == Some(crate::TaskState::Completed) {
                let process_failed = self
                    .launch(service)
                    .and_then(|(process, _)| self.processes.state(process))
                    .is_none_or(|state| !matches!(state, crate::process::ProcessState::Running));
                if !crate::SCHEDULER.reclaim_completed(task) {
                    return Err(ServiceRuntimeError::TaskStop);
                }
                self.tasks[index] = None;
                self.supervisor.unregister(service);
                if process_failed {
                    self.sync_dynamic_service_failed(service);
                } else {
                    self.sync_dynamic_service_stopped(service);
                }
            }
        }
        if let Some(handles) = self.pending_restart.take() {
            let mut services = Vec::new();
            let Some(registry) = self.dynamic_services.as_ref() else {
                return Err(ServiceRuntimeError::Resources);
            };
            if services.try_reserve(handles.len()).is_err() {
                return Err(ServiceRuntimeError::Resources);
            }
            for handle in &handles {
                registry
                    .validate_lifecycle_handle(*handle)
                    .map_err(|_| ServiceRuntimeError::StaleGeneration)?;
                let service = builtin_service_for_handle(&self.service_handles, *handle)
                    .ok_or(ServiceRuntimeError::StaleGeneration)?;
                services.push(service);
            }
            if services.iter().any(|service| self.tasks[service.index()].is_some()) {
                self.pending_restart = Some(handles);
                return Ok(false);
            }
            // A service restart must invalidate its IPC capabilities and event
            // handles before the replacement task is published. The existing
            // full teardown is the single path that reclaims and rebuilds the
            // runtime topology with a new service epoch.
            self.restart(bundle, runtime_guard)?;
            self.supervisor.clear_startup_grace();
            #[cfg(feature = "qemu-proof")]
            crate::proof::manager_restart_completed();
            return Ok(true);
        }
        if let Some(failed) = SERVICE_IMAGES.iter().find_map(|spec| {
            (self.dynamic_service_state(spec.service())
                == Some(crate::runtime_services::ServiceState::Failed))
            .then_some(spec.service())
        }) {
            if failed == ServiceId::Network
                && !self.uses_package_image(ServiceId::Network)
                && !self.uses_package_image(ServiceId::Fetch)
            {
                self.restart_network(bundle, runtime_guard)?;
            } else {
                self.restart(bundle, runtime_guard)?;
            }
            return Ok(true);
        }
        let mut heartbeats = [0; SERVICE_COUNT];
        let mut process_states = [None; SERVICE_COUNT];
        for spec in SERVICE_IMAGES {
            let index = spec.service().index();
            heartbeats[index] = self.heartbeat_tick(spec.service());
            process_states[index] = if self.tasks[index].is_some() {
                self.launch(spec.service()).and_then(|(process, _)| self.processes.state(process))
            } else {
                None
            };
        }
        if let Some(failed) = self.supervisor.poll(now, heartbeats, process_states) {
            if failed == ServiceId::Network
                && !self.uses_package_image(ServiceId::Network)
                && !self.uses_package_image(ServiceId::Fetch)
            {
                self.restart_network(bundle, runtime_guard)?;
            } else {
                self.restart(bundle, runtime_guard)?;
            }
            return Ok(true);
        }
        Ok(false)
    }

    fn request_stop_program(&mut self, slot: usize) -> Result<(), ServiceRuntimeError> {
        let program = self.programs.get(slot).ok_or(ServiceRuntimeError::TaskStop)?;
        let task = program.task.ok_or(ServiceRuntimeError::TaskStop)?;
        if crate::SCHEDULER.request_stop(task) {
            Ok(())
        } else {
            Err(ServiceRuntimeError::TaskStop)
        }
    }

    fn start_program(
        &mut self,
        slot: usize,
        record: logos_abi::ServiceManagerRecord,
        runtime_guard: &mut crate::arch::ServiceRuntimeGuard,
    ) -> Result<(), ServiceRuntimeError> {
        if slot >= MAX_PROGRAMS || self.programs[slot].task.is_some() {
            return Err(ServiceRuntimeError::TaskCapacity);
        }
        let target = logos_abi::PackageTarget::program(&record.name[..record.name_len as usize])
            .ok_or(ServiceRuntimeError::Image)?;
        let request = self
            .next_package_request_target(logos_abi::PackageOperation::Lookup, target, 0, 0, 0)
            .map_err(ServiceRuntimeError::Process)?;
        let response = self
            .package_exchange(request, &mut [], runtime_guard)
            .map_err(ServiceRuntimeError::Process)?;
        if response.status != logos_abi::PackageStatus::Ok {
            return Err(ServiceRuntimeError::Image);
        }
        let package_bytes = response.package_bytes as usize;
        let package_generation = response.package_generation;
        let (payload_offset, payload_length) = {
            let mut reader = RuntimePackageReader::new(
                self,
                runtime_guard,
                target,
                package_generation,
                0,
                package_bytes,
            );
            let mut scratch = [0; crate::loader::PAGE_SIZE];
            let header = logos_package::validate_package_v2(&mut reader, &mut scratch)
                .map_err(|_| ServiceRuntimeError::Image)?;
            if header.manifest.kind != logos_package::PackageKind::Program
                || header.manifest.target != logos_package::PackageTarget::None
                || header.manifest.name.as_bytes() != &record.name[..record.name_len as usize]
            {
                return Err(ServiceRuntimeError::Image);
            }
            (logos_package::PACKAGE_HEADER_V2_BYTES, header.payload_length as usize)
        };
        let plan = {
            let mut reader = RuntimePackageReader::new(
                self,
                runtime_guard,
                target,
                package_generation,
                payload_offset,
                payload_length,
            );
            crate::process::ElfLoadPlan::parse_reader(&mut reader)
                .map_err(|_| ServiceRuntimeError::Image)?
        };
        let owner = OwnerId::new(100 + slot as u16).ok_or(ServiceRuntimeError::Resources)?;
        let mut image = LoadedImage::load_with_stack_pages_for_owner(
            plan,
            &mut self.frame_pool,
            crate::process::USER_STACK_PAGES,
            owner,
        )
        .map_err(ServiceRuntimeError::Load)?;
        let mut reader = RuntimePackageReader::new(
            self,
            runtime_guard,
            target,
            package_generation,
            payload_offset,
            payload_length,
        );
        let mut scratch = [0; crate::loader::PAGE_SIZE];
        let mut memory = IdentityPageTableMemory;
        if let Err(error) = image.populate_reader(plan, &mut reader, &mut scratch, &mut memory) {
            image.reclaim(&mut self.frame_pool);
            return Err(ServiceRuntimeError::Populate(error));
        }
        let mut tables = PageTableBuilder::new_for_owner(&mut self.frame_pool, &mut memory, owner)
            .map_err(ServiceRuntimeError::PageTableRoot)?;
        if let Err(error) = tables.map_image(&image, &mut self.frame_pool, &mut memory) {
            tables.reclaim(&mut self.frame_pool, &mut memory);
            image.reclaim(&mut self.frame_pool);
            return Err(ServiceRuntimeError::PageTableMap(error));
        }
        let process = match self.processes.start_plan(plan) {
            Ok(process) => process,
            Err(error) => {
                tables.reclaim(&mut self.frame_pool, &mut memory);
                image.reclaim(&mut self.frame_pool);
                return Err(ServiceRuntimeError::Process(error));
            }
        };
        let Some(root) = AddressSpaceRoot::new(tables.root().raw() as usize) else {
            let _ = self.processes.exit(process, 1);
            let _ = self.processes.reclaim(process);
            tables.reclaim(&mut self.frame_pool, &mut memory);
            image.reclaim(&mut self.frame_pool);
            return Err(ServiceRuntimeError::Process(ProcessError::AddressSpace));
        };
        self.processes.bind_address_space_root(process, root).map_err(|error| {
            let _ = self.processes.exit(process, 1);
            let _ = self.processes.reclaim(process);
            tables.reclaim(&mut self.frame_pool, &mut memory);
            image.reclaim(&mut self.frame_pool);
            ServiceRuntimeError::Process(error)
        })?;
        if let Err(error) = map_loaded_pages(&mut self.processes, process, &image) {
            let _ = self.processes.exit(process, 1);
            let _ = self.processes.reclaim(process);
            tables.reclaim(&mut self.frame_pool, &mut memory);
            image.reclaim(&mut self.frame_pool);
            return Err(ServiceRuntimeError::Process(error));
        }
        let launch = match self.processes.user_launch(process, image.entry(), image.stack_top()) {
            Ok(launch) => launch,
            Err(error) => {
                let _ = self.processes.exit(process, 1);
                let _ = self.processes.reclaim(process);
                tables.reclaim(&mut self.frame_pool, &mut memory);
                image.reclaim(&mut self.frame_pool);
                return Err(ServiceRuntimeError::Process(error));
            }
        };
        let task = match crate::SCHEDULER.spawn_user(service_task_entry, process, launch) {
            Ok(task) => task,
            Err(error) => {
                let _ = self.processes.exit(process, 1);
                let _ = self.processes.reclaim(process);
                tables.reclaim(&mut self.frame_pool, &mut memory);
                image.reclaim(&mut self.frame_pool);
                return Err(match error {
                    crate::SpawnError::Capacity => ServiceRuntimeError::TaskCapacity,
                    crate::SpawnError::AddressSpace => ServiceRuntimeError::TaskAddressSpace,
                    crate::SpawnError::UserLaunch => ServiceRuntimeError::TaskLaunch,
                });
            }
        };
        let program = &mut self.programs[slot];
        program.manager_slot = record.program_slot;
        program.generation = record.program_generation;
        program.name = record.name;
        program.name_len = record.name_len;
        program.process = Some(process);
        program.task = Some(task);
        program.image = image;
        program.table.write(tables);
        program.table_ready = true;
        if !self.manager.mark_program_running(slot, program.generation) {
            return Err(ServiceRuntimeError::StaleGeneration);
        }
        Ok(())
    }

    fn stop_tasks(
        &mut self,
        runtime_guard: &mut crate::arch::ServiceRuntimeGuard,
    ) -> Result<(), ServiceRuntimeError> {
        for task in self.tasks.iter().flatten().copied() {
            if !crate::SCHEDULER.request_stop(task) {
                return Err(ServiceRuntimeError::TaskStop);
            }
        }
        for task in self.tasks.iter().flatten().copied() {
            let mut waited = 0;
            while crate::SCHEDULER.state(task) != Some(crate::TaskState::Completed) {
                if waited == 1024 {
                    return Err(ServiceRuntimeError::TaskStop);
                }
                runtime_guard.pause();
                crate::sleep_current_for(1);
                runtime_guard.resume();
                waited += 1;
            }
            if !crate::SCHEDULER.reclaim_completed(task) {
                return Err(ServiceRuntimeError::TaskStop);
            }
        }
        self.tasks.fill(None);
        for slot in 0..MAX_PROGRAMS {
            let Some(task) = self.programs[slot].task else { continue };
            if !crate::SCHEDULER.request_stop(task) {
                return Err(ServiceRuntimeError::TaskStop);
            }
            let mut waited = 0;
            while crate::SCHEDULER.state(task) != Some(crate::TaskState::Completed) {
                if waited == 1024 {
                    return Err(ServiceRuntimeError::TaskStop);
                }
                runtime_guard.pause();
                crate::sleep_current_for(1);
                runtime_guard.resume();
                waited += 1;
            }
            if !crate::SCHEDULER.reclaim_completed(task) {
                return Err(ServiceRuntimeError::TaskStop);
            }
            if let Some(process) = self.programs[slot].process.take() {
                if self.processes.state(process) == Some(crate::process::ProcessState::Running) {
                    self.processes.exit(process, 0xff).map_err(ServiceRuntimeError::Process)?;
                }
                self.processes.reclaim(process).map_err(ServiceRuntimeError::Process)?;
            }
            if self.programs[slot].table_ready {
                let mut memory = IdentityPageTableMemory;
                unsafe { self.programs[slot].table.assume_init_mut() }
                    .reclaim(&mut self.frame_pool, &mut memory);
                self.programs[slot].table_ready = false;
            }
            self.programs[slot].image.reclaim(&mut self.frame_pool);
            self.programs[slot].task = None;
            let generation = self.programs[slot].generation;
            let _ = self.manager.mark_program_stopped(slot, generation);
        }
        Ok(())
    }

    fn reclaim_resources(&mut self) -> Result<(), ServiceRuntimeError> {
        crate::runtime_events::clear_hardware_event(
            crate::runtime_events::HardwareEventSource::Network,
        );
        crate::runtime_events::clear_hardware_event(
            crate::runtime_events::HardwareEventSource::Keyboard,
        );
        self.storage_map_windows = [[None; crate::storage_ipc::STORAGE_MAP_WINDOWS_PER_CLIENT];
            crate::storage_ipc::STORAGE_MAP_CLIENTS];
        if let Some(mut registry) = self.dynamic_ipc.take() {
            if let Some(events) = self.dynamic_events.as_mut() {
                let generation = self.service_epoch.wrapping_sub(1).max(1) as u32;
                if let Ok(core) = dynamic_core_handle(generation) {
                    registry.destroy_service(core, events);
                }
                for service in self.service_handles {
                    if service.is_valid() {
                        registry.destroy_service(service, events);
                    }
                }
            }
        }
        self.dynamic_services = None;
        self.service_handles = [logos_abi::ServiceHandle::EMPTY; SERVICE_COUNT];
        self.dynamic_events = None;
        self.keyboard_event = logos_abi::EventHandle::EMPTY;
        self.storage_response = None;
        self.device_response = None;
        self.storage_map_response = None;
        self.package_request = None;
        self.package_capability = logos_abi::CapabilityHandle::EMPTY;
        self.package_response = None;
        self.network_packet_response = None;
        self.network_packet_sequence = self.network_packet_sequence.wrapping_add(1).max(1);
        for frame in &mut self.storage_data_frames {
            if let Some(frame) = frame.take() {
                self.frame_pool.release(frame).map_err(|_| ServiceRuntimeError::Resources)?;
            }
        }
        if let Some(frame) = self.network_config_frame {
            self.frame_pool.release(frame).map_err(|_| ServiceRuntimeError::Resources)?;
            self.network_config_frame = None;
        }
        for frame in &mut self.network_packet_frames {
            if let Some(frame) = frame.take() {
                self.frame_pool.release(frame).map_err(|_| ServiceRuntimeError::Resources)?;
            }
        }
        if let Some(frame) = self.keyboard_frame {
            self.frame_pool.release(frame).map_err(|_| ServiceRuntimeError::Resources)?;
            self.keyboard_frame = None;
        }
        if let Some(frame) = self.framebuffer_config_frame {
            self.frame_pool.release(frame).map_err(|_| ServiceRuntimeError::Resources)?;
            self.framebuffer_config_frame = None;
        }
        for index in 0..SERVICE_COUNT {
            if let Some(frame) = self.ipc_staging_frames[index] {
                self.frame_pool.release(frame).map_err(|_| ServiceRuntimeError::Resources)?;
                self.ipc_staging_frames[index] = None;
            }
        }
        for index in 0..SERVICE_COUNT {
            while let Some(frame) = self.service_heaps[index].frames.pop() {
                if self.frame_pool.release(frame).is_err() {
                    self.service_heaps[index].frames.push(frame);
                    return Err(ServiceRuntimeError::Resources);
                }
            }
            self.service_heaps[index].frames = Vec::new();
            self.service_heaps[index].quota_pages = 0;
            if self.service_bootstrap_frames[index] != 0 {
                let frame = FrameAddress::from_raw(self.service_bootstrap_frames[index]);
                self.service_bootstrap_frames[index] = 0;
                self.frame_pool.release(frame).map_err(|_| ServiceRuntimeError::Resources)?;
            }
        }
        for frame in &mut self.user_kdf_workspace {
            if *frame != 0 {
                let address = FrameAddress::from_raw(*frame);
                *frame = 0;
                self.frame_pool.release(address).map_err(|_| ServiceRuntimeError::Resources)?;
            }
        }
        for index in 0..SERVICE_COUNT {
            if let Some((process, _)) = self.launches[index].take() {
                if self.processes.state(process) == Some(crate::process::ProcessState::Running) {
                    self.processes.fault(process, 0xff).map_err(ServiceRuntimeError::Process)?;
                }
                self.processes.reclaim(process).map_err(ServiceRuntimeError::Process)?;
            }
            if self.table_ready[index] {
                let mut memory = IdentityPageTableMemory;
                unsafe { self.tables[index].assume_init_mut() }
                    .reclaim(&mut self.frame_pool, &mut memory);
                self.table_ready[index] = false;
            }
            self.images[index].reclaim(&mut self.frame_pool);
        }
        for program in &mut self.programs {
            if let Some(process) = program.process.take() {
                if self.processes.state(process) == Some(crate::process::ProcessState::Running) {
                    let _ = self.processes.exit(process, 0xff);
                }
                let _ = self.processes.reclaim(process);
            }
            if program.table_ready {
                let mut memory = IdentityPageTableMemory;
                unsafe { program.table.assume_init_mut() }
                    .reclaim(&mut self.frame_pool, &mut memory);
                program.table_ready = false;
            }
            program.image.reclaim(&mut self.frame_pool);
            program.task = None;
            program.manager_slot = u8::MAX;
        }
        self.pending_program_start = None;
        self.startup = ServiceStartup::new();
        Ok(())
    }

    fn reclaim_prepared_packages(&mut self) {
        for prepared in &mut self.prepared_packages {
            if let Some(mut prepared) = prepared.take() {
                prepared.image.reclaim(&mut self.frame_pool);
            }
        }
    }
}

fn bootstrap_queue_capacity(index: usize) -> usize {
    if matches!(
        logos_abi::IpcEndpointId::from_index(index),
        Some(logos_abi::IpcEndpointId::FlowToDevice) | Some(logos_abi::IpcEndpointId::DeviceToFlow)
    ) {
        return 8;
    }
    match logos_abi::ipc_message_type(index) {
        Some(logos_abi::IpcMessageType::Input) => 32,
        Some(logos_abi::IpcMessageType::Render) => 1,
        Some(logos_abi::IpcMessageType::Bytes) => 8,
        _ => 1,
    }
}

impl Default for ServiceRuntime {
    fn default() -> Self {
        Self::new()
    }
}

fn reserve_active_page_tables(pool: &mut FramePool, root: usize) -> bool {
    const ADDRESS_MASK: u64 = 0x000f_ffff_ffff_f000;
    const PRESENT: u64 = 1;
    const HUGE: u64 = 1 << 7;
    let mut frames = [0u64; MAX_ACTIVE_PAGE_TABLE_FRAMES];
    let mut levels = [0u8; MAX_ACTIVE_PAGE_TABLE_FRAMES];
    let mut count = 1;
    frames[0] = (root as u64) & ADDRESS_MASK;
    levels[0] = 4;
    while count != 0 {
        count -= 1;
        let frame = frames[count];
        let level = levels[count];
        if frame == 0 {
            continue;
        }
        pool.reserve(FrameAddress::from_raw(frame));
        if level == 1 {
            continue;
        }
        for index in 0..512u64 {
            let entry = unsafe { core::ptr::read_volatile((frame + index * 8) as *const u64) };
            if entry & PRESENT == 0 || entry & HUGE != 0 {
                continue;
            }
            if count == frames.len() {
                return false;
            }
            frames[count] = entry & ADDRESS_MASK;
            levels[count] = level - 1;
            count += 1;
        }
    }
    true
}

fn initialize_framebuffer_config(
    frame: FrameAddress,
    framebuffer: crate::boot_resources::FramebufferInfo,
) {
    let format = match framebuffer.format() {
        crate::boot_resources::PixelFormat::Bgr8 => logos_abi::FramebufferFormat::Bgr8,
        crate::boot_resources::PixelFormat::Rgb8 => logos_abi::FramebufferFormat::Rgb8,
    };
    let config = logos_abi::FramebufferConfig::new(
        framebuffer.bytes(),
        framebuffer.width(),
        framebuffer.height(),
        framebuffer.stride(),
        format,
    );
    // The frame is identity-mapped in the kernel root before it is mapped
    // read-only by policy into the Display address space.
    unsafe { (frame.raw() as usize as *mut logos_abi::FramebufferConfig).write(config) };
}

fn map_loaded_pages(
    processes: &mut crate::process::ProcessTable,
    process: ProcessHandle,
    image: &LoadedImage,
) -> Result<(), ProcessError> {
    let mut index = 0;
    while index < image.page_count() {
        let Some(first) = image.page(index) else {
            return Err(ProcessError::AddressSpace);
        };
        let mut pages = 1;
        while index + pages < image.page_count() {
            let Some(previous) = image.page(index + pages - 1) else {
                return Err(ProcessError::AddressSpace);
            };
            let Some(next) = image.page(index + pages) else {
                return Err(ProcessError::AddressSpace);
            };
            if previous.flags() != next.flags()
                || previous.virtual_address() + crate::loader::PAGE_SIZE != next.virtual_address()
                || previous.frame().raw() + crate::loader::PAGE_SIZE as u64 != next.frame().raw()
            {
                break;
            }
            pages += 1;
        }
        let mapping = VirtualMapping::new(
            first.virtual_address(),
            first.frame().raw() as usize,
            pages,
            first.flags(),
        )
        .ok_or(ProcessError::AddressSpace)?;
        processes.map(process, mapping)?;
        index += pages;
    }
    Ok(())
}

fn service_task_entry() {
    let cpu = crate::current_cpu();
    let Some(task) = crate::SCHEDULER.current_task(cpu) else {
        crate::arch_fatal(b"LogOS vNext: service task");
    };
    let Some(launch) = crate::SCHEDULER.user_launch(task) else {
        crate::arch_fatal(b"LogOS vNext: service launch");
    };
    crate::arch::enter_user_launch(launch.launch());
}
