pub const SERVICE: crate::platform::services::Service = crate::platform::services::Service::Network;

use crate::drivers::network;
use crate::sched::native_task::{
    self, Handle, NetworkClientEndpoint, NetworkDeviceEndpoint, NetworkEventEndpoint,
    NetworkServerEndpoint,
};
use logos_abi::{NetworkOperation, NetworkRequest};
use logos_core::capabilities::CapabilityKind;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NetworkClientSlot {
    Terminal,
    Gateway,
}

#[derive(Clone, Copy)]
enum CompletionTarget {
    Task(Handle),
    #[cfg(feature = "test-hooks")]
    Probe,
}

#[derive(Clone, Copy)]
struct PendingClient {
    slot: Option<NetworkClientSlot>,
    request: logos_abi::NetworkRequest,
    owner: u64,
    endpoint: NetworkClientEndpoint,
    server: NetworkServerEndpoint,
    target: CompletionTarget,
}

#[derive(Clone, Copy)]
pub struct PendingDevice {
    pub request: logos_abi::NetworkDeviceRequest,
}

#[derive(Clone, Copy)]
pub struct Resources {
    pub owner: u64,
    pub rx: logos_abi::PageHandle,
    pub rx_virtual: u64,
    pub tx: logos_abi::PageHandle,
    pub tx_virtual: u64,
}

#[derive(Clone, Copy)]
struct NetworkReadiness {
    info: Option<logos_abi::NetworkInfo>,
    probe_pending: Option<u32>,
    probe_due: u64,
    next_probe_id: u32,
}

impl NetworkReadiness {
    const fn new() -> Self {
        Self { info: None, probe_pending: None, probe_due: 0, next_probe_id: 0x8000_0001 }
    }

    fn reset(&mut self) {
        *self = Self::new();
    }
}

pub struct NetworkRuntime {
    task: Option<Handle>,
    device_endpoint: NetworkDeviceEndpoint,
    event_endpoint: NetworkEventEndpoint,
    server_endpoint: Option<NetworkServerEndpoint>,
    active_client: Option<PendingClient>,
    device: Option<network::Device>,
    resources: Option<Resources>,
    pending: Option<PendingDevice>,
    device_generation: u32,
    failures: u32,
    degraded: bool,
    readiness: NetworkReadiness,
}

impl NetworkRuntime {
    pub const fn task(&self) -> Option<Handle> {
        self.task
    }

    pub const fn device_endpoint(&self) -> NetworkDeviceEndpoint {
        self.device_endpoint
    }

    #[cfg(feature = "test-hooks")]
    pub const fn server_endpoint(&self) -> Option<NetworkServerEndpoint> {
        self.server_endpoint
    }

    pub const fn device_generation(&self) -> u32 {
        self.device_generation
    }

    pub const fn has_device(&self) -> bool {
        self.device.is_some()
    }

    pub const fn configured(&self) -> bool {
        match self.readiness.info {
            Some(info) => info.configuration != 0,
            None => false,
        }
    }

    #[cfg_attr(not(feature = "test-hooks"), allow(dead_code))]
    pub const fn info(&self) -> Option<logos_abi::NetworkInfo> {
        self.readiness.info
    }

    #[cfg_attr(not(feature = "test-hooks"), allow(dead_code))]
    pub const fn has_resources(&self) -> bool {
        self.resources.is_some()
    }

    pub const fn resources(&self) -> Option<Resources> {
        self.resources
    }

    pub fn new(device: Option<network::Device>) -> Self {
        let device_generation =
            device.as_ref().map_or(0, |device| u32::from(device.info().generation));
        Self {
            task: None,
            device_endpoint: NetworkDeviceEndpoint::unavailable(),
            event_endpoint: NetworkEventEndpoint::unavailable(),
            server_endpoint: None,
            active_client: None,
            device,
            resources: None,
            pending: None,
            device_generation,
            failures: 0,
            degraded: false,
            readiness: NetworkReadiness::new(),
        }
    }

