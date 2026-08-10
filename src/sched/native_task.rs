use crate::platform::services::{EndpointDescriptor, EndpointKind};
use crate::{
    arch::cpu::{EntryState, Privilege},
    mm::{address_space::AddressSpace, memory::PhysicalMemory},
    platform::payload::Payload,
    sched::scheduler::{Event, Runnable, TaskState},
};

const TASKS: usize = 5;

pub type EndpointPages = &'static [EndpointDescriptor];
const MAX_ENDPOINTS: usize = 14;

#[derive(Clone, Copy)]
struct EndpointMapping {
    kind: EndpointKind,
    physical: u64,
    virtual_address: u64,
    generation: u32,
}

#[derive(Clone, Copy)]
struct EndpointMappings {
    entries: [Option<EndpointMapping>; MAX_ENDPOINTS],
}

impl EndpointMappings {
    const fn from_context(
        entries: [Option<crate::mm::address_space::ContextMappingEntry>; 14],
    ) -> Self {
        let mut mappings = [None; MAX_ENDPOINTS];
        let mut index = 0;
        while index < entries.len() {
            if let Some(entry) = entries[index] {
                mappings[index] = Some(EndpointMapping {
                    kind: entry.kind,
                    physical: entry.physical,
                    virtual_address: entry.virtual_address,
                    generation: 1,
                });
            }
            index += 1;
        }
        Self { entries: mappings }
    }

    fn get(&self, kind: EndpointKind) -> Option<EndpointMapping> {
        self.entries.iter().flatten().find(|entry| entry.kind == kind).copied()
    }

    fn set_generation(&mut self, generation: u32) {
        for entry in self.entries.iter_mut().flatten() {
            entry.generation = generation;
        }
    }
}

pub struct Task<'a> {
    privilege: &'a Privilege,
    payload: Payload,
    space: AddressSpace,
    entry: u64,
    context_physical: u64,
    context: u64,
    endpoint_mappings: EndpointMappings,
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
pub struct SessionClientEndpoint {
    page_physical: u64,
    service_generation: u32,
    endpoint_generation: u32,
}

impl SessionClientEndpoint {
    pub fn reply(
        self,
        id: u32,
        status: logos_abi::service::SessionStatus,
        reply: logos_abi::SessionReply,
    ) -> bool {
        unsafe {
            logos_abi::service::SessionClientPage::reply_at(
                self.page_physical,
                self.service_generation,
                self.endpoint_generation,
                id,
                status,
                reply,
            )
        }
    }
}

#[derive(Clone, Copy)]
pub struct SessionServerEndpoint {
    page_physical: u64,
    service_generation: u32,
    endpoint_generation: u32,
}

impl SessionServerEndpoint {
    pub fn deliver(self, id: u32, caller: u64, request: logos_abi::SessionRequest) -> bool {
        unsafe {
            logos_abi::service::SessionServerPage::deliver_at(
                self.page_physical,
                self.service_generation,
                self.endpoint_generation,
                id,
                caller,
                request,
            )
        }
    }

    pub fn reply(self, id: u32) -> Option<logos_abi::service::SessionServerReply> {
        unsafe {
            logos_abi::service::SessionServerPage::take_reply_at(
                self.page_physical,
                self.service_generation,
                self.endpoint_generation,
                id,
            )
        }
    }
}

#[derive(Clone, Copy)]
pub struct EffectEndpoint {
    page_physical: u64,
    service_generation: u32,
    endpoint_generation: u32,
}

impl EffectEndpoint {
    pub fn request(self) -> Option<logos_abi::service::EffectMessage> {
        unsafe {
            logos_abi::service::EffectPage::take_at(
                self.page_physical,
                self.service_generation,
                self.endpoint_generation,
            )
        }
    }

    pub fn reply(self, id: u32, reply: logos_abi::EffectReply) -> bool {
        unsafe {
            logos_abi::service::EffectPage::reply_at(
                self.page_physical,
                self.service_generation,
                self.endpoint_generation,
                id,
                reply,
            )
        }
    }

    pub fn waiting_id(self) -> Option<u32> {
        unsafe {
            logos_abi::service::EffectPage::waiting_id_at(
                self.page_physical,
                self.service_generation,
                self.endpoint_generation,
            )
        }
    }
}

#[derive(Clone, Copy)]
pub struct SyscallEndpoint {
    client: SessionClientEndpoint,
}

#[derive(Clone, Copy)]
pub struct StoreClientEndpoint {
    context_physical: u64,
    page_physical: u64,
    service_generation: u32,
    endpoint_generation: u32,
}

#[derive(Clone, Copy)]
pub struct StoreServerEndpoint {
    context_physical: u64,
    page_physical: u64,
    service_generation: u32,
    endpoint_generation: u32,
}

#[derive(Clone, Copy)]
pub struct BlockClientEndpoint {
    context_physical: u64,
    page_physical: u64,
    service_generation: u32,
    endpoint_generation: u32,
}

#[derive(Clone, Copy)]
#[allow(dead_code)]
pub struct NetworkEndpoint {
    context_physical: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct NetworkClientEndpoint {
    page_physical: u64,
    stream_page_physical: u64,
    service_generation: u32,
    endpoint_generation: u32,
}

impl NetworkClientEndpoint {
    pub fn configure_transfer(self, handle: logos_abi::PageHandle) -> bool {
        unsafe {
            logos_core::native_service::NetworkClientPage::configure_transfer_at(
                self.page_physical,
                self.service_generation,
                self.endpoint_generation,
                handle,
            )
        }
    }

