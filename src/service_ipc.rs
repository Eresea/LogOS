//! Fixed shared endpoint-page allocation for the terminal graph.

use core::{mem, ptr};

use logos_abi::{
    EndpointHeader, IpcCapability, IpcCapabilityPage, IpcRights, IpcStatus, Notify, ServiceId,
};

use crate::{
    frame_pool::{FrameAddress, FramePool, FramePoolError},
    page_table::PageTableMemory,
};

const ENDPOINTS: [logos_abi::IpcEndpointId; 22] = [
    logos_abi::IpcEndpointId::InputToTerminal,
    logos_abi::IpcEndpointId::TerminalToDisplay,
    logos_abi::IpcEndpointId::TerminalToSession,
    logos_abi::IpcEndpointId::SessionToTerminal,
    logos_abi::IpcEndpointId::SessionToFlow,
    logos_abi::IpcEndpointId::FlowToSession,
    logos_abi::IpcEndpointId::FlowToStorage,
    logos_abi::IpcEndpointId::StorageToFlow,
    logos_abi::IpcEndpointId::FlowToNetwork,
    logos_abi::IpcEndpointId::NetworkToFlow,
    logos_abi::IpcEndpointId::FlowToFetch,
    logos_abi::IpcEndpointId::FetchToFlow,
    logos_abi::IpcEndpointId::FetchToStorage,
    logos_abi::IpcEndpointId::StorageToFetch,
    logos_abi::IpcEndpointId::FetchToNetwork,
    logos_abi::IpcEndpointId::NetworkToFetch,
    logos_abi::IpcEndpointId::FlowToDevice,
    logos_abi::IpcEndpointId::DeviceToFlow,
    logos_abi::IpcEndpointId::FlowToUser,
    logos_abi::IpcEndpointId::UserToFlow,
    logos_abi::IpcEndpointId::UserToStorage,
    logos_abi::IpcEndpointId::StorageToUser,
];
pub const MAX_ENDPOINTS: usize = ENDPOINTS.len();
pub const SERVICE_EPOCH: u64 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IpcEndpoint {
    endpoint_id: logos_abi::IpcEndpointId,
    producer: ServiceId,
    consumer: ServiceId,
    generation: u16,
    service_epoch: u64,
    frame: FrameAddress,
}

impl IpcEndpoint {
    pub const fn id(self) -> logos_abi::IpcEndpointId {
        self.endpoint_id
    }

    pub const fn producer(self) -> ServiceId {
        self.producer
    }

    pub const fn consumer(self) -> ServiceId {
        self.consumer
    }

    pub const fn generation(self) -> u16 {
        self.generation
    }

    pub const fn header(self) -> EndpointHeader {
        EndpointHeader::new(self.generation, self.service_epoch)
    }

