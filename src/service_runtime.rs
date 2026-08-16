//! Post-UEFI service image and address-space ownership.

use core::{
    mem::MaybeUninit,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};

#[cfg(feature = "storage-proof")]
use core::sync::atomic::AtomicU8;

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
    framebuffer_config_frame: Option<FrameAddress>,
    keyboard_frame: Option<FrameAddress>,
    tasks: [Option<crate::TaskHandle>; SERVICE_COUNT],
    heartbeat_ticks: [AtomicU64; SERVICE_COUNT],
    supervisor: LiveSupervisor,
    ipc_generation: u16,
    service_epoch: u64,
    storage_response: Option<logos_abi::StorageResponse>,
    suppressed_heartbeats: [AtomicBool; SERVICE_COUNT],
    frame_pool_ready: bool,
    #[cfg(feature = "storage-proof")]
    storage_api_proof_mode: AtomicU8,
    #[cfg(feature = "storage-proof")]
    storage_api_proof_pending: AtomicU8,
    #[cfg(feature = "storage-proof")]
    storage_api_proof_missing: AtomicU8,
    #[cfg(feature = "storage-proof")]
    storage_api_proof_reported: AtomicU8,
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
            framebuffer_config_frame: None,
            keyboard_frame: None,
            tasks: [None; SERVICE_COUNT],
            heartbeat_ticks: [const { AtomicU64::new(0) }; SERVICE_COUNT],
            supervisor: LiveSupervisor::new(),
            ipc_generation: 1,
            service_epoch: 1,
            storage_response: None,
            suppressed_heartbeats: [const { AtomicBool::new(false) }; SERVICE_COUNT],
            frame_pool_ready: false,
            #[cfg(feature = "storage-proof")]
            storage_api_proof_mode: AtomicU8::new(0),
            #[cfg(feature = "storage-proof")]
            storage_api_proof_pending: AtomicU8::new(0),
            #[cfg(feature = "storage-proof")]
            storage_api_proof_missing: AtomicU8::new(0),
            #[cfg(feature = "storage-proof")]
            storage_api_proof_reported: AtomicU8::new(0),
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

    fn start_inner(&mut self, bundle: &ServiceImageBundle) -> Result<(), ServiceRuntimeError> {
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
            let stack_pages = if service == ServiceId::Storage {
                crate::process::STORAGE_STACK_PAGES
            } else {
                crate::process::USER_STACK_PAGES
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
        for spec in SERVICE_IMAGES {
            let service = spec.service();
            let Some((process, launch)) = self.launch(service) else {
                return Err(ServiceRuntimeError::TaskLaunch);
            };
            let task = crate::SCHEDULER.spawn_user(service_task_entry, process, launch).map_err(
                |error| match error {
                    crate::SpawnError::Capacity => ServiceRuntimeError::TaskCapacity,
                    crate::SpawnError::AddressSpace => ServiceRuntimeError::TaskAddressSpace,
                    crate::SpawnError::UserLaunch => ServiceRuntimeError::TaskLaunch,
                },
            )?;
            self.tasks[service.index()] = Some(task);
            self.heartbeat_ticks[service.index()].store(crate::current_ticks(), Ordering::Release);
            self.supervisor.register(service, crate::current_ticks());
            self.startup.start(service).map_err(ServiceRuntimeError::Startup)?;
        }
        Ok(())
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

    #[cfg(feature = "qemu-proof")]
    pub(crate) fn suppress_heartbeat(&self, service: ServiceId) {
        self.suppressed_heartbeats[service.index()].store(true, Ordering::Release);
    }

    pub(crate) fn heartbeat_tick(&self, service: ServiceId) -> u64 {
        self.heartbeat_ticks[service.index()].load(Ordering::Acquire)
    }

    #[cfg(feature = "storage-proof")]
    fn observe_storage_api_request(&self, bytes: &[u8]) {
        if bytes.len() != core::mem::size_of::<logos_abi::IpcBytes>() {
            return;
        }
        let message =
            unsafe { core::ptr::read_unaligned(bytes.as_ptr().cast::<logos_abi::IpcBytes>()) };
        let Ok(request) = logos_abi::StorageApiRequest::decode(&message) else {
            return;
        };
        self.storage_api_proof_pending.store(request.operation as u8, Ordering::Release);
    }

    #[cfg(feature = "storage-proof")]
    fn observe_storage_api_response(&self, bytes: &[u8]) {
        if bytes.len() != core::mem::size_of::<logos_abi::IpcBytes>() {
            return;
        }
        let message =
            unsafe { core::ptr::read_unaligned(bytes.as_ptr().cast::<logos_abi::IpcBytes>()) };
        let Ok(response) = logos_abi::StorageApiResponse::decode(&message) else {
            return;
        };
        let operation = self.storage_api_proof_pending.load(Ordering::Acquire);
        if operation == logos_abi::StorageApiOperation::CreateFile as u8 {
            if self.storage_api_proof_mode.load(Ordering::Acquire) == 0 {
                if response.status == logos_abi::StorageApiStatus::Ok {
                    self.storage_api_proof_mode.store(1, Ordering::Release);
                } else if response.status == logos_abi::StorageApiStatus::AlreadyExists {
                    self.storage_api_proof_mode.store(2, Ordering::Release);
                }
            }
        }
        let mode = self.storage_api_proof_mode.load(Ordering::Acquire);
        let expected_data: &[u8] = if mode == 1 { b"durable-api" } else { b"recovered-api" };
        if operation == logos_abi::StorageApiOperation::Read as u8
            && response.status == logos_abi::StorageApiStatus::Ok
            && !response.more
            && response.data == expected_data
            && mode != 0
        {
            let marker: &[u8] = if mode == 1 {
                b"LogOS vNext: storage command API PASS"
            } else {
                b"LogOS vNext: storage command API recovery PASS"
            };
            let expected = if mode == 1 { 1 } else { 2 };
            if self
                .storage_api_proof_reported
                .compare_exchange(0, expected, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                crate::arch_proof_line(marker);
            }
        }
        if response.status == logos_abi::StorageApiStatus::NotFound {
            let missing = self.storage_api_proof_missing.fetch_add(1, Ordering::AcqRel) + 1;
            if missing == 2 {
                crate::arch_proof_line(b"LogOS vNext: storage command API cleanup PASS");
            }
        }
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
        if service == ServiceId::Commands
            && capability.endpoint_index()
                == Some(logos_abi::IpcEndpointId::CommandsToStorage as usize)
        {
            self.observe_storage_api_request(bytes);
        }
        let outcome = graph.send(service, capability, bytes);
        if outcome.notified {
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
        #[cfg(feature = "storage-proof")]
        if outcome.status == logos_abi::IpcStatus::Ok
            && service == ServiceId::Commands
            && index == logos_abi::IpcEndpointId::StorageToCommands as usize
        {
            self.observe_storage_api_response(bytes);
        }
        if outcome.notified {
            crate::arch::signal_events(logos_abi::ipc_write_event_mask(
                capability.endpoint as usize,
            ));
        }
        outcome
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
            let Some((process, _)) = self.launch(spec.service()) else {
                return false;
            };
            let mut staging = false;
            let mut capabilities = false;
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
            }
            if !staging || !capabilities {
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
    pub fn restart(&mut self, bundle: &ServiceImageBundle) -> Result<(), ServiceRuntimeError> {
        crate::arch_proof_line(b"LogOS vNext: service restart begin");
        self.ipc_generation = self.ipc_generation.wrapping_add(1).max(1);
        self.service_epoch = self.service_epoch.wrapping_add(1).max(1);
        if !self.supervisor.prepare_restart() {
            return Err(ServiceRuntimeError::RestartLimit);
        }
        self.stop_tasks()?;
        crate::arch::prepare_task_address_space(0);
        let result = crate::arch::restart_critical_section(|| {
            crate::arch_proof_line(b"LogOS vNext: service tasks quiesced");
            crate::arch::disable_keyboard_irq();
            crate::arch::reset_events();
            self.reclaim_resources()?;
            crate::arch_proof_line(b"LogOS vNext: service resources reclaimed");
            self.start(bundle)?;
            crate::arch_proof_line(b"LogOS vNext: service graph rebuilt");
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
            }
            result
        });
        if result.is_ok() {
            crate::arch_proof_line(b"LogOS vNext: service restart complete");
        }
        result
    }

    pub fn supervise(
        &mut self,
        bundle: &ServiceImageBundle,
        now: u64,
    ) -> Result<bool, ServiceRuntimeError> {
        let mut heartbeats = [0; SERVICE_COUNT];
        let mut process_states = [None; SERVICE_COUNT];
        for spec in SERVICE_IMAGES {
            let index = spec.service().index();
            heartbeats[index] = self.heartbeat_tick(spec.service());
            process_states[index] =
                self.launch(spec.service()).and_then(|(process, _)| self.processes.state(process));
        }
        if self.supervisor.poll(now, heartbeats, process_states).is_some() {
            self.restart(bundle)?;
            return Ok(true);
        }
        Ok(false)
    }

    fn stop_tasks(&mut self) -> Result<(), ServiceRuntimeError> {
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
                crate::sleep_current_for(1);
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
        if let Some(frame) = self.storage_data_frame {
            self.frame_pool.release(frame).map_err(|_| ServiceRuntimeError::Resources)?;
            self.storage_data_frame = None;
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
            | logos_abi::IpcEndpointId::SessionToCommands
            | logos_abi::IpcEndpointId::CommandsToSession
            | logos_abi::IpcEndpointId::CommandsToStorage
            | logos_abi::IpcEndpointId::StorageToCommands => (frame as *mut logos_abi::StreamIpc)
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
