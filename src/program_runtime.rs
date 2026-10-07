//! Bounded program lifecycle: start, stop, reclaim and process lookup.
//!
//! `ProgramRuntime` owns the fixed program slots and everything a running
//! program holds: its process, address-space tables, loaded image, surface IPC
//! endpoints and the private staging/bootstrap frames. Frame pool, process
//! table, IPC and event registries and the scheduler are borrowed through
//! [`ProgramDeps`]; nothing here creates or stores them. Package admission and
//! image population stay with the caller, which hands over a populated
//! [`LoadedImage`]; `start` owns it from then on and reclaims it on failure.

use alloc::{boxed::Box, vec::Vec};

use logos_abi::{CapabilityHandle, ServiceHandle};

use crate::{
    SpawnError, TaskEntry, TaskHandle, TaskState,
    frame_pool::{FrameAddress, FramePool},
    loader::{LoadedImage, PageSink, map_loaded_pages},
    memory::OwnerId,
    page_table::{PageTableBuilder, PageTableError, PageTableMemory},
    process::{
        AddressSpaceRoot, MappingFlags, ProcessError, ProcessHandle, ProcessState, ProcessTable,
        VirtualMapping,
    },
    runtime_events::RuntimeEventRegistry,
    runtime_ipc::{PROGRAM_SURFACE_DRAW_MESSAGE_BYTES, RuntimeIpcRegistry},
    scheduler::Scheduler,
    service_ipc::IpcError,
    service_manager::MAX_PROGRAM_SLOTS,
};

pub(crate) const MAX_PROGRAMS: usize = MAX_PROGRAM_SLOTS;
const PROGRAM_CLIENT_HANDLE_BASE: u32 = 0x8000_0000;
const PROGRAM_SURFACE_REQUEST_QUEUE: usize = 1;
const PROGRAM_SURFACE_RESPONSE_QUEUE: usize = 1;
const PROGRAM_SURFACE_INPUT_QUEUE: usize = 32;
const PROGRAM_SURFACE_RENDER_QUEUE: usize = 1;
const PROGRAM_SURFACE_DRAW_QUEUE: usize = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProgramError {
    Resources,
    PageTableRoot(PageTableError),
    PageTableMap(PageTableError),
    Process(ProcessError),
    Ipc(IpcError),
    IpcPrivateMapping(PageTableError),
    IpcPrivateProcess(ProcessError),
    TaskCapacity,
    TaskAddressSpace,
    TaskLaunch,
    TaskStop,
}

/// How a reaped program ended; the caller maps this onto manager state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProgramExit {
    Exited,
    Faulted,
    /// Forced stop, or a slot that had no process left to inspect.
    Stopped,
}

/// Borrowed dependencies for lifecycle calls.
pub(crate) struct ProgramDeps<'a, M> {
    pub frame_pool: &'a mut FramePool,
    pub processes: &'a mut ProcessTable,
    pub ipc: &'a mut Option<RuntimeIpcRegistry>,
    pub events: &'a mut Option<RuntimeEventRegistry>,
    pub scheduler: &'a Scheduler,
    pub memory: &'a mut M,
}

/// Everything `start` needs besides its dependencies.
pub(crate) struct ProgramLaunch {
    pub slot: usize,
    pub record: logos_abi::ServiceManagerRecord,
    /// Populated image owned by [`ProgramRuntime::owner_for_slot`].
    pub image: LoadedImage,
    pub plan: crate::process::ElfLoadPlan,
    pub atrium: ServiceHandle,
    pub ipc_generation: u16,
    pub service_epoch: u64,
    pub entry: TaskEntry,
}

pub(crate) fn program_client_handle(
    slot: usize,
    generation: u32,
) -> Result<ServiceHandle, ProgramError> {
    let slot = u32::try_from(slot).map_err(|_| ProgramError::Resources)?;
    let index = PROGRAM_CLIENT_HANDLE_BASE.checked_add(slot).ok_or(ProgramError::Resources)?;
    ServiceHandle::new(index, generation).ok_or(ProgramError::Resources)
}