    pub fn bind(
        &mut self,
        task: Handle,
        server_endpoint: NetworkServerEndpoint,
        device_endpoint: NetworkDeviceEndpoint,
        event_endpoint: NetworkEventEndpoint,
        resources: Resources,
    ) -> bool {
        if self.active_client.is_some() {
            return false;
        }
        let device_generation = self.device_generation;
        let device_endpoint = device_endpoint.with_device_generation(device_generation);
        let event_endpoint = event_endpoint.with_device_generation(device_generation);
        if device_generation == 0
            || !device_endpoint.configure(resources.rx, resources.tx)
            || !event_endpoint.configure(resources.rx)
        {
            return false;
        }
        self.task = Some(task);
        self.server_endpoint = Some(server_endpoint);
        self.device_endpoint = device_endpoint;
        self.event_endpoint = event_endpoint;
        self.resources = Some(resources);
        self.degraded = false;
        self.readiness.reset();
        true
    }

    pub fn reset(&mut self, scheduler: &mut native_task::Scheduler<'_>) -> bool {
        let (Some(device), Some(resources), Some(task)) =
            (self.device.as_mut(), self.resources, self.task)
        else {
            return false;
        };
        if !device.reset() {
            self.failures = self.failures.saturating_add(1);
            self.degraded = true;
            return false;
        }
        let generation = u32::from(device.info().generation);
        let device_endpoint = self.device_endpoint.with_device_generation(generation);
        let event_endpoint = self.event_endpoint.with_device_generation(generation);
        if !device_endpoint.reset(generation, resources.rx, resources.tx)
            || !event_endpoint.reset(generation, resources.rx)
        {
            self.failures = self.failures.saturating_add(1);
            self.degraded = true;
            return false;
        }
        self.device_generation = generation;
        self.device_endpoint = device_endpoint;
        self.event_endpoint = event_endpoint;
        self.pending = None;
        self.readiness.reset();
        scheduler.wake(task) && scheduler.run(task)
    }

    fn reset_with_reply(
        &mut self,
        request: logos_abi::NetworkDeviceRequest,
        status: logos_abi::NetworkStatus,
        scheduler: &mut native_task::Scheduler<'_>,
    ) -> bool {
        let (Some(device), Some(resources), Some(task)) =
            (self.device.as_ref(), self.resources, self.task)
        else {
            return false;
        };
        let info = device.info();
        let reply = logos_abi::NetworkDeviceReply {
            id: request.id,
            status,
            generation: info.generation,
            info: network_info(info),
        };
        let old_endpoint = self.device_endpoint;
        let generation = u32::from(info.generation);
        let accepted = if generation == self.device_generation {
            old_endpoint.reply(reply)
        } else {
            old_endpoint.reset_with_reply(generation, resources.rx, resources.tx, reply)
        };
        if !accepted {
            return false;
        }
        let event_endpoint = self.event_endpoint.with_device_generation(generation);
        if !event_endpoint.reset(generation, resources.rx) {
            return false;
        }
        self.device_generation = generation;
        self.device_endpoint = old_endpoint.with_device_generation(generation);
        self.event_endpoint = event_endpoint;
        self.pending = None;
        self.readiness.reset();
        scheduler.wake(task) && scheduler.run(task)
    }

    fn poll_readiness(&mut self, scheduler: &mut native_task::Scheduler<'_>, tick: u64) -> bool {
        let Some(server) = self.server_endpoint else { return true };
        let Some(task) = self.task else { return true };
        if let Some(id) = self.readiness.probe_pending {
            if let Some(reply) = server.response(id) {
                self.readiness.info = Some(reply.info);
                self.readiness.probe_pending = None;
                self.readiness.probe_due = tick.saturating_add(64);
            }
            return true;
        }
        if self.configured() || self.active_client.is_some() || tick < self.readiness.probe_due {
            return true;
        }
        let id = self.readiness.next_probe_id;
        self.readiness.next_probe_id = id.wrapping_add(1).max(1);
        let request = logos_abi::NetworkRequest {
            id,
            operation: NetworkOperation::Status,
            endpoint: logos_abi::NetworkEndpoint(0),
            peer: logos_abi::NetworkScope(0),
            page: logos_abi::PageHandle(0),
            length: 0,
            generation: 0,
            deadline: u64::MAX / 2,
        };
        if !server.deliver(0, request) {
            self.readiness.probe_due = tick.saturating_add(64);
            let _ = server.reset();
            return true;
        }
        self.readiness.probe_pending = Some(id);
        if scheduler.wake(task) && scheduler.run(task) {
            true
        } else {
            self.readiness.probe_pending = None;
            self.readiness.probe_due = tick.saturating_add(64);
            let _ = server.reset();
            true
        }
    }

