use crate::{
    arch::cpu::{EntryState, Privilege},
    mm::{address_space::AddressSpace, memory::PhysicalMemory},
    platform::payload::Payload,
    sched::scheduler::{Event, Runnable, TaskState},
};

const TASKS: usize = 5;

#[derive(Clone, Copy, Default)]
pub struct EndpointPages {
    pub input: bool,
    pub display: bool,
}

impl EndpointPages {
    pub const NONE: Self = Self { input: false, display: false };
    pub const TERMINAL: Self = Self { input: true, display: true };
}

pub struct Task<'a> {
    privilege: &'a Privilege,
    payload: Payload,
    space: AddressSpace,
    entry: u64,
    context_physical: u64,
    context: u64,
    input_page_physical: Option<u64>,
    display_page_physical: Option<u64>,
    endpoint_pages: EndpointPages,
    generation: u32,
    started: bool,
    event: Event,
    complete: bool,
    gate: crate::arch::cpu::GateState,
}

#[derive(Clone, Copy)]
pub struct InputEndpoint {
    page_physical: Option<u64>,
    generation: u32,
}

impl InputEndpoint {
    pub fn deliver(self, input: logos_abi::InputEvent) -> bool {
        self.page_physical.is_some_and(|page| unsafe {
            logos_core::native_service::InputPage::deliver_at(page, self.generation, input.byte())
        })
    }

    #[cfg(feature = "test-hooks")]
    pub fn deliver_raw(self, input: u8) -> bool {
        self.page_physical.is_some_and(|page| unsafe {
            logos_abi::service::InputPage::deliver_at(page, self.generation, input)
        })
    }
}

#[derive(Clone, Copy)]
pub struct SyscallEndpoint {
    context_physical: u64,
}

#[derive(Clone, Copy)]
#[allow(dead_code)]
pub struct StoreEndpoint {
    context_physical: u64,
}

#[derive(Clone, Copy)]
#[allow(dead_code)]
pub struct BlockEndpoint {
    context_physical: u64,
}

#[derive(Clone, Copy)]
#[allow(dead_code)]
pub struct NetworkEndpoint {
    context_physical: u64,
}

#[derive(Clone, Copy)]
pub struct RemoteEndpoint {
    context_physical: u64,
}

impl RemoteEndpoint {
    pub fn request(self) -> Option<logos_core::native_service::RemoteGateRequest> {
        unsafe { logos_core::native_service::ControlPage::remote_gate_at(self.context_physical) }
    }

    pub fn reply(self, reply: logos_core::native_service::RemoteGateReply) -> bool {
        unsafe {
            logos_core::native_service::ControlPage::reply_remote_gate_at(
                self.context_physical,
                reply,
            )
        }
    }
}

#[derive(Clone, Copy)]
pub struct SessionEndpoint {
    context_physical: u64,
}

#[derive(Clone, Copy)]
pub struct DisplayEndpoint {
    context_physical: u64,
    page_physical: Option<u64>,
    generation: u32,
}

impl DisplayEndpoint {
    pub fn pending(self) -> bool {
        self.page_physical.is_some_and(|page| unsafe {
            logos_core::native_service::DisplayPage::pending_at(page, self.generation)
        })
    }

    #[allow(dead_code)]
    pub const fn context(self) -> u64 {
        self.context_physical
    }

    pub const fn page(self) -> Option<u64> {
        self.page_physical
    }

    pub const fn generation(self) -> u32 {
        self.generation
    }
}

impl SyscallEndpoint {
    pub fn request(self) -> Option<logos_abi::SessionRequest> {
        unsafe { logos_core::native_service::ControlPage::syscall_at(self.context_physical) }
    }

    pub fn reply(self, bytes: &[u8]) -> bool {
        unsafe { logos_core::native_service::ControlPage::reply_at(self.context_physical, bytes) }
    }

    #[cfg(feature = "test-hooks")]
    pub fn reply_matches(self, expected: &[u8]) -> bool {
        unsafe { logos_core::native_service::ControlPage::response_at(self.context_physical) }
            .is_some_and(|response| response.text[..response.length] == *expected)
    }
}

impl StoreEndpoint {
    pub const fn unavailable() -> Self {
        Self { context_physical: 0 }
    }

    pub const fn available(self) -> bool {
        self.context_physical != 0
    }

    pub const fn context(self) -> u64 {
        self.context_physical
    }

    pub fn request(self) -> Option<logos_abi::StoreRequest> {
        if !self.available() {
            return None;
        }
        unsafe { logos_core::native_service::ControlPage::store_at(self.context_physical) }
    }

