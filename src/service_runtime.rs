//! Post-UEFI service image and address-space ownership.

use core::mem::MaybeUninit;

use logos_abi::{CapabilityKind, ServiceId};

use crate::{
    frame_pool::{FrameAddress, FramePool},
    loader::{LoadError, LoadedImage},
    page_table::{IdentityPageTableMemory, PageTableBuilder, PageTableError, PageTableMemory},
    process::{
        AddressSpaceRoot, Capabilities, MappingFlags, ProcessError, ProcessHandle, UserLaunch,
        VirtualMapping,
    },
    service_images::SERVICE_IMAGES,
    service_ipc::{IpcError, ServiceIpcGraph},
    service_loader::ServiceImageBundle,
    service_startup::ServiceStartup,
};

const SERVICE_COUNT: usize = SERVICE_IMAGES.len();

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
    IpcMapping(PageTableError),
    IpcProcess(ProcessError),
    Framebuffer(PageTableError),
    FramebufferProcess(ProcessError),
    FramebufferConfig(PageTableError),
    FramebufferConfigProcess(ProcessError),
    Keyboard(PageTableError),
    KeyboardProcess(ProcessError),
    TaskCapacity,
    TaskAddressSpace,
    TaskLaunch,
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
    framebuffer_config_frame: Option<FrameAddress>,
    keyboard_frame: Option<FrameAddress>,
    tasks: [Option<crate::TaskHandle>; SERVICE_COUNT],
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
            ],
            tables: [const { MaybeUninit::uninit() }; SERVICE_COUNT],
            table_ready: [false; SERVICE_COUNT],
            processes: crate::process::ProcessTable::new(),
            launches: [None; SERVICE_COUNT],
            startup: ServiceStartup::new(),
            ipc: None,
            framebuffer_config_frame: None,
            keyboard_frame: None,
            tasks: [None; SERVICE_COUNT],
        }
    }

    pub fn start(&mut self, bundle: &ServiceImageBundle) -> Result<(), ServiceRuntimeError> {
        let resources = crate::arch::boot_resources().ok_or(ServiceRuntimeError::Resources)?;
        self.frame_pool
            .initialize(resources.memory_map())
            .map_err(|_| ServiceRuntimeError::Resources)?;

        for (index, spec) in SERVICE_IMAGES.iter().enumerate() {
            let service = spec.service();
            let image = unsafe { bundle.image(service) }.ok_or(ServiceRuntimeError::Image)?;
            self.startup.mark_image(service).map_err(ServiceRuntimeError::Startup)?;
            let plan = spec.validate_image(image).map_err(|_| ServiceRuntimeError::Image)?;
            let loaded =
                LoadedImage::load(plan, &mut self.frame_pool).map_err(ServiceRuntimeError::Load)?;
            let mut memory = IdentityPageTableMemory;
            if let Err(error) = loaded.populate(plan, image, &mut memory) {
                let mut loaded = loaded;
                loaded.reclaim(&mut self.frame_pool);
                return Err(ServiceRuntimeError::Populate(error));
            }
            let mut tables = match PageTableBuilder::new(&mut self.frame_pool, &mut memory) {
                Ok(tables) => tables,
                Err(error) => {
                    let mut loaded = loaded;
                    loaded.reclaim(&mut self.frame_pool);
                    return Err(ServiceRuntimeError::PageTableRoot(error));
                }
            };
            if let Err(error) = tables.map_image(&loaded, &mut self.frame_pool, &mut memory) {
                tables.reclaim(&mut self.frame_pool, &mut memory);
                let mut loaded = loaded;
                loaded.reclaim(&mut self.frame_pool);
                return Err(ServiceRuntimeError::PageTableMap(error));
            }
            let process = self
                .processes
                .start(image, spec.process_kind(), capabilities(spec))
                .map_err(ServiceRuntimeError::Process)?;
            let root = AddressSpaceRoot::new(tables.root().raw() as usize)
                .ok_or(ServiceRuntimeError::Process(ProcessError::AddressSpace))?;
            if let Err(error) = self.processes.bind_address_space_root(process, root) {
                let _ = self.processes.exit(process, 1);
                let _ = self.processes.reclaim(process);
                return Err(ServiceRuntimeError::Process(error));
            }
            if let Err(error) = map_loaded_pages(&mut self.processes, process, &loaded) {
                let _ = self.processes.exit(process, 1);
                let _ = self.processes.reclaim(process);
                return Err(ServiceRuntimeError::Process(error));
            }
            let launch = self
                .processes
                .user_launch(process, loaded.entry(), loaded.stack_top())
                .map_err(ServiceRuntimeError::Process)?;
            self.launches[index] = Some((process, launch));
            self.images[index] = loaded;
            self.tables[index].write(tables);
            self.table_ready[index] = true;
            self.startup.mark_address_space(service).map_err(ServiceRuntimeError::Startup)?;
            self.startup.mark_process(service).map_err(ServiceRuntimeError::Startup)?;
        }
        let mut memory = IdentityPageTableMemory;
        let graph = ServiceIpcGraph::allocate(&mut self.frame_pool, &mut memory)
            .map_err(ServiceRuntimeError::Ipc)?;
        for endpoint_index in 0..graph.count() {
            let endpoint = graph
                .endpoint(endpoint_index)
                .ok_or(ServiceRuntimeError::Ipc(IpcError::Capacity))?;
            initialize_ipc_page(endpoint, endpoint_index);
            for service in [endpoint.producer(), endpoint.consumer()] {
                let index = service_index(service);
                let Some((process, _)) = self.launch(service) else {
                    return Err(ServiceRuntimeError::IpcProcess(ProcessError::InvalidHandle));
                };
                let tables = unsafe { self.tables[index].assume_init_mut() };
                tables
                    .map_raw_page(
                        endpoint.virtual_address(),
                        endpoint.frame(),
                        MappingFlags::DATA,
                        &mut self.frame_pool,
                        &mut memory,
                    )
                    .map_err(ServiceRuntimeError::IpcMapping)?;
                let mapping = VirtualMapping::new(
                    endpoint.virtual_address(),
                    endpoint.frame().raw() as usize,
                    1,
                    MappingFlags::DATA,
                )
                .ok_or(ServiceRuntimeError::IpcProcess(ProcessError::AddressSpace))?;
                self.processes.map(process, mapping).map_err(ServiceRuntimeError::IpcProcess)?;
            }
        }
        self.ipc = Some(graph);
        let framebuffer = resources.framebuffer().ok_or(ServiceRuntimeError::Resources)?;
        self.map_framebuffer(framebuffer)?;
        let framebuffer_config_frame =
            self.frame_pool.allocate().map_err(|_| ServiceRuntimeError::Resources)?;
        memory.clear(framebuffer_config_frame).map_err(ServiceRuntimeError::FramebufferConfig)?;
        initialize_framebuffer_config(framebuffer_config_frame, framebuffer);
        self.map_framebuffer_config(framebuffer_config_frame)?;
        self.framebuffer_config_frame = Some(framebuffer_config_frame);
        let keyboard_frame =
            self.frame_pool.allocate().map_err(|_| ServiceRuntimeError::Resources)?;
        memory.clear(keyboard_frame).map_err(ServiceRuntimeError::Keyboard)?;
        self.map_keyboard_ring(keyboard_frame)?;
        self.keyboard_frame = Some(keyboard_frame);
        for spec in SERVICE_IMAGES {
            self.startup.mark_launch_ready(spec.service()).map_err(ServiceRuntimeError::Startup)?;
        }
        Ok(())
    }

    pub fn image(&self, service: ServiceId) -> Option<&LoadedImage> {
        let index = service_index(service);
        if self.table_ready[index] { Some(&self.images[index]) } else { None }
    }

    pub fn root(&self, service: ServiceId) -> Option<usize> {
        let index = service_index(service);
        if !self.table_ready[index] {
            return None;
        }
        // SAFETY: `table_ready` is set only after the corresponding builder is
        // initialized and remains true for the runtime lifetime.
        Some(unsafe { self.tables[index].assume_init_ref().root().raw() as usize })
    }

    pub fn launch(&self, service: ServiceId) -> Option<(ProcessHandle, UserLaunch)> {
        self.launches[service_index(service)]
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
        let index = service_index(service);
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
        let index = service_index(service);
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

    fn map_framebuffer_config(&mut self, frame: FrameAddress) -> Result<(), ServiceRuntimeError> {
        let service = ServiceId::Display;
        let index = service_index(service);
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
            self.tasks[service_index(service)] = Some(task);
            self.startup.start(service).map_err(ServiceRuntimeError::Startup)?;
        }
        Ok(())
    }
}