    pub fn poll(&mut self, scheduler: &mut native_task::Scheduler<'_>, tick: u64) -> bool {
        if !self.device_endpoint.pending() && !self.poll_readiness(scheduler, tick) {
            return false;
        }
        let (Some(device), Some(resources), Some(task)) =
            (self.device.as_mut(), self.resources, self.task)
        else {
            return true;
        };
        if let Some(pending) = self.pending {
            if tick >= pending.request.deadline {
                let reset = device.reset();
                let status = if reset {
                    logos_abi::NetworkStatus::TimedOut
                } else {
                    logos_abi::NetworkStatus::Reset
                };
                return self.reset_with_reply(pending.request, status, scheduler);
            }
            match device.complete_transmit() {
                Ok(Some(())) => {
                    let info = device.info();
                    let reply = logos_abi::NetworkDeviceReply {
                        id: pending.request.id,
                        status: logos_abi::NetworkStatus::Complete,
                        generation: info.generation,
                        info: network_info(info),
                    };
                    self.pending = None;
                    return self.device_endpoint.reply(reply)
                        && scheduler.wake(task)
                        && scheduler.run(task);
                }
                Ok(None) => return true,
                Err(_) => {
                    let _ = device.reset();
                    return self.reset_with_reply(
                        pending.request,
                        logos_abi::NetworkStatus::Reset,
                        scheduler,
                    );
                }
            }
        }
        let device_message = self.device_endpoint.request();
        if device_message.is_none() && self.device_endpoint.pending() {
            self.degraded = true;
            return false;
        }
        if let Some(message) = device_message {
            let request = message.request;
            let info = device.info();
            let response = match request.operation {
                logos_abi::NetworkDeviceOperation::Info => Some(logos_abi::NetworkDeviceReply {
                    id: request.id,
                    status: logos_abi::NetworkStatus::Complete,
                    generation: info.generation,
                    info: network_info(info),
                }),
                logos_abi::NetworkDeviceOperation::Reset => {
                    if request.generation == info.generation && device.reset() {
                        return self.reset_with_reply(
                            request,
                            logos_abi::NetworkStatus::Complete,
                            scheduler,
                        );
                    }
                    Some(logos_abi::NetworkDeviceReply {
                        id: request.id,
                        status: logos_abi::NetworkStatus::Reset,
                        generation: device.info().generation,
                        info: network_info(device.info()),
                    })
                }
                logos_abi::NetworkDeviceOperation::Transmit => {
                    if request.generation != info.generation || message.tx_page != resources.tx {
                        Some(logos_abi::NetworkDeviceReply {
                            id: request.id,
                            status: logos_abi::NetworkStatus::Reset,
                            generation: info.generation,
                            info: network_info(info),
                        })
                    } else {
                        let frame = unsafe {
                            core::slice::from_raw_parts(
                                resources.tx_virtual as *const u8,
                                usize::from(request.length),
                            )
                        };
                        match device.transmit(frame) {
                            Ok(()) => {
                                self.pending = Some(PendingDevice { request });
                                return true;
                            }
                            Err(error) => Some(logos_abi::NetworkDeviceReply {
                                id: request.id,
                                status: match error {
                                    network::NetworkError::Busy => logos_abi::NetworkStatus::Busy,
                                    network::NetworkError::Length => {
                                        logos_abi::NetworkStatus::Invalid
                                    }
                                    network::NetworkError::Device => logos_abi::NetworkStatus::Io,
                                },
                                generation: info.generation,
                                info: network_info(info),
                            }),
                        }
                    }
                }
            };
            if let Some(reply) = response {
                let ok = self.device_endpoint.reply(reply)
                    && scheduler.wake(task)
                    && scheduler.run(task);
                return ok;
            }
            return true;
        }
        if !self.event_endpoint.waiting() {
            return true;
        }
        if self.event_endpoint.deadline().is_some_and(|deadline| tick >= deadline) {
            let event = logos_abi::NetworkEvent {
                id: tick.try_into().unwrap_or(1).max(1),
                kind: logos_abi::NetworkEventKind::Timer,
                generation: device.info().generation,
                device_generation: self.device_generation,
                page: logos_abi::PageHandle(0),
                length: 0,
                now: tick.max(1),
                metadata: [0; 16],
            };
            let ok =
                self.event_endpoint.deliver(event) && scheduler.wake(task) && scheduler.run(task);
            return ok;
        }
        let frame = unsafe {
            core::slice::from_raw_parts_mut(
                resources.rx_virtual as *mut u8,
                logos_abi::NETWORK_MAX_FRAME,
            )
        };
        match device.receive(frame) {
            Ok(Some(length)) => {
                let event = logos_abi::NetworkEvent {
                    id: tick.try_into().unwrap_or(1).max(1),
                    kind: logos_abi::NetworkEventKind::Frame,
                    generation: device.info().generation,
                    device_generation: self.device_generation,
                    page: resources.rx,
                    length: length as u16,
                    now: tick.max(1),
                    metadata: [0; 16],
                };
                let ok = self.event_endpoint.deliver(event)
                    && scheduler.wake(task)
                    && scheduler.run(task);
                ok
            }
            Ok(None) => true,
            Err(_) => {
                crate::debug::write_line(b"LogOS: network driver reset");
                self.reset(scheduler)
            }
        }
    }

