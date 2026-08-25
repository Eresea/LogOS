//! Runtime-owned IPC topology.
//!
//! The v5 ownership model: queues are private to Core, grants are explicit,
//! and every externally visible identity carries a generation.

use alloc::vec::Vec;

use crate::frame_pool::{FrameAddress, FramePool};
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
    fn with_generation(generation: u32) -> Self {
        Self { generation: generation.max(1), value: None }
    }
}

struct EndpointRecord {
    handle: EndpointHandle,
    producer: ServiceHandle,
    consumer: ServiceHandle,
    contract_id: u16,
    message_bytes: usize,
    queue_capacity: usize,
    service_epoch: u64,
    read_event: EventHandle,
    write_event: EventHandle,
    queue: QueueStorage,
    queue_head: usize,
    queue_tail: usize,
    queue_len: usize,
}

enum QueueStorage {
    Heap(Vec<QueueMessage>),
    Frames(Vec<FrameAddress>),
}

struct QueueMessage {
    bytes: [u8; IPC_PAGE_BYTES],
}

impl QueueMessage {
    const fn empty() -> Self {
        Self { bytes: [0; IPC_PAGE_BYTES] }
    }
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
    generation_seed: u32,
    capability_generation: u32,
    queue_budget_messages: usize,
    queue_committed_messages: usize,
}

impl RuntimeIpcRegistry {
    pub fn new() -> Self {
        Self::new_with_generation(1)
    }

    pub fn new_with_generation(generation: u32) -> Self {
        Self::new_with_generation_and_budget(generation, usize::MAX)
    }