struct ProgramSlot {
    generation: u32,
    client: ServiceHandle,
    ipc_staging: Option<FrameAddress>,
    bootstrap: Option<FrameAddress>,
    process: Option<ProcessHandle>,
    task: Option<TaskHandle>,
    image: Option<Box<LoadedImage>>,
    table: Option<Box<PageTableBuilder>>,
}

impl ProgramSlot {
    const fn empty() -> Self {
        Self {
            generation: 0,
            client: ServiceHandle::EMPTY,
            ipc_staging: None,
            bootstrap: None,
            process: None,
            task: None,
            image: None,
            table: None,
        }
    }
}

#[derive(Clone, Copy)]
struct ProgramSurfaceCapabilities {
    request: CapabilityHandle,
    response: CapabilityHandle,
    input: CapabilityHandle,
    render: CapabilityHandle,
    draw: CapabilityHandle,
}

pub(crate) struct ProgramRuntime {
    slots: [ProgramSlot; MAX_PROGRAMS],
}

impl ProgramRuntime {
    pub(crate) const fn new() -> Self {
        Self { slots: [const { ProgramSlot::empty() }; MAX_PROGRAMS] }
    }

    /// Frame owner the caller must use when loading the image for `slot`.
    pub(crate) fn owner_for_slot(slot: usize) -> Option<OwnerId> {
        OwnerId::new(100 + u16::try_from(slot).ok()?)
    }

    /// True when `slot` is in range and holds no running program.
    pub(crate) fn slot_available(&self, slot: usize) -> bool {
        self.slots.get(slot).is_some_and(|program| program.task.is_none())
    }

    pub(crate) fn task(&self, slot: usize) -> Option<TaskHandle> {
        self.slots.get(slot).and_then(|program| program.task)
    }

    pub(crate) fn request_stop(
        &self,
        scheduler: &Scheduler,
        slot: usize,
    ) -> Result<(), ProgramError> {
        let task = self.task(slot).ok_or(ProgramError::TaskStop)?;
        if scheduler.request_stop(task) { Ok(()) } else { Err(ProgramError::TaskStop) }
    }

    pub(crate) fn client_for_process(&self, process: ProcessHandle) -> Option<ServiceHandle> {
        self.slot_for_process(process).map(|slot| self.slots[slot].client)
    }

    pub(crate) fn staging_for_process(&self, process: ProcessHandle) -> Option<FrameAddress> {
        self.slot_for_process(process).and_then(|slot| self.slots[slot].ipc_staging)
    }

    fn slot_for_process(&self, process: ProcessHandle) -> Option<usize> {
        self.slots
            .iter()
            .position(|program| program.process == Some(process) && program.client.is_valid())
    }

