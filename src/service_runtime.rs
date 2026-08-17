//! Post-UEFI service image and address-space ownership.

use core::{
    mem::MaybeUninit,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};

use logos_abi::ServiceId;

use crate::memory::{ExclusionKind, MemoryExclusion};
use crate::{
    frame_pool::{FrameAddress, FramePool},
    loader::{LoadError, LoadedImage},
    page_table::{IdentityPageTableMemory, PageTableBuilder, PageTableError, PageTableMemory},
    process::{
        AddressSpaceRoot, MappingFlags, ProcessError, ProcessHandle, UserLaunch, VirtualMapping,
    },
    service_images::SERVICE_IMAGES,
    service_ipc::{IpcError, ServiceIpcGraph},
    service_loader::ServiceImageBundle,
    service_manager::{ManagerAction, ServiceManager},
    service_startup::ServiceStartup,
    supervisor::{EndpointIdentity, LiveSupervisor},
};

const SERVICE_COUNT: usize = SERVICE_IMAGES.len();
const MAX_ACTIVE_PAGE_TABLE_FRAMES: usize = 4096;

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
    ipc: Option<ServiceIpcGraph>,
    ipc_staging_frames: [Option<FrameAddress>; SERVICE_COUNT],
    ipc_capability_frames: [Option<FrameAddress>; SERVICE_COUNT],
    storage_data_frame: Option<FrameAddress>,
    network_config: logos_abi::NetworkConfig,
    network_config_frame: Option<FrameAddress>,
    network_packet_frames: [Option<FrameAddress>; logos_abi::NETWORK_PACKET_PAGE_COUNT],
    framebuffer_config_frame: Option<FrameAddress>,
    keyboard_frame: Option<FrameAddress>,
    tasks: [Option<crate::TaskHandle>; SERVICE_COUNT],
    heartbeat_ticks: [AtomicU64; SERVICE_COUNT],
    supervisor: LiveSupervisor,
    manager: ServiceManager,
    manager_capability_frames: [Option<FrameAddress>; SERVICE_COUNT],
    manager_generation: u32,
    pending_restart: Option<([ServiceId; crate::service_manager::MAX_SERVICE_SLOTS], usize)>,
    ipc_generation: u16,
    service_epoch: u64,
    storage_response: Option<logos_abi::StorageResponse>,
    network_packet_response: Option<logos_abi::NetworkPacketDescriptor>,
    network_packet_sequence: u32,
    suppressed_heartbeats: [AtomicBool; SERVICE_COUNT],
    frame_pool_ready: bool,
    #[cfg(feature = "storage-proof")]
    storage_proof: crate::storage_proof::StorageProofObserver,
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
            images: [
                LoadedImage::empty(),
                LoadedImage::empty(),
                LoadedImage::empty(),
                LoadedImage::empty(),
                LoadedImage::empty(),
                LoadedImage::empty(),
                LoadedImage::empty(),
                LoadedImage::empty(),
            ],
            tables: [const { MaybeUninit::uninit() }; SERVICE_COUNT],
            table_ready: [false; SERVICE_COUNT],
            processes: crate::process::ProcessTable::new(),
            launches: [None; SERVICE_COUNT],
            startup: ServiceStartup::new(),
            ipc: None,
            ipc_staging_frames: [None; SERVICE_COUNT],
            ipc_capability_frames: [None; SERVICE_COUNT],
            storage_data_frame: None,
            network_config: logos_abi::NetworkConfig::disabled(),
            network_config_frame: None,
            network_packet_frames: [None; logos_abi::NETWORK_PACKET_PAGE_COUNT],
            framebuffer_config_frame: None,
            keyboard_frame: None,
            tasks: [None; SERVICE_COUNT],
            heartbeat_ticks: [const { AtomicU64::new(0) }; SERVICE_COUNT],
            supervisor: LiveSupervisor::new(),
            manager: ServiceManager::new(),
            manager_capability_frames: [None; SERVICE_COUNT],
            manager_generation: 1,
            pending_restart: None,
            ipc_generation: 1,
            service_epoch: 1,
            storage_response: None,
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
            self.reclaim_resources()?;
            return Err(error);
        }
        Ok(())
    }

    pub fn configure_network(&mut self, config: logos_abi::NetworkConfig) {
        self.network_config = config;
        self.manager.set_network_enabled(config.is_enabled());
    }

    fn start_inner(&mut self, bundle: &ServiceImageBundle) -> Result<(), ServiceRuntimeError> {
        self.manager.set_network_enabled(self.network_config.is_enabled());
        let resources = crate::arch::boot_resources().ok_or(ServiceRuntimeError::Resources)?;
        if !self.frame_pool_ready {
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
                    .initialize_with_exclusions(resources.memory_map(), &[exclusion])
                    .map_err(|_| ServiceRuntimeError::Resources)?;
            } else {
                self.frame_pool
                    .initialize(resources.memory_map())
                    .map_err(|_| ServiceRuntimeError::Resources)?;
            }
            if !reserve_active_page_tables(&mut self.frame_pool, crate::arch::current_cr3()) {
                return Err(ServiceRuntimeError::Resources);
            }
            self.frame_pool.reserve(FrameAddress::from_raw(0x8000));
            crate::arch::reserve_kernel_frames(&mut self.frame_pool);
            self.frame_pool_ready = true;
        }

        for (index, spec) in SERVICE_IMAGES.iter().enumerate() {
            let service = spec.service();
            let image = unsafe { bundle.image(service) }.ok_or(ServiceRuntimeError::Image)?;
            let plan = spec.validate_image(image).map_err(|_| ServiceRuntimeError::Image)?;
            let stack_pages = match service {
                ServiceId::Storage => crate::process::STORAGE_STACK_PAGES,
                ServiceId::Network => crate::process::NETWORK_STACK_PAGES,
                _ => crate::process::USER_STACK_PAGES,
            };
            let mut loaded =
                LoadedImage::load_with_stack_pages(plan, &mut self.frame_pool, stack_pages)
                    .map_err(ServiceRuntimeError::Load)?;
            let mut memory = IdentityPageTableMemory;
            if let Err(error) = loaded.populate(plan, image, &mut memory) {
                loaded.reclaim(&mut self.frame_pool);
                return Err(ServiceRuntimeError::Populate(error));
            }
            let mut tables = match PageTableBuilder::new(&mut self.frame_pool, &mut memory) {
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
            let process = match self.processes.start(image) {
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
        let mut memory = IdentityPageTableMemory;
        let graph = ServiceIpcGraph::allocate_with_identity(
            &mut self.frame_pool,
            &mut memory,
            self.ipc_generation,
            self.service_epoch,
        )
        .map_err(ServiceRuntimeError::Ipc)?;
        for endpoint_index in 0..graph.count() {
            let endpoint = graph
                .endpoint(endpoint_index)
                .ok_or(ServiceRuntimeError::Ipc(IpcError::Capacity))?;
            initialize_ipc_page(endpoint);
        }
        self.ipc = Some(graph);
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
                let data =
                    self.frame_pool.allocate().map_err(|_| ServiceRuntimeError::Resources)?;
                self.storage_data_frame = Some(data);
                memory.clear(data).map_err(ServiceRuntimeError::IpcPrivateMapping)?;
                self.map_ipc_private_page(
                    process,
                    data,
                    logos_abi::STORAGE_DATA_BASE,
                    MappingFlags::DATA,
                )?;
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
            let capabilities = if service == ServiceId::Storage {
                let mut page = self
                    .ipc
                    .as_ref()
                    .ok_or(ServiceRuntimeError::Ipc(IpcError::Capacity))?
                    .capabilities(service)
                    .map_err(ServiceRuntimeError::Ipc)?;
                page.capabilities[2] = logos_abi::IpcCapability::new(
                    crate::storage_ipc::STORAGE_REQUEST_ENDPOINT,
                    logos_abi::IpcRights::Send,
                    self.ipc_generation,
                    self.service_epoch,
                )
                .ok_or(ServiceRuntimeError::Ipc(IpcError::InvalidIdentity))?;
                page.capabilities[3] = logos_abi::IpcCapability::new(
                    crate::storage_ipc::STORAGE_RESPONSE_ENDPOINT,
                    logos_abi::IpcRights::Receive,
                    self.ipc_generation,
                    self.service_epoch,
                )
                .ok_or(ServiceRuntimeError::Ipc(IpcError::InvalidIdentity))?;
                page
            } else if service == ServiceId::Network {
                let mut page = self
                    .ipc
                    .as_ref()
                    .ok_or(ServiceRuntimeError::Ipc(IpcError::Capacity))?
                    .capabilities(service)
                    .map_err(ServiceRuntimeError::Ipc)?;
                page.capabilities[0] = logos_abi::IpcCapability::new(
                    logos_abi::IpcEndpointId::NetworkToCore.index(),
                    logos_abi::IpcRights::Send,
                    self.ipc_generation,
                    self.service_epoch,
                )
                .ok_or(ServiceRuntimeError::Ipc(IpcError::InvalidIdentity))?;
                page.capabilities[1] = logos_abi::IpcCapability::new(
                    logos_abi::IpcEndpointId::CoreToNetwork.index(),
                    logos_abi::IpcRights::Receive,
                    self.ipc_generation,
                    self.service_epoch,
                )
                .ok_or(ServiceRuntimeError::Ipc(IpcError::InvalidIdentity))?;
                page
            } else {
                self.ipc
                    .as_ref()
                    .ok_or(ServiceRuntimeError::Ipc(IpcError::Capacity))?
                    .capabilities(service)
                    .map_err(ServiceRuntimeError::Ipc)?
            };
            let capability_frame =
                self.frame_pool.allocate().map_err(|_| ServiceRuntimeError::Resources)?;
            self.ipc_capability_frames[index] = Some(capability_frame);
            memory.clear(capability_frame).map_err(ServiceRuntimeError::IpcPrivateMapping)?;
            unsafe {
                (capability_frame.raw() as usize as *mut logos_abi::IpcCapabilityPage)
                    .write(capabilities);
            }
            self.map_ipc_private_page(
                process,
                capability_frame,
                logos_abi::IPC_CAPABILITY_BASE,
                MappingFlags::READ_ONLY_DATA,
            )?;
            let manager_rights = manager_rights(service);
            if manager_rights != logos_abi::ManagerRights::NONE {
                let manager_frame =
                    self.frame_pool.allocate().map_err(|_| ServiceRuntimeError::Resources)?;
                self.manager_capability_frames[service.index()] = Some(manager_frame);
                memory.clear(manager_frame).map_err(ServiceRuntimeError::IpcPrivateMapping)?;
                let capability = logos_abi::ManagerCapability::new(
                    self.manager_generation,
                    manager_rights,
                    self.service_epoch,
                )
                .ok_or(ServiceRuntimeError::StaleGeneration)?;
                unsafe {
                    (manager_frame.raw() as usize as *mut logos_abi::ManagerCapabilityPage)
                        .write(logos_abi::ManagerCapabilityPage { capability });
                }
                self.map_ipc_private_page(
                    process,
                    manager_frame,
                    logos_abi::MANAGER_CAPABILITY_BASE,
                    MappingFlags::READ_ONLY_DATA,
                )?;
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

    pub fn image(&self, service: ServiceId) -> Option<&LoadedImage> {
        let index = service.index();
        if self.table_ready[index] { Some(&self.images[index]) } else { None }
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
        if self.manager.state(ServiceId::Network.index()) == Some(logos_abi::ManagerState::Stopped)
        {
            self.queue_network_link();
        }
        for service in crate::service_images::SERVICE_START_ORDER {
            if (service == ServiceId::Network || service == ServiceId::Fetch)
                && self.manager.state(ServiceId::Network.index())
                    == Some(logos_abi::ManagerState::Disabled)
            {
                continue;
            }
            self.start_service_task(service)?;
            self.startup.start(service).map_err(ServiceRuntimeError::Startup)?;
        }
        self.manager.initialize_running();
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
        self.manager.mark_running(service);
        Ok(())
    }

    /// Reset the bounded image-owned memory and private staging before a
    /// stopped service is started again. The process identity is reused, but
    /// its code, data, BSS, and stack pages are restored to the retained ELF.
    fn reset_service_image(&mut self, service: ServiceId) -> Result<(), ServiceRuntimeError> {
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
            if let Some(frame) = self.storage_data_frame {
                memory.clear(frame).map_err(ServiceRuntimeError::IpcPrivateMapping)?;
            }
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
            self.manager.mark_stopping(service);
        }
        Ok(stop_requested)
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
            self.manager.mark_stopped(service);
            return Ok(false);
        }
        Err(ServiceRuntimeError::TaskStop)
    }

    pub(crate) fn record_heartbeat(
        &self,
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

    pub(crate) fn ipc_send(
        &mut self,
        process: ProcessHandle,
        capability_slot: usize,
        length: usize,
    ) -> crate::service_ipc::IpcOutcome {
        let Some(service) = self.service_for_process(process) else {
            return crate::service_ipc::IpcOutcome {
                status: logos_abi::IpcStatus::Unauthorized,
                notified: false,
            };
        };
        let Some(graph) = self.ipc.as_ref() else {
            return crate::service_ipc::IpcOutcome {
                status: logos_abi::IpcStatus::Disconnected,
                notified: false,
            };
        };
        let Some(capability_frame) = self.ipc_capability_frames[service.index()] else {
            return crate::service_ipc::IpcOutcome {
                status: logos_abi::IpcStatus::Unauthorized,
                notified: false,
            };
        };
        let Some(capability) = (unsafe {
            (&*(capability_frame.raw() as usize as *const logos_abi::IpcCapabilityPage))
                .get(capability_slot)
        }) else {
            return crate::service_ipc::IpcOutcome {
                status: logos_abi::IpcStatus::Unauthorized,
                notified: false,
            };
        };
        if length > crate::loader::PAGE_SIZE {
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
            && capability.endpoint_index() == Some(logos_abi::IpcEndpointId::NetworkToCore.index())
        {
            if length != core::mem::size_of::<logos_abi::NetworkPacketDescriptor>()
                || capability.rights != logos_abi::IpcRights::Send
                || capability.generation != self.ipc_generation
                || capability.service_epoch != self.service_epoch
            {
                return crate::service_ipc::IpcOutcome {
                    status: logos_abi::IpcStatus::Unauthorized,
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
            && capability.endpoint_index() == Some(logos_abi::IpcEndpointId::FlowToNetwork.index())
            && self.manager.state(ServiceId::Network.index())
                == Some(logos_abi::ManagerState::Disabled)
        {
            if length != core::mem::size_of::<logos_abi::IpcBytes>() {
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
            let response_capability = logos_abi::IpcCapability::new(
                logos_abi::IpcEndpointId::NetworkToFlow.index(),
                logos_abi::IpcRights::Send,
                self.ipc_generation,
                self.service_epoch,
            )
            .ok_or(crate::service_ipc::IpcError::InvalidIdentity)
            .map_err(ServiceRuntimeError::Ipc)
            .unwrap_or(logos_abi::IpcCapability::EMPTY);
            let bytes = unsafe {
                core::slice::from_raw_parts(
                    staging_frame.raw() as usize as *const u8,
                    core::mem::size_of::<logos_abi::IpcBytes>(),
                )
            };
            let outcome = graph.send(ServiceId::Network, response_capability, bytes);
            if outcome.notified {
                crate::arch::signal_events(logos_abi::ipc_read_event_mask(
                    logos_abi::IpcEndpointId::NetworkToFlow.index(),
                ));
            }
            return outcome;
        }
        if service == ServiceId::Storage
            && capability.endpoint_index() == Some(crate::storage_ipc::STORAGE_REQUEST_ENDPOINT)
        {
            if capability.rights != logos_abi::IpcRights::Send
                || capability.generation != self.ipc_generation
                || capability.service_epoch != self.service_epoch
            {
                return crate::service_ipc::IpcOutcome {
                    status: logos_abi::IpcStatus::Unauthorized,
                    notified: false,
                };
            }
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
                capability_slot,
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
                    } else if let Some(data) = self.storage_data_frame {
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
            self.storage_response = Some(response);
            crate::arch::signal_events(logos_abi::ipc_write_event_mask(
                crate::storage_ipc::STORAGE_RESPONSE_ENDPOINT,
            ));
            return crate::service_ipc::IpcOutcome {
                status: logos_abi::IpcStatus::Ok,
                notified: true,
            };
        }
        let bytes = unsafe {
            core::slice::from_raw_parts(staging_frame.raw() as usize as *const u8, length)
        };
        #[cfg(feature = "storage-proof")]
        if service == ServiceId::Flow
            && capability.endpoint_index() == Some(logos_abi::IpcEndpointId::FlowToStorage as usize)
        {
            self.storage_proof.observe_request(bytes);
        }
        let outcome = graph.send(service, capability, bytes);
        if outcome.notified
            || (outcome.status == logos_abi::IpcStatus::Ok
                && (capability.endpoint_index()
                    == Some(logos_abi::IpcEndpointId::FlowToNetwork.index())
                    || capability.endpoint_index()
                        == Some(logos_abi::IpcEndpointId::NetworkToFlow.index())))
        {
            crate::arch::signal_events(logos_abi::ipc_read_event_mask(
                capability.endpoint as usize,
            ));
        }
        outcome
    }

    pub(crate) fn ipc_receive(
        &mut self,
        process: ProcessHandle,
        capability_slot: usize,
    ) -> crate::service_ipc::IpcOutcome {
        let Some(service) = self.service_for_process(process) else {
            return crate::service_ipc::IpcOutcome {
                status: logos_abi::IpcStatus::Unauthorized,
                notified: false,
            };
        };
        let Some(graph) = self.ipc.as_ref() else {
            return crate::service_ipc::IpcOutcome {
                status: logos_abi::IpcStatus::Disconnected,
                notified: false,
            };
        };
        let Some(capability_frame) = self.ipc_capability_frames[service.index()] else {
            return crate::service_ipc::IpcOutcome {
                status: logos_abi::IpcStatus::Unauthorized,
                notified: false,
            };
        };
        let Some(capability) = (unsafe {
            (&*(capability_frame.raw() as usize as *const logos_abi::IpcCapabilityPage))
                .get(capability_slot)
        }) else {
            return crate::service_ipc::IpcOutcome {
                status: logos_abi::IpcStatus::Unauthorized,
                notified: false,
            };
        };
        let Some(index) = capability.endpoint_index() else {
            return crate::service_ipc::IpcOutcome {
                status: logos_abi::IpcStatus::Unauthorized,
                notified: false,
            };
        };
        if service == ServiceId::Storage && index == crate::storage_ipc::STORAGE_RESPONSE_ENDPOINT {
            if capability.rights != logos_abi::IpcRights::Receive
                || capability.generation != self.ipc_generation
                || capability.service_epoch != self.service_epoch
            {
                return crate::service_ipc::IpcOutcome {
                    status: logos_abi::IpcStatus::Unauthorized,
                    notified: false,
                };
            }
            let Some(response) = self.storage_response.take() else {
                return crate::service_ipc::IpcOutcome {
                    status: logos_abi::IpcStatus::Empty,
                    notified: false,
                };
            };
            if let Some(staging_frame) = self.ipc_staging_frames[service.index()] {
                unsafe {
                    core::ptr::write_unaligned(
                        staging_frame.raw() as usize as *mut logos_abi::StorageResponse,
                        response,
                    );
                }
                return crate::service_ipc::IpcOutcome {
                    status: logos_abi::IpcStatus::Ok,
                    notified: false,
                };
            }
            return crate::service_ipc::IpcOutcome {
                status: logos_abi::IpcStatus::Unauthorized,
                notified: false,
            };
        }
        if service == ServiceId::Network && index == logos_abi::IpcEndpointId::CoreToNetwork.index()
        {
            if capability.rights != logos_abi::IpcRights::Receive
                || capability.generation != self.ipc_generation
                || capability.service_epoch != self.service_epoch
            {
                return crate::service_ipc::IpcOutcome {
                    status: logos_abi::IpcStatus::Unauthorized,
                    notified: false,
                };
            }
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
                self.network_packet_sequence = self.network_packet_sequence.wrapping_add(1).max(1);
                response
            };
            if let Some(staging_frame) = self.ipc_staging_frames[service.index()] {
                unsafe {
                    core::ptr::write_unaligned(
                        staging_frame.raw() as usize as *mut logos_abi::NetworkPacketDescriptor,
                        response,
                    );
                }
                return crate::service_ipc::IpcOutcome {
                    status: logos_abi::IpcStatus::Ok,
                    notified: false,
                };
            }
            return crate::service_ipc::IpcOutcome {
                status: logos_abi::IpcStatus::Unauthorized,
                notified: false,
            };
        }
        let length = crate::service_ipc::ServiceIpcGraph::message_size(index);
        let Some(staging_frame) = self.ipc_staging_frames[service.index()] else {
            return crate::service_ipc::IpcOutcome {
                status: logos_abi::IpcStatus::Unauthorized,
                notified: false,
            };
        };
        let bytes = unsafe {
            core::slice::from_raw_parts_mut(staging_frame.raw() as usize as *mut u8, length)
        };
        let outcome = graph.receive(service, capability, bytes);
        #[cfg(feature = "qemu-proof")]
        if outcome.status == logos_abi::IpcStatus::Ok
            && service == ServiceId::Flow
            && index == logos_abi::IpcEndpointId::NetworkToFlow.index()
        {
            let message =
                unsafe { core::ptr::read_unaligned(bytes.as_ptr().cast::<logos_abi::IpcBytes>()) };
            if message.kind == logos_abi::MessageKind::NetworkResponse
                && message.len as usize == core::mem::size_of::<logos_abi::NetworkResponse>()
            {
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
        if outcome.status == logos_abi::IpcStatus::Ok
            && service == ServiceId::Flow
            && index == logos_abi::IpcEndpointId::StorageToFlow as usize
        {
            self.storage_proof.observe_response(bytes);
        }
        if outcome.notified {
            crate::arch::signal_events(logos_abi::ipc_write_event_mask(
                capability.endpoint as usize,
            ));
        }
        outcome
    }

    pub(crate) fn manager_call(
        &mut self,
        process: ProcessHandle,
        capability_slot: usize,
        length: usize,
    ) -> logos_abi::IpcStatus {
        let Some(service) = self.service_for_process(process) else {
            return logos_abi::IpcStatus::Unauthorized;
        };
        if capability_slot != logos_abi::MANAGER_CAPABILITY_SLOT {
            return logos_abi::IpcStatus::Unauthorized;
        }
        if length != core::mem::size_of::<logos_abi::ManagerRequest>() {
            return logos_abi::IpcStatus::Malformed;
        }
        let Some(capability_frame) = self.manager_capability_frames[service.index()] else {
            return logos_abi::IpcStatus::Unauthorized;
        };
        let capability = unsafe {
            (capability_frame.raw() as usize as *const logos_abi::ManagerCapabilityPage)
                .read()
                .capability
        };
        if capability.is_empty()
            || capability.generation != self.manager_generation
            || capability.service_epoch != self.service_epoch
        {
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
        let request =
            unsafe { core::ptr::read_unaligned(bytes.cast::<logos_abi::ManagerRequest>()) };
        let mut decision = self.manager.request(request, capability.rights);
        match decision.action {
            ManagerAction::None => {}
            ManagerAction::Start(service) => {
                if self.reset_service_image(service).is_err()
                    || self.start_service_task(service).is_err()
                {
                    self.manager.mark_failed(service);
                    decision.response.status = logos_abi::ManagerStatus::Capacity;
                    self.refresh_manager_response_record(&mut decision.response);
                }
            }
            ManagerAction::Stop(service) => match self.request_stop_service(service) {
                Ok(true) => {}
                Ok(false) => {
                    decision.response.status = logos_abi::ManagerStatus::Ok;
                    if let Some(record) = self.manager.record(service.index()) {
                        decision.response.record = record;
                    }
                }
                Err(_) => {
                    self.manager.mark_failed(service);
                    decision.response.status = logos_abi::ManagerStatus::Busy;
                    self.refresh_manager_response_record(&mut decision.response);
                }
            },
            ManagerAction::Restart(services, count) => {
                if self.pending_restart.is_some()
                    || services[..count].iter().any(|service| {
                        self.tasks[service.index()]
                            .is_none_or(|task| crate::SCHEDULER.state(task).is_none())
                    })
                {
                    decision.response.status = logos_abi::ManagerStatus::Busy;
                } else {
                    let mut admitted = 0;
                    for service in &services[..count] {
                        if self.request_stop_task(*service).is_err() {
                            self.manager.mark_failed(*service);
                            decision.response.status = logos_abi::ManagerStatus::Busy;
                            self.refresh_manager_response_record(&mut decision.response);
                            if admitted != 0 {
                                self.manager.mark_restart_stopping(&services[..admitted]);
                            }
                            break;
                        }
                        admitted += 1;
                    }
                    if admitted == count {
                        self.manager.mark_restart_stopping(&services[..count]);
                        self.refresh_manager_response_record(&mut decision.response);
                        self.pending_restart = Some((services, count));
                    }
                }
            }
        }
        unsafe {
            core::ptr::write_unaligned(
                staging_frame.raw() as usize as *mut logos_abi::ManagerResponse,
                decision.response,
            );
        }
        logos_abi::IpcStatus::Ok
    }

    fn refresh_manager_response_record(&self, response: &mut logos_abi::ManagerResponse) {
        if let Some(record) = self.manager.record(usize::from(response.record.slot)) {
            response.record = record;
        }
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
            logos_abi::MANAGER_CAPABILITY_SLOT,
            core::mem::size_of::<logos_abi::ManagerRequest>(),
        ) != logos_abi::IpcStatus::Ok
        {
            return None;
        }
        Some(unsafe {
            core::ptr::read_unaligned(frame.raw() as usize as *const logos_abi::ManagerResponse)
        })
    }

    fn service_for_process(&self, process: ProcessHandle) -> Option<ServiceId> {
        SERVICE_IMAGES.iter().find_map(|spec| {
            self.launch(spec.service())
                .is_some_and(|(current, _)| current == process)
                .then_some(spec.service())
        })
    }

    #[cfg(feature = "qemu-proof")]
    pub(crate) fn hostile_ipc_layout_valid(&self) -> bool {
        let legacy_end =
            logos_abi::SERVICE_IPC_BASE + logos_abi::IPC_ENDPOINT_COUNT * crate::loader::PAGE_SIZE;
        for spec in SERVICE_IMAGES {
            let service = spec.service();
            let Some((process, _)) = self.launch(service) else {
                return false;
            };
            let mut staging = false;
            let mut capabilities = false;
            let mut manager_capability = false;
            for mapping_index in 0..crate::process::MAX_MAPPINGS_PER_ADDRESS_SPACE {
                let Some(mapping) = self.processes.mapping(process, mapping_index) else {
                    continue;
                };
                let address = mapping.virtual_address();
                let Some(mapping_bytes) = mapping.pages().checked_mul(crate::loader::PAGE_SIZE)
                else {
                    return false;
                };
                let Some(mapping_end) = address.checked_add(mapping_bytes) else {
                    return false;
                };
                if address < legacy_end && mapping_end > logos_abi::SERVICE_IPC_BASE {
                    return false;
                }
                if address == logos_abi::IPC_STAGING_BASE {
                    staging = mapping.flags() == MappingFlags::DATA;
                }
                if address == logos_abi::IPC_CAPABILITY_BASE {
                    capabilities = mapping.flags() == MappingFlags::READ_ONLY_DATA;
                }
                if address == logos_abi::MANAGER_CAPABILITY_BASE {
                    manager_capability = mapping.flags() == MappingFlags::READ_ONLY_DATA;
                }
            }
            if !staging || !capabilities {
                return false;
            }
            if service == ServiceId::Flow && !manager_capability {
                return false;
            }
            if service != ServiceId::Flow && manager_capability {
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
        self.manager_generation = self.manager_generation.wrapping_add(1).max(1);
        self.network_config.service_epoch =
            self.network_config.service_epoch.wrapping_add(1).max(1);
        if !self.supervisor.prepare_restart() {
            return Err(ServiceRuntimeError::RestartLimit);
        }
        self.manager.prepare_graph_restart();
        self.stop_tasks(runtime_guard)?;
        crate::arch::prepare_task_address_space(0);
        crate::arch::restart_critical_section(|| {
            crate::arch::disable_keyboard_irq();
            crate::arch::reset_events();
            self.reclaim_resources()?;
            self.start(bundle)?;
            for suppressed in &self.suppressed_heartbeats {
                suppressed.store(false, Ordering::Release);
            }
            let old_identity = EndpointIdentity {
                generation: self.ipc_generation.wrapping_sub(1).max(1),
                service_epoch: self.service_epoch.wrapping_sub(1).max(1),
            };
            let stale_rejected = self.ipc.as_ref().is_some_and(|graph| {
                (0..graph.count()).all(|index| {
                    graph.endpoint(index).is_some_and(|endpoint| {
                        !old_identity_matches(old_identity, endpoint.header())
                    })
                })
            });
            if !stale_rejected {
                return Err(ServiceRuntimeError::StaleGeneration);
            }
            let result = self.start_tasks();
            if result.is_ok() {
                crate::arch::enable_keyboard_irq();
                crate::arch::finish_service_runtime_transition();
            }
            result
        })
    }

    fn restart_network(
        &mut self,
        runtime_guard: &mut crate::arch::ServiceRuntimeGuard,
    ) -> Result<(), ServiceRuntimeError> {
        let _restart_gate = ServiceRestartGate::acquire();
        if !self.supervisor.prepare_targeted_restart(ServiceId::Network) {
            return Err(ServiceRuntimeError::RestartLimit);
        }
        crate::arch::begin_service_runtime_transition();
        self.manager.mark_stopping(ServiceId::Network);
        self.network_config.service_epoch =
            self.network_config.service_epoch.wrapping_add(1).max(1);
        let index = ServiceId::Network.index();
        if let Some(task) = self.tasks[index] {
            if crate::SCHEDULER.state(task) != Some(crate::TaskState::Completed)
                && !crate::SCHEDULER.request_stop(task)
            {
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
            self.tasks[index] = None;
            self.supervisor.unregister(ServiceId::Network);
        }
        self.reset_service_image(ServiceId::Network)?;
        self.queue_network_link();
        self.start_service_task(ServiceId::Network)?;
        if self.tasks[ServiceId::Fetch.index()].is_none() {
            self.reset_service_image(ServiceId::Fetch)?;
            self.start_service_task(ServiceId::Fetch)?;
        }
        self.manager.restart_complete(&[ServiceId::Fetch, ServiceId::Network]);
        crate::arch::finish_service_runtime_transition();
        #[cfg(feature = "qemu-proof")]
        crate::proof::network_restart_completed();
        Ok(())
    }

    pub fn supervise(
        &mut self,
        bundle: &ServiceImageBundle,
        now: u64,
        runtime_guard: &mut crate::arch::ServiceRuntimeGuard,
    ) -> Result<bool, ServiceRuntimeError> {
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
                    self.manager.mark_failed(service);
                } else {
                    self.manager.mark_stopped(service);
                }
            }
        }
        if let Some((services, count)) = self.pending_restart.take() {
            if services[..count].iter().any(|service| self.tasks[service.index()].is_some()) {
                self.pending_restart = Some((services, count));
                return Ok(false);
            }
            if (count == 1 && services[0] == ServiceId::Network)
                || (count == 2
                    && services[0] == ServiceId::Fetch
                    && services[1] == ServiceId::Network)
            {
                self.restart_network(runtime_guard)?;
                return Ok(true);
            }
            let mut restart_failed = false;
            for service in services[..count].iter().rev() {
                if self.reset_service_image(*service).is_err()
                    || self.start_service_task(*service).is_err()
                {
                    restart_failed = true;
                    break;
                }
            }
            if restart_failed {
                self.restart(bundle, runtime_guard)?;
                return Ok(true);
            }
            self.manager.restart_complete(&services[..count]);
            #[cfg(feature = "qemu-proof")]
            crate::proof::manager_restart_completed();
            return Ok(true);
        }
        if let Some(failed) = SERVICE_IMAGES.iter().find_map(|spec| {
            (self.manager.state(spec.service().index()) == Some(logos_abi::ManagerState::Failed))
                .then_some(spec.service())
        }) {
            if failed == ServiceId::Network {
                self.restart_network(runtime_guard)?;
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
            if failed == ServiceId::Network {
                self.restart_network(runtime_guard)?;
            } else {
                self.restart(bundle, runtime_guard)?;
            }
            return Ok(true);
        }
        Ok(false)
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
        Ok(())
    }

    fn reclaim_resources(&mut self) -> Result<(), ServiceRuntimeError> {
        if let Some(graph) = self.ipc.as_mut() {
            graph.disconnect();
            graph.reclaim(&mut self.frame_pool).map_err(ServiceRuntimeError::Ipc)?;
        }
        self.ipc = None;
        self.storage_response = None;
        self.network_packet_response = None;
        self.network_packet_sequence = self.network_packet_sequence.wrapping_add(1).max(1);
        if let Some(frame) = self.storage_data_frame {
            self.frame_pool.release(frame).map_err(|_| ServiceRuntimeError::Resources)?;
            self.storage_data_frame = None;
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
            if let Some(frame) = self.ipc_capability_frames[index] {
                self.frame_pool.release(frame).map_err(|_| ServiceRuntimeError::Resources)?;
                self.ipc_capability_frames[index] = None;
            }
        }
        for frame in &mut self.manager_capability_frames {
            if let Some(frame) = frame.take() {
                self.frame_pool.release(frame).map_err(|_| ServiceRuntimeError::Resources)?;
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
        self.startup = ServiceStartup::new();
        Ok(())
    }
}

fn old_identity_matches(identity: EndpointIdentity, header: logos_abi::EndpointHeader) -> bool {
    logos_abi::MessageIdentity::new(identity.generation, identity.service_epoch).accepts(header)
}

const fn manager_rights(service: ServiceId) -> logos_abi::ManagerRights {
    match service {
        ServiceId::Flow => logos_abi::ManagerRights::ALL,
        _ => logos_abi::ManagerRights::NONE,
    }
}

impl Default for ServiceRuntime {
    fn default() -> Self {
        Self::new()
    }
}

fn initialize_ipc_page(endpoint: crate::service_ipc::IpcEndpoint) {
    let frame = endpoint.frame().raw() as usize;
    // The frame pool is identity-mapped in the kernel root. Endpoint pages stay
    // kernel-owned; services use private staging pages and syscalls instead.
    unsafe {
        match endpoint.id() {
            logos_abi::IpcEndpointId::InputToTerminal => (frame as *mut logos_abi::InputIpc)
                .write(logos_abi::InputIpc::new(endpoint.header())),
            logos_abi::IpcEndpointId::TerminalToDisplay => (frame as *mut logos_abi::RenderIpc)
                .write(logos_abi::RenderIpc::new(endpoint.header())),
            logos_abi::IpcEndpointId::TerminalToSession
            | logos_abi::IpcEndpointId::SessionToTerminal
            | logos_abi::IpcEndpointId::SessionToFlow
            | logos_abi::IpcEndpointId::FlowToSession
            | logos_abi::IpcEndpointId::FlowToStorage
            | logos_abi::IpcEndpointId::StorageToFlow
            | logos_abi::IpcEndpointId::FlowToNetwork
            | logos_abi::IpcEndpointId::NetworkToFlow
            | logos_abi::IpcEndpointId::FlowToFetch
            | logos_abi::IpcEndpointId::FetchToFlow
            | logos_abi::IpcEndpointId::FetchToStorage
            | logos_abi::IpcEndpointId::StorageToFetch
            | logos_abi::IpcEndpointId::FetchToNetwork
            | logos_abi::IpcEndpointId::NetworkToFetch => (frame as *mut logos_abi::StreamIpc)
                .write(logos_abi::StreamIpc::new(endpoint.header())),
            _ => {}
        }
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