    pub fn new_with_generation_and_budget(generation: u32, queue_budget_messages: usize) -> Self {
        let generation_seed = generation.max(1);
        Self {
            endpoints: Vec::new(),
            capabilities: Vec::new(),
            generation_seed,
            capability_generation: next_generation(generation_seed),
            queue_budget_messages,
            queue_committed_messages: 0,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_endpoint(
        &mut self,
        producer: ServiceHandle,
        consumer: ServiceHandle,
        contract_id: u16,
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
        if queue_capacity > self.queue_budget_messages.saturating_sub(self.queue_committed_messages)
        {
            return Err(IpcStatus::Full);
        }
        let mut queue = Vec::new();
        if queue.try_reserve(queue_capacity).is_err() {
            return Err(IpcStatus::Disconnected);
        }
        for _ in 0..queue_capacity {
            queue.push(QueueMessage::empty());
        }
        self.create_endpoint_with_queue(
            producer,
            consumer,
            contract_id,
            message_bytes,
            queue_capacity,
            service_epoch,
            QueueStorage::Heap(queue),
            events,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_endpoint_with_frames(
        &mut self,
        producer: ServiceHandle,
        consumer: ServiceHandle,
        contract_id: u16,
        message_bytes: usize,
        queue_capacity: usize,
        service_epoch: u64,
        queue_frames: &[FrameAddress],
        events: &mut RuntimeEventRegistry,
    ) -> Result<EndpointHandle, IpcStatus> {
        if queue_frames.len() != queue_capacity {
            return Err(IpcStatus::Malformed);
        }
        let mut queue = Vec::new();
        queue.try_reserve(queue_capacity).map_err(|_| IpcStatus::Disconnected)?;
        queue.extend_from_slice(queue_frames);
        self.create_endpoint_with_queue(
            producer,
            consumer,
            contract_id,
            message_bytes,
            queue_capacity,
            service_epoch,
            QueueStorage::Frames(queue),
            events,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn create_endpoint_with_queue(
        &mut self,
        producer: ServiceHandle,
        consumer: ServiceHandle,
        contract_id: u16,
        message_bytes: usize,
        queue_capacity: usize,
        service_epoch: u64,
        queue: QueueStorage,
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
        if queue_capacity > self.queue_budget_messages.saturating_sub(self.queue_committed_messages)
        {
            return Err(IpcStatus::Full);
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
            contract_id,
            message_bytes,
            queue_capacity,
            service_epoch,
            read_event,
            write_event,
            queue,
            queue_head: 0,
            queue_tail: 0,
            queue_len: 0,
        });
        self.queue_committed_messages =
            self.queue_committed_messages.saturating_add(queue_capacity);
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
        let slot = self.allocate_capability_record()?;
        let handle = CapabilityHandle::new(slot as u32, self.capabilities[slot].generation)
            .ok_or(IpcStatus::Malformed)?;
        self.capabilities[slot].value =
            Some(CapabilityRecord { handle, owner, endpoint, rights, service_epoch });
        Ok(handle)
    }

    pub fn capability_for(
        &self,
        owner: ServiceHandle,
        endpoint: EndpointHandle,
        rights: IpcRights,
    ) -> Result<CapabilityHandle, IpcStatus> {
        self.capabilities
            .iter()
            .filter_map(|slot| slot.value.as_ref())
            .find(|grant| {
                grant.owner == owner && grant.endpoint == endpoint && grant.rights == rights
            })
            .map(|grant| grant.handle)
            .ok_or(IpcStatus::Unauthorized)
    }

    pub fn ownership_counts(&self, owner: ServiceHandle) -> (usize, usize) {
        let endpoints = self
            .endpoints
            .iter()
            .filter_map(|slot| slot.value.as_ref())
            .filter(|endpoint| endpoint.producer == owner || endpoint.consumer == owner)
            .count();
        let capabilities = self
            .capabilities
            .iter()
            .filter_map(|slot| slot.value.as_ref())
            .filter(|capability| capability.owner == owner)
            .count();
        (endpoints, capabilities)
    }

    pub fn destroy_endpoint(
        &mut self,
        endpoint: EndpointHandle,
        events: &mut RuntimeEventRegistry,
    ) -> Result<(), IpcStatus> {
        let _ = self.destroy_endpoint_inner(endpoint, events)?;
        Ok(())
    }

    pub fn destroy_endpoint_with_pool(
        &mut self,
        endpoint: EndpointHandle,
        events: &mut RuntimeEventRegistry,
        frames: &mut FramePool,
    ) -> Result<(), IpcStatus> {
        let queue_frames = self.destroy_endpoint_inner(endpoint, events)?;
        for frame in queue_frames {
            frames.release(frame).map_err(|_| IpcStatus::Disconnected)?;
        }
        Ok(())
    }

    fn destroy_endpoint_inner(
        &mut self,
        endpoint: EndpointHandle,
        events: &mut RuntimeEventRegistry,
    ) -> Result<Vec<FrameAddress>, IpcStatus> {
        let slot = self.endpoint_slot(endpoint)?;
        let mut queue_frames = Vec::new();
        if let Some(record) = self.endpoints[slot].value.as_ref() {
            self.queue_committed_messages =
                self.queue_committed_messages.saturating_sub(record.queue_capacity);
            let _ = events.destroy_event(record.consumer, record.read_event);
            let _ = events.destroy_event(record.producer, record.write_event);
            if let QueueStorage::Frames(frames) = &record.queue {
                queue_frames.try_reserve(frames.len()).map_err(|_| IpcStatus::Disconnected)?;
                queue_frames.extend_from_slice(frames);
            }
        }
        self.endpoints[slot].value = None;
        self.endpoints[slot].generation = next_generation(self.endpoints[slot].generation);
        for capability in &mut self.capabilities {
            if capability.value.as_ref().is_some_and(|grant| grant.endpoint == endpoint) {
                capability.value = None;
                capability.generation = next_generation(capability.generation);
            }
        }
        Ok(queue_frames)
    }

    pub fn endpoint_contract_id(&self, endpoint: EndpointHandle) -> Result<u16, IpcStatus> {
        Ok(self.endpoint(endpoint)?.contract_id)
    }

    pub fn endpoint_matches(
        &self,
        endpoint: EndpointHandle,
        producer: ServiceHandle,
        consumer: ServiceHandle,
        contract_id: u16,
    ) -> Result<bool, IpcStatus> {
        let record = self.endpoint(endpoint)?;
        Ok(record.producer == producer
            && record.consumer == consumer
            && record.contract_id == contract_id)
    }

    pub fn all_endpoint_generations_differ(&self, generation: u32) -> bool {
        self.endpoints
            .iter()
            .filter_map(|slot| slot.value.as_ref())
            .all(|endpoint| endpoint.handle.generation() != generation)
    }

    pub fn find_endpoint(
        &self,
        producer: ServiceHandle,
        consumer: ServiceHandle,
        contract_id: u16,
    ) -> Result<EndpointHandle, IpcStatus> {
        if !producer.is_valid() || !consumer.is_valid() || contract_id == 0 {
            return Err(IpcStatus::Malformed);
        }
        self.endpoints
            .iter()
            .filter_map(|slot| slot.value.as_ref())
            .find(|endpoint| {
                endpoint.producer == producer
                    && endpoint.consumer == consumer
                    && endpoint.contract_id == contract_id
            })
            .map(|endpoint| endpoint.handle)
            .ok_or(IpcStatus::Disconnected)
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
        self.destroy_service_inner(service, events, None);
    }

    pub fn destroy_service_with_pool(
        &mut self,
        service: ServiceHandle,
        events: &mut RuntimeEventRegistry,
        frames: &mut FramePool,
    ) {
        self.destroy_service_inner(service, events, Some(frames));
    }

    pub fn drain_queue_frames(&mut self, events: &mut RuntimeEventRegistry) -> Vec<FrameAddress> {
        let endpoints: Vec<_> = self
            .endpoints
            .iter()
            .filter_map(|slot| slot.value.as_ref().map(|record| record.handle))
            .collect();
        let mut frames = Vec::new();
        for endpoint in endpoints {
            if let Ok(queue_frames) = self.destroy_endpoint_inner(endpoint, events) {
                let _ = frames.try_reserve(queue_frames.len());
                frames.extend(queue_frames);
            }
        }
        frames
    }

    fn destroy_service_inner(
        &mut self,
        service: ServiceHandle,
        events: &mut RuntimeEventRegistry,
        mut frames: Option<&mut FramePool>,
    ) {
        let endpoints: Vec<_> = self
            .endpoints
            .iter()
            .filter_map(|slot| slot.value.as_ref())
            .filter(|endpoint| endpoint.producer == service || endpoint.consumer == service)
            .map(|endpoint| endpoint.handle)
            .collect();
        for endpoint in endpoints {
            if let Some(frames) = frames.as_deref_mut() {
                let _ = self.destroy_endpoint_with_pool(endpoint, events, frames);
            } else {
                let _ = self.destroy_endpoint(endpoint, events);
            }
        }
        for capability in &mut self.capabilities {
            if capability.value.as_ref().is_some_and(|grant| grant.owner == service) {
                capability.value = None;
                capability.generation = next_generation(capability.generation);
            }
        }
        events.destroy_service(service);
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
        if endpoint.queue_len >= endpoint.queue_capacity {
            return IpcStatus::Full;
        }
        let index = endpoint.queue_tail;
        match &mut endpoint.queue {
            QueueStorage::Heap(queue) => {
                queue[index].bytes[..bytes.len()].copy_from_slice(bytes);
            }
            QueueStorage::Frames(queue) => unsafe {
                core::ptr::copy_nonoverlapping(
                    bytes.as_ptr(),
                    queue[index].raw() as usize as *mut u8,
                    bytes.len(),
                );
            },
        }
        endpoint.queue_tail = (index + 1) % endpoint.queue_capacity;
        endpoint.queue_len += 1;
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
        if endpoint.queue_len == 0 {
            return IpcStatus::Empty;
        }
        let index = endpoint.queue_head;
        match &endpoint.queue {
            QueueStorage::Heap(queue) => {
                bytes.copy_from_slice(&queue[index].bytes[..endpoint.message_bytes]);
            }
            QueueStorage::Frames(queue) => unsafe {
                core::ptr::copy_nonoverlapping(
                    queue[index].raw() as usize as *const u8,
                    bytes.as_mut_ptr(),
                    endpoint.message_bytes,
                );
            },
        }
        endpoint.queue_head = (index + 1) % endpoint.queue_capacity;
        endpoint.queue_len -= 1;
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
                response.cursor = request.cursor.saturating_add(written as u64);
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
                contract_id: endpoint.contract_id,
                message_bytes: endpoint.message_bytes as u16,
                queue_capacity: endpoint.queue_capacity as u16,
                event: if capability.rights == IpcRights::Send {
                    endpoint.write_event
                } else {
                    endpoint.read_event
                },
                name_len: 0,
                reserved: [0; 1],
                name: [0; logos_abi::MAX_SERVICE_NAME_BYTES],
            };
            written += 1;
            seen += 1;
        }
        response.count = written as u8;
        if response.flags & DIRECTORY_FLAG_MORE == 0 {
            response.cursor = request.cursor.saturating_add(written as u64);
        }
        DirectoryStatus::Ok
    }

    pub fn directory_endpoints(
        &self,
        request: DirectoryRequest,
        response: &mut DirectoryResponse,
    ) -> DirectoryStatus {
        if !request.is_valid() || request.operation != logos_abi::DirectoryOperation::Endpoints {
            return DirectoryStatus::Malformed;
        }
        *response =
            DirectoryResponse::empty(request.operation, DirectoryStatus::Ok, request.request_id);
        let mut seen = 0u64;
        let mut written = 0usize;
        for endpoint in self.endpoints.iter().filter_map(|slot| slot.value.as_ref()) {
            if seen < request.cursor {
                seen += 1;
                continue;
            }
            if written == DIRECTORY_RECORDS_PER_PAGE {
                response.flags |= DIRECTORY_FLAG_MORE;
                response.cursor = request.cursor.saturating_add(written as u64);
                break;
            }
            response.records[written] = logos_abi::DirectoryRecord {
                kind: DirectoryRecordKind::Endpoint,
                rights: 0,
                flags: 0,
                handle: endpoint.handle.raw(),
                peer: endpoint.consumer,
                contract_id: endpoint.contract_id,
                message_bytes: endpoint.message_bytes as u16,
                queue_capacity: endpoint.queue_capacity as u16,
                event: EventHandle::EMPTY,
                name_len: 0,
                reserved: [0; 1],
                name: [0; logos_abi::MAX_SERVICE_NAME_BYTES],
            };
            written += 1;
            seen += 1;
        }
        response.count = written as u8;
        if response.flags & DIRECTORY_FLAG_MORE == 0 {
            response.cursor = request.cursor.saturating_add(written as u64);
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
        self.endpoints.push(Slot::with_generation(self.generation_seed));
        Ok(self.endpoints.len() - 1)
    }

    fn allocate_capability_record(&mut self) -> Result<usize, IpcStatus> {
        if let Some((index, _)) =
            self.capabilities.iter().enumerate().find(|(_, slot)| slot.value.is_none())
        {
            return Ok(index);
        }
        self.capabilities.try_reserve(1).map_err(|_| IpcStatus::Disconnected)?;
        self.capabilities.push(Slot::with_generation(self.capability_generation));
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
        assert_eq!(registry.ownership_counts(producer), (1, 1));
        assert_eq!(registry.ownership_counts(consumer), (1, 1));
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
    fn dynamic_queue_wraps_without_allocating_on_send_or_receive() {
        let (producer, consumer) = services();
        let mut registry = RuntimeIpcRegistry::new();
        let mut events = RuntimeEventRegistry::new();
        let endpoint =
            registry.create_endpoint(producer, consumer, 1, 1, 2, 1, &mut events).unwrap();
        let send = registry.grant(producer, endpoint, IpcRights::Send).unwrap();
        let receive = registry.grant(consumer, endpoint, IpcRights::Receive).unwrap();

        assert_eq!(registry.send(producer, send, &[1], &mut events), IpcStatus::Ok);
        assert_eq!(registry.send(producer, send, &[2], &mut events), IpcStatus::Ok);
        assert_eq!(registry.send(producer, send, &[3], &mut events), IpcStatus::Full);

        let mut output = [0];
        assert_eq!(registry.receive(consumer, receive, &mut output, &mut events), IpcStatus::Ok);
        assert_eq!(output, [1]);
        assert_eq!(registry.send(producer, send, &[3], &mut events), IpcStatus::Ok);
        assert_eq!(registry.receive(consumer, receive, &mut output, &mut events), IpcStatus::Ok);
        assert_eq!(output, [2]);
        assert_eq!(registry.receive(consumer, receive, &mut output, &mut events), IpcStatus::Ok);
        assert_eq!(output, [3]);
    }

    #[test]
    fn queue_budget_is_reclaimed_when_an_endpoint_is_destroyed() {
        let (producer, consumer) = services();
        let mut registry = RuntimeIpcRegistry::new_with_generation_and_budget(1, 1);
        let mut events = RuntimeEventRegistry::new();
        let endpoint =
            registry.create_endpoint(producer, consumer, 1, 1, 1, 1, &mut events).unwrap();
        assert_eq!(
            registry.create_endpoint(producer, consumer, 1, 1, 1, 1, &mut events),
            Err(IpcStatus::Full)
        );
        registry.destroy_endpoint(endpoint, &mut events).unwrap();
        assert!(registry.create_endpoint(producer, consumer, 1, 1, 1, 2, &mut events).is_ok());
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
    fn dynamic_capabilities_do_not_alias_bootstrap_grants() {
        let (producer, consumer) = services();
        let generation = 7;
        let mut registry = RuntimeIpcRegistry::new_with_generation(generation);
        let mut events = RuntimeEventRegistry::new_with_generation(generation);
        let endpoint =
            registry.create_endpoint(producer, consumer, 1, 1, 1, 1, &mut events).unwrap();
        let capability = registry.grant(producer, endpoint, IpcRights::Send).unwrap();

        assert_eq!(capability.index(), 0);
        assert_ne!(capability, CapabilityHandle::new(0, generation).unwrap());
    }

    #[test]
    fn destroying_service_invalidates_owned_ipc_and_event_handles() {
        let (producer, consumer) = services();
        let mut registry = RuntimeIpcRegistry::new();
        let mut events = RuntimeEventRegistry::new();
        let endpoint =
            registry.create_endpoint(producer, consumer, 1, 1, 1, 1, &mut events).unwrap();
        let capability = registry.grant(producer, endpoint, IpcRights::Send).unwrap();
        let event = events.create_event(producer).unwrap();
        let set = events.create_set(producer).unwrap();
        events.add(producer, set, event).unwrap();

        registry.destroy_service(producer, &mut events);

        assert_eq!(
            registry.capability_endpoint(producer, capability, IpcRights::Send),
            Err(IpcStatus::Stale)
        );
        assert_eq!(events.signal_irq(event), Err(crate::runtime_events::EventError::Stale));
        assert_eq!(
            events.destroy_set(producer, set),
            Err(crate::runtime_events::EventError::Stale)
        );
    }

    #[test]
    fn runtime_generation_seed_rejects_handles_from_a_previous_runtime() {
        let (producer, consumer) = services();
        let mut old = RuntimeIpcRegistry::new_with_generation(1);
        let mut old_events = RuntimeEventRegistry::new_with_generation(1);
        let old_endpoint =
            old.create_endpoint(producer, consumer, 1, 1, 1, 1, &mut old_events).unwrap();
        let old_capability = old.grant(producer, old_endpoint, IpcRights::Send).unwrap();

        let mut current = RuntimeIpcRegistry::new_with_generation(2);
        let mut current_events = RuntimeEventRegistry::new_with_generation(2);
        let current_endpoint =
            current.create_endpoint(producer, consumer, 1, 1, 1, 2, &mut current_events).unwrap();
        let current_capability =
            current.grant(producer, current_endpoint, IpcRights::Send).unwrap();

        assert_ne!(old_endpoint, current_endpoint);
        assert_ne!(old_capability, current_capability);
        assert_eq!(
            current.send(producer, old_capability, &[1], &mut current_events),
            IpcStatus::Stale
        );
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
    fn capability_lookup_is_owner_and_direction_bound() {
        let (producer, consumer) = services();
        let mut registry = RuntimeIpcRegistry::new();
        let mut events = RuntimeEventRegistry::new();
        let endpoint =
            registry.create_endpoint(producer, consumer, 1, 4, 1, 1, &mut events).unwrap();
        let send = registry.grant(producer, endpoint, IpcRights::Send).unwrap();
        assert_eq!(registry.capability_for(producer, endpoint, IpcRights::Send), Ok(send));
        assert_eq!(
            registry.capability_for(consumer, endpoint, IpcRights::Send),
            Err(IpcStatus::Unauthorized)
        );
        assert_eq!(
            registry.capability_for(producer, endpoint, IpcRights::Receive),
            Err(IpcStatus::Unauthorized)
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
        assert_eq!(response.records[0].contract_id, 1);
        assert_eq!(response.records[0].event, registry.endpoint_events(endpoint).unwrap().1);
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
                let owner = ServiceHandle::new(raw_service as u32, 1).unwrap();
                for rights in [IpcRights::Send, IpcRights::Receive] {
                    let owns_endpoint = match rights {
                        IpcRights::Send => producer == owner,
                        IpcRights::Receive => consumer == owner,
                    };
                    if owns_endpoint {
                        registry.grant(owner, endpoint, rights).unwrap();
                    }
                }
            }
        }
    }
}