    /// Start a program in `launch.slot`. Consumes the image; every failure
    /// leaves no frame, endpoint, process or task behind.
    pub(crate) fn start<M: PageTableMemory + PageSink>(
        &mut self,
        deps: &mut ProgramDeps<'_, M>,
        launch: ProgramLaunch,
    ) -> Result<(), ProgramError> {
        let ProgramLaunch {
            slot,
            record,
            mut image,
            plan,
            atrium,
            ipc_generation,
            service_epoch,
            entry,
        } = launch;
        let Some(owner) = Self::owner_for_slot(slot).filter(|_| self.slot_available(slot)) else {
            image.reclaim(deps.frame_pool);
            return Err(ProgramError::TaskCapacity);
        };
        let mut tables = match PageTableBuilder::new_for_owner(deps.frame_pool, deps.memory, owner)
        {
            Ok(tables) => tables,
            Err(error) => {
                image.reclaim(deps.frame_pool);
                return Err(ProgramError::PageTableRoot(error));
            }
        };
        if let Err(error) = tables.map_image(&image, deps.frame_pool, deps.memory) {
            abort_start(deps, None, Some(tables), image, ServiceHandle::EMPTY, None, None);
            return Err(ProgramError::PageTableMap(error));
        }
        let process = match deps.processes.start_plan(plan) {
            Ok(process) => process,
            Err(error) => {
                abort_start(deps, None, Some(tables), image, ServiceHandle::EMPTY, None, None);
                return Err(ProgramError::Process(error));
            }
        };
        macro_rules! fail_early {
            ($error:expr) => {{
                abort_start(
                    deps,
                    Some(process),
                    Some(tables),
                    image,
                    ServiceHandle::EMPTY,
                    None,
                    None,
                );
                return Err($error);
            }};
        }
        let Some(root) = AddressSpaceRoot::new(tables.root().raw() as usize) else {
            fail_early!(ProgramError::Process(ProcessError::AddressSpace));
        };
        if let Err(error) = deps.processes.bind_address_space_root(process, root) {
            fail_early!(ProgramError::Process(error));
        }
        if let Err(error) = map_loaded_pages(deps.processes, process, &image) {
            fail_early!(ProgramError::Process(error));
        }
        let client = match program_client_handle(slot, record.program_generation) {
            Ok(client) => client,
            Err(error) => fail_early!(error),
        };
        let capabilities = match provision_surface_ipc(deps, client, atrium, service_epoch) {
            Ok(capabilities) => capabilities,
            Err(error) => fail_early!(error),
        };
        macro_rules! fail_late {
            ($error:expr, $staging:expr, $bootstrap:expr) => {{
                abort_start(deps, Some(process), Some(tables), image, client, $staging, $bootstrap);
                return Err($error);
            }};
        }
        let Ok(staging) = deps.frame_pool.allocate_for(owner) else {
            fail_late!(ProgramError::Resources, None, None);
        };
        let Ok(bootstrap) = deps.frame_pool.allocate_for(owner) else {
            fail_late!(ProgramError::Resources, Some(staging), None);
        };
        if PageTableMemory::clear(deps.memory, staging).is_err()
            || PageTableMemory::clear(deps.memory, bootstrap).is_err()
        {
            fail_late!(
                ProgramError::IpcPrivateMapping(PageTableError::InvalidVirtualAddress),
                Some(staging),
                Some(bootstrap)
            );
        }
        let page = logos_abi::ProgramBootstrapPage {
            abi_version: logos_abi::RUNTIME_ABI_VERSION,
            flags: 0,
            ipc_generation,
            reserved: 0,
            program_generation: record.program_generation,
            client,
            surface_request: capabilities.request,
            surface_response: capabilities.response,
            surface_input: capabilities.input,
            surface_render: capabilities.render,
            surface_draw: capabilities.draw,
        };
        // SAFETY: `ProgramBootstrapPage` is plain `repr(C)` data; the bytes are
        // copied into the frame exactly as the former raw-pointer store did.
        let page_bytes = unsafe {
            core::slice::from_raw_parts(
                (&page as *const logos_abi::ProgramBootstrapPage).cast::<u8>(),
                core::mem::size_of::<logos_abi::ProgramBootstrapPage>(),
            )
        };
        if PageSink::write(deps.memory, bootstrap, 0, page_bytes).is_err() {
            fail_late!(
                ProgramError::IpcPrivateMapping(PageTableError::InvalidVirtualAddress),
                Some(staging),
                Some(bootstrap)
            );
        }
        for (frame, address, flags) in [
            (staging, logos_abi::IPC_STAGING_BASE, MappingFlags::DATA),
            (bootstrap, logos_abi::PROGRAM_BOOTSTRAP_BASE, MappingFlags::READ_ONLY_DATA),
        ] {
            if tables.map_raw_page(address, frame, flags, deps.frame_pool, deps.memory).is_err() {
                fail_late!(
                    ProgramError::IpcPrivateMapping(PageTableError::InvalidVirtualAddress),
                    Some(staging),
                    Some(bootstrap)
                );
            }
            let Some(mapping) = VirtualMapping::new(address, frame.raw() as usize, 1, flags) else {
                fail_late!(
                    ProgramError::IpcPrivateProcess(ProcessError::AddressSpace),
                    Some(staging),
                    Some(bootstrap)
                );
            };
            if deps.processes.map(process, mapping).is_err() {
                fail_late!(
                    ProgramError::IpcPrivateProcess(ProcessError::AddressSpace),
                    Some(staging),
                    Some(bootstrap)
                );
            }
        }
        let user_launch =
            match deps.processes.user_launch(process, image.entry(), image.stack_top()) {
                Ok(launch) => launch,
                Err(error) => {
                    fail_late!(ProgramError::Process(error), Some(staging), Some(bootstrap))
                }
            };
        let task = match deps.scheduler.spawn_user(entry, process, user_launch) {
            Ok(task) => task,
            Err(error) => fail_late!(
                match error {
                    SpawnError::Capacity => ProgramError::TaskCapacity,
                    SpawnError::AddressSpace => ProgramError::TaskAddressSpace,
                    SpawnError::UserLaunch => ProgramError::TaskLaunch,
                },
                Some(staging),
                Some(bootstrap)
            ),
        };
        self.slots[slot] = ProgramSlot {
            generation: record.program_generation,
            client,
            ipc_staging: Some(staging),
            bootstrap: Some(bootstrap),
            process: Some(process),
            task: Some(task),
            image: Some(Box::new(image)),
            table: Some(Box::new(tables)),
        };
        Ok(())
    }

