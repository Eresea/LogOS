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
    pub session_client: bool,
    pub session_server: bool,
    pub effect: bool,
    pub store_client: bool,
    pub store_server: bool,
    pub block_client: bool,
    pub network_device: bool,
    pub network_event: bool,
}

impl EndpointPages {
    pub const NONE: Self = Self {
        input: false,
        display: false,
        session_client: false,
        session_server: false,
        effect: false,
        store_client: false,
        store_server: false,
        block_client: false,
        network_device: false,
        network_event: false,
    };
    pub const TERMINAL: Self =
        Self { input: true, display: true, session_client: true, store_client: true, ..Self::NONE };
    pub const SESSIONS: Self = Self { session_server: true, effect: true, ..Self::NONE };
    pub const STORAGE: Self = Self { store_server: true, block_client: true, ..Self::NONE };
    pub const NETWORK: Self = Self { network_device: true, network_event: true, ..Self::NONE };
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
    session_client_page_physical: Option<u64>,
    session_server_page_physical: Option<u64>,
    effect_page_physical: Option<u64>,
    store_client_page_physical: Option<u64>,
    store_server_page_physical: Option<u64>,
    block_client_page_physical: Option<u64>,
    network_device_page_physical: Option<u64>,
    network_event_page_physical: Option<u64>,
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