    pub const fn frame(self) -> FrameAddress {
        self.frame
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IpcError {
    Capacity,
    Exhausted,
    Memory,
    InvalidIdentity,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IpcOutcome {
    pub status: IpcStatus,
    pub notified: bool,
}

impl IpcOutcome {
    const fn new(status: IpcStatus, notified: bool) -> Self {
        Self { status, notified }
    }
}

pub struct ServiceIpcGraph {
    endpoints: [Option<IpcEndpoint>; MAX_ENDPOINTS],
    count: usize,
}

impl ServiceIpcGraph {
    pub fn allocate_with_identity<M: PageTableMemory>(
        pool: &mut FramePool,
        memory: &mut M,
        generation: u16,
        service_epoch: u64,
    ) -> Result<Self, IpcError> {
        if generation == 0 || service_epoch == 0 {
            return Err(IpcError::InvalidIdentity);
        }
        let mut graph = Self { endpoints: [None; MAX_ENDPOINTS], count: 0 };
        for (index, endpoint_id) in ENDPOINTS.into_iter().enumerate() {
            let producer = endpoint_id.producer();
            let consumer = endpoint_id.consumer();
            let frame = match pool.allocate() {
                Ok(frame) => frame,
                Err(FramePoolError::Exhausted) => {
                    if graph.reclaim(pool).is_err() {
                        return Err(IpcError::Memory);
                    }
                    return Err(IpcError::Exhausted);
                }
                Err(FramePoolError::InvalidMap) => {
                    if graph.reclaim(pool).is_err() {
                        return Err(IpcError::Memory);
                    }
                    return Err(IpcError::Memory);
                }
            };
            graph.endpoints[index] = Some(IpcEndpoint {
                endpoint_id,
                producer,
                consumer,
                generation,
                service_epoch,
                frame,
            });
            graph.count += 1;
            if memory.clear(frame).is_err() {
                graph.reclaim(pool).map_err(|_| IpcError::Memory)?;
                return Err(IpcError::Memory);
            }
        }
        Ok(graph)
    }

    /// Disconnect every initialized endpoint queue.
    pub fn disconnect(&self) {
        for endpoint in self.endpoints[..self.count].iter().flatten() {
            disconnect_ipc_page(*endpoint);
        }
    }

    /// Release every endpoint page owned by this graph.
    pub fn reclaim(&mut self, pool: &mut FramePool) -> Result<(), IpcError> {
        for index in 0..self.count {
            let Some(endpoint) = self.endpoints[index] else { continue };
            if pool.release(endpoint.frame).is_err() {
                return Err(IpcError::Memory);
            }
            self.endpoints[index] = None;
        }
        if self.count == 0 || self.endpoints[..self.count].iter().all(Option::is_none) {
            self.count = 0;
        }
        Ok(())
    }

    pub const fn count(&self) -> usize {
        self.count
    }

    pub const fn endpoint(&self, index: usize) -> Option<IpcEndpoint> {
        if index < self.count { self.endpoints[index] } else { None }
    }

    pub fn capabilities(&self, service: ServiceId) -> Result<IpcCapabilityPage, IpcError> {
        let mut page = IpcCapabilityPage::empty();
        for index in 0..self.count {
            let Some(endpoint) = self.endpoints[index] else { continue };
            let rights = if endpoint.producer() == service {
                Some(IpcRights::Send)
            } else if endpoint.consumer() == service {
                Some(IpcRights::Receive)
            } else {
                None
            };
            let Some(rights) = rights else { continue };
            let Some(slot) = logos_abi::ipc_capability_slot(service, endpoint.id(), rights) else {
                return Err(IpcError::InvalidIdentity);
            };
            if slot >= page.capabilities.len() || !page.capabilities[slot].is_empty() {
                return Err(IpcError::Capacity);
            }
            let endpoint_id = endpoint.id();
            page.capabilities[slot] = logos_abi::IpcCapability::new(
                endpoint_id.index(),
                rights,
                endpoint.generation(),
                endpoint.header().service_epoch,
            )
            .ok_or(IpcError::InvalidIdentity)?;
        }
        Ok(page)
    }

    pub fn send(&self, service: ServiceId, capability: IpcCapability, bytes: &[u8]) -> IpcOutcome {
        let index = match self.authorized_endpoint(service, capability, IpcRights::Send) {
            Ok(index) => index,
            Err(status) => return IpcOutcome::new(status, false),
        };
        if bytes.len() != Self::message_size(self.endpoint(index).unwrap().id().index()) {
            return IpcOutcome::new(IpcStatus::Malformed, false);
        }
        let Some(frame) = self.endpoint(index).map(|endpoint| endpoint.frame().raw() as usize)
        else {
            return IpcOutcome::new(IpcStatus::Unauthorized, false);
        };
        let identity = capability_identity(capability);
        let endpoint_id = self.endpoint(index).unwrap().id().index();
        let result = unsafe { send_bytes(frame, endpoint_id, identity, bytes) };
        match result {
            Ok(notify) => IpcOutcome::new(IpcStatus::Ok, notify == Notify::Notified),
            Err(IpcStatus::Full) => IpcOutcome::new(IpcStatus::Full, false),
            Err(status) => IpcOutcome::new(status, false),
        }
    }

    pub fn receive(
        &self,
        service: ServiceId,
        capability: IpcCapability,
        bytes: &mut [u8],
    ) -> IpcOutcome {
        let index = match self.authorized_endpoint(service, capability, IpcRights::Receive) {
            Ok(index) => index,
            Err(status) => return IpcOutcome::new(status, false),
        };
        if bytes.len() != Self::message_size(self.endpoint(index).unwrap().id().index()) {
            return IpcOutcome::new(IpcStatus::Malformed, false);
        }
        let Some(frame) = self.endpoint(index).map(|endpoint| endpoint.frame().raw() as usize)
        else {
            return IpcOutcome::new(IpcStatus::Unauthorized, false);
        };
        let identity = capability_identity(capability);
        let endpoint_id = self.endpoint(index).unwrap().id().index();
        let result = unsafe { receive_bytes(frame, endpoint_id, identity, bytes) };
        match result {
            Ok(notify) => IpcOutcome::new(IpcStatus::Ok, notify == Notify::Notified),
            Err(IpcStatus::Empty) => IpcOutcome::new(IpcStatus::Empty, false),
            Err(status) => IpcOutcome::new(status, false),
        }
    }

    fn authorized_endpoint(
        &self,
        service: ServiceId,
        capability: IpcCapability,
        rights: IpcRights,
    ) -> Result<usize, IpcStatus> {
        let Some(index) = capability.endpoint_index() else {
            return Err(IpcStatus::Unauthorized);
        };
        let Some((graph_index, endpoint)) =
            self.endpoints[..self.count].iter().enumerate().find_map(|(graph_index, endpoint)| {
                endpoint
                    .filter(|endpoint| endpoint.id().index() == index)
                    .map(|endpoint| (graph_index, endpoint))
            })
        else {
            return Err(IpcStatus::Unauthorized);
        };
        let owns_endpoint = match rights {
            IpcRights::Send => endpoint.producer() == service,
            IpcRights::Receive => endpoint.consumer() == service,
        };
        if !owns_endpoint || !capability.rights.allows(rights) {
            return Err(IpcStatus::Unauthorized);
        }
        if capability.generation != endpoint.generation()
            || capability.service_epoch != endpoint.header().service_epoch
        {
            return Err(IpcStatus::Stale);
        }
        Ok(graph_index)
    }

    pub fn message_size(index: usize) -> usize {
        logos_abi::ipc_message_size(index).unwrap_or(0)
    }
}

fn capability_identity(capability: IpcCapability) -> logos_abi::MessageIdentity {
    logos_abi::MessageIdentity::new(capability.generation, capability.service_epoch)
}

unsafe fn send_bytes(
    frame: usize,
    index: usize,
    identity: logos_abi::MessageIdentity,
    bytes: &[u8],
) -> Result<Notify, IpcStatus> {
    if index == logos_abi::IpcEndpointId::FlowToDevice as usize {
        return unsafe { send_typed::<logos_abi::DeviceRequest, 8>(frame, identity, bytes) };
    }
    if index == logos_abi::IpcEndpointId::DeviceToFlow as usize {
        return unsafe { send_typed::<logos_abi::DeviceResponse, 8>(frame, identity, bytes) };
    }
    match logos_abi::ipc_message_type(index) {
        Some(logos_abi::IpcMessageType::Input) => unsafe {
            send_typed::<logos_abi::InputMessage, 32>(frame, identity, bytes)
        },
        Some(logos_abi::IpcMessageType::Render) => unsafe {
            send_typed::<logos_abi::RenderMessage, 1>(frame, identity, bytes)
        },
        Some(logos_abi::IpcMessageType::Bytes) => unsafe {
            send_typed::<logos_abi::IpcBytes, 8>(frame, identity, bytes)
        },
        Some(logos_abi::IpcMessageType::Packet) => Err(IpcStatus::Unauthorized),
        _ => Err(IpcStatus::Unauthorized),
    }
}

unsafe fn receive_bytes(
    frame: usize,
    index: usize,
    identity: logos_abi::MessageIdentity,
    bytes: &mut [u8],
) -> Result<Notify, IpcStatus> {
    if index == logos_abi::IpcEndpointId::FlowToDevice as usize {
        return unsafe { receive_typed::<logos_abi::DeviceRequest, 8>(frame, identity, bytes) };
    }
    if index == logos_abi::IpcEndpointId::DeviceToFlow as usize {
        return unsafe { receive_typed::<logos_abi::DeviceResponse, 8>(frame, identity, bytes) };
    }
    match logos_abi::ipc_message_type(index) {
        Some(logos_abi::IpcMessageType::Input) => unsafe {
            receive_typed::<logos_abi::InputMessage, 32>(frame, identity, bytes)
        },
        Some(logos_abi::IpcMessageType::Render) => unsafe {
            receive_typed::<logos_abi::RenderMessage, 1>(frame, identity, bytes)
        },
        Some(logos_abi::IpcMessageType::Bytes) => unsafe {
            receive_typed::<logos_abi::IpcBytes, 8>(frame, identity, bytes)
        },
        Some(logos_abi::IpcMessageType::Packet) => Err(IpcStatus::Unauthorized),
        _ => Err(IpcStatus::Unauthorized),
    }
}

unsafe fn send_typed<T: Copy, const N: usize>(
    frame: usize,
    identity: logos_abi::MessageIdentity,
    bytes: &[u8],
) -> Result<Notify, IpcStatus> {
    if bytes.len() != mem::size_of::<T>() {
        return Err(IpcStatus::Malformed);
    }
    let entry = unsafe { ptr::read_unaligned(bytes.as_ptr() as *const T) };
    let ring = unsafe { &*(frame as *const logos_abi::SharedIpc<T, N>) };
    ring.send(identity, entry).map_err(|error| match error {
        logos_abi::SharedSendError::Full => IpcStatus::Full,
        logos_abi::SharedSendError::Stale => IpcStatus::Stale,
        logos_abi::SharedSendError::Disconnected => IpcStatus::Disconnected,
    })
}

unsafe fn receive_typed<T: Copy, const N: usize>(
    frame: usize,
    identity: logos_abi::MessageIdentity,
    bytes: &mut [u8],
) -> Result<Notify, IpcStatus> {
    if bytes.len() != mem::size_of::<T>() {
        return Err(IpcStatus::Malformed);
    }
    let ring = unsafe { &*(frame as *const logos_abi::SharedIpc<T, N>) };
    let (entry, notify) = ring.receive_with_notify(identity).map_err(|error| match error {
        logos_abi::SharedReceiveError::Empty => IpcStatus::Empty,
        logos_abi::SharedReceiveError::Stale => IpcStatus::Stale,
        logos_abi::SharedReceiveError::Disconnected => IpcStatus::Disconnected,
    })?;
    unsafe { ptr::write_unaligned(bytes.as_mut_ptr() as *mut T, entry) };
    Ok(notify)
}

fn disconnect_ipc_page(endpoint: IpcEndpoint) {
    let frame = endpoint.frame.raw() as usize;
    // All service tasks are quiesced before this is called, so replacing the
    // page object cannot race with an old producer or consumer.
    unsafe {
        match endpoint.id() {
            logos_abi::IpcEndpointId::InputToTerminal => {
                (*(frame as *const logos_abi::InputIpc)).disconnect()
            }
            logos_abi::IpcEndpointId::TerminalToDisplay => {
                (*(frame as *const logos_abi::RenderIpc)).disconnect()
            }
            logos_abi::IpcEndpointId::TerminalToSession
            | logos_abi::IpcEndpointId::SessionToTerminal
            | logos_abi::IpcEndpointId::SessionToFlow
            | logos_abi::IpcEndpointId::FlowToSession => {
                (*(frame as *const logos_abi::StreamIpc)).disconnect()
            }
            logos_abi::IpcEndpointId::FlowToStorage
            | logos_abi::IpcEndpointId::StorageToFlow
            | logos_abi::IpcEndpointId::FlowToNetwork
            | logos_abi::IpcEndpointId::NetworkToFlow
            | logos_abi::IpcEndpointId::FlowToFetch
            | logos_abi::IpcEndpointId::FetchToFlow
            | logos_abi::IpcEndpointId::FetchToStorage
            | logos_abi::IpcEndpointId::StorageToFetch
            | logos_abi::IpcEndpointId::FetchToNetwork
            | logos_abi::IpcEndpointId::NetworkToFetch => {
                (*(frame as *const logos_abi::StreamIpc)).disconnect()
            }
            logos_abi::IpcEndpointId::FlowToDevice => {
                (*(frame as *const logos_abi::SharedIpc<logos_abi::DeviceRequest, 8>)).disconnect()
            }
            logos_abi::IpcEndpointId::FlowToUser
            | logos_abi::IpcEndpointId::UserToFlow
            | logos_abi::IpcEndpointId::UserToStorage
            | logos_abi::IpcEndpointId::StorageToUser => {
                (*(frame as *const logos_abi::StreamIpc)).disconnect()
            }
            logos_abi::IpcEndpointId::DeviceToFlow => {
                (*(frame as *const logos_abi::SharedIpc<logos_abi::DeviceResponse, 8>)).disconnect()
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::page_table::PageTableError;
    use crate::{boot_resources::MemoryDescriptor, frame_pool::FramePool};

    struct Memory;
    impl PageTableMemory for Memory {
        fn clear(&mut self, _frame: FrameAddress) -> Result<(), PageTableError> {
            Ok(())
        }
        fn read(&self, _frame: FrameAddress, _index: usize) -> Result<u64, PageTableError> {
            Ok(0)
        }
        fn write(
            &mut self,
            _frame: FrameAddress,
            _index: usize,
            _value: u64,
        ) -> Result<(), PageTableError> {
            Ok(())
        }
    }

    #[test]
    fn graph_allocates_fixed_generation_stamped_endpoint_pages() {
        let mut map = crate::boot_resources::MemoryMap::new();
        map.push(MemoryDescriptor::new(0x1000, 24, true).unwrap()).unwrap();
        let mut pool = FramePool::empty();
        pool.initialize(&map).unwrap();
        let mut memory = Memory;
        let graph =
            ServiceIpcGraph::allocate_with_identity(&mut pool, &mut memory, 1, SERVICE_EPOCH)
                .unwrap();
        assert_eq!(graph.count(), 22);
        assert_eq!(graph.endpoint(0).unwrap().producer(), ServiceId::Input);
        assert_eq!(graph.endpoint(0).unwrap().consumer(), ServiceId::Terminal);
        assert_eq!(graph.endpoint(5).unwrap().producer(), ServiceId::Flow);
        assert_eq!(graph.endpoint(5).unwrap().consumer(), ServiceId::Session);
        assert_eq!(graph.endpoint(6).unwrap().id(), logos_abi::IpcEndpointId::FlowToStorage);
        assert_eq!(graph.endpoint(7).unwrap().id(), logos_abi::IpcEndpointId::StorageToFlow);
        assert_eq!(graph.endpoint(8).unwrap().id(), logos_abi::IpcEndpointId::FlowToNetwork);
        assert_eq!(graph.endpoint(9).unwrap().id(), logos_abi::IpcEndpointId::NetworkToFlow);
        assert_eq!(graph.endpoint(10).unwrap().id(), logos_abi::IpcEndpointId::FlowToFetch);
        assert_eq!(graph.endpoint(15).unwrap().id(), logos_abi::IpcEndpointId::NetworkToFetch);
        assert_eq!(graph.endpoint(16).unwrap().id(), logos_abi::IpcEndpointId::FlowToDevice);
        assert_eq!(graph.endpoint(17).unwrap().id(), logos_abi::IpcEndpointId::DeviceToFlow);
        assert_eq!(graph.endpoint(18).unwrap().id(), logos_abi::IpcEndpointId::FlowToUser);
        assert_eq!(graph.endpoint(19).unwrap().id(), logos_abi::IpcEndpointId::UserToFlow);
        assert_eq!(graph.endpoint(20).unwrap().id(), logos_abi::IpcEndpointId::UserToStorage);
        assert_eq!(graph.endpoint(21).unwrap().id(), logos_abi::IpcEndpointId::StorageToUser);
        assert_eq!(graph.endpoint(5).unwrap().generation(), 1);
        let terminal = graph.capabilities(ServiceId::Terminal).unwrap();
        assert_eq!(terminal.get(0).unwrap().rights, IpcRights::Receive);
        assert_eq!(terminal.get(1).unwrap().rights, IpcRights::Send);
        assert_eq!(terminal.get(3).unwrap().rights, IpcRights::Receive);
        assert_eq!(terminal.get(4), None);
        let commands = graph.capabilities(ServiceId::Flow).unwrap();
        assert_eq!(commands.get(2).unwrap().endpoint_index(), Some(8));
        assert_eq!(commands.get(3).unwrap().endpoint_index(), Some(9));
        assert_eq!(commands.get(4).unwrap().endpoint_index(), Some(12));
        assert_eq!(commands.get(5).unwrap().endpoint_index(), Some(13));
        let storage = graph.capabilities(ServiceId::Storage).unwrap();
        assert_eq!(storage.get(0).unwrap().endpoint_index(), Some(8));
        assert_eq!(storage.get(1).unwrap().endpoint_index(), Some(9));
        assert_eq!(storage.get(2), None);
        let network = graph.capabilities(ServiceId::Network).unwrap();
        assert_eq!(network.get(2).unwrap().endpoint_index(), Some(12));
        assert_eq!(network.get(3).unwrap().endpoint_index(), Some(13));
        let device = graph.capabilities(ServiceId::Device).unwrap();
        assert_eq!(device.get(2).unwrap().endpoint_index(), Some(24));
        assert_eq!(device.get(3).unwrap().endpoint_index(), Some(25));
    }

    #[test]
    fn graph_rejects_wrong_direction_stale_and_malformed_operations() {
        let mut map = crate::boot_resources::MemoryMap::new();
        map.push(MemoryDescriptor::new(0x1000, 24, true).unwrap()).unwrap();
        let mut pool = FramePool::empty();
        pool.initialize(&map).unwrap();
        let mut memory = Memory;
        let graph = ServiceIpcGraph::allocate_with_identity(&mut pool, &mut memory, 2, 9).unwrap();
        let input_send = graph.capabilities(ServiceId::Input).unwrap().get(0).unwrap();
        assert_eq!(
            graph.send(ServiceId::Terminal, input_send, &[]).status,
            logos_abi::IpcStatus::Unauthorized
        );
        let stale = logos_abi::IpcCapability::new(0, IpcRights::Send, 1, 9).unwrap();
        assert_eq!(graph.send(ServiceId::Input, stale, &[]).status, logos_abi::IpcStatus::Stale);
        assert_eq!(
            graph.send(ServiceId::Input, input_send, &[]).status,
            logos_abi::IpcStatus::Malformed
        );
        let wrong_rights = IpcCapability::new(0, IpcRights::Receive, 2, 9).unwrap();
        assert_eq!(
            graph.send(ServiceId::Input, wrong_rights, &[]).status,
            logos_abi::IpcStatus::Unauthorized
        );
        let forged_owner = IpcCapability::new(0, IpcRights::Send, 2, 9).unwrap();
        assert_eq!(
            graph.send(ServiceId::Terminal, forged_owner, &[]).status,
            logos_abi::IpcStatus::Unauthorized
        );
        let oversized = [0; logos_abi::IPC_PAGE_BYTES];
        assert_eq!(
            graph.send(ServiceId::Input, input_send, &oversized).status,
            logos_abi::IpcStatus::Malformed
        );
    }

    #[test]
    fn graph_rejects_invalid_identity_before_allocating() {
        let mut map = crate::boot_resources::MemoryMap::new();
        map.push(MemoryDescriptor::new(0x1000, 24, true).unwrap()).unwrap();
        let mut pool = FramePool::empty();
        pool.initialize(&map).unwrap();
        let mut memory = Memory;
        assert!(matches!(
            ServiceIpcGraph::allocate_with_identity(&mut pool, &mut memory, 0, 1),
            Err(IpcError::InvalidIdentity)
        ));
        assert!(matches!(
            ServiceIpcGraph::allocate_with_identity(&mut pool, &mut memory, 1, 0),
            Err(IpcError::InvalidIdentity)
        ));
    }

    #[test]
    fn graph_reclaims_partial_allocation_on_exhaustion() {
        let mut map = crate::boot_resources::MemoryMap::new();
        map.push(MemoryDescriptor::new(0x1000, 5, true).unwrap()).unwrap();
        let mut pool = FramePool::empty();
        pool.initialize(&map).unwrap();
        let mut memory = Memory;
        let available = pool.available();

        assert!(matches!(
            ServiceIpcGraph::allocate_with_identity(&mut pool, &mut memory, 1, 1),
            Err(IpcError::Exhausted)
        ));
        assert_eq!(pool.available(), available);
    }
}
