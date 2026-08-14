//! Fixed shared endpoint-page allocation for the terminal graph.

use core::{mem, ptr};

use logos_abi::{
    EndpointHeader, IpcCapability, IpcCapabilityPage, IpcRights, IpcStatus, Notify,
    SERVICE_IPC_BASE, ServiceId,
};

use crate::{
    frame_pool::{FrameAddress, FramePool, FramePoolError},
    page_table::PageTableMemory,
};

const ENDPOINTS: [(ServiceId, ServiceId); 6] = [
    (ServiceId::Input, ServiceId::Terminal),
    (ServiceId::Terminal, ServiceId::Display),
    (ServiceId::Terminal, ServiceId::Session),
    (ServiceId::Session, ServiceId::Terminal),
    (ServiceId::Session, ServiceId::Commands),
    (ServiceId::Commands, ServiceId::Session),
];
pub const MAX_ENDPOINTS: usize = ENDPOINTS.len();
pub const IPC_BASE: usize = SERVICE_IPC_BASE;
pub const SERVICE_EPOCH: u64 = 1;
const PAGE_SIZE: usize = 4096;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IpcEndpoint {
    producer: ServiceId,
    consumer: ServiceId,
    generation: u16,
    service_epoch: u64,
    virtual_address: usize,
    frame: FrameAddress,
}

impl IpcEndpoint {
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

    pub const fn virtual_address(self) -> usize {
        self.virtual_address
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
        for (index, (producer, consumer)) in ENDPOINTS.into_iter().enumerate() {
            let frame = pool.allocate().map_err(|error| match error {
                FramePoolError::Exhausted => IpcError::Exhausted,
                FramePoolError::InvalidMap => IpcError::Memory,
            })?;
            if memory.clear(frame).is_err() {
                let _ = pool.release(frame);
                graph.reclaim(pool);
                return Err(IpcError::Memory);
            }
            graph.endpoints[index] = Some(IpcEndpoint {
                producer,
                consumer,
                generation,
                service_epoch,
                virtual_address: IPC_BASE + index * PAGE_SIZE,
                frame,
            });
            graph.count += 1;
        }
        Ok(graph)
    }

    /// Disconnect and release every endpoint page owned by this graph.
    pub fn reclaim(&mut self, pool: &mut FramePool) {
        for (index, endpoint) in self.endpoints[..self.count].iter().flatten().enumerate() {
            disconnect_ipc_page(*endpoint, index);
            let _ = pool.release(endpoint.frame);
        }
        self.endpoints.fill(None);
        self.count = 0;
    }

    pub const fn count(&self) -> usize {
        self.count
    }

    pub const fn endpoint(&self, index: usize) -> Option<IpcEndpoint> {
        if index < self.count { self.endpoints[index] } else { None }
    }

    pub fn capabilities(&self, service: ServiceId) -> Result<IpcCapabilityPage, IpcError> {
        let mut page = IpcCapabilityPage::empty();
        let mut slot = 0;
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
            if slot == page.capabilities.len() {
                return Err(IpcError::Capacity);
            }
            page.capabilities[slot] = logos_abi::IpcCapability::new(
                index,
                rights,
                endpoint.generation(),
                endpoint.header().service_epoch,
            )
            .ok_or(IpcError::InvalidIdentity)?;
            slot += 1;
        }
        Ok(page)
    }

    pub fn send(&self, service: ServiceId, capability: IpcCapability, bytes: &[u8]) -> IpcOutcome {
        let index = match self.authorized_endpoint(service, capability, IpcRights::Send) {
            Ok(index) => index,
            Err(status) => return IpcOutcome::new(status, false),
        };
        if bytes.len() != Self::message_size(index) {
            return IpcOutcome::new(IpcStatus::Malformed, false);
        }
        let Some(frame) = self.endpoint(index).map(|endpoint| endpoint.frame().raw() as usize)
        else {
            return IpcOutcome::new(IpcStatus::Unauthorized, false);
        };
        let identity = capability_identity(capability);
        let result = unsafe { send_bytes(frame, index, identity, bytes) };
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
        if bytes.len() != Self::message_size(index) {
            return IpcOutcome::new(IpcStatus::Malformed, false);
        }
        let Some(frame) = self.endpoint(index).map(|endpoint| endpoint.frame().raw() as usize)
        else {
            return IpcOutcome::new(IpcStatus::Unauthorized, false);
        };
        let identity = capability_identity(capability);
        let result = unsafe { receive_bytes(frame, index, identity, bytes) };
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
        let Some(endpoint) = self.endpoint(index) else {
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
        Ok(index)
    }

    pub fn message_size(index: usize) -> usize {
        endpoint_message_size(index)
    }
}