    pub fn dma_pages(self) -> Option<logos_core::native_service::NetworkDmaResources> {
        let raw = unsafe {
            (self.context_physical as *const logos_core::native_service::ControlPage)
                .read_volatile()
        };
        let page = raw.network_device_page;
        if page == 0 || raw.generation == 0 {
            return None;
        }
        let device = unsafe {
            (page as *const logos_core::native_service::NetworkDevicePage).read_volatile()
        };
        let (rx_handle, tx_handle) = unsafe {
            logos_core::native_service::NetworkDevicePage::dma_at(
                page,
                raw.generation,
                raw.generation,
                device.device_generation,
            )?
        };
        Some(logos_core::native_service::NetworkDmaResources {
            rx_handle,
            rx_address: self.context_physical.checked_sub(19 * logos_abi::PAGE_SIZE as u64)?,
            tx_handle,
            tx_address: self.context_physical.checked_sub(20 * logos_abi::PAGE_SIZE as u64)?,
        })
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

    pub fn take_response(self, expected_id: u32) -> Option<logos_abi::NetworkReply> {
        unsafe {
            logos_core::native_service::ControlPage::take_network_reply_at(
                self.context_physical,
                expected_id,
            )
        }
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

impl SessionEndpoint {
    #[cfg_attr(not(feature = "test-hooks"), allow(dead_code))]
    pub const fn context(self) -> u64 {
        self.context_physical
    }

    pub fn deliver(self, request: logos_abi::SessionRequest) -> bool {
        self.server.deliver(1, 0, request)
    }

    pub fn deliver_id(self, id: u32, caller: u64, request: logos_abi::SessionRequest) -> bool {
        self.server.deliver(id, caller, request)
    }

    pub fn reply(self) -> Option<logos_abi::SessionReply> {
        self.server.reply(1).map(|response| response.reply)
    }

    pub fn reply_id(self, id: u32) -> Option<logos_abi::service::SessionServerReply> {
        self.server.reply(id)
    }

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
        let Some(mapping) = space.map_context(
            memory,
            endpoint_pages.input,
            endpoint_pages.display,
            endpoint_pages.session_client,
            endpoint_pages.session_server,
            endpoint_pages.effect,
            endpoint_pages.store_client,
            endpoint_pages.store_server,
            endpoint_pages.block_client,
            endpoint_pages.network_device,
            endpoint_pages.network_event,
        ) else {
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
            session_client_page_physical: mapping.session_client.map(|(physical, _)| physical),
            session_server_page_physical: mapping.session_server.map(|(physical, _)| physical),
            effect_page_physical: mapping.effect.map(|(physical, _)| physical),
            store_client_page_physical: mapping.store_client.map(|(physical, _)| physical),
            store_server_page_physical: mapping.store_server.map(|(physical, _)| physical),
            block_client_page_physical: mapping.block_client.map(|(physical, _)| physical),
            network_device_page_physical: mapping.network_device.map(|(physical, _)| physical),
            network_event_page_physical: mapping.network_event.map(|(physical, _)| physical),
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

    pub fn syscall_endpoint(&self) -> SyscallEndpoint {
        SyscallEndpoint { client: self.session_client_endpoint().unwrap() }
    }

    pub fn display_endpoint(&self) -> Option<DisplayEndpoint> {
        self.display_page_physical.map(|page_physical| DisplayEndpoint {
            context_physical: self.context_physical,
            page_physical: Some(page_physical),
            generation: self.generation,
        })
    }

    pub fn session_client_endpoint(&self) -> Option<SessionClientEndpoint> {
        self.session_client_page_physical.map(|page_physical| SessionClientEndpoint {
            page_physical,
            service_generation: self.generation,
            endpoint_generation: self.generation,
        })
    }

    pub fn session_server_endpoint(&self) -> Option<SessionServerEndpoint> {
        self.session_server_page_physical.map(|page_physical| SessionServerEndpoint {
            page_physical,
            service_generation: self.generation,
            endpoint_generation: self.generation,
        })
    }

    pub fn effect_endpoint(&self) -> Option<EffectEndpoint> {
        self.effect_page_physical.map(|page_physical| EffectEndpoint {
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
        if let Some(page) = self.session_client_page_physical {
            if !unsafe {
                logos_core::native_service::SessionClientPage::reset_at(
                    page, generation, generation,
                )
            } {
                return false;
            }
        }
        if let Some(page) = self.session_server_page_physical {
            if !unsafe {
                logos_core::native_service::SessionServerPage::reset_at(
                    page, generation, generation,
                )
            } {
                return false;
            }
        }
        if let Some(page) = self.effect_page_physical {
            if !unsafe {
                logos_core::native_service::EffectPage::reset_at(page, generation, generation)
            } {
                return false;
            }
        }
        if let Some(page) = self.store_client_page_physical {
            if !unsafe {
                logos_core::native_service::StoreClientPage::reset_at(page, generation, generation)
            } {
                return false;
            }
        }
        if let Some(page) = self.store_server_page_physical {
            if !unsafe {
                logos_core::native_service::StoreServerPage::reset_at(page, generation, generation)
            } {
                return false;
            }
        }
        if let Some(page) = self.block_client_page_physical {
            if !unsafe {
                logos_core::native_service::BlockClientPage::reset_at(page, generation, generation)
            } {
                return false;
            }
        }
        if let Some(page) = self.network_device_page_physical {
            if !unsafe {
                logos_core::native_service::NetworkDevicePage::reset_generation_at(
                    page, generation, generation,
                )
            } {
                return false;
            }
        }
        if let Some(page) = self.network_event_page_physical {
            if !unsafe {
                logos_core::native_service::NetworkEventPage::reset_generation_at(
                    page, generation, generation,
                )
            } {
                return false;
            }
        }
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
        self.store_client_page_physical.map(|page_physical| StoreClientEndpoint {
            context_physical: self.context_physical,
            page_physical,
            service_generation: self.generation,
            endpoint_generation: self.generation,
        })
    }

    pub fn store_server_endpoint(&self) -> Option<StoreServerEndpoint> {
        self.store_server_page_physical.map(|page_physical| StoreServerEndpoint {
            context_physical: self.context_physical,
            page_physical,
            service_generation: self.generation,
            endpoint_generation: self.generation,
        })
    }

    pub fn block_client_endpoint(&self) -> Option<BlockClientEndpoint> {
        self.block_client_page_physical.map(|page_physical| BlockClientEndpoint {
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

    pub fn network_device_endpoint(&self, device_generation: u32) -> Option<NetworkDeviceEndpoint> {
        self.network_device_page_physical.map(|page_physical| NetworkDeviceEndpoint {
            page_physical,
            service_generation: self.generation,
            endpoint_generation: self.generation,
            device_generation,
        })
    }

    pub fn network_event_endpoint(&self, device_generation: u32) -> Option<NetworkEventEndpoint> {
        self.network_event_page_physical.map(|page_physical| NetworkEventEndpoint {
            page_physical,
            service_generation: self.generation,
            endpoint_generation: self.generation,
            device_generation,
        })
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

    pub fn store_client_endpoint(&self, handle: Handle) -> Option<StoreClientEndpoint> {
        self.entry(handle)?.task.store_client_endpoint()
    }

    pub fn store_server_endpoint(&self, handle: Handle) -> Option<StoreServerEndpoint> {
        self.entry(handle)?.task.store_server_endpoint()
    }

    pub fn remote_endpoint(&self, handle: Handle) -> Option<RemoteEndpoint> {
        Some(self.entry(handle)?.task.remote_endpoint())
    }

    pub fn block_client_endpoint(&self, handle: Handle) -> Option<BlockClientEndpoint> {
        self.entry(handle)?.task.block_client_endpoint()
    }

    pub fn network_endpoint(&self, handle: Handle) -> Option<NetworkEndpoint> {
        Some(self.entry(handle)?.task.network_endpoint())
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