    pub fn transfer_page(self) -> Option<logos_abi::PageHandle> {
        unsafe {
            logos_core::native_service::NetworkClientPage::transfer_page_at(
                self.page_physical,
                self.service_generation,
                self.endpoint_generation,
            )
        }
    }

    #[cfg_attr(not(feature = "test-hooks"), allow(dead_code))]
    pub fn issue(self, request: logos_abi::NetworkRequest) -> bool {
        unsafe {
            logos_core::native_service::NetworkClientPage::request_at(
                self.page_physical,
                self.service_generation,
                self.endpoint_generation,
                request,
            )
        }
    }

    pub fn request(self) -> Option<logos_abi::NetworkRequest> {
        unsafe {
            logos_core::native_service::NetworkClientPage::request_at_page(
                self.page_physical,
                self.service_generation,
                self.endpoint_generation,
            )
        }
    }

    pub fn mark_processing(self) -> bool {
        unsafe {
            logos_core::native_service::NetworkClientPage::mark_processing_at(
                self.page_physical,
                self.service_generation,
                self.endpoint_generation,
            )
        }
    }

    pub fn reply(self, reply: logos_abi::NetworkReply) -> bool {
        unsafe {
            logos_core::native_service::NetworkClientPage::reply_at(
                self.page_physical,
                self.service_generation,
                self.endpoint_generation,
                reply,
            )
        }
    }

    pub fn reply_request(self, reply: logos_abi::NetworkReply) -> bool {
        unsafe {
            logos_core::native_service::NetworkClientPage::reply_request_at(
                self.page_physical,
                self.service_generation,
                self.endpoint_generation,
                reply,
            )
        }
    }

    #[cfg_attr(not(feature = "test-hooks"), allow(dead_code))]
    pub fn response(self, expected_id: u32) -> Option<logos_abi::NetworkReply> {
        unsafe {
            logos_core::native_service::NetworkClientPage::finish_at(
                self.page_physical,
                self.service_generation,
                self.endpoint_generation,
                expected_id,
            )
        }
    }