    pub fn deliver(self, request: logos_abi::StoreRequest) -> bool {
        if !self.available() {
            return false;
        }
        unsafe {
            logos_core::native_service::ControlPage::deliver_store_at(
                self.context_physical,
                request,
            )
        }
    }

    pub fn response(self, expected_id: u32) -> Option<logos_abi::StoreReply> {
        if !self.available() {
            return None;
        }
        unsafe {
            logos_core::native_service::ControlPage::store_reply_at(
                self.context_physical,
                expected_id,
            )
        }
    }

    pub fn reply(self, reply: logos_abi::StoreReply) -> bool {
        if !self.available() {
            return false;
        }
        unsafe {
            logos_core::native_service::ControlPage::reply_store_at(self.context_physical, reply)
        }
    }

    pub fn configure_shared_page(self, page: logos_abi::PageHandle) -> bool {
        if !self.available() {
            return false;
        }
        unsafe {
            logos_core::native_service::ControlPage::configure_shared_page_at(
                self.context_physical,
                page,
            )
        }
    }

    pub fn remap_shared_page(self, page: logos_abi::PageHandle) -> bool {
        if !self.available() {
            return false;
        }
        unsafe {
            logos_abi::service::ControlPage::remap_shared_page_at(self.context_physical, page)
        }
    }
}

impl BlockEndpoint {
    pub const fn unavailable() -> Self {
        Self { context_physical: 0 }
    }

    pub const fn available(self) -> bool {
        self.context_physical != 0
    }

    pub const fn context(self) -> u64 {
        self.context_physical
    }

    pub fn configure(self, page: logos_core::native_service::BlockPage) -> bool {
        if !self.available() {
            return false;
        }
        unsafe {
            logos_core::native_service::ControlPage::configure_block_page_at(
                self.context_physical,
                page,
            )
        }
    }

    #[allow(dead_code)]
    pub fn request(self) -> Option<logos_abi::BlockRequest> {
        if !self.available() {
            return None;
        }
        unsafe { logos_core::native_service::ControlPage::block_at(self.context_physical) }
    }

    #[allow(dead_code)]
    pub fn reply(self, reply: logos_abi::BlockReply) -> bool {
        if !self.available() {
            return false;
        }
        unsafe {
            logos_core::native_service::ControlPage::reply_block_at(self.context_physical, reply)
        }
    }
}

#[allow(dead_code)]
impl NetworkEndpoint {
    pub const fn context(self) -> u64 {
        self.context_physical
    }

    pub fn configure(self, pages: logos_core::native_service::NetworkPages) -> bool {
        unsafe {
            logos_core::native_service::ControlPage::configure_network_pages_at(
                self.context_physical,
                pages.rx_handle,
                pages.tx_handle,
            )
        }
    }

    pub fn request(self) -> Option<logos_abi::NetworkRequest> {
        unsafe { logos_core::native_service::ControlPage::network_at(self.context_physical) }
    }

    pub fn deliver(self, request: logos_abi::NetworkRequest) -> bool {
        unsafe {
            logos_core::native_service::ControlPage::deliver_network_at(
                self.context_physical,
                request,
            )
        }
    }

    pub fn deliver_for_owner(self, request: logos_abi::NetworkRequest, owner: u64) -> bool {
        unsafe {
            logos_abi::service::ControlPage::deliver_network_for_owner_at(
                self.context_physical,
                request,
                owner,
            )
        }
    }

    pub fn reply(self, reply: logos_abi::NetworkReply) -> bool {
        unsafe {
            logos_core::native_service::ControlPage::reply_network_at(self.context_physical, reply)
        }
    }

    pub fn response(self, expected_id: u32) -> Option<logos_abi::NetworkReply> {
        unsafe {
            logos_core::native_service::ControlPage::network_reply_at(
                self.context_physical,
                expected_id,
            )
        }
    }

    pub fn deliver_event(self, event: logos_abi::NetworkEvent) -> bool {
        unsafe {
            logos_core::native_service::ControlPage::deliver_network_event_at(
                self.context_physical,
                event,
            )
        }
    }

    pub fn event(self) -> Option<logos_abi::NetworkEvent> {
        unsafe { logos_core::native_service::ControlPage::network_event_at(self.context_physical) }
    }

    pub fn device_request(self) -> Option<logos_abi::NetworkDeviceRequest> {
        unsafe { logos_core::native_service::ControlPage::network_device_at(self.context_physical) }
    }

    pub fn reply_device(self, reply: logos_abi::NetworkDeviceReply) -> bool {
        unsafe {
            logos_core::native_service::ControlPage::reply_network_device_at(
                self.context_physical,
                reply,
            )
        }
    }

