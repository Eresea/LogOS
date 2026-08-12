//! Fixed shared endpoint-page allocation for the terminal graph.

use logos_abi::{EndpointHeader, SERVICE_IPC_BASE, ServiceId};

use crate::{
    frame_pool::{FrameAddress, FramePool, FramePoolError},
    page_table::PageTableMemory,
};

pub const MAX_ENDPOINTS: usize = 5;
pub const IPC_BASE: usize = SERVICE_IPC_BASE;
pub const SERVICE_EPOCH: u64 = 1;
const PAGE_SIZE: usize = 4096;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IpcEndpoint {
    producer: ServiceId,
    consumer: ServiceId,
    generation: u16,
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
        EndpointHeader::new(self.generation, SERVICE_EPOCH)
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
}

pub struct ServiceIpcGraph {
    endpoints: [Option<IpcEndpoint>; MAX_ENDPOINTS],
    count: usize,
}

impl ServiceIpcGraph {
    pub fn allocate<M: PageTableMemory>(
        pool: &mut FramePool,
        memory: &mut M,
    ) -> Result<Self, IpcError> {
        let pairs = [
            (ServiceId::Input, ServiceId::Terminal),
            (ServiceId::Terminal, ServiceId::Display),
            (ServiceId::Terminal, ServiceId::Session),
            (ServiceId::Session, ServiceId::Terminal),
            (ServiceId::Session, ServiceId::Commands),
        ];
        let mut graph = Self { endpoints: [None; MAX_ENDPOINTS], count: 0 };
        for (index, (producer, consumer)) in pairs.into_iter().enumerate() {
            if index == MAX_ENDPOINTS {
                return Err(IpcError::Capacity);
            }
            let frame = pool.allocate().map_err(|error| match error {
                FramePoolError::Exhausted => IpcError::Exhausted,
                FramePoolError::InvalidMap => IpcError::Memory,
            })?;
            if memory.clear(frame).is_err() {
                let _ = pool.release(frame);
                return Err(IpcError::Memory);
            }
            graph.endpoints[index] = Some(IpcEndpoint {
                producer,
                consumer,
                generation: 1,
                virtual_address: IPC_BASE + index * PAGE_SIZE,
                frame,
            });
            graph.count += 1;
        }
        Ok(graph)
    }

    pub const fn count(&self) -> usize {
        self.count
    }

    pub const fn endpoint(&self, index: usize) -> Option<IpcEndpoint> {
        if index < self.count { self.endpoints[index] } else { None }
    }

    pub fn for_service(&self, service: ServiceId, mut visit: impl FnMut(IpcEndpoint)) {
        for endpoint in self.endpoints[..self.count].iter().flatten() {
            if endpoint.producer == service || endpoint.consumer == service {
                visit(*endpoint);
            }
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
        let graph = ServiceIpcGraph::allocate(&mut pool, &mut memory).unwrap();
        assert_eq!(graph.count(), 5);
        assert_eq!(graph.endpoint(0).unwrap().producer(), ServiceId::Input);
        assert_eq!(graph.endpoint(0).unwrap().consumer(), ServiceId::Terminal);
        assert_eq!(graph.endpoint(4).unwrap().virtual_address(), IPC_BASE + 4 * PAGE_SIZE);
        assert_eq!(graph.endpoint(4).unwrap().generation(), 1);
    }
}
