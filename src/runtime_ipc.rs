//! Runtime-owned IPC topology.
//!
//! The legacy service graph remains available during the ABI migration. This
//! registry is the v5 ownership model: queues are private to Core, grants are
//! explicit, and every externally visible identity carries a generation.

use alloc::{collections::VecDeque, vec::Vec};

use crate::runtime_events::RuntimeEventRegistry;
use logos_abi::{
    CapabilityHandle, DIRECTORY_FLAG_MORE, DIRECTORY_RECORDS_PER_PAGE, DirectoryRecordKind,
    DirectoryRequest, DirectoryResponse, DirectoryStatus, EndpointHandle, EventHandle,
    IPC_PAGE_BYTES, IpcRights, IpcStatus, ServiceHandle,
};

struct Slot<T> {
    generation: u32,
    value: Option<T>,
}

impl<T> Slot<T> {
    fn empty() -> Self {
        Self { generation: 1, value: None }
    }
}

struct EndpointRecord {
    handle: EndpointHandle,
    producer: ServiceHandle,
    consumer: ServiceHandle,
    message_kind: u8,
    message_bytes: usize,
    queue_capacity: usize,
    service_epoch: u64,
    read_event: EventHandle,
    write_event: EventHandle,
    queue: VecDeque<Vec<u8>>,
}

struct CapabilityRecord {
    handle: CapabilityHandle,
    owner: ServiceHandle,
    endpoint: EndpointHandle,
    rights: IpcRights,
    service_epoch: u64,
}

/// Core-owned dynamic IPC records. `Vec` growth is task-context work; IRQ
/// producers only signal already-created event objects in later slices.
pub struct RuntimeIpcRegistry {
    endpoints: Vec<Slot<EndpointRecord>>,
    capabilities: Vec<Slot<CapabilityRecord>>,
}