    pub fn deliver_device_reply(self, reply: logos_abi::NetworkDeviceReply) -> bool {
        unsafe {
            logos_core::native_service::ControlPage::deliver_network_device_reply_at(
                self.context_physical,
                reply,
            )
        }
    }
}

impl SessionEndpoint {
    #[cfg_attr(not(feature = "test-hooks"), allow(dead_code))]
    pub const fn context(self) -> u64 {
        self.context_physical
    }

    pub fn deliver(self, request: logos_abi::SessionRequest) -> bool {
        unsafe {
            logos_core::native_service::ControlPage::deliver_session_at(
                self.context_physical,
                request,
            )
        }
    }

    pub fn reply(self) -> Option<logos_abi::SessionReply> {
        unsafe { logos_core::native_service::ControlPage::session_reply_at(self.context_physical) }
    }

    pub fn effect(self) -> Option<logos_abi::EffectRequest> {
        unsafe { logos_core::native_service::ControlPage::session_effect_at(self.context_physical) }
    }

    pub fn reply_effect(self, reply: logos_abi::EffectResult) -> bool {
        unsafe {
            logos_core::native_service::ControlPage::reply_effect_at(self.context_physical, reply)
        }
    }

    #[allow(dead_code)]
    pub fn reply_effect_with_text(self, reply: logos_abi::EffectReply) -> bool {
        unsafe {
            logos_core::native_service::ControlPage::reply_effect_with_text_at(
                self.context_physical,
                reply,
            )
        }
    }
}

impl<'a> Task<'a> {
    pub fn load(
        memory: &mut PhysicalMemory,
        payload: Payload,
        privilege: &'a Privilege,
        endpoint_pages: EndpointPages,
    ) -> Option<Self> {
        let mut space = AddressSpace::new(memory)?;
        let Some(entry) = space.map_image(memory, payload) else {
            let _ = space.release(memory);
            return None;
        };
        let Some(mapping) = space.map_context(memory, endpoint_pages.input, endpoint_pages.display)
        else {
            let _ = space.release(memory);
            return None;
        };
        let (context_physical, context) = mapping.context;
        Some(Self {
            privilege,
            payload,
            space,
            entry,
            context_physical,
            context,
            input_page_physical: mapping.input.map(|(physical, _)| physical),
            display_page_physical: mapping.display.map(|(physical, _)| physical),
            endpoint_pages,
            generation: 1,
            started: false,
            event: Event::INPUT,
            complete: false,
            gate: crate::arch::cpu::GateState::new(),
        })
    }

    pub fn start(&mut self) -> bool {
        let state =
            self.privilege.run_entry(&mut self.space, self.entry, self.context, &mut self.gate);
        self.started = true;
        self.advance(state)
    }

    pub fn input_endpoint(&self) -> Option<InputEndpoint> {
        self.input_page_physical.map(|page_physical| InputEndpoint {
            page_physical: Some(page_physical),
            generation: self.generation,
        })
    }

    pub const fn syscall_endpoint(&self) -> SyscallEndpoint {
        SyscallEndpoint { context_physical: self.context_physical }
    }

    pub fn display_endpoint(&self) -> Option<DisplayEndpoint> {
        self.display_page_physical.map(|page_physical| DisplayEndpoint {
            context_physical: self.context_physical,
            page_physical: Some(page_physical),
            generation: self.generation,
        })
    }

    pub fn set_generation(&mut self, generation: u16) -> bool {
        let generation = u32::from(generation.max(1));
        if !unsafe {
            logos_core::native_service::ControlPage::set_generation_at(
                self.context_physical,
                generation,
            )
        } {
            return false;
        }
        if let Some(page) = self.input_page_physical {
            if !unsafe { logos_core::native_service::InputPage::reset_at(page, generation) } {
                return false;
            }
        }
        if let Some(page) = self.display_page_physical {
            if !unsafe { logos_core::native_service::DisplayPage::reset_at(page, generation) } {
                return false;
            }
        }
        self.generation = generation;
        true
    }

    pub const fn session_endpoint(&self) -> SessionEndpoint {
        SessionEndpoint { context_physical: self.context_physical }
    }

    #[allow(dead_code)]
    pub const fn store_endpoint(&self) -> StoreEndpoint {
        StoreEndpoint { context_physical: self.context_physical }
    }

    #[allow(dead_code)]
    pub const fn block_endpoint(&self) -> BlockEndpoint {
        BlockEndpoint { context_physical: self.context_physical }
    }

