use crate::BlockIndex;

pub const MAX_BLOCK_REQUESTS: usize = 8;
pub const MAX_BLOCKS_PER_REQUEST: u16 = 16;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BufferToken(u64);

impl BufferToken {
    pub const NONE: Self = Self(0);

    pub const fn new(value: u64) -> Option<Self> {
        if value == 0 { None } else { Some(Self(value)) }
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BlockOperation {
    Read,
    Write,
    Flush,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BlockRequest {
    pub operation: BlockOperation,
    pub start: BlockIndex,
    pub blocks: u16,
    pub buffer: BufferToken,
}

impl BlockRequest {
    pub const fn read(start: BlockIndex, blocks: u16, buffer: BufferToken) -> Self {
        Self { operation: BlockOperation::Read, start, blocks, buffer }
    }

    pub const fn write(start: BlockIndex, blocks: u16, buffer: BufferToken) -> Self {
        Self { operation: BlockOperation::Write, start, blocks, buffer }
    }

    pub const fn flush() -> Self {
        Self {
            operation: BlockOperation::Flush,
            start: BlockIndex::new(0),
            blocks: 0,
            buffer: BufferToken::NONE,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BlockRequestId {
    slot: u16,
    generation: u64,
}

impl BlockRequestId {
    pub const fn from_parts(slot: u16, generation: u64) -> Option<Self> {
        if slot >= MAX_BLOCK_REQUESTS as u16 || generation == 0 {
            None
        } else {
            Some(Self { slot, generation })
        }
    }

    pub const fn slot(self) -> u16 {
        self.slot
    }

    pub const fn generation(self) -> u64 {
        self.generation
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BlockStatus {
    Success,
    Io,
    Unsupported,
    TimedOut,
    DeviceReset,
    Cancelled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BlockCompletion {
    pub status: BlockStatus,
    pub blocks_completed: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BlockRequestError {
    Full,
    InvalidRequest,
    GenerationExhausted,
    Stale,
    NotInFlight,
    NotCompleted,
    CompletionAlreadyAvailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RequestState {
    Free,
    Ready,
    InFlight,
    Completed,
}

#[derive(Clone, Copy)]
struct RequestSlot {
    generation: u64,
    state: RequestState,
    request: Option<BlockRequest>,
    completion: Option<BlockCompletion>,
}

impl RequestSlot {
    const EMPTY: Self =
        Self { generation: 0, state: RequestState::Free, request: None, completion: None };
}

/// Fixed request lifecycle for future device adapters.
///
/// The table owns request identity and state transitions, but not data pages.
/// `BufferToken` remains opaque so a later Core boundary can bind it to a
/// validated page grant without changing request completion semantics.
pub struct BlockRequestTable<const CAPACITY: usize = MAX_BLOCK_REQUESTS> {
    slots: [RequestSlot; CAPACITY],
    cursor: usize,
}

impl<const CAPACITY: usize> BlockRequestTable<CAPACITY> {
    pub const fn new() -> Self {
        Self { slots: [RequestSlot::EMPTY; CAPACITY], cursor: 0 }
    }

    pub const fn capacity(&self) -> usize {
        CAPACITY
    }

    pub fn submit(&mut self, request: BlockRequest) -> Result<BlockRequestId, BlockRequestError> {
        validate_request(request)?;
        let Some((slot, state)) =
            self.slots.iter_mut().enumerate().find(|(_, slot)| slot.state == RequestState::Free)
        else {
            return Err(BlockRequestError::Full);
        };

        state.generation =
            state.generation.checked_add(1).ok_or(BlockRequestError::GenerationExhausted)?;
        state.state = RequestState::Ready;
        state.request = Some(request);
        state.completion = None;
        Ok(BlockRequestId { slot: slot as u16, generation: state.generation })
    }

    pub fn claim_next(&mut self) -> Option<(BlockRequestId, BlockRequest)> {
        if CAPACITY == 0 {
            return None;
        }
        for offset in 0..CAPACITY {
            let index = (self.cursor + offset) % CAPACITY;
            let slot = &mut self.slots[index];
            if slot.state == RequestState::Ready {
                slot.state = RequestState::InFlight;
                self.cursor = (index + 1) % CAPACITY;
                return Some((
                    BlockRequestId { slot: index as u16, generation: slot.generation },
                    slot.request.expect("ready request has payload"),
                ));
            }
        }
        None
    }

    pub fn complete(
        &mut self,
        id: BlockRequestId,
        completion: BlockCompletion,
    ) -> Result<(), BlockRequestError> {
        let slot = self.slot_mut(id)?;
        if slot.state != RequestState::InFlight {
            return Err(BlockRequestError::NotInFlight);
        }
        let request = slot.request.expect("in-flight request has payload");
        validate_completion(request, completion)?;
        slot.completion = Some(completion);
        slot.state = RequestState::Completed;
        Ok(())
    }

    pub fn cancel(&mut self, id: BlockRequestId) -> Result<(), BlockRequestError> {
        let slot = self.slot_mut(id)?;
        if !matches!(slot.state, RequestState::Ready | RequestState::InFlight) {
            return Err(BlockRequestError::NotInFlight);
        }
        slot.completion =
            Some(BlockCompletion { status: BlockStatus::Cancelled, blocks_completed: 0 });
        slot.state = RequestState::Completed;
        Ok(())
    }

    pub fn take_completion(
        &mut self,
        id: BlockRequestId,
    ) -> Result<BlockCompletion, BlockRequestError> {
        let slot = self.slot_mut(id)?;
        if slot.state != RequestState::Completed {
            return Err(BlockRequestError::NotCompleted);
        }
        let completion = slot.completion.take().expect("completed request has result");
        slot.request = None;
        slot.state = RequestState::Free;
        Ok(completion)
    }

    fn slot_mut(&mut self, id: BlockRequestId) -> Result<&mut RequestSlot, BlockRequestError> {
        let Some(slot) = self.slots.get_mut(id.slot as usize) else {
            return Err(BlockRequestError::Stale);
        };
        if slot.generation != id.generation || slot.state == RequestState::Free {
            return Err(BlockRequestError::Stale);
        }
        Ok(slot)
    }
}

impl<const CAPACITY: usize> Default for BlockRequestTable<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

pub(crate) fn validate_request(request: BlockRequest) -> Result<(), BlockRequestError> {
    match request.operation {
        BlockOperation::Flush => {
            if request.blocks != 0 || request.buffer != BufferToken::NONE {
                return Err(BlockRequestError::InvalidRequest);
            }
        }
        BlockOperation::Read | BlockOperation::Write => {
            if request.blocks == 0
                || request.blocks > MAX_BLOCKS_PER_REQUEST
                || request.buffer == BufferToken::NONE
                || request.start.get().checked_add(request.blocks as u64).is_none()
            {
                return Err(BlockRequestError::InvalidRequest);
            }
        }
    }
    Ok(())
}

fn validate_completion(
    request: BlockRequest,
    completion: BlockCompletion,
) -> Result<(), BlockRequestError> {
    if completion.blocks_completed > request.blocks {
        return Err(BlockRequestError::InvalidRequest);
    }
    if completion.status == BlockStatus::Success && completion.blocks_completed != request.blocks {
        return Err(BlockRequestError::InvalidRequest);
    }
    if request.operation == BlockOperation::Flush && completion.blocks_completed != 0 {
        return Err(BlockRequestError::InvalidRequest);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buffer() -> BufferToken {
        BufferToken::new(1).unwrap()
    }

    #[test]
    fn request_round_trip_is_generation_safe() {
        let mut table = BlockRequestTable::<2>::new();
        let id = table.submit(BlockRequest::read(BlockIndex::new(4), 2, buffer())).unwrap();
        let (claimed, request) = table.claim_next().unwrap();
        assert_eq!(claimed, id);
        assert_eq!(request.start, BlockIndex::new(4));
        table
            .complete(id, BlockCompletion { status: BlockStatus::Success, blocks_completed: 2 })
            .unwrap();
        assert_eq!(
            table.take_completion(id).unwrap(),
            BlockCompletion { status: BlockStatus::Success, blocks_completed: 2 }
        );
        assert_eq!(table.take_completion(id), Err(BlockRequestError::Stale));

        let next = table.submit(BlockRequest::flush()).unwrap();
        assert_ne!(next.generation(), id.generation());
        assert_eq!(next.slot(), id.slot());
    }

    #[test]
    fn capacity_and_claims_are_bounded() {
        let mut table = BlockRequestTable::<2>::new();
        let first = table.submit(BlockRequest::flush()).unwrap();
        let second = table.submit(BlockRequest::flush()).unwrap();
        assert_eq!(table.submit(BlockRequest::flush()), Err(BlockRequestError::Full));
        assert!(table.claim_next().is_some());
        assert!(table.claim_next().is_some());
        assert!(table.claim_next().is_none());
        table.cancel(first).unwrap();
        table.cancel(second).unwrap();
    }

    #[test]
    fn invalid_request_shapes_are_rejected() {
        let mut table = BlockRequestTable::<1>::new();
        assert_eq!(
            table.submit(BlockRequest::read(BlockIndex::new(0), 0, buffer())),
            Err(BlockRequestError::InvalidRequest)
        );
        assert_eq!(
            table.submit(BlockRequest::read(
                BlockIndex::new(0),
                MAX_BLOCKS_PER_REQUEST + 1,
                buffer()
            )),
            Err(BlockRequestError::InvalidRequest)
        );
        assert_eq!(
            table.submit(BlockRequest::read(BlockIndex::new(0), 1, BufferToken::NONE)),
            Err(BlockRequestError::InvalidRequest)
        );
        assert_eq!(
            table.submit(BlockRequest {
                operation: BlockOperation::Flush,
                start: BlockIndex::new(1),
                blocks: 0,
                buffer: buffer()
            }),
            Err(BlockRequestError::InvalidRequest)
        );
    }

    #[test]
    fn cancellation_completes_ready_and_inflight_requests() {
        let mut table = BlockRequestTable::<2>::new();
        let ready = table.submit(BlockRequest::flush()).unwrap();
        table.cancel(ready).unwrap();
        assert_eq!(table.take_completion(ready).unwrap().status, BlockStatus::Cancelled);

        let inflight = table.submit(BlockRequest::flush()).unwrap();
        table.claim_next().unwrap();
        table.cancel(inflight).unwrap();
        assert_eq!(table.take_completion(inflight).unwrap().status, BlockStatus::Cancelled);
    }

    #[test]
    fn completion_status_and_counts_are_validated() {
        let mut table = BlockRequestTable::<1>::new();
        let id = table.submit(BlockRequest::read(BlockIndex::new(0), 2, buffer())).unwrap();
        table.claim_next().unwrap();
        assert_eq!(
            table.complete(
                id,
                BlockCompletion { status: BlockStatus::Success, blocks_completed: 1 }
            ),
            Err(BlockRequestError::InvalidRequest)
        );
        assert_eq!(
            table.complete(id, BlockCompletion { status: BlockStatus::Io, blocks_completed: 3 }),
            Err(BlockRequestError::InvalidRequest)
        );
        table
            .complete(id, BlockCompletion { status: BlockStatus::Io, blocks_completed: 1 })
            .unwrap();
    }

    #[test]
    fn request_generation_exhaustion_is_explicit() {
        let mut table = BlockRequestTable::<1>::new();
        table.slots[0].generation = u64::MAX;

        assert_eq!(
            table.submit(BlockRequest::flush()),
            Err(BlockRequestError::GenerationExhausted)
        );
        assert_eq!(table.slots[0].state, RequestState::Free);
    }
}