impl RuntimeIpcRegistry {
    pub fn new() -> Self {
        Self { endpoints: Vec::new(), capabilities: Vec::new() }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_endpoint(
        &mut self,
        producer: ServiceHandle,
        consumer: ServiceHandle,
        message_kind: u8,
        message_bytes: usize,
        queue_capacity: usize,
        service_epoch: u64,
        events: &mut RuntimeEventRegistry,
    ) -> Result<EndpointHandle, IpcStatus> {
        if !producer.is_valid()
            || !consumer.is_valid()
            || producer == consumer
            || message_bytes == 0
            || message_bytes > IPC_PAGE_BYTES
            || queue_capacity == 0
            || queue_capacity > usize::from(u16::MAX)
            || service_epoch == 0
        {
            return Err(IpcStatus::Malformed);
        }
        let slot = self.allocate_endpoint_slot()?;
        let handle = EndpointHandle::new(slot as u32, self.endpoints[slot].generation)
            .ok_or(IpcStatus::Malformed)?;
        let read_event = events.create_event(consumer).map_err(|_| IpcStatus::Disconnected)?;
        let write_event = match events.create_event(producer) {
            Ok(event) => event,
            Err(error) => {
                let _ = events.destroy_event(consumer, read_event);
                return Err(match error {
                    crate::runtime_events::EventError::Unauthorized => IpcStatus::Unauthorized,
                    _ => IpcStatus::Disconnected,
                });
            }
        };
        self.endpoints[slot].value = Some(EndpointRecord {
            handle,
            producer,
            consumer,
            message_kind,
            message_bytes,
            queue_capacity,
            service_epoch,
            read_event,
            write_event,
            queue: VecDeque::new(),
        });
        Ok(handle)
    }

    pub fn grant(
        &mut self,
        owner: ServiceHandle,
        endpoint: EndpointHandle,
        rights: IpcRights,
    ) -> Result<CapabilityHandle, IpcStatus> {
        let (producer, consumer, service_epoch) = {
            let endpoint_record = self.endpoint(endpoint)?;
            (endpoint_record.producer, endpoint_record.consumer, endpoint_record.service_epoch)
        };
        let owns_endpoint = match rights {
            IpcRights::Send => producer == owner,
            IpcRights::Receive => consumer == owner,
        };
        if !owns_endpoint {
            return Err(IpcStatus::Unauthorized);
        }
        let slot = self.allocate_capability_slot()?;
        let handle = CapabilityHandle::new(slot as u32, self.capabilities[slot].generation)
            .ok_or(IpcStatus::Malformed)?;
        self.capabilities[slot].value =
            Some(CapabilityRecord { handle, owner, endpoint, rights, service_epoch });
        Ok(handle)
    }

    pub fn destroy_endpoint(
        &mut self,
        endpoint: EndpointHandle,
        events: &mut RuntimeEventRegistry,
    ) -> Result<(), IpcStatus> {
        let slot = self.endpoint_slot(endpoint)?;
        if let Some(record) = self.endpoints[slot].value.as_ref() {
            let _ = events.destroy_event(record.consumer, record.read_event);
            let _ = events.destroy_event(record.producer, record.write_event);
        }
        self.endpoints[slot].value = None;
        self.endpoints[slot].generation = next_generation(self.endpoints[slot].generation);
        for capability in &mut self.capabilities {
            if capability.value.as_ref().is_some_and(|grant| grant.endpoint == endpoint) {
                capability.value = None;
                capability.generation = next_generation(capability.generation);
            }
        }
        Ok(())
    }

    pub fn endpoint_message_kind(&self, endpoint: EndpointHandle) -> Result<u8, IpcStatus> {
        Ok(self.endpoint(endpoint)?.message_kind)
    }

    pub fn endpoint_events(
        &self,
        endpoint: EndpointHandle,
    ) -> Result<(EventHandle, EventHandle), IpcStatus> {
        let endpoint = self.endpoint(endpoint)?;
        Ok((endpoint.read_event, endpoint.write_event))
    }

    pub fn validate_capability(
        &self,
        caller: ServiceHandle,
        capability: CapabilityHandle,
        rights: IpcRights,
        message_bytes: usize,
    ) -> Result<EndpointHandle, IpcStatus> {
        let (endpoint, expected_bytes) = self.capability_endpoint(caller, capability, rights)?;
        if message_bytes != expected_bytes {
            return Err(IpcStatus::Malformed);
        }
        Ok(endpoint)
    }

    pub fn capability_endpoint(
        &self,
        caller: ServiceHandle,
        capability: CapabilityHandle,
        rights: IpcRights,
    ) -> Result<(EndpointHandle, usize), IpcStatus> {
        let grant = self.capability(capability)?;
        if grant.owner != caller || grant.rights != rights {
            return Err(IpcStatus::Unauthorized);
        }
        let endpoint = self.endpoint(grant.endpoint)?;
        if grant.service_epoch != endpoint.service_epoch {
            return Err(IpcStatus::Stale);
        }
        let message_bytes = endpoint.message_bytes;
        let endpoint_handle = endpoint.handle;
        Ok((endpoint_handle, message_bytes))
    }

    pub fn destroy_service(&mut self, service: ServiceHandle, events: &mut RuntimeEventRegistry) {
        let endpoints: Vec<_> = self
            .endpoints
            .iter()
            .filter_map(|slot| slot.value.as_ref())
            .filter(|endpoint| endpoint.producer == service || endpoint.consumer == service)
            .map(|endpoint| endpoint.handle)
            .collect();
        for endpoint in endpoints {
            let _ = self.destroy_endpoint(endpoint, events);
        }
        for capability in &mut self.capabilities {
            if capability.value.as_ref().is_some_and(|grant| grant.owner == service) {
                capability.value = None;
                capability.generation = next_generation(capability.generation);
            }
        }
    }

    pub fn send(
        &mut self,
        caller: ServiceHandle,
        capability: CapabilityHandle,
        bytes: &[u8],
        events: &mut RuntimeEventRegistry,
    ) -> IpcStatus {
        let (owner, endpoint_handle, rights, service_epoch) = match self.capability(capability) {
            Ok(grant) => (grant.owner, grant.endpoint, grant.rights, grant.service_epoch),
            Err(status) => return status,
        };
        if owner != caller || rights != IpcRights::Send {
            return IpcStatus::Unauthorized;
        }
        let endpoint = match self.endpoint_mut(endpoint_handle) {
            Ok(endpoint) => endpoint,
            Err(status) => return status,
        };
        if service_epoch != endpoint.service_epoch {
            return IpcStatus::Stale;
        }
        if bytes.len() != endpoint.message_bytes {
            return IpcStatus::Malformed;
        }
        if endpoint.queue.len() >= endpoint.queue_capacity {
            return IpcStatus::Full;
        }
        let mut message = Vec::new();
        if message.try_reserve(bytes.len()).is_err() {
            return IpcStatus::Disconnected;
        }
        message.extend_from_slice(bytes);
        endpoint.queue.push_back(message);
        let _ = events.signal_irq(endpoint.read_event);
        IpcStatus::Ok
    }

    pub fn receive(
        &mut self,
        caller: ServiceHandle,
        capability: CapabilityHandle,
        bytes: &mut [u8],
        events: &mut RuntimeEventRegistry,
    ) -> IpcStatus {
        let (owner, endpoint_handle, rights, service_epoch) = match self.capability(capability) {
            Ok(grant) => (grant.owner, grant.endpoint, grant.rights, grant.service_epoch),
            Err(status) => return status,
        };
        if owner != caller || rights != IpcRights::Receive {
            return IpcStatus::Unauthorized;
        }
        let endpoint = match self.endpoint_mut(endpoint_handle) {
            Ok(endpoint) => endpoint,
            Err(status) => return status,
        };
        if service_epoch != endpoint.service_epoch {
            return IpcStatus::Stale;
        }
        if bytes.len() != endpoint.message_bytes {
            return IpcStatus::Malformed;
        }
        let Some(message) = endpoint.queue.front() else { return IpcStatus::Empty };
        bytes.copy_from_slice(message);
        endpoint.queue.pop_front();
        let _ = events.signal_irq(endpoint.write_event);
        IpcStatus::Ok
    }

    pub fn directory(
        &self,
        request: DirectoryRequest,
        response: &mut DirectoryResponse,
    ) -> DirectoryStatus {
        if !request.is_valid() || request.operation != logos_abi::DirectoryOperation::Capabilities {
            return DirectoryStatus::Malformed;
        }
        *response =
            DirectoryResponse::empty(request.operation, DirectoryStatus::Ok, request.request_id);
        let mut seen = 0u64;
        let mut written = 0usize;
        for capability in self.capabilities.iter().filter_map(|slot| slot.value.as_ref()) {
            if capability.owner != request.subject {
                continue;
            }
            if seen < request.cursor {
                seen += 1;
                continue;
            }
            if written == DIRECTORY_RECORDS_PER_PAGE {
                response.flags |= DIRECTORY_FLAG_MORE;
                response.cursor = request.cursor + written as u64;
                break;
            }
            let Some(endpoint) = self.endpoint(capability.endpoint).ok() else { continue };
            response.records[written] = logos_abi::DirectoryRecord {
                kind: DirectoryRecordKind::Capability,
                rights: capability.rights as u8,
                flags: 0,
                handle: capability.handle.raw(),
                peer: if endpoint.producer == capability.owner {
                    endpoint.consumer
                } else {
                    endpoint.producer
                },
                message_bytes: endpoint.message_bytes as u16,
                queue_capacity: endpoint.queue_capacity as u16,
                name_len: 0,
                reserved: [0; 3],
                name: [0; logos_abi::MAX_SERVICE_NAME_BYTES],
            };
            written += 1;
            seen += 1;
        }
        response.count = written as u8;
        if response.flags & DIRECTORY_FLAG_MORE == 0 {
            response.cursor = request.cursor + written as u64;
        }
        DirectoryStatus::Ok
    }

    fn allocate_endpoint_slot(&mut self) -> Result<usize, IpcStatus> {
        if let Some((index, _)) =
            self.endpoints.iter().enumerate().find(|(_, slot)| slot.value.is_none())
        {
            return Ok(index);
        }
        self.endpoints.try_reserve(1).map_err(|_| IpcStatus::Disconnected)?;
        self.endpoints.push(Slot::empty());
        Ok(self.endpoints.len() - 1)
    }

    fn allocate_capability_slot(&mut self) -> Result<usize, IpcStatus> {
        if let Some((index, _)) =
            self.capabilities.iter().enumerate().find(|(_, slot)| slot.value.is_none())
        {
            return Ok(index);
        }
        self.capabilities.try_reserve(1).map_err(|_| IpcStatus::Disconnected)?;
        self.capabilities.push(Slot::empty());
        Ok(self.capabilities.len() - 1)
    }

    fn endpoint_slot(&self, handle: EndpointHandle) -> Result<usize, IpcStatus> {
        let index = usize::try_from(handle.index()).map_err(|_| IpcStatus::Stale)?;
        let Some(slot) = self.endpoints.get(index) else { return Err(IpcStatus::Stale) };
        if slot.generation != handle.generation() || slot.value.is_none() {
            return Err(IpcStatus::Stale);
        }
        Ok(index)
    }

    fn endpoint(&self, handle: EndpointHandle) -> Result<&EndpointRecord, IpcStatus> {
        let slot = self.endpoint_slot(handle)?;
        self.endpoints[slot].value.as_ref().ok_or(IpcStatus::Stale)
    }

    fn endpoint_mut(&mut self, handle: EndpointHandle) -> Result<&mut EndpointRecord, IpcStatus> {
        let slot = self.endpoint_slot(handle)?;
        self.endpoints[slot].value.as_mut().ok_or(IpcStatus::Stale)
    }

    fn capability(&self, handle: CapabilityHandle) -> Result<&CapabilityRecord, IpcStatus> {
        let index = usize::try_from(handle.index()).map_err(|_| IpcStatus::Stale)?;
        let Some(slot) = self.capabilities.get(index) else { return Err(IpcStatus::Stale) };
        if slot.generation != handle.generation() || slot.value.is_none() {
            return Err(IpcStatus::Stale);
        }
        slot.value.as_ref().ok_or(IpcStatus::Stale)
    }
}

impl Default for RuntimeIpcRegistry {
    fn default() -> Self {
        Self::new()
    }
}

fn next_generation(current: u32) -> u32 {
    current.wrapping_add(1).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn services() -> (ServiceHandle, ServiceHandle) {
        (ServiceHandle::new(1, 1).unwrap(), ServiceHandle::new(2, 1).unwrap())
    }

    #[test]
    fn dynamic_endpoint_roundtrip_enforces_exact_size_and_backpressure() {
        let (producer, consumer) = services();
        let mut registry = RuntimeIpcRegistry::new();
        let mut events = RuntimeEventRegistry::new();
        let endpoint =
            registry.create_endpoint(producer, consumer, 7, 3, 1, 9, &mut events).unwrap();
        let (read_event, write_event) = registry.endpoint_events(endpoint).unwrap();
        assert_ne!(read_event, write_event);
        assert_eq!(read_event.generation(), endpoint.generation());
        assert_eq!(write_event.generation(), endpoint.generation());
        let read_set = events.create_set(consumer).unwrap();
        events.add(consumer, read_set, read_event).unwrap();
        let write_set = events.create_set(producer).unwrap();
        events.add(producer, write_set, write_event).unwrap();
        let send = registry.grant(producer, endpoint, IpcRights::Send).unwrap();
        let receive = registry.grant(consumer, endpoint, IpcRights::Receive).unwrap();
        assert_eq!(registry.send(producer, send, &[1, 2], &mut events), IpcStatus::Malformed);
        assert_eq!(registry.send(producer, send, &[1, 2, 3], &mut events), IpcStatus::Ok);
        assert_eq!(
            events.wait_any(consumer, read_set, 1, Some(10)),
            Ok(crate::runtime_events::EventWait::Ready(read_event))
        );
        assert_eq!(registry.send(producer, send, &[4, 5, 6], &mut events), IpcStatus::Full);
        let mut output = [0; 3];
        assert_eq!(registry.receive(consumer, receive, &mut output, &mut events), IpcStatus::Ok);
        assert_eq!(output, [1, 2, 3]);
        assert_eq!(
            events.wait_any(producer, write_set, 1, Some(10)),
            Ok(crate::runtime_events::EventWait::Ready(write_event))
        );
        assert_eq!(registry.receive(consumer, receive, &mut output, &mut events), IpcStatus::Empty);
    }

    #[test]
    fn stale_and_forged_capabilities_cannot_access_reused_endpoints() {
        let (producer, consumer) = services();
        let mut registry = RuntimeIpcRegistry::new();
        let mut events = RuntimeEventRegistry::new();
        let endpoint =
            registry.create_endpoint(producer, consumer, 1, 1, 1, 1, &mut events).unwrap();
        let capability = registry.grant(producer, endpoint, IpcRights::Send).unwrap();
        assert_eq!(registry.send(consumer, capability, &[1], &mut events), IpcStatus::Unauthorized);
        let (read_event, write_event) = registry.endpoint_events(endpoint).unwrap();
        registry.destroy_endpoint(endpoint, &mut events).unwrap();
        assert_eq!(events.signal_irq(read_event), Err(crate::runtime_events::EventError::Stale));
        assert_eq!(events.signal_irq(write_event), Err(crate::runtime_events::EventError::Stale));
        assert_eq!(registry.send(producer, capability, &[1], &mut events), IpcStatus::Stale);
        let replacement =
            registry.create_endpoint(producer, consumer, 1, 1, 1, 2, &mut events).unwrap();
        assert_ne!(endpoint, replacement);
        assert_eq!(registry.send(producer, capability, &[1], &mut events), IpcStatus::Stale);
    }

    #[test]
    fn capability_validation_checks_owner_rights_and_exact_size() {
        let (producer, consumer) = services();
        let mut registry = RuntimeIpcRegistry::new();
        let mut events = RuntimeEventRegistry::new();
        let endpoint =
            registry.create_endpoint(producer, consumer, 1, 4, 1, 1, &mut events).unwrap();
        let send = registry.grant(producer, endpoint, IpcRights::Send).unwrap();
        assert_eq!(registry.validate_capability(producer, send, IpcRights::Send, 4), Ok(endpoint));
        assert_eq!(
            registry.validate_capability(consumer, send, IpcRights::Send, 4),
            Err(IpcStatus::Unauthorized)
        );
        assert_eq!(
            registry.validate_capability(producer, send, IpcRights::Receive, 4),
            Err(IpcStatus::Unauthorized)
        );
        assert_eq!(
            registry.validate_capability(producer, send, IpcRights::Send, 3),
            Err(IpcStatus::Malformed)
        );
    }

    #[test]
    fn capability_directory_is_cursored() {
        let (producer, consumer) = services();
        let mut registry = RuntimeIpcRegistry::new();
        let mut events = RuntimeEventRegistry::new();
        let endpoint =
            registry.create_endpoint(producer, consumer, 1, 4, 2, 1, &mut events).unwrap();
        let _ = registry.grant(producer, endpoint, IpcRights::Send).unwrap();
        let mut request = DirectoryRequest::new(logos_abi::DirectoryOperation::Capabilities, 17);
        request.subject = producer;
        let mut response = DirectoryResponse::empty(
            request.operation,
            DirectoryStatus::Malformed,
            request.request_id,
        );
        assert_eq!(registry.directory(request, &mut response), DirectoryStatus::Ok);
        assert_eq!(response.count, 1);
        assert!(response.is_valid_for(request));
    }

    #[test]
    fn typed_payload_limit_is_the_ipc_page_not_compact_bytes_limit() {
        let (producer, consumer) = services();
        let mut registry = RuntimeIpcRegistry::new();
        let mut events = RuntimeEventRegistry::new();
        let endpoint = registry
            .create_endpoint(producer, consumer, 2, logos_abi::IPC_PAGE_BYTES, 1, 1, &mut events)
            .unwrap();
        let capability = registry.grant(producer, endpoint, IpcRights::Send).unwrap();
        assert_eq!(
            registry.send(producer, capability, &[0; logos_abi::IPC_PAGE_BYTES], &mut events),
            IpcStatus::Ok
        );
    }

    #[test]
    fn all_abi_endpoint_payloads_fit_the_dynamic_registry() {
        let (producer, consumer) = services();
        let mut registry = RuntimeIpcRegistry::new();
        let mut events = RuntimeEventRegistry::new();
        for raw in 0..logos_abi::IPC_ENDPOINT_COUNT {
            let bytes = logos_abi::ipc_message_size(raw).unwrap();
            registry.create_endpoint(producer, consumer, 0, bytes, 1, 1, &mut events).unwrap();
        }
    }

    #[test]
    fn built_in_capability_grants_fit_the_dynamic_registry() {
        let core = ServiceHandle::new(u32::MAX, 1).unwrap();
        let mut registry = RuntimeIpcRegistry::new();
        let mut events = RuntimeEventRegistry::new();
        for raw in 0..logos_abi::IPC_ENDPOINT_COUNT {
            let endpoint_id = logos_abi::IpcEndpointId::from_index(raw).unwrap();
            let core_producer = matches!(
                endpoint_id,
                logos_abi::IpcEndpointId::CoreToStorage
                    | logos_abi::IpcEndpointId::CoreToNetwork
                    | logos_abi::IpcEndpointId::CoreToDevice
                    | logos_abi::IpcEndpointId::CoreToStoragePackage
                    | logos_abi::IpcEndpointId::CoreToStorageMap
            );
            let core_consumer = matches!(
                endpoint_id,
                logos_abi::IpcEndpointId::StorageToCore
                    | logos_abi::IpcEndpointId::NetworkToCore
                    | logos_abi::IpcEndpointId::DeviceToCore
                    | logos_abi::IpcEndpointId::StoragePackageToCore
                    | logos_abi::IpcEndpointId::StorageMapToCore
            );
            let producer = if core_producer {
                core
            } else {
                ServiceHandle::new(endpoint_id.producer().index() as u32, 1).unwrap()
            };
            let consumer = if core_consumer {
                core
            } else if matches!(
                endpoint_id,
                logos_abi::IpcEndpointId::FetchToStorage | logos_abi::IpcEndpointId::FetchToNetwork
            ) {
                let service = if endpoint_id == logos_abi::IpcEndpointId::FetchToStorage {
                    logos_abi::ServiceId::Storage
                } else {
                    logos_abi::ServiceId::Network
                };
                ServiceHandle::new(service.index() as u32, 1).unwrap()
            } else {
                ServiceHandle::new(endpoint_id.consumer().index() as u32, 1).unwrap()
            };
            let endpoint = registry
                .create_endpoint(
                    producer,
                    consumer,
                    0,
                    logos_abi::ipc_message_size(raw).unwrap(),
                    1,
                    1,
                    &mut events,
                )
                .unwrap();
            for raw_service in 0..10 {
                let service = logos_abi::ServiceId::from_index(raw_service).unwrap();
                let owner = ServiceHandle::new(raw_service as u32, 1).unwrap();
                for rights in [IpcRights::Send, IpcRights::Receive] {
                    if logos_abi::ipc_capability_slot(service, endpoint_id, rights).is_some() {
                        registry.grant(owner, endpoint, rights).unwrap();
                    }
                }
            }
        }
    }
}