    /// Reclaim `slot` if its task has completed. `Ok(None)` means still running
    /// (or empty). Process-state-dependent errors are tolerated, as before.
    pub(crate) fn reap<M: PageTableMemory>(
        &mut self,
        deps: &mut ProgramDeps<'_, M>,
        slot: usize,
    ) -> Result<Option<(u32, ProgramExit)>, ProgramError> {
        let Some(task) = self.task(slot) else { return Ok(None) };
        if deps.scheduler.state(task) != Some(TaskState::Completed) {
            return Ok(None);
        }
        if !deps.scheduler.reclaim_completed(task) {
            return Err(ProgramError::TaskStop);
        }
        let generation = self.slots[slot].generation;
        let Some(process) = self.slots[slot].process.take() else {
            return Ok(Some((generation, ProgramExit::Stopped)));
        };
        let state = deps.processes.state(process).unwrap_or(ProcessState::Faulted(0xff));
        let forced_stop = matches!(state, ProcessState::Running);
        if forced_stop {
            let _ = deps.processes.exit(process, 0xff);
        }
        let terminal = deps.processes.state(process).unwrap_or(ProcessState::Faulted(0xff));
        let _ = deps.processes.reclaim(process);
        self.release_surface_resources(deps, slot);
        self.release_tables_and_image(deps, slot);
        let exit = if matches!(terminal, ProcessState::Exited(_)) && !forced_stop {
            ProgramExit::Exited
        } else if matches!(terminal, ProcessState::Faulted(_)) {
            ProgramExit::Faulted
        } else {
            ProgramExit::Stopped
        };
        Ok(Some((generation, exit)))
    }

    /// Reclaim a program whose task the caller has already waited to
    /// completion (runtime shutdown). Unlike `reap`, process errors propagate.
    pub(crate) fn finish_stop<M: PageTableMemory>(
        &mut self,
        deps: &mut ProgramDeps<'_, M>,
        slot: usize,
    ) -> Result<u32, ProgramError> {
        let task = self.task(slot).ok_or(ProgramError::TaskStop)?;
        if !deps.scheduler.reclaim_completed(task) {
            return Err(ProgramError::TaskStop);
        }
        if let Some(process) = self.slots[slot].process.take() {
            if deps.processes.state(process) == Some(ProcessState::Running) {
                deps.processes.exit(process, 0xff).map_err(ProgramError::Process)?;
            }
            deps.processes.reclaim(process).map_err(ProgramError::Process)?;
        }
        self.release_surface_resources(deps, slot);
        self.release_tables_and_image(deps, slot);
        Ok(self.slots[slot].generation)
    }

    /// Drop every program without involving the scheduler (runtime restart).
    pub(crate) fn discard_all<M: PageTableMemory>(&mut self, deps: &mut ProgramDeps<'_, M>) {
        for slot in 0..MAX_PROGRAMS {
            if let Some(process) = self.slots[slot].process.take() {
                if deps.processes.state(process) == Some(ProcessState::Running) {
                    let _ = deps.processes.exit(process, 0xff);
                }
                let _ = deps.processes.reclaim(process);
            }
            self.release_surface_resources(deps, slot);
            self.release_tables_and_image(deps, slot);
        }
    }