    #[cfg_attr(not(feature = "test-hooks"), allow(dead_code))]
    pub fn poll_device_proof(&mut self, tick: u64) -> bool {
        let (Some(device), Some(resources)) = (self.device.as_mut(), self.resources) else {
            return true;
        };
        if let Some(pending) = self.pending {
            if tick >= pending.request.deadline {
                let status = if device.reset() {
                    logos_abi::NetworkStatus::TimedOut
                } else {
                    logos_abi::NetworkStatus::Reset
                };
                return self.complete_device_proof(pending.request, status, resources);
            }
            return match device.complete_transmit() {
                Ok(Some(())) => {
                    self.pending = None;
                    self.device_endpoint.reply(logos_abi::NetworkDeviceReply {
                        id: pending.request.id,
                        status: logos_abi::NetworkStatus::Complete,
                        generation: device.info().generation,
                        info: network_info(device.info()),
                    })
                }
                Ok(None) => true,
                Err(_) => {
                    let _ = device.reset();
                    self.complete_device_proof(
                        pending.request,
                        logos_abi::NetworkStatus::Reset,
                        resources,
                    )
                }
            };
        }
        let Some(message) = self.device_endpoint.request() else {
            return !self.device_endpoint.pending();
        };
        let request = message.request;
        let info = device.info();
        match request.operation {
            logos_abi::NetworkDeviceOperation::Info => {
                self.device_endpoint.reply(logos_abi::NetworkDeviceReply {
                    id: request.id,
                    status: logos_abi::NetworkStatus::Complete,
                    generation: info.generation,
                    info: network_info(info),
                })
            }
            logos_abi::NetworkDeviceOperation::Reset => {
                if request.generation == info.generation && device.reset() {
                    self.complete_device_proof(
                        request,
                        logos_abi::NetworkStatus::Complete,
                        resources,
                    )
                } else {
                    self.device_endpoint.reply(logos_abi::NetworkDeviceReply {
                        id: request.id,
                        status: logos_abi::NetworkStatus::Reset,
                        generation: device.info().generation,
                        info: network_info(device.info()),
                    })
                }
            }
            logos_abi::NetworkDeviceOperation::Transmit => {
                if request.generation != info.generation || message.tx_page != resources.tx {
                    self.device_endpoint.reply(logos_abi::NetworkDeviceReply {
                        id: request.id,
                        status: logos_abi::NetworkStatus::Reset,
                        generation: info.generation,
                        info: network_info(info),
                    })
                } else {
                    let frame = [0; logos_abi::NETWORK_MIN_FRAME];
                    match device.transmit(&frame[..usize::from(request.length)]) {
                        Ok(()) => {
                            self.pending = Some(PendingDevice { request });
                            true
                        }
                        Err(error) => self.device_endpoint.reply(logos_abi::NetworkDeviceReply {
                            id: request.id,
                            status: match error {
                                network::NetworkError::Busy => logos_abi::NetworkStatus::Busy,
                                network::NetworkError::Length => logos_abi::NetworkStatus::Invalid,
                                network::NetworkError::Device => logos_abi::NetworkStatus::Io,
                            },
                            generation: info.generation,
                            info: network_info(info),
                        }),
                    }
                }
            }
        }
    }