    #[allow(dead_code)]
    pub const fn network_endpoint(&self) -> NetworkEndpoint {
        NetworkEndpoint { context_physical: self.context_physical }
    }

    pub const fn remote_endpoint(&self) -> RemoteEndpoint {
        RemoteEndpoint { context_physical: self.context_physical }
    }

    pub fn map_shared_owned(&mut self, memory: &mut PhysicalMemory) -> Option<u64> {
        self.space.map_shared_owned(memory)
    }

    pub fn map_shared_borrowed(&mut self, address: u64) -> bool {
        self.space.map_shared_borrowed(address)
    }

    pub fn remap_shared_borrowed(&mut self, address: u64) -> bool {
        self.space.remap_shared_borrowed(address)
    }

    pub fn map_block_owned(&mut self, memory: &mut PhysicalMemory) -> Option<(u64, u64)> {
        self.space.map_block_owned(memory)
    }

    pub fn map_network_owned(
        &mut self,
        memory: &mut PhysicalMemory,
    ) -> Option<((u64, u64), (u64, u64))> {
        self.space.map_network_owned(memory)
    }

    pub fn map_heap(&mut self, memory: &mut PhysicalMemory) -> Option<u64> {
        self.space.map_heap(memory)
    }

    pub fn resume(&mut self) -> bool {
        let state = self.privilege.resume_entry(&mut self.space, self.context, &mut self.gate);
        self.advance(state)
    }

    pub const fn complete(&self) -> bool {
        self.complete
    }

    pub fn release(self, memory: &mut PhysicalMemory) -> bool {
        self.space.release(memory)
    }

    fn run_state(&mut self) -> TaskState {
        if self.complete {
            return TaskState::Complete;
        }
        if !self.started {
            return if self.start() { TaskState::Blocked(self.event) } else { TaskState::Failed };
        }
        if self.resume() {
            if self.complete { TaskState::Complete } else { TaskState::Blocked(self.event) }
        } else {
            TaskState::Failed
        }
    }

    fn advance(&mut self, state: Option<EntryState>) -> bool {
        match state {
            Some(EntryState::Input) | Some(EntryState::Command) | Some(EntryState::Display) => {
                self.event = if state == Some(EntryState::Command) {
                    Event::COMMAND
                } else if state == Some(EntryState::Display) {
                    Event::DISPLAY
                } else {
                    Event::INPUT
                };
                true
            }
            Some(EntryState::Returned) => {
                self.complete = unsafe {
                    logos_core::native_service::ControlPage::complete_at(self.context_physical)
                };
                self.complete
            }
            Some(EntryState::Panic) => false,
            Some(EntryState::Fault(_)) => false,
            None => false,
        }
    }
}

impl Runnable for Task<'_> {
    fn run(&mut self) -> TaskState {
        self.run_state()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Handle(u32);

impl Handle {
    pub const fn unavailable() -> Self {
        Self(0)
    }

    pub const fn available(self) -> bool {
        self.0 != 0
    }
    pub const fn generation(self) -> u16 {
        (self.0 >> 16) as u16
    }

    const fn index(self) -> usize {
        self.0 as u16 as usize
    }

    const fn new(index: usize, generation: u16) -> Self {
        Self((generation as u32) << 16 | index as u32)
    }
}

struct Entry<'a> {
    task: Task<'a>,
    waiting: Option<Event>,
    generation: u16,
}

pub struct Scheduler<'a> {
    tasks: [Option<Entry<'a>>; TASKS],
    generations: [u16; TASKS],
    next: usize,
}

impl<'a> Scheduler<'a> {
    pub const fn new() -> Self {
        Self { tasks: [const { None }; TASKS], generations: [1; TASKS], next: 0 }
    }

    pub fn spawn(&mut self, mut task: Task<'a>) -> Option<Handle> {
        for (index, slot) in self.tasks.iter_mut().enumerate() {
            if slot.is_none() {
                let generation = self.generations[index];
                if !task.set_generation(generation) {
                    return None;
                }
                *slot = Some(Entry { task, waiting: None, generation });
                return Some(Handle::new(index, generation));
            }
        }
        None
    }

    pub fn input_endpoint(&self, handle: Handle) -> Option<InputEndpoint> {
        self.entry(handle)?.task.input_endpoint()
    }

    pub fn syscall_endpoint(&self, handle: Handle) -> Option<SyscallEndpoint> {
        Some(self.entry(handle)?.task.syscall_endpoint())
    }

    pub fn display_endpoint(&self, handle: Handle) -> Option<DisplayEndpoint> {
        self.entry(handle)?.task.display_endpoint()
    }