    /// Destroy every program's surface endpoints, keeping the slots intact.
    pub(crate) fn destroy_all_surface_ipc(
        &self,
        ipc: &mut Option<RuntimeIpcRegistry>,
        events: &mut Option<RuntimeEventRegistry>,
        frame_pool: &mut FramePool,
    ) {
        for program in &self.slots {
            if program.client.is_valid() {
                destroy_surface_ipc(ipc, events, frame_pool, program.client);
            }
        }
    }

    fn release_surface_resources<M>(&mut self, deps: &mut ProgramDeps<'_, M>, slot: usize) {
        let client = self.slots[slot].client;
        if client.is_valid() {
            destroy_surface_ipc(deps.ipc, deps.events, deps.frame_pool, client);
        }
        let program = &mut self.slots[slot];
        for frame in [program.ipc_staging.take(), program.bootstrap.take()].into_iter().flatten() {
            let _ = deps.frame_pool.release(frame);
        }
        program.client = ServiceHandle::EMPTY;
    }

    fn release_tables_and_image<M: PageTableMemory>(
        &mut self,
        deps: &mut ProgramDeps<'_, M>,
        slot: usize,
    ) {
        let program = &mut self.slots[slot];
        if let Some(mut table) = program.table.take() {
            table.reclaim(deps.frame_pool, deps.memory);
        }
        if let Some(mut image) = program.image.take() {
            image.reclaim(deps.frame_pool);
        }
        program.task = None;
    }
}

fn destroy_surface_ipc(
    ipc: &mut Option<RuntimeIpcRegistry>,
    events: &mut Option<RuntimeEventRegistry>,
    frame_pool: &mut FramePool,
    client: ServiceHandle,
) {
    if let (Some(ipc), Some(events)) = (ipc.as_mut(), events.as_mut()) {
        ipc.destroy_service_with_pool(client, events, frame_pool);
    }
}

fn abort_start<M: PageTableMemory>(
    deps: &mut ProgramDeps<'_, M>,
    process: Option<ProcessHandle>,
    tables: Option<PageTableBuilder>,
    mut image: LoadedImage,
    client: ServiceHandle,
    staging: Option<FrameAddress>,
    bootstrap: Option<FrameAddress>,
) {
    if client.is_valid() {
        destroy_surface_ipc(deps.ipc, deps.events, deps.frame_pool, client);
    }
    if let Some(process) = process {
        let _ = deps.processes.exit(process, 1);
        let _ = deps.processes.reclaim(process);
    }
    if let Some(mut tables) = tables {
        tables.reclaim(deps.frame_pool, deps.memory);
    }
    for frame in [staging, bootstrap].into_iter().flatten() {
        let _ = deps.frame_pool.release(frame);
    }
    image.reclaim(deps.frame_pool);
}