fn capability_identity(capability: IpcCapability) -> logos_abi::MessageIdentity {
    logos_abi::MessageIdentity::new(capability.generation, capability.service_epoch)
}

fn endpoint_message_size(index: usize) -> usize {
    match index {
        0 => mem::size_of::<logos_abi::InputMessage>(),
        1 => mem::size_of::<logos_abi::RenderMessage>(),
        2..=5 => mem::size_of::<logos_abi::IpcBytes>(),
        _ => 0,
    }
}

unsafe fn send_bytes(
    frame: usize,
    index: usize,
    identity: logos_abi::MessageIdentity,
    bytes: &[u8],
) -> Result<Notify, IpcStatus> {
    match index {
        0 => unsafe { send_typed::<logos_abi::InputMessage, 32>(frame, identity, bytes) },
        1 => unsafe { send_typed::<logos_abi::RenderMessage, 1>(frame, identity, bytes) },
        2..=5 => unsafe { send_typed::<logos_abi::IpcBytes, 8>(frame, identity, bytes) },
        _ => Err(IpcStatus::Unauthorized),
    }
}

unsafe fn receive_bytes(
    frame: usize,
    index: usize,
    identity: logos_abi::MessageIdentity,
    bytes: &mut [u8],
) -> Result<Notify, IpcStatus> {
    match index {
        0 => unsafe { receive_typed::<logos_abi::InputMessage, 32>(frame, identity, bytes) },
        1 => unsafe { receive_typed::<logos_abi::RenderMessage, 1>(frame, identity, bytes) },
        2..=5 => unsafe { receive_typed::<logos_abi::IpcBytes, 8>(frame, identity, bytes) },
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

fn disconnect_ipc_page(endpoint: IpcEndpoint, index: usize) {
    let frame = endpoint.frame.raw() as usize;
    // All service tasks are quiesced before this is called, so replacing the
    // page object cannot race with an old producer or consumer.
    unsafe {
        match index {
            0 => (*(frame as *const logos_abi::InputIpc)).disconnect(),
            1 => (*(frame as *const logos_abi::RenderIpc)).disconnect(),
            2..=5 => (*(frame as *const logos_abi::StreamIpc)).disconnect(),
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
        map.push(MemoryDescriptor::new(0x1000, 8, true).unwrap()).unwrap();
        let mut pool = FramePool::empty();
        pool.initialize(&map).unwrap();
        let mut memory = Memory;
        let graph =
            ServiceIpcGraph::allocate_with_identity(&mut pool, &mut memory, 1, SERVICE_EPOCH)
                .unwrap();
        assert_eq!(graph.count(), 6);
        assert_eq!(graph.endpoint(0).unwrap().producer(), ServiceId::Input);
        assert_eq!(graph.endpoint(0).unwrap().consumer(), ServiceId::Terminal);
        assert_eq!(graph.endpoint(5).unwrap().producer(), ServiceId::Commands);
        assert_eq!(graph.endpoint(5).unwrap().consumer(), ServiceId::Session);
        assert_eq!(graph.endpoint(5).unwrap().virtual_address(), IPC_BASE + 5 * PAGE_SIZE);
        assert_eq!(graph.endpoint(5).unwrap().generation(), 1);
        let terminal = graph.capabilities(ServiceId::Terminal).unwrap();
        assert_eq!(terminal.get(0).unwrap().rights, IpcRights::Receive);
        assert_eq!(terminal.get(1).unwrap().rights, IpcRights::Send);
        assert_eq!(terminal.get(3).unwrap().rights, IpcRights::Receive);
        assert_eq!(terminal.get(4), None);
    }

    #[test]
    fn graph_rejects_wrong_direction_stale_and_malformed_operations() {
        let mut map = crate::boot_resources::MemoryMap::new();
        map.push(MemoryDescriptor::new(0x1000, 8, true).unwrap()).unwrap();
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
        map.push(MemoryDescriptor::new(0x1000, 8, true).unwrap()).unwrap();
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
}