impl Default for ServiceRuntime {
    fn default() -> Self {
        Self::new()
    }
}

const fn service_index(service: ServiceId) -> usize {
    match service {
        ServiceId::Input => 0,
        ServiceId::Display => 1,
        ServiceId::Terminal => 2,
        ServiceId::Session => 3,
        ServiceId::Commands => 4,
    }
}

fn capabilities(spec: &crate::service_images::ServiceImageSpec) -> Capabilities {
    let mut capabilities = Capabilities::NONE;
    let mut index = 0;
    while index < spec.capability_count() {
        let Some(grant) = spec.capability(index) else {
            break;
        };
        match grant.kind {
            CapabilityKind::IpcEndpoint => capabilities.endpoints = true,
            CapabilityKind::KeyboardBytes => capabilities.input = true,
            CapabilityKind::Framebuffer => capabilities.display = true,
            CapabilityKind::ProcessControl => capabilities.process_control = true,
            CapabilityKind::ServiceControl => {}
        }
        index += 1;
    }
    capabilities
}

fn initialize_ipc_page(endpoint: crate::service_ipc::IpcEndpoint, index: usize) {
    let frame = endpoint.frame().raw() as usize;
    // The frame pool is identity-mapped in the kernel root. The same physical
    // page is then mapped into exactly the two endpoint participants.
    unsafe {
        match index {
            0 => (frame as *mut logos_abi::InputIpc)
                .write(logos_abi::InputIpc::new(endpoint.header())),
            1 => (frame as *mut logos_abi::RenderIpc)
                .write(logos_abi::RenderIpc::new(endpoint.header())),
            2..=5 => (frame as *mut logos_abi::StreamIpc)
                .write(logos_abi::StreamIpc::new(endpoint.header())),
            _ => {}
        }
    }
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