    pub fn publish_stream(self, record: logos_abi::NetworkStreamRecord) -> bool {
        self.stream_page_physical != 0
            && unsafe {
                logos_abi::service::StreamPage::publish_at(
                    self.stream_page_physical,
                    self.service_generation,
                    self.endpoint_generation,
                    record,
                )
            }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct NetworkServerEndpoint {
    context_physical: u64,
    page_physical: u64,
    service_generation: u32,
    endpoint_generation: u32,
}

impl NetworkServerEndpoint {
    pub fn deliver(self, caller: u64, request: logos_abi::NetworkRequest) -> bool {
        let accepted = unsafe {
            logos_core::native_service::NetworkServerPage::deliver_at(
                self.page_physical,
                self.service_generation,
                self.endpoint_generation,
                caller,
                request,
            )
        };
        if !accepted {
            let page = unsafe {
                (self.page_physical as *const logos_abi::service::NetworkServerPage).read_volatile()
            };
            if page.service_generation != self.service_generation
                || page.endpoint_generation != self.endpoint_generation
            {
                crate::debug::write_line(b"LogOS: network server identity mismatch");
            } else {
                crate::debug::write_line(b"LogOS: network server state not ready");
            }
        }
        let notified = unsafe {
            logos_abi::service::ControlPage::notify_at(
                self.context_physical,
                logos_abi::service::NETWORK_REQUEST,
            )
        };
        if accepted && !notified {
            crate::debug::write_line(b"LogOS: network server notify failed");
            let context = unsafe {
                (self.context_physical as *const logos_abi::service::ControlPage).read_volatile()
            };
            crate::debug::write_line(if context.abi != logos_abi::service::ABI {
                b"LogOS: network notify context abi mismatch"
            } else if context.status != logos_abi::service::ACKNOWLEDGED
                && context.operation == logos_abi::service::NETWORK_WAIT
            {
                b"LogOS: network notify context waiting"
            } else if context.status != logos_abi::service::ACKNOWLEDGED
                && context.operation == logos_abi::service::NETWORK_DEVICE_REQUEST
            {
                b"LogOS: network notify context device"
            } else if context.status != logos_abi::service::ACKNOWLEDGED
                && context.operation == logos_abi::service::NETWORK_REQUEST
            {
                b"LogOS: network notify context request"
            } else if context.status != logos_abi::service::ACKNOWLEDGED
                && context.operation == logos_abi::service::NETWORK_REPLY
            {
                b"LogOS: network notify context reply"
            } else if context.status != logos_abi::service::ACKNOWLEDGED
                && context.operation == logos_abi::service::COMPLETE
            {
                b"LogOS: network notify context complete"
            } else if context.status != logos_abi::service::ACKNOWLEDGED
                && context.operation == logos_abi::service::READY
            {
                b"LogOS: network notify context ready"
            } else if context.status != logos_abi::service::ACKNOWLEDGED
                && context.operation == logos_abi::service::NETWORK_EVENT
            {
                b"LogOS: network notify context event"
            } else if context.status != logos_abi::service::ACKNOWLEDGED
                && context.operation == logos_abi::service::NETWORK_DEVICE_REPLY
            {
                b"LogOS: network notify context device reply"
            } else if context.status != logos_abi::service::ACKNOWLEDGED && context.operation == 0 {
                b"LogOS: network notify context empty"
            } else if context.status != logos_abi::service::ACKNOWLEDGED {
                b"LogOS: network notify context unacknowledged"
            } else {
                b"LogOS: network notify context invalid"
            });
        }
        accepted && notified
    }

    pub fn idle(self) -> bool {
        if self.page_physical == 0 {
            return false;
        }
        let page = unsafe {
            (self.page_physical as *const logos_abi::service::NetworkServerPage).read_volatile()
        };
        page.service_generation == self.service_generation
            && page.endpoint_generation == self.endpoint_generation
            && logos_abi::service::NetworkPageState::from_wire(page.state)
                == Some(logos_abi::service::NetworkPageState::Ready)
    }

    pub fn response(self, expected_id: u32) -> Option<logos_abi::NetworkReply> {
        unsafe {
            logos_core::native_service::NetworkServerPage::finish_at(
                self.page_physical,
                self.service_generation,
                self.endpoint_generation,
                expected_id,
            )
        }
    }

    pub fn reset(self) -> bool {
        unsafe {
            logos_core::native_service::NetworkServerPage::reset_at(
                self.page_physical,
                self.service_generation,
                self.endpoint_generation,
            )
        }
    }
}

#[derive(Clone, Copy)]
pub struct RemoteEndpoint {
    page_physical: u64,
    service_generation: u32,
    endpoint_generation: u32,
}

impl RemoteEndpoint {
    pub const fn generation(self) -> u32 {
        self.service_generation
    }

    pub fn request(self) -> Option<logos_core::native_service::RemotePageRequest> {
        unsafe {
            logos_core::native_service::RemotePage::take_at(
                self.page_physical,
                self.service_generation,
                self.endpoint_generation,
            )
        }
    }

    pub fn reply(self, reply: logos_core::native_service::RemotePageReply) -> bool {
        unsafe {
            logos_core::native_service::RemotePage::reply_at(
                self.page_physical,
                self.service_generation,
                self.endpoint_generation,
                reply,
            )
        }
    }
}

#[derive(Clone, Copy)]
pub struct SessionEndpoint {
    context_physical: u64,
    server: SessionServerEndpoint,
    effect: EffectEndpoint,
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

    #[allow(dead_code)]
    pub const fn page(self) -> Option<u64> {
        self.page_physical
    }

    pub const fn generation(self) -> u32 {
        self.generation
    }
}

impl SyscallEndpoint {
    pub fn message(self) -> Option<logos_abi::service::SessionClientRequest> {
        unsafe {
            logos_abi::service::SessionClientPage::current_request_at(
                self.client.page_physical,
                self.client.service_generation,
                self.client.endpoint_generation,
            )
        }
    }

    pub fn request(self) -> Option<logos_abi::SessionRequest> {
        self.message().map(|message| message.request)
    }

    pub fn reply(self, bytes: &[u8]) -> bool {
        let Some(message) = self.message() else {
            return false;
        };
        self.reply_id(message.id, logos_abi::service::SessionStatus::Complete, bytes)
    }

    pub fn reply_id(
        self,
        id: u32,
        status: logos_abi::service::SessionStatus,
        bytes: &[u8],
    ) -> bool {
        logos_abi::SessionReply::from_bytes(bytes)
            .is_some_and(|reply| self.client.reply(id, status, reply))
    }

    #[cfg(feature = "test-hooks")]
    pub fn reply_matches(self, expected: &[u8]) -> bool {
        unsafe {
            logos_abi::service::SessionClientPage::reply_at_current(
                self.client.page_physical,
                self.client.service_generation,
                self.client.endpoint_generation,
            )
        }
        .is_some_and(|response| response.reply.text[..response.reply.length] == *expected)
    }
}

impl StoreClientEndpoint {
    pub const fn unavailable() -> Self {
        Self {
            context_physical: 0,
            page_physical: 0,
            service_generation: 0,
            endpoint_generation: 0,
        }
    }

    pub const fn available(self) -> bool {
        self.page_physical != 0
    }

    pub fn request(self) -> Option<logos_abi::StoreRequest> {
        if !self.available() {
            return None;
        }
        unsafe {
            logos_abi::service::StoreClientPage::current_request_at(
                self.page_physical,
                self.service_generation,
                self.endpoint_generation,
            )
        }
    }

    #[cfg_attr(not(feature = "test-hooks"), allow(dead_code))]
    pub fn deliver(self, request: logos_abi::StoreRequest) -> bool {
        if !self.available() {
            return false;
        }
        let accepted = unsafe {
            logos_abi::service::StoreClientPage::request_at(
                self.page_physical,
                self.service_generation,
                self.endpoint_generation,
                request,
            )
        };
        accepted
            && unsafe {
                logos_abi::service::ControlPage::notify_at(
                    self.context_physical,
                    logos_abi::service::STORE_REQUEST,
                )
            }
    }

    #[cfg_attr(not(feature = "test-hooks"), allow(dead_code))]
    pub fn response(self, expected_id: u32) -> Option<logos_abi::StoreReply> {
        if !self.available() {
            return None;
        }
        unsafe {
            logos_abi::service::StoreClientPage::finish_at(
                self.page_physical,
                self.service_generation,
                self.endpoint_generation,
                expected_id,
            )
        }
    }

    pub fn reply(self, reply: logos_abi::StoreReply) -> bool {
        if !self.available() {
            return false;
        }
        let accepted = unsafe {
            logos_abi::service::StoreClientPage::reply_at(
                self.page_physical,
                self.service_generation,
                self.endpoint_generation,
                reply,
            )
        };
        accepted
            && unsafe {
                logos_abi::service::ControlPage::notify_at(
                    self.context_physical,
                    logos_abi::service::STORE_REPLY,
                )
            }
    }

    pub fn configure_transfer(self, page: logos_abi::PageHandle) -> bool {
        if !self.available() {
            return false;
        }
        unsafe {
            logos_abi::service::StoreClientPage::configure_transfer_at(
                self.page_physical,
                self.service_generation,
                self.endpoint_generation,
                page,
            )
        }
    }

    pub fn transfer_page(self) -> Option<logos_abi::PageHandle> {
        if !self.available() {
            return None;
        }
        unsafe {
            logos_abi::service::StoreClientPage::transfer_page_at(
                self.page_physical,
                self.service_generation,
                self.endpoint_generation,
            )
        }
    }
}

impl StoreServerEndpoint {
    pub const fn unavailable() -> Self {
        Self {
            context_physical: 0,
            page_physical: 0,
            service_generation: 0,
            endpoint_generation: 0,
        }
    }

    pub const fn available(self) -> bool {
        self.page_physical != 0
    }

    #[cfg_attr(not(feature = "test-hooks"), allow(dead_code))]
    pub const fn context(self) -> u64 {
        self.context_physical
    }

    pub fn deliver(self, request: logos_abi::StoreRequest, caller: u64) -> bool {
        if !self.available() {
            return false;
        }
        let accepted = unsafe {
            logos_abi::service::StoreServerPage::deliver_at(
                self.page_physical,
                self.service_generation,
                self.endpoint_generation,
                caller,
                request,
            )
        };
        accepted
            && unsafe {
                logos_abi::service::ControlPage::notify_at(
                    self.context_physical,
                    logos_abi::service::STORE_REQUEST,
                )
            }
    }

    pub fn response(self, expected_id: u32) -> Option<logos_abi::StoreReply> {
        if !self.available() {
            return None;
        }
        unsafe {
            logos_abi::service::StoreServerPage::take_reply_at(
                self.page_physical,
                self.service_generation,
                self.endpoint_generation,
                expected_id,
            )
        }
    }

    pub fn reply(self, reply: logos_abi::StoreReply) -> bool {
        self.available()
            && unsafe {
                logos_abi::service::StoreServerPage::reply_at(
                    self.page_physical,
                    self.service_generation,
                    self.endpoint_generation,
                    reply,
                )
            }
    }

    pub fn configure_transfer(self, page: logos_abi::PageHandle) -> bool {
        if !self.available() {
            return false;
        }
        unsafe {
            logos_abi::service::StoreServerPage::configure_transfer_at(
                self.page_physical,
                self.service_generation,
                self.endpoint_generation,
                page,
            )
        }
    }

    pub fn status(self) -> Option<u32> {
        if !self.available() {
            return None;
        }
        unsafe {
            logos_abi::service::StoreServerPage::status_at(
                self.page_physical,
                self.service_generation,
                self.endpoint_generation,
            )
        }
    }

    pub fn waiting(self) -> bool {
        self.available()
            && unsafe {
                logos_abi::service::StoreServerPage::waiting_at(
                    self.page_physical,
                    self.service_generation,
                    self.endpoint_generation,
                )
            }
    }
}

impl BlockClientEndpoint {
    pub const fn unavailable() -> Self {
        Self {
            context_physical: 0,
            page_physical: 0,
            service_generation: 0,
            endpoint_generation: 0,
        }
    }

    pub const fn available(self) -> bool {
        self.page_physical != 0
    }

    pub fn configure_transfer(self, page: logos_abi::PageHandle) -> bool {
        if !self.available() {
            return false;
        }
        unsafe {
            logos_abi::service::BlockClientPage::configure_transfer_at(
                self.page_physical,
                self.service_generation,
                self.endpoint_generation,
                page,
            )
        }
    }

    pub fn request(self) -> Option<logos_abi::BlockRequest> {
        if !self.available() {
            return None;
        }
        unsafe {
            logos_abi::service::BlockClientPage::take_at(
                self.page_physical,
                self.service_generation,
                self.endpoint_generation,
            )
        }
    }

    #[cfg_attr(not(feature = "test-hooks"), allow(dead_code))]
    pub fn deliver(self, request: logos_abi::BlockRequest) -> bool {
        if !self.available() {
            return false;
        }
        let accepted = unsafe {
            logos_abi::service::BlockClientPage::request_at(
                self.page_physical,
                self.service_generation,
                self.endpoint_generation,
                request,
            )
        };
        accepted
            && unsafe {
                logos_abi::service::ControlPage::notify_at(
                    self.context_physical,
                    logos_abi::service::BLOCK_REQUEST,
                )
            }
    }

    pub fn reply(self, reply: logos_abi::BlockReply) -> bool {
        if !self.available() {
            return false;
        }
        let accepted = unsafe {
            logos_abi::service::BlockClientPage::reply_at(
                self.page_physical,
                self.service_generation,
                self.endpoint_generation,
                reply,
            )
        };
        accepted
            && unsafe {
                logos_abi::service::ControlPage::notify_at(
                    self.context_physical,
                    logos_abi::service::BLOCK_REPLY,
                )
            }
    }

    #[cfg_attr(not(feature = "test-hooks"), allow(dead_code))]
    pub fn response(self, expected_id: u32) -> Option<logos_abi::BlockReply> {
        if !self.available() {
            return None;
        }
        unsafe {
            logos_abi::service::BlockClientPage::finish_at(
                self.page_physical,
                self.service_generation,
                self.endpoint_generation,
                expected_id,
            )
        }
    }
}

#[allow(dead_code)]
impl NetworkEndpoint {
    pub const fn context(self) -> u64 {
        self.context_physical
    }
}

#[derive(Clone, Copy)]
pub struct NetworkDeviceEndpoint {
    page_physical: u64,
    service_generation: u32,
    endpoint_generation: u32,
    device_generation: u32,
}

impl NetworkDeviceEndpoint {
    pub const fn unavailable() -> Self {
        Self {
            page_physical: 0,
            service_generation: 0,
            endpoint_generation: 0,
            device_generation: 0,
        }
    }

    pub const fn available(self) -> bool {
        self.page_physical != 0
    }

    pub const fn with_device_generation(self, device_generation: u32) -> Self {
        Self { device_generation, ..self }
    }

    pub fn configure(self, rx: logos_abi::PageHandle, tx: logos_abi::PageHandle) -> bool {
        self.available()
            && unsafe {
                logos_abi::service::NetworkDevicePage::configure_at(
                    self.page_physical,
                    self.service_generation,
                    self.endpoint_generation,
                    self.device_generation,
                    rx,
                    tx,
                )
            }
    }

    pub fn request(self) -> Option<logos_abi::service::NetworkDeviceMessage> {
        self.available().then(|| unsafe {
            logos_abi::service::NetworkDevicePage::take_request_at(
                self.page_physical,
                self.service_generation,
                self.endpoint_generation,
                self.device_generation,
            )
        })?
    }

    #[cfg_attr(not(feature = "test-hooks"), allow(dead_code))]
    pub fn issue(self, request: logos_abi::NetworkDeviceRequest) -> bool {
        if !self.available() {
            return false;
        }
        let page = self.page_physical as *const logos_abi::service::NetworkDevicePage;
        let page = unsafe { page.read_volatile() };
        if page.service_generation != self.service_generation
            || page.endpoint_generation != self.endpoint_generation
            || page.device_generation != self.device_generation
        {
            return false;
        }
        if page.state != 1 {
            return false;
        }
        unsafe {
            logos_abi::service::NetworkDevicePage::request_at(
                self.page_physical,
                self.service_generation,
                self.endpoint_generation,
                self.device_generation,
                request,
            )
        }
    }

    #[cfg_attr(not(feature = "test-hooks"), allow(dead_code))]
    pub fn response(self, expected_id: u32) -> Option<logos_abi::NetworkDeviceReply> {
        self.available().then(|| unsafe {
            logos_abi::service::NetworkDevicePage::take_reply_at(
                self.page_physical,
                self.service_generation,
                self.endpoint_generation,
                self.device_generation,
                expected_id,
            )
        })?
    }

    pub fn pending(self) -> bool {
        self.available()
            && unsafe {
                logos_abi::service::NetworkDevicePage::pending_at(
                    self.page_physical,
                    self.service_generation,
                    self.endpoint_generation,
                    self.device_generation,
                )
            }
    }

    pub fn reply(self, reply: logos_abi::NetworkDeviceReply) -> bool {
        self.available()
            && unsafe {
                logos_abi::service::NetworkDevicePage::complete_at(
                    self.page_physical,
                    self.service_generation,
                    self.endpoint_generation,
                    self.device_generation,
                    reply,
                )
            }
    }

    pub fn reset_with_reply(
        self,
        device_generation: u32,
        rx: logos_abi::PageHandle,
        tx: logos_abi::PageHandle,
        reply: logos_abi::NetworkDeviceReply,
    ) -> bool {
        self.available()
            && unsafe {
                logos_abi::service::NetworkDevicePage::reset_with_reply_at(
                    self.page_physical,
                    self.service_generation,
                    self.endpoint_generation,
                    device_generation,
                    rx,
                    tx,
                    reply,
                )
            }
    }

    pub fn reset(
        self,
        device_generation: u32,
        rx: logos_abi::PageHandle,
        tx: logos_abi::PageHandle,
    ) -> bool {
        self.available()
            && unsafe {
                logos_abi::service::NetworkDevicePage::reset_at(
                    self.page_physical,
                    self.service_generation,
                    self.endpoint_generation,
                    device_generation,
                    rx,
                    tx,
                )
            }
    }
}

#[derive(Clone, Copy)]
pub struct NetworkEventEndpoint {
    page_physical: u64,
    service_generation: u32,
    endpoint_generation: u32,
    device_generation: u32,
}

impl NetworkEventEndpoint {
    pub const fn unavailable() -> Self {
        Self {
            page_physical: 0,
            service_generation: 0,
            endpoint_generation: 0,
            device_generation: 0,
        }
    }

    pub const fn available(self) -> bool {
        self.page_physical != 0
    }

    pub const fn with_device_generation(self, device_generation: u32) -> Self {
        Self { device_generation, ..self }
    }

    pub fn configure(self, rx: logos_abi::PageHandle) -> bool {
        self.available()
            && unsafe {
                logos_abi::service::NetworkEventPage::configure_at(
                    self.page_physical,
                    self.service_generation,
                    self.endpoint_generation,
                    self.device_generation,
                    rx,
                )
            }
    }

    pub fn waiting(self) -> bool {
        self.available()
            && unsafe {
                logos_abi::service::NetworkEventPage::waiting_at(
                    self.page_physical,
                    self.service_generation,
                    self.endpoint_generation,
                    self.device_generation,
                )
            }
    }

    pub fn deadline(self) -> Option<u64> {
        self.available().then(|| unsafe {
            logos_abi::service::NetworkEventPage::deadline_at(
                self.page_physical,
                self.service_generation,
                self.endpoint_generation,
                self.device_generation,
            )
        })?
    }

    pub fn deliver(self, event: logos_abi::NetworkEvent) -> bool {
        self.available()
            && unsafe {
                logos_abi::service::NetworkEventPage::deliver_at(
                    self.page_physical,
                    self.service_generation,
                    self.endpoint_generation,
                    self.device_generation,
                    event,
                )
            }
    }

    pub fn reset(self, device_generation: u32, rx: logos_abi::PageHandle) -> bool {
        self.available()
            && unsafe {
                logos_abi::service::NetworkEventPage::reset_at(
                    self.page_physical,
                    self.service_generation,
                    self.endpoint_generation,
                    device_generation,
                    rx,
                )
            }
    }
}

#[derive(Clone, Copy)]
pub struct NetworkStreamEndpoint {
    page_physical: u64,
    service_generation: u32,
    endpoint_generation: u32,
}

impl NetworkStreamEndpoint {
    pub const fn unavailable() -> Self {
        Self { page_physical: 0, service_generation: 0, endpoint_generation: 0 }
    }

    pub const fn available(self) -> bool {
        self.page_physical != 0
    }

    pub fn take_next(self) -> Option<logos_abi::NetworkStreamRecord> {
        self.available().then(|| unsafe {
            logos_abi::service::StreamPage::take_next_at(
                self.page_physical,
                self.service_generation,
                self.endpoint_generation,
            )
        })?
    }
}

impl SessionEndpoint {
    #[cfg_attr(not(feature = "test-hooks"), allow(dead_code))]
    pub const fn context(self) -> u64 {
        self.context_physical
    }

    #[cfg_attr(not(feature = "test-hooks"), allow(dead_code))]
    pub fn deliver(self, request: logos_abi::SessionRequest) -> bool {
        self.server.deliver(1, 0, request)
    }

    pub fn deliver_id(self, id: u32, caller: u64, request: logos_abi::SessionRequest) -> bool {
        self.server.deliver(id, caller, request)
    }

    #[cfg_attr(not(feature = "test-hooks"), allow(dead_code))]
    pub fn reply(self) -> Option<logos_abi::SessionReply> {
        self.server.reply(1).map(|response| response.reply)
    }

    pub fn reply_id(self, id: u32) -> Option<logos_abi::service::SessionServerReply> {
        self.server.reply(id)
    }

    #[cfg_attr(not(feature = "test-hooks"), allow(dead_code))]
    pub fn effect(self) -> Option<logos_abi::EffectRequest> {
        self.effect.request().map(|message| message.request)
    }

    pub fn effect_id(self, id: u32) -> Option<logos_abi::EffectRequest> {
        self.effect.request().filter(|message| message.id == id).map(|message| message.request)
    }

    pub fn reply_effect(self, reply: logos_abi::EffectResult) -> bool {
        self.reply_effect_with_text(logos_abi::EffectReply::new(reply, &[]))
    }

    #[allow(dead_code)]
    pub fn reply_effect_with_text(self, reply: logos_abi::EffectReply) -> bool {
        self.effect.waiting_id().is_some_and(|id| self.effect.reply(id, reply))
    }
}

impl<'a> Task<'a> {
    fn endpoint_mapping(&self, kind: EndpointKind) -> Option<u64> {
        self.endpoint_mappings
            .get(kind)
            .filter(|mapping| mapping.generation == self.generation && mapping.virtual_address != 0)
            .map(|mapping| mapping.physical)
    }

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
        let Some(mapping) = space.map_context(memory, endpoint_pages) else {
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
            endpoint_mappings: EndpointMappings::from_context(mapping.entries),
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
        self.endpoint_mapping(EndpointKind::Input).map(|page_physical| InputEndpoint {
            page_physical: Some(page_physical),
            generation: self.generation,
        })
    }

    pub fn syscall_endpoint(&self) -> SyscallEndpoint {
        SyscallEndpoint { client: self.session_client_endpoint().unwrap() }
    }

    pub fn display_endpoint(&self) -> Option<DisplayEndpoint> {
        self.endpoint_mapping(EndpointKind::Display).map(|page_physical| DisplayEndpoint {
            context_physical: self.context_physical,
            page_physical: Some(page_physical),
            generation: self.generation,
        })
    }

    pub fn session_client_endpoint(&self) -> Option<SessionClientEndpoint> {
        self.endpoint_mapping(EndpointKind::SessionClient).map(|page_physical| {
            SessionClientEndpoint {
                page_physical,
                service_generation: self.generation,
                endpoint_generation: self.generation,
            }
        })
    }

    pub fn session_server_endpoint(&self) -> Option<SessionServerEndpoint> {
        self.endpoint_mapping(EndpointKind::SessionServer).map(|page_physical| {
            SessionServerEndpoint {
                page_physical,
                service_generation: self.generation,
                endpoint_generation: self.generation,
            }
        })
    }

    pub fn effect_endpoint(&self) -> Option<EffectEndpoint> {
        self.endpoint_mapping(EndpointKind::Effect).map(|page_physical| EffectEndpoint {
            page_physical,
            service_generation: self.generation,
            endpoint_generation: self.generation,
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
        if let Some(page) = self.endpoint_mapping(EndpointKind::Input) {
            if !unsafe { logos_core::native_service::InputPage::reset_at(page, generation) } {
                return false;
            }
        }
        if let Some(page) = self.endpoint_mapping(EndpointKind::Display) {
            if !unsafe { logos_core::native_service::DisplayPage::reset_at(page, generation) } {
                return false;
            }
        }
        if let Some(page) = self.endpoint_mapping(EndpointKind::SessionClient) {
            if !unsafe {
                logos_core::native_service::SessionClientPage::reset_at(
                    page, generation, generation,
                )
            } {
                return false;
            }
        }
        if let Some(page) = self.endpoint_mapping(EndpointKind::SessionServer) {
            if !unsafe {
                logos_core::native_service::SessionServerPage::reset_at(
                    page, generation, generation,
                )
            } {
                return false;
            }
        }
        if let Some(page) = self.endpoint_mapping(EndpointKind::Effect) {
            if !unsafe {
                logos_core::native_service::EffectPage::reset_at(page, generation, generation)
            } {
                return false;
            }
        }
        if let Some(page) = self.endpoint_mapping(EndpointKind::StoreClient) {
            if !unsafe {
                logos_core::native_service::StoreClientPage::reset_at(page, generation, generation)
            } {
                return false;
            }
        }
        if let Some(page) = self.endpoint_mapping(EndpointKind::StoreServer) {
            if !unsafe {
                logos_core::native_service::StoreServerPage::reset_at(page, generation, generation)
            } {
                return false;
            }
        }
        if let Some(page) = self.endpoint_mapping(EndpointKind::BlockClient) {
            if !unsafe {
                logos_core::native_service::BlockClientPage::reset_at(page, generation, generation)
            } {
                return false;
            }
        }
        if let Some(page) = self.endpoint_mapping(EndpointKind::Remote) {
            if !unsafe {
                logos_core::native_service::RemotePage::reset_at(page, generation, generation)
            } {
                return false;
            }
        }
        if let Some(page) = self.endpoint_mapping(EndpointKind::NetworkDevice) {
            if !unsafe {
                logos_core::native_service::NetworkDevicePage::reset_generation_at(
                    page, generation, generation,
                )
            } {
                return false;
            }
        }
        if let Some(page) = self.endpoint_mapping(EndpointKind::NetworkClient) {
            if !unsafe {
                logos_core::native_service::NetworkClientPage::reset_at(
                    page, generation, generation,
                )
            } {
                return false;
            }
        }
        if let Some(page) = self.endpoint_mapping(EndpointKind::NetworkServer) {
            if !unsafe {
                logos_core::native_service::NetworkServerPage::reset_at(
                    page, generation, generation,
                )
            } {
                return false;
            }
        }
        if let Some(page) = self.endpoint_mapping(EndpointKind::NetworkEvent) {
            if !unsafe {
                logos_core::native_service::NetworkEventPage::reset_generation_at(
                    page, generation, generation,
                )
            } {
                return false;
            }
        }
        if let Some(page) = self.endpoint_mapping(EndpointKind::NetworkStream) {
            if !unsafe {
                logos_core::native_service::StreamPage::reset_at(page, generation, generation)
            } {
                return false;
            }
        }
        self.endpoint_mappings.set_generation(generation);
        self.generation = generation;
        true
    }

    pub fn session_endpoint(&self) -> SessionEndpoint {
        SessionEndpoint {
            context_physical: self.context_physical,
            server: self.session_server_endpoint().unwrap(),
            effect: self.effect_endpoint().unwrap(),
        }
    }

    pub fn store_client_endpoint(&self) -> Option<StoreClientEndpoint> {
        self.endpoint_mapping(EndpointKind::StoreClient).map(|page_physical| StoreClientEndpoint {
            context_physical: self.context_physical,
            page_physical,
            service_generation: self.generation,
            endpoint_generation: self.generation,
        })
    }

    pub fn store_server_endpoint(&self) -> Option<StoreServerEndpoint> {
        self.endpoint_mapping(EndpointKind::StoreServer).map(|page_physical| StoreServerEndpoint {
            context_physical: self.context_physical,
            page_physical,
            service_generation: self.generation,
            endpoint_generation: self.generation,
        })
    }

    pub fn block_client_endpoint(&self) -> Option<BlockClientEndpoint> {
        self.endpoint_mapping(EndpointKind::BlockClient).map(|page_physical| BlockClientEndpoint {
            context_physical: self.context_physical,
            page_physical,
            service_generation: self.generation,
            endpoint_generation: self.generation,
        })
    }

    #[allow(dead_code)]
    pub const fn network_endpoint(&self) -> NetworkEndpoint {
        NetworkEndpoint { context_physical: self.context_physical }
    }

    pub fn network_client_endpoint(&self) -> Option<NetworkClientEndpoint> {
        self.endpoint_mapping(EndpointKind::NetworkClient).map(|page_physical| {
            NetworkClientEndpoint {
                page_physical,
                stream_page_physical: self
                    .endpoint_mapping(EndpointKind::NetworkStream)
                    .unwrap_or(0),
                service_generation: self.generation,
                endpoint_generation: self.generation,
            }
        })
    }

    pub fn network_server_endpoint(&self) -> Option<NetworkServerEndpoint> {
        self.endpoint_mapping(EndpointKind::NetworkServer).map(|page_physical| {
            NetworkServerEndpoint {
                context_physical: self.context_physical,
                page_physical,
                service_generation: self.generation,
                endpoint_generation: self.generation,
            }
        })
    }

    pub fn network_device_endpoint(&self, device_generation: u32) -> Option<NetworkDeviceEndpoint> {
        self.endpoint_mapping(EndpointKind::NetworkDevice).map(|page_physical| {
            NetworkDeviceEndpoint {
                page_physical,
                service_generation: self.generation,
                endpoint_generation: self.generation,
                device_generation,
            }
        })
    }

    pub fn network_event_endpoint(&self, device_generation: u32) -> Option<NetworkEventEndpoint> {
        self.endpoint_mapping(EndpointKind::NetworkEvent).map(|page_physical| {
            NetworkEventEndpoint {
                page_physical,
                service_generation: self.generation,
                endpoint_generation: self.generation,
                device_generation,
            }
        })
    }

    pub fn network_stream_endpoint(&self) -> Option<NetworkStreamEndpoint> {
        self.endpoint_mapping(EndpointKind::NetworkStream).map(|page_physical| {
            NetworkStreamEndpoint {
                page_physical,
                service_generation: self.generation,
                endpoint_generation: self.generation,
            }
        })
    }

    pub fn remote_endpoint(&self) -> Option<RemoteEndpoint> {
        self.endpoint_mapping(EndpointKind::Remote).map(|page_physical| RemoteEndpoint {
            page_physical,
            service_generation: self.generation,
            endpoint_generation: self.generation,
        })
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
            Some(EntryState::Panic) => {
                crate::debug::write_line(b"LogOS: native task panic");
                false
            }
            Some(EntryState::Fault(fault)) => {
                crate::debug::write_line(b"LogOS: native task fault");
                crate::debug::write_hex_u64_line(b"LogOS: fault vector=", u64::from(fault.vector));
                crate::debug::write_hex_u64_line(b"LogOS: fault error=", fault.error);
                crate::debug::write_hex_u64_line(b"LogOS: fault rip=", fault.rip);
                crate::debug::write_hex_u64_line(b"LogOS: fault cr2=", fault.cr2);
                false
            }
            None => {
                crate::debug::write_line(b"LogOS: native task returned no state");
                false
            }
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

    pub fn store_client_endpoint(&self, handle: Handle) -> Option<StoreClientEndpoint> {
        self.entry(handle)?.task.store_client_endpoint()
    }

    pub fn store_server_endpoint(&self, handle: Handle) -> Option<StoreServerEndpoint> {
        self.entry(handle)?.task.store_server_endpoint()
    }

    pub fn remote_endpoint(&self, handle: Handle) -> Option<RemoteEndpoint> {
        self.entry(handle)?.task.remote_endpoint()
    }

    pub fn block_client_endpoint(&self, handle: Handle) -> Option<BlockClientEndpoint> {
        self.entry(handle)?.task.block_client_endpoint()
    }

    pub fn network_endpoint(&self, handle: Handle) -> Option<NetworkEndpoint> {
        Some(self.entry(handle)?.task.network_endpoint())
    }

    pub fn network_client_endpoint(&self, handle: Handle) -> Option<NetworkClientEndpoint> {
        self.entry(handle)?.task.network_client_endpoint()
    }

    pub fn network_server_endpoint(&self, handle: Handle) -> Option<NetworkServerEndpoint> {
        self.entry(handle)?.task.network_server_endpoint()
    }

    pub fn network_device_endpoint(
        &self,
        handle: Handle,
        device_generation: u32,
    ) -> Option<NetworkDeviceEndpoint> {
        self.entry(handle)?.task.network_device_endpoint(device_generation)
    }

    pub fn network_event_endpoint(
        &self,
        handle: Handle,
        device_generation: u32,
    ) -> Option<NetworkEventEndpoint> {
        self.entry(handle)?.task.network_event_endpoint(device_generation)
    }

    pub fn network_stream_endpoint(&self, handle: Handle) -> Option<NetworkStreamEndpoint> {
        self.entry(handle)?.task.network_stream_endpoint()
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

    /// Wake a task for a bounded notification that may arrive while it is
    /// already runnable. A failed task remains non-wakeable.
    pub fn wake_or_ready(&mut self, handle: Handle) -> bool {
        let Some(entry) = self.entry_mut(handle) else { return false };
        if entry.waiting == Some(Event::FAILURE) {
            return false;
        }
        if entry.waiting.is_some() {
            entry.waiting = None;
            crate::platform::trace::record(crate::platform::trace::Event::TaskWoken);
        }
        true
    }

    /// Deliver input without interrupting a task blocked on another phase.
    pub fn notify_input(&mut self, handle: Handle) -> Option<bool> {
        let entry = self.entry_mut(handle)?;
        match entry.waiting {
            Some(Event::FAILURE) => Some(false),
            Some(Event::INPUT) => {
                entry.waiting = None;
                crate::platform::trace::record(crate::platform::trace::Event::TaskWoken);
                Some(self.run(handle))
            }
            Some(_) => None,
            None => None,
        }
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