    #[cfg_attr(not(feature = "test-hooks"), allow(dead_code))]
    fn complete_device_proof(
        &mut self,
        request: logos_abi::NetworkDeviceRequest,
        status: logos_abi::NetworkStatus,
        resources: Resources,
    ) -> bool {
        let Some(device) = self.device.as_ref() else { return false };
        let info = device.info();
        let generation = u32::from(info.generation);
        let reply = logos_abi::NetworkDeviceReply {
            id: request.id,
            status,
            generation: info.generation,
            info: network_info(info),
        };
        let accepted = if generation == self.device_generation {
            self.device_endpoint.reply(reply)
        } else {
            self.device_endpoint.reset_with_reply(generation, resources.rx, resources.tx, reply)
        };
        if !accepted || !self.event_endpoint.reset(generation, resources.rx) {
            return false;
        }
        self.device_generation = generation;
        self.device_endpoint = self.device_endpoint.with_device_generation(generation);
        self.event_endpoint = self.event_endpoint.with_device_generation(generation);
        self.pending = None;
        true
    }

    fn validate_request(
        client: NetworkClientEndpoint,
        request: NetworkRequest,
        session: &crate::platform::session::Context,
        capabilities: &logos_core::capabilities::CapabilityManager,
        shared_pages: &logos_core::shared_pages::SharedPages,
        owner: u64,
    ) -> Result<Option<(logos_abi::PageHandle, u64)>, logos_abi::NetworkStatus> {
        if !request.valid_shape() {
            return Err(logos_abi::NetworkStatus::Invalid);
        }
        if let Some((kind, scope)) = capability(request)
            && !session.allows_scoped64(capabilities, kind, scope)
        {
            return Err(logos_abi::NetworkStatus::Denied);
        }
        if !matches!(
            request.operation,
            NetworkOperation::SendTo
                | NetworkOperation::Write
                | NetworkOperation::ReceiveFrom
                | NetworkOperation::Read
        ) {
            return Ok(None);
        }
        let Some(transfer_page) = client.transfer_page() else {
            return Err(logos_abi::NetworkStatus::Invalid);
        };
        if transfer_page != request.page
            || request.length as usize > logos_abi::PAGE_SIZE - NETWORK_PAYLOAD_OFFSET as usize
        {
            return Err(logos_abi::NetworkStatus::Invalid);
        }
        let Some(address) = shared_pages.address(owner, transfer_page) else {
            return Err(logos_abi::NetworkStatus::Invalid);
        };
        Ok(Some((transfer_page, address)))
    }

    fn reply_request(
        client: NetworkClientEndpoint,
        target: CompletionTarget,
        request: NetworkRequest,
        status: logos_abi::NetworkStatus,
        scheduler: &mut native_task::Scheduler<'_>,
    ) -> bool {
        let published = client.mark_processing() && client.reply(error_reply(request, status));
        published && Self::complete_target(target, scheduler)
    }

    fn reply_unprocessed_request(
        client: NetworkClientEndpoint,
        target: CompletionTarget,
        request: NetworkRequest,
        status: logos_abi::NetworkStatus,
        scheduler: &mut native_task::Scheduler<'_>,
    ) -> bool {
        let published = client.reply_request(error_reply(request, status));
        published && Self::complete_target(target, scheduler)
    }

    fn complete_target(
        target: CompletionTarget,
        scheduler: &mut native_task::Scheduler<'_>,
    ) -> bool {
        match target {
            CompletionTarget::Task(handle) => {
                !scheduler.failed(handle) && scheduler.wake(handle) && scheduler.run(handle)
            }
            #[cfg(feature = "test-hooks")]
            CompletionTarget::Probe => true,
        }
    }

    fn finish_active(
        &mut self,
        reply: logos_abi::NetworkReply,
        scheduler: &mut native_task::Scheduler<'_>,
    ) -> bool {
        let Some(current) = self.active_client.take() else { return false };
        let published = current.endpoint.reply(reply);
        let reset = current.server.reset();
        let completed = published && reset && Self::complete_target(current.target, scheduler);
        completed
    }

    pub fn invalidate_client(
        &mut self,
        slot: NetworkClientSlot,
        scheduler: &mut native_task::Scheduler<'_>,
        status: logos_abi::NetworkStatus,
    ) -> bool {
        let Some(current) = self.active_client else { return true };
        if current.slot != Some(slot) {
            return true;
        }
        self.finish_active(error_reply(current.request, status), scheduler)
    }

    pub fn invalidate_active(
        &mut self,
        scheduler: &mut native_task::Scheduler<'_>,
        status: logos_abi::NetworkStatus,
    ) -> bool {
        let Some(current) = self.active_client else { return true };
        self.finish_active(error_reply(current.request, status), scheduler)
    }

