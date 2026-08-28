use logos_storage::{
    BlockCompletion, BlockRequest, BlockRequestId, DmaArena, DmaError, Expired,
    MAX_VIRTIO_QUEUE_DEPTH, TransportError, TransportRequestId, VirtioBlkChain, VirtioTransport,
};

#[cfg(target_os = "uefi")]
pub(crate) mod gpu;

pub const CORE_VIRTIO_DMA_PAGES: usize = 32;

#[derive(Debug)]
pub struct CoreVirtioError {
    pub transport: TransportError,
}

impl From<TransportError> for CoreVirtioError {
    fn from(transport: TransportError) -> Self {
        Self { transport }
    }
}

/// Core-owned VirtIO block state. PCI register access and interrupt entry code
/// call this façade after validating device capabilities and DMA memory.
pub struct CoreVirtioBlock<const DEPTH: usize = MAX_VIRTIO_QUEUE_DEPTH> {
    transport: VirtioTransport<DEPTH>,
    dma: DmaArena<CORE_VIRTIO_DMA_PAGES>,
}

impl<const DEPTH: usize> CoreVirtioBlock<DEPTH> {
    pub fn new(
        device_features: u64,
        writable: bool,
        dma_base: u64,
    ) -> Result<Self, CoreVirtioError> {
        let transport = VirtioTransport::new(device_features, writable)?;
        let dma = DmaArena::new(dma_base)
            .map_err(|_| CoreVirtioError { transport: TransportError::Stale })?;
        Ok(Self { transport, dma })
    }

    pub fn submit(
        &mut self,
        request_id: BlockRequestId,
        request: BlockRequest,
        deadline: u64,
    ) -> Result<(TransportRequestId, VirtioBlkChain), CoreVirtioError> {
        Ok(self.transport.submit(request_id, request, deadline)?)
    }

    pub fn complete(
        &mut self,
        request_id: TransportRequestId,
        device_status: u8,
    ) -> Result<BlockCompletion, CoreVirtioError> {
        Ok(self.transport.complete(request_id, device_status)?)
    }

    pub fn expire(&mut self, now: u64) -> Expired<DEPTH> {
        self.transport.expire(now)
    }

    pub fn reset(&mut self) -> usize {
        self.transport.reset()
    }

    pub fn dma_pages(&self) -> usize {
        self.dma.capacity()
    }

    pub fn dma_lease(&mut self, pages: u16) -> Result<logos_storage::DmaLease, DmaError> {
        self.dma.lease(pages)
    }

    pub fn release_dma(&mut self, lease: logos_storage::DmaLease) -> Result<(), DmaError> {
        self.dma.release(lease)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use logos_storage::{
        BlockIndex, BlockRequestTable, BufferToken, VIRTIO_BLK_F_FLUSH, VIRTIO_BLK_STATUS_OK,
        VIRTIO_F_VERSION_1,
    };

    #[test]
    fn core_facade_owns_dma_and_transport_lifecycle() {
        let mut core =
            CoreVirtioBlock::<1>::new(VIRTIO_F_VERSION_1 | VIRTIO_BLK_F_FLUSH, true, 0x40_000)
                .unwrap();
        assert_eq!(core.dma_pages(), CORE_VIRTIO_DMA_PAGES);
        let lease = core.dma_lease(1).unwrap();
        let mut requests = BlockRequestTable::<1>::new();
        let request =
            BlockRequest::read(BlockIndex::new(0), 1, BufferToken::new(0x40_000).unwrap());
        let id = requests.submit(request).unwrap();
        requests.claim_next().unwrap();
        let (transport_id, _) = core.submit(id, request, 100).unwrap();
        assert_eq!(
            core.complete(transport_id, VIRTIO_BLK_STATUS_OK).unwrap().status,
            logos_storage::BlockStatus::Success
        );
        core.release_dma(lease).unwrap();
    }
}
