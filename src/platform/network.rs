pub const SERVICE: crate::platform::services::Service = crate::platform::services::Service::Network;

use crate::drivers::network;
use crate::sched::native_task::{
    self, Handle, NetworkClientEndpoint, NetworkDeviceEndpoint, NetworkEventEndpoint,
    NetworkServerEndpoint,
};
use logos_abi::{NetworkOperation, NetworkRequest};
use logos_core::capabilities::CapabilityKind;

#[derive(Clone, Copy)]
pub struct PendingClient {
    pub request: logos_abi::NetworkRequest,
    pub owner: u64,
    pub endpoint: Option<NetworkClientEndpoint>,
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

pub struct NetworkRuntime {
    task: Option<Handle>,
    device_endpoint: NetworkDeviceEndpoint,
    event_endpoint: NetworkEventEndpoint,
    server_endpoint: Option<NetworkServerEndpoint>,
    clients: [Option<PendingClient>; 2],
    device: Option<network::Device>,
    resources: Option<Resources>,
    pending: Option<PendingDevice>,
    device_generation: u32,
    failures: u32,
    degraded: bool,
}

impl NetworkRuntime {
    pub const fn task(&self) -> Option<Handle> {
        self.task
    }

    pub const fn device_endpoint(&self) -> NetworkDeviceEndpoint {
        self.device_endpoint
    }

    pub const fn event_endpoint(&self) -> NetworkEventEndpoint {
        self.event_endpoint
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
            clients: [None, None],
            device,
            resources: None,
            pending: None,
            device_generation,
            failures: 0,
            degraded: false,
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
        self.clients = [None, None];
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
        self.clients = [None, None];
        scheduler.wake(task) && scheduler.run(task)
    }

    pub fn poll(&mut self, scheduler: &mut native_task::Scheduler<'_>, tick: u64) -> bool {
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
                                return scheduler.wake(task) && scheduler.run(task);
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
                return self.device_endpoint.reply(reply)
                    && scheduler.wake(task)
                    && scheduler.run(task);
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
            return self.event_endpoint.deliver(event)
                && scheduler.wake(task)
                && scheduler.run(task);
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
                self.event_endpoint.deliver(event) && scheduler.wake(task) && scheduler.run(task)
            }
            Ok(None) => true,
            Err(_) => self.reset(scheduler),
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

    #[allow(clippy::too_many_arguments)]
    pub fn relay_client(
        &mut self,
        slot: usize,
        client: NetworkClientEndpoint,
        handle: Handle,
        scheduler: &mut native_task::Scheduler<'_>,
        session: &crate::platform::session::Context,
        capabilities: &logos_core::capabilities::CapabilityManager,
        shared_pages: &logos_core::shared_pages::SharedPages,
        owner: u64,
        tick: u64,
    ) -> bool {
        let Some(server) = self.server_endpoint else { return true };
        let Some(service_handle) = self.task else { return true };
        let Some(slot) = self.clients.get_mut(slot) else { return false };
        if let Some(current) = *slot {
            if tick >= current.request.deadline {
                *slot = None;
                let reply = error_reply(current.request, logos_abi::NetworkStatus::TimedOut);
                let _ = server.reset();
                return current.endpoint.is_some_and(|endpoint| endpoint.reply(reply))
                    && scheduler.wake(handle)
                    && scheduler.run(handle);
            }
            if let Some(reply) = server.response(current.request.id) {
                if !reply.valid_for(current.request) {
                    return false;
                }
                *slot = None;
                if matches!(
                    current.request.operation,
                    NetworkOperation::ReceiveFrom | NetworkOperation::Read
                ) && reply.status == logos_abi::NetworkStatus::Complete
                    && let (Some(source), Some(network_pages)) = (
                        shared_pages.address(current.owner, current.request.page),
                        self.resources
                            .as_ref()
                            .map(|resources| (resources.tx, resources.tx_virtual)),
                    )
                {
                    let _ = source;
                    let target = network_pages.1 + NETWORK_PAYLOAD_OFFSET;
                    unsafe {
                        core::ptr::copy_nonoverlapping(
                            target as *const u8,
                            source as *mut u8,
                            reply.length as usize,
                        );
                    }
                }
                return current.endpoint.is_some_and(|endpoint| endpoint.reply(reply))
                    && scheduler.wake(handle)
                    && scheduler.run(handle);
            }
            return true;
        }
        let Some(request) = client.request() else { return true };
        if !request.valid_shape() {
            return client.mark_processing()
                && client.reply(error_reply(request, status_for(request, false)))
                && scheduler.wake(handle)
                && scheduler.run(handle);
        }
        if let Some((kind, scope)) = capability(request)
            && !session.allows_scoped64(capabilities, kind, scope)
        {
            return client.mark_processing()
                && client.reply(denied_reply(request))
                && scheduler.wake(handle)
                && scheduler.run(handle);
        }
        let page = (request.page.0 != 0).then_some(request.page);
        if matches!(
            request.operation,
            NetworkOperation::SendTo
                | NetworkOperation::Write
                | NetworkOperation::ReceiveFrom
                | NetworkOperation::Read
        ) && page.is_none()
        {
            return client.mark_processing()
                && client.reply(error_reply(request, logos_abi::NetworkStatus::Invalid))
                && scheduler.wake(handle)
                && scheduler.run(handle);
        }
        if matches!(request.operation, NetworkOperation::SendTo | NetworkOperation::Write) {
            let Some(source) = page.and_then(|page| shared_pages.address(owner, page)) else {
                return client.mark_processing()
                    && client.reply(error_reply(request, logos_abi::NetworkStatus::Invalid))
                    && scheduler.wake(handle)
                    && scheduler.run(handle);
            };
            let Some(resources) = self.resources else { return true };
            unsafe {
                core::ptr::copy_nonoverlapping(
                    source as *const u8,
                    (resources.tx_virtual + NETWORK_PAYLOAD_OFFSET) as *mut u8,
                    request.length as usize,
                );
            }
        }
        if !client.mark_processing() {
            return false;
        }
        if !server.deliver(owner, request) {
            return true;
        }
        *slot = Some(PendingClient { request, owner, endpoint: Some(client) });
        scheduler.wake(service_handle) && scheduler.run(service_handle)
    }
}

const NETWORK_PAYLOAD_OFFSET: u64 = 2048;

fn error_reply(
    request: NetworkRequest,
    status: logos_abi::NetworkStatus,
) -> logos_abi::NetworkReply {
    logos_abi::NetworkReply {
        id: request.id,
        status,
        endpoint: logos_abi::NetworkEndpoint(0),
        generation: request.generation,
        source_address: 0,
        source_port: 0,
        length: 0,
        info: logos_abi::NetworkInfo::default(),
        counters: logos_abi::NetworkCounters::default(),
    }
}

fn denied_reply(request: NetworkRequest) -> logos_abi::NetworkReply {
    let mut reply = error_reply(request, logos_abi::NetworkStatus::Denied);
    reply.counters.denied = 1;
    reply
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

pub fn status_for(request: NetworkRequest, allowed: bool) -> logos_abi::NetworkStatus {
    if !request.valid_shape() {
        logos_abi::NetworkStatus::Invalid
    } else if !allowed {
        logos_abi::NetworkStatus::Denied
    } else {
        logos_abi::NetworkStatus::Complete
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
        assert_eq!(status_for(request, false), logos_abi::NetworkStatus::Denied);
    }
}