    fn completed_reply(
        &self,
        current: PendingClient,
        reply: logos_abi::NetworkReply,
        shared_pages: &logos_core::shared_pages::SharedPages,
    ) -> logos_abi::NetworkReply {
        if !reply.valid_for(current.request) {
            return error_reply(current.request, logos_abi::NetworkStatus::Invalid);
        }
        if reply.status != logos_abi::NetworkStatus::Complete
            || !matches!(
                current.request.operation,
                NetworkOperation::ReceiveFrom | NetworkOperation::Read
            )
        {
            return reply;
        }
        let Some(transfer_page) = current.endpoint.transfer_page() else {
            return error_reply(current.request, logos_abi::NetworkStatus::Invalid);
        };
        if transfer_page != current.request.page
            || reply.length > current.request.length
            || reply.length as usize > logos_abi::PAGE_SIZE - NETWORK_PAYLOAD_OFFSET as usize
        {
            return error_reply(current.request, logos_abi::NetworkStatus::Invalid);
        }
        let Some(destination) = shared_pages.address(current.owner, transfer_page) else {
            return error_reply(current.request, logos_abi::NetworkStatus::Invalid);
        };
        let Some(resources) = self.resources else {
            return error_reply(current.request, logos_abi::NetworkStatus::Invalid);
        };
        unsafe {
            core::ptr::copy_nonoverlapping(
                (resources.tx_virtual + NETWORK_PAYLOAD_OFFSET) as *const u8,
                destination as *mut u8,
                reply.length as usize,
            );
        }
        reply
    }

    #[allow(clippy::too_many_arguments)]
    fn relay(
        &mut self,
        slot: Option<NetworkClientSlot>,
        target: CompletionTarget,
        client: NetworkClientEndpoint,
        scheduler: &mut native_task::Scheduler<'_>,
        session: &crate::platform::session::Context,
        capabilities: &logos_core::capabilities::CapabilityManager,
        shared_pages: &logos_core::shared_pages::SharedPages,
        owner: u64,
        tick: u64,
    ) -> bool {
        let Some(server) = self.server_endpoint else { return true };
        let Some(service_handle) = self.task else { return true };
        if self.readiness.probe_pending.is_some() {
            return true;
        }
        if let Some(current) = self.active_client {
            if self.server_endpoint != Some(current.server) {
                return self.finish_active(
                    error_reply(current.request, logos_abi::NetworkStatus::Reset),
                    scheduler,
                );
            }
            if current.slot != slot {
                let Some(request) = client.request() else { return true };
                let status = match Self::validate_request(
                    client,
                    request,
                    session,
                    capabilities,
                    shared_pages,
                    owner,
                ) {
                    Ok(_) => logos_abi::NetworkStatus::Busy,
                    Err(status) => status,
                };
                return Self::reply_request(client, target, request, status, scheduler);
            }
            if current.endpoint != client {
                return false;
            }
            if tick >= current.request.deadline {
                return self.finish_active(
                    error_reply(current.request, logos_abi::NetworkStatus::TimedOut),
                    scheduler,
                );
            }
            if let Some(reply) = current.server.response(current.request.id) {
                return self
                    .finish_active(self.completed_reply(current, reply, shared_pages), scheduler);
            }
            return true;
        }
        let Some(request) = client.request() else { return true };
        let transfer = match Self::validate_request(
            client,
            request,
            session,
            capabilities,
            shared_pages,
            owner,
        ) {
            Ok(transfer) => transfer,
            Err(status) => return Self::reply_request(client, target, request, status, scheduler),
        };
        if matches!(request.operation, NetworkOperation::SendTo | NetworkOperation::Write) {
            let Some((_, source)) = transfer else { return false };
            let Some(resources) = self.resources else {
                return Self::reply_request(
                    client,
                    target,
                    request,
                    logos_abi::NetworkStatus::Io,
                    scheduler,
                );
            };
            unsafe {
                core::ptr::copy_nonoverlapping(
                    source as *const u8,
                    (resources.tx_virtual + NETWORK_PAYLOAD_OFFSET) as *mut u8,
                    request.length as usize,
                );
            }
        }
        if !server.deliver(owner, request) {
            let reset = server.reset();
            let reply = Self::reply_unprocessed_request(
                client,
                target,
                request,
                logos_abi::NetworkStatus::Io,
                scheduler,
            );
            return reset && reply;
        }
        if !client.mark_processing() {
            let reset = server.reset();
            let reply = Self::reply_unprocessed_request(
                client,
                target,
                request,
                logos_abi::NetworkStatus::Io,
                scheduler,
            );
            return reset && reply;
        }
        self.active_client =
            Some(PendingClient { slot, request, owner, endpoint: client, server, target });
        if !scheduler.wake(service_handle) || !scheduler.run(service_handle) {
            return self
                .finish_active(error_reply(request, logos_abi::NetworkStatus::Io), scheduler);
        }
        true
    }

