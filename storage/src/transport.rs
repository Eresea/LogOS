use crate::BufferToken;
use crate::virtio::{MAX_VIRTIO_QUEUE_DEPTH, VirtioBlkChain, VirtioBlkQueue, VirtioQueueError};
use crate::{BlockCompletion, BlockRequest, BlockRequestId};

pub const VIRTIO_F_VERSION_1: u64 = 1 << 32;
pub const VIRTIO_BLK_F_FLUSH: u64 = 1 << 9;
pub const DMA_PAGE_BYTES: u64 = 4096;
pub const DEFAULT_REQUEST_TIMEOUT: u64 = 1_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NegotiatedFeatures(u64);

impl NegotiatedFeatures {
    pub const fn contains(self, feature: u64) -> bool {
        self.0 & feature == feature
    }

    pub const fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FeatureError {
    VersionRequired,
    FlushRequired,
}

pub const fn negotiate_features(
    device_features: u64,
    writable: bool,
) -> Result<NegotiatedFeatures, FeatureError> {
    if device_features & VIRTIO_F_VERSION_1 == 0 {
        return Err(FeatureError::VersionRequired);
    }
    if writable && device_features & VIRTIO_BLK_F_FLUSH == 0 {
        return Err(FeatureError::FlushRequired);
    }
    Ok(NegotiatedFeatures(device_features & (VIRTIO_F_VERSION_1 | VIRTIO_BLK_F_FLUSH)))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DmaAddress(u64);

impl DmaAddress {
    pub const fn new(value: u64) -> Option<Self> {
        if value == 0 || value % DMA_PAGE_BYTES != 0 { None } else { Some(Self(value)) }
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DmaLease {
    address: DmaAddress,
    pages: u16,
}

impl DmaLease {
    pub const fn address(self) -> DmaAddress {
        self.address
    }

    pub const fn pages(self) -> u16 {
        self.pages
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DmaError {
    InvalidBase,
    InvalidLength,
    Exhausted,
    Stale,
}

/// Fixed physical-page ownership model used by the Core adapter.
pub struct DmaArena<const PAGES: usize> {
    base: DmaAddress,
    used: [bool; PAGES],
}

impl<const PAGES: usize> DmaArena<PAGES> {
    pub fn new(base: u64) -> Result<Self, DmaError> {
        let Some(base) = DmaAddress::new(base) else {
            return Err(DmaError::InvalidBase);
        };
        Ok(Self { base, used: [false; PAGES] })
    }

    pub const fn capacity(&self) -> usize {
        PAGES
    }

    pub fn lease(&mut self, pages: u16) -> Result<DmaLease, DmaError> {
        let pages = pages as usize;
        if pages == 0 || pages > PAGES {
            return Err(DmaError::InvalidLength);
        }
        let Some(start) =
            self.used.windows(pages).position(|window| window.iter().all(|used| !*used))
        else {
            return Err(DmaError::Exhausted);
        };
        self.used[start..start + pages].fill(true);
        let offset = (start as u64).checked_mul(DMA_PAGE_BYTES).ok_or(DmaError::InvalidLength)?;
        let address =
            DmaAddress::new(self.base.0.checked_add(offset).ok_or(DmaError::InvalidLength)?)
                .ok_or(DmaError::InvalidLength)?;
        Ok(DmaLease { address, pages: pages as u16 })
    }

    pub fn release(&mut self, lease: DmaLease) -> Result<(), DmaError> {
        if lease.pages == 0 || lease.address.0 < self.base.0 {
            return Err(DmaError::Stale);
        }
        let offset = lease.address.0 - self.base.0;
        if offset % DMA_PAGE_BYTES != 0 {
            return Err(DmaError::Stale);
        }
        let start = usize::try_from(offset / DMA_PAGE_BYTES).map_err(|_| DmaError::Stale)?;
        let end = start.checked_add(lease.pages as usize).ok_or(DmaError::Stale)?;
        if end > PAGES || !self.used[start..end].iter().all(|used| *used) {
            return Err(DmaError::Stale);
        }
        self.used[start..end].fill(false);
        Ok(())
    }

    pub fn buffer_token(&self, lease: DmaLease) -> Result<BufferToken, DmaError> {
        if lease.pages == 0 || lease.address.0 < self.base.0 {
            return Err(DmaError::Stale);
        }
        let offset = lease.address.0 - self.base.0;
        let start = usize::try_from(offset / DMA_PAGE_BYTES).map_err(|_| DmaError::Stale)?;
        let end = start.checked_add(lease.pages as usize).ok_or(DmaError::Stale)?;
        if offset % DMA_PAGE_BYTES != 0
            || end > PAGES
            || !self.used[start..end].iter().all(|used| *used)
        {
            return Err(DmaError::Stale);
        }
        BufferToken::new(lease.address.0).ok_or(DmaError::InvalidLength)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransportRequestId {
    request: BlockRequestId,
    epoch: u64,
}

impl TransportRequestId {
    pub const fn request(self) -> BlockRequestId {
        self.request
    }

    pub const fn epoch(self) -> u64 {
        self.epoch
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Pending {
    id: TransportRequestId,
    deadline: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransportError {
    Feature(FeatureError),
    Queue(VirtioQueueError),
    Full,
    Stale,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Expired<const DEPTH: usize> {
    ids: [Option<TransportRequestId>; DEPTH],
    count: usize,
}

impl<const DEPTH: usize> Expired<DEPTH> {
    pub const fn empty() -> Self {
        Self { ids: [None; DEPTH], count: 0 }
    }

    pub const fn len(self) -> usize {
        self.count
    }

    pub const fn is_empty(self) -> bool {
        self.count == 0
    }

    pub const fn get(self, index: usize) -> Option<TransportRequestId> {
        if index >= self.count { None } else { self.ids[index] }
    }
}

/// Host-tested Core transport state. Hardware code supplies the PCI/MMIO
/// register operations; this type owns feature, generation, timeout, and
/// queue state transitions without an allocator.
pub struct VirtioTransport<const DEPTH: usize = MAX_VIRTIO_QUEUE_DEPTH> {
    features: NegotiatedFeatures,
    queue: VirtioBlkQueue<DEPTH>,
    pending: [Option<Pending>; DEPTH],
    epoch: u64,
}

impl<const DEPTH: usize> VirtioTransport<DEPTH> {
    pub fn new(device_features: u64, writable: bool) -> Result<Self, TransportError> {
        Ok(Self {
            features: negotiate_features(device_features, writable)
                .map_err(TransportError::Feature)?,
            queue: VirtioBlkQueue::new(),
            pending: [None; DEPTH],
            epoch: 1,
        })
    }

    pub const fn features(&self) -> NegotiatedFeatures {
        self.features
    }

    pub const fn epoch(&self) -> u64 {
        self.epoch
    }

    pub fn submit(
        &mut self,
        request_id: BlockRequestId,
        request: BlockRequest,
        deadline: u64,
    ) -> Result<(TransportRequestId, VirtioBlkChain), TransportError> {
        let Some(slot) = self.pending.iter().position(Option::is_none) else {
            return Err(TransportError::Full);
        };
        let chain = self.queue.submit(request_id, request).map_err(TransportError::Queue)?;
        let id = TransportRequestId { request: request_id, epoch: self.epoch };
        self.pending[slot] = Some(Pending { id, deadline });
        Ok((id, chain))
    }

    pub fn complete(
        &mut self,
        id: TransportRequestId,
        device_status: u8,
    ) -> Result<BlockCompletion, TransportError> {
        let Some(slot) =
            self.pending.iter().position(|pending| pending.is_some_and(|p| p.id == id))
        else {
            return Err(TransportError::Stale);
        };
        let completion =
            self.queue.complete(id.request, device_status).map_err(TransportError::Queue)?.1;
        self.pending[slot] = None;
        Ok(completion)
    }

    pub fn expire(&mut self, now: u64) -> Expired<DEPTH> {
        let mut expired = Expired::empty();
        for pending in &mut self.pending {
            let Some(value) = *pending else { continue };
            if value.deadline > now || expired.count == DEPTH {
                continue;
            }
            if self.queue.cancel(value.id.request).is_ok() {
                expired.ids[expired.count] = Some(value.id);
                expired.count += 1;
                *pending = None;
            }
        }
        expired
    }

    pub fn reset(&mut self) -> usize {
        let count = self.pending.iter().filter(|pending| pending.is_some()).count();
        self.pending.fill(None);
        self.queue = VirtioBlkQueue::new();
        self.epoch = self.epoch.wrapping_add(1).max(1);
        count
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::virtio::{VIRTIO_BLK_STATUS_OK, VIRTIO_BLK_STATUS_UNSUPP};
    use crate::{BlockIndex, BlockRequestTable, BlockStatus, BufferToken};

    fn buffer() -> BufferToken {
        BufferToken::new(0x1000).unwrap()
    }

    #[test]
    fn feature_negotiation_requires_modern_version_and_flush_for_writes() {
        assert_eq!(negotiate_features(0, false), Err(FeatureError::VersionRequired));
        assert_eq!(negotiate_features(VIRTIO_F_VERSION_1, true), Err(FeatureError::FlushRequired));
        let features = negotiate_features(VIRTIO_F_VERSION_1 | VIRTIO_BLK_F_FLUSH, true).unwrap();
        assert!(features.contains(VIRTIO_F_VERSION_1));
        assert!(features.contains(VIRTIO_BLK_F_FLUSH));
    }

    #[test]
    fn dma_arena_only_releases_owned_aligned_pages() {
        let mut arena = DmaArena::<4>::new(0x20_000).unwrap();
        let lease = arena.lease(2).unwrap();
        assert_eq!(lease.address().get(), 0x20_000);
        assert_eq!(arena.release(lease), Ok(()));
        assert_eq!(arena.release(lease), Err(DmaError::Stale));
    }

    #[test]
    fn transport_times_out_and_resets_stale_requests() {
        let mut table = BlockRequestTable::<2>::new();
        let request_id = table.submit(BlockRequest::read(BlockIndex::new(1), 1, buffer())).unwrap();
        table.claim_next().unwrap();
        let mut transport =
            VirtioTransport::<2>::new(VIRTIO_F_VERSION_1 | VIRTIO_BLK_F_FLUSH, true).unwrap();
        let (id, _) = transport
            .submit(request_id, BlockRequest::read(BlockIndex::new(1), 1, buffer()), 5)
            .unwrap();
        let expired = transport.expire(5);
        assert_eq!(expired.get(0), Some(id));
        let old_epoch = transport.epoch();
        assert_eq!(transport.reset(), 0);
        assert_ne!(transport.epoch(), old_epoch);
    }

    #[test]
    fn transport_completes_out_of_order_and_rejects_stale_epoch() {
        let mut table = BlockRequestTable::<2>::new();
        let first = table.submit(BlockRequest::read(BlockIndex::new(0), 1, buffer())).unwrap();
        let second = table.submit(BlockRequest::read(BlockIndex::new(1), 1, buffer())).unwrap();
        let mut transport =
            VirtioTransport::<2>::new(VIRTIO_F_VERSION_1 | VIRTIO_BLK_F_FLUSH, true).unwrap();
        let (first_id, _) = transport
            .submit(first, BlockRequest::read(BlockIndex::new(0), 1, buffer()), 100)
            .unwrap();
        let (second_id, _) = transport
            .submit(second, BlockRequest::read(BlockIndex::new(1), 1, buffer()), 100)
            .unwrap();
        assert_eq!(
            transport.complete(second_id, VIRTIO_BLK_STATUS_UNSUPP).unwrap().status,
            BlockStatus::Unsupported
        );
        assert_eq!(
            transport.complete(first_id, VIRTIO_BLK_STATUS_OK).unwrap().status,
            BlockStatus::Success
        );
        transport.reset();
        assert_eq!(transport.complete(first_id, VIRTIO_BLK_STATUS_OK), Err(TransportError::Stale));
    }
}