    pub fn session_endpoint(&self, handle: Handle) -> Option<SessionEndpoint> {
        Some(self.entry(handle)?.task.session_endpoint())
    }

    pub fn store_endpoint(&self, handle: Handle) -> Option<StoreEndpoint> {
        Some(self.entry(handle)?.task.store_endpoint())
    }

    pub fn remote_endpoint(&self, handle: Handle) -> Option<RemoteEndpoint> {
        Some(self.entry(handle)?.task.remote_endpoint())
    }

    pub fn block_endpoint(&self, handle: Handle) -> Option<BlockEndpoint> {
        Some(self.entry(handle)?.task.block_endpoint())
    }

    pub fn network_endpoint(&self, handle: Handle) -> Option<NetworkEndpoint> {
        Some(self.entry(handle)?.task.network_endpoint())
    }

    pub fn task_mut(&mut self, handle: Handle) -> Option<&mut Task<'a>> {
        let entry = self.entry_mut(handle)?;
        Some(&mut entry.task)
    }

    pub fn run_next(&mut self) -> bool {
        for _ in 0..TASKS {
            let index = self.next;
            self.next = (self.next + 1) % TASKS;
            if self.run_index(index) {
                return true;
            }
        }
        false
    }

    pub fn run(&mut self, handle: Handle) -> bool {
        self.entry(handle).is_some() && self.run_index(handle.index())
    }

    pub fn wake(&mut self, handle: Handle) -> bool {
        let Some(entry) = self.entry_mut(handle) else { return false };
        if entry.waiting.is_none() {
            return false;
        }
        entry.waiting = None;
        crate::platform::trace::record(crate::platform::trace::Event::TaskWoken);
        true
    }

    pub fn fail(&mut self, handle: Handle) -> bool {
        let Some(entry) = self.entry_mut(handle) else { return false };
        if entry.waiting == Some(Event::FAILURE) {
            return false;
        }
        entry.waiting = Some(Event::FAILURE);
        crate::platform::trace::record(crate::platform::trace::Event::Fault);
        true
    }

    pub fn failed(&self, handle: Handle) -> bool {
        self.entry(handle).is_some_and(|entry| entry.waiting == Some(Event::FAILURE))
    }

    pub fn replace(
        &mut self,
        handle: Handle,
        memory: &mut PhysicalMemory,
        configure: impl FnOnce(&mut Task<'a>, &mut PhysicalMemory) -> bool,
    ) -> Option<Handle> {
        let entry = self.entry(handle)?;
        if entry.waiting != Some(Event::FAILURE) {
            return None;
        }
        let index = handle.index();
        let generation = self.generations[index].wrapping_add(1).max(1);
        let mut replacement = Task::load(
            memory,
            entry.task.payload,
            entry.task.privilege,
            entry.task.endpoint_pages,
        )?;
        if !replacement.set_generation(generation) {
            let _ = replacement.release(memory);
            return None;
        }
        if !configure(&mut replacement, memory) {
            let _ = replacement.release(memory);
            return None;
        }
        let old = self.tasks[index].take()?;
        self.generations[index] = generation;
        if !old.task.release(memory) {
            let _ = replacement.release(memory);
            return None;
        }
        self.tasks[index] = Some(Entry { task: replacement, waiting: None, generation });
        Some(Handle::new(index, generation))
    }

    fn entry(&self, handle: Handle) -> Option<&Entry<'a>> {
        self.tasks
            .get(handle.index())?
            .as_ref()
            .filter(|entry| entry.generation == handle.generation())
    }

    fn entry_mut(&mut self, handle: Handle) -> Option<&mut Entry<'a>> {
        self.tasks
            .get_mut(handle.index())?
            .as_mut()
            .filter(|entry| entry.generation == handle.generation())
    }

    fn run_index(&mut self, index: usize) -> bool {
        let Some(mut entry) = self.tasks[index].take() else { return false };
        if entry.waiting.is_some() {
            self.tasks[index] = Some(entry);
            return false;
        }
        match entry.task.run_state() {
            TaskState::Ready => self.tasks[index] = Some(entry),
            TaskState::Blocked(event) => {
                entry.waiting = Some(event);
                self.tasks[index] = Some(entry);
                crate::platform::trace::record(crate::platform::trace::Event::TaskBlocked);
            }
            TaskState::Complete => {
                self.generations[index] = self.generations[index].wrapping_add(1).max(1);
            }
            TaskState::Failed => {
                entry.waiting = Some(Event::FAILURE);
                self.tasks[index] = Some(entry);
                crate::platform::trace::record(crate::platform::trace::Event::Fault);
            }
        }
        true
    }
}