    #[allow(clippy::too_many_arguments)]
    pub fn relay_client(
        &mut self,
        slot: NetworkClientSlot,
        client: NetworkClientEndpoint,
        handle: Handle,
        scheduler: &mut native_task::Scheduler<'_>,
        session: &crate::platform::session::Context,
        capabilities: &logos_core::capabilities::CapabilityManager,
        shared_pages: &logos_core::shared_pages::SharedPages,
        owner: u64,
        tick: u64,
    ) -> bool {
        self.relay(
            Some(slot),
            CompletionTarget::Task(handle),
            client,
            scheduler,
            session,
            capabilities,
            shared_pages,
            owner,
            tick,
        )
    }

    #[cfg(feature = "test-hooks")]
    #[allow(clippy::too_many_arguments)]
    pub fn relay_probe(
        &mut self,
        client: NetworkClientEndpoint,
        scheduler: &mut native_task::Scheduler<'_>,
        session: &crate::platform::session::Context,
        capabilities: &logos_core::capabilities::CapabilityManager,
        shared_pages: &logos_core::shared_pages::SharedPages,
        owner: u64,
        tick: u64,
    ) -> bool {
        self.relay(
            None,
            CompletionTarget::Probe,
            client,
            scheduler,
            session,
            capabilities,
            shared_pages,
            owner,
            tick,
        )
    }
}

const NETWORK_PAYLOAD_OFFSET: u64 = 2048;

fn error_reply(
    request: NetworkRequest,
    status: logos_abi::NetworkStatus,
) -> logos_abi::NetworkReply {
    let mut counters = logos_abi::NetworkCounters::default();
    if status == logos_abi::NetworkStatus::Denied {
        counters.denied = 1;
    }
    logos_abi::NetworkReply {
        id: request.id,
        status,
        endpoint: logos_abi::NetworkEndpoint(0),
        generation: request.generation,
        source_address: 0,
        source_port: 0,
        length: 0,
        info: logos_abi::NetworkInfo::default(),
        counters,
    }
}

fn network_info(info: network::Info) -> logos_abi::NetworkInfo {
    logos_abi::NetworkInfo {
        mac: info.mac,
        mtu: info.mtu,
        generation: info.generation,
        link_up: 1,
        ..logos_abi::NetworkInfo::default()
    }
}

pub fn capability(request: NetworkRequest) -> Option<(CapabilityKind, u64)> {
    match request.operation {
        NetworkOperation::Bind | NetworkOperation::Listen => {
            Some((CapabilityKind::NetworkBind, request.peer.0))
        }
        NetworkOperation::SendTo | NetworkOperation::Echo | NetworkOperation::Write => {
            Some((CapabilityKind::NetworkSend, request.peer.0))
        }
        NetworkOperation::ReceiveFrom | NetworkOperation::Accept | NetworkOperation::Read => {
            Some((CapabilityKind::NetworkReceive, request.peer.0))
        }
        NetworkOperation::Status | NetworkOperation::Cancel | NetworkOperation::Close => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use logos_abi::{NetworkEndpoint, NetworkProtocol, NetworkScope, PageHandle};

    #[test]
    fn capability_mapping_is_exact() {
        let request = NetworkRequest {
            id: 1,
            operation: NetworkOperation::SendTo,
            endpoint: NetworkEndpoint::new(1, 1).unwrap(),
            peer: NetworkScope::new(NetworkProtocol::Udp, 0xc000_0201, 4001),
            page: PageHandle(1),
            length: 1,
            generation: 1,
            deadline: 1,
        };
        assert_eq!(capability(request), Some((CapabilityKind::NetworkSend, request.peer.0)));
        assert_eq!(
            error_reply(request, logos_abi::NetworkStatus::Denied).status,
            logos_abi::NetworkStatus::Denied
        );
    }
}