fn provision_surface_ipc<M>(
    deps: &mut ProgramDeps<'_, M>,
    client: ServiceHandle,
    atrium: ServiceHandle,
    service_epoch: u64,
) -> Result<ProgramSurfaceCapabilities, ProgramError> {
    let specs = [
        (
            client,
            atrium,
            logos_abi::IPC_CONTRACT_ATRIUM_SURFACE_REQUEST,
            core::mem::size_of::<logos_abi::AtriumSurfaceRequest>(),
            PROGRAM_SURFACE_REQUEST_QUEUE,
        ),
        (
            atrium,
            client,
            logos_abi::IPC_CONTRACT_ATRIUM_SURFACE_RESPONSE,
            core::mem::size_of::<logos_abi::AtriumSurfaceResponse>(),
            PROGRAM_SURFACE_RESPONSE_QUEUE,
        ),
        (
            atrium,
            client,
            logos_abi::IPC_CONTRACT_ATRIUM_SURFACE_INPUT,
            core::mem::size_of::<logos_abi::AtriumSurfaceInput>(),
            PROGRAM_SURFACE_INPUT_QUEUE,
        ),
        (
            client,
            atrium,
            logos_abi::IPC_CONTRACT_RENDER,
            core::mem::size_of::<logos_abi::RenderMessage>(),
            PROGRAM_SURFACE_RENDER_QUEUE,
        ),
        (
            client,
            atrium,
            logos_abi::IPC_CONTRACT_ATRIUM_SURFACE_DRAW,
            PROGRAM_SURFACE_DRAW_MESSAGE_BYTES,
            PROGRAM_SURFACE_DRAW_QUEUE,
        ),
    ];
    let result = (|| {
        for (producer, consumer, contract, bytes, queue_capacity) in specs {
            let mut queue_frames = Vec::new();
            if queue_frames.try_reserve(queue_capacity).is_err() {
                return Err(ProgramError::Resources);
            }
            for _ in 0..queue_capacity {
                match deps.frame_pool.allocate_for(OwnerId::KERNEL) {
                    Ok(frame) => queue_frames.push(frame),
                    Err(_) => {
                        for frame in queue_frames {
                            let _ = deps.frame_pool.release(frame);
                        }
                        return Err(ProgramError::Resources);
                    }
                }
            }
            let endpoint = match (deps.ipc.as_mut(), deps.events.as_mut()) {
                (Some(ipc), Some(events)) => ipc.create_endpoint_with_frames(
                    producer,
                    consumer,
                    contract,
                    bytes,
                    queue_capacity,
                    service_epoch,
                    &queue_frames,
                    events,
                ),
                _ => Err(logos_abi::IpcStatus::Disconnected),
            };
            let Ok(endpoint) = endpoint else {
                for frame in queue_frames {
                    let _ = deps.frame_pool.release(frame);
                }
                return Err(ProgramError::Ipc(IpcError::Capacity));
            };
            let granted = match deps.ipc.as_mut() {
                Some(ipc) => ipc
                    .grant(producer, endpoint, logos_abi::IpcRights::Send)
                    .and_then(|_| ipc.grant(consumer, endpoint, logos_abi::IpcRights::Receive)),
                None => Err(logos_abi::IpcStatus::Disconnected),
            };
            if granted.is_err() {
                if let (Some(ipc), Some(events)) = (deps.ipc.as_mut(), deps.events.as_mut()) {
                    let _ = ipc.destroy_endpoint_with_pool(endpoint, events, deps.frame_pool);
                }
                return Err(ProgramError::Ipc(IpcError::Capacity));
            }
        }
        let capability = |peer: ServiceHandle, contract: u16, rights: logos_abi::IpcRights| {
            program_capability(deps.ipc.as_ref(), client, peer, contract, rights)
        };
        Ok(ProgramSurfaceCapabilities {
            request: capability(
                atrium,
                logos_abi::IPC_CONTRACT_ATRIUM_SURFACE_REQUEST,
                logos_abi::IpcRights::Send,
            )?,
            response: capability(
                atrium,
                logos_abi::IPC_CONTRACT_ATRIUM_SURFACE_RESPONSE,
                logos_abi::IpcRights::Receive,
            )?,
            input: capability(
                atrium,
                logos_abi::IPC_CONTRACT_ATRIUM_SURFACE_INPUT,
                logos_abi::IpcRights::Receive,
            )?,
            render: capability(atrium, logos_abi::IPC_CONTRACT_RENDER, logos_abi::IpcRights::Send)?,
            draw: capability(
                atrium,
                logos_abi::IPC_CONTRACT_ATRIUM_SURFACE_DRAW,
                logos_abi::IpcRights::Send,
            )?,
        })
    })();
    if result.is_err() {
        destroy_surface_ipc(deps.ipc, deps.events, deps.frame_pool, client);
    }
    result
}

fn program_capability(
    ipc: Option<&RuntimeIpcRegistry>,
    client: ServiceHandle,
    peer: ServiceHandle,
    contract: u16,
    rights: logos_abi::IpcRights,
) -> Result<CapabilityHandle, ProgramError> {
    let ipc = ipc.ok_or(ProgramError::Ipc(IpcError::Capacity))?;
    let send = rights == logos_abi::IpcRights::Send;
    let endpoint = ipc
        .find_endpoint(if send { client } else { peer }, if send { peer } else { client }, contract)
        .map_err(|_| ProgramError::Ipc(IpcError::Capacity))?;
    ipc.capability_for(client, endpoint, rights).map_err(|_| ProgramError::Ipc(IpcError::Capacity))
}
