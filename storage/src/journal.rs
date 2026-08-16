use crate::{BLOCK_BYTES, Block, BlockError, BlockIndex, BlockStore};

pub const FORMAT_VERSION: u16 = 1;
pub const PROVISIONED_BLANK_MAGIC: &[u8; 8] = b"LOGOSBLK";
/// Reserved record kind used only for internal transaction commit markers.
pub const JOURNAL_COMMIT_KIND: u16 = u16::MAX;
pub const MAX_RECORDS_PER_TRANSACTION: usize = 8;

const SUPERBLOCK_A: BlockIndex = BlockIndex::new(0);
const SUPERBLOCK_B: BlockIndex = BlockIndex::new(1);
const JOURNAL_START: u64 = 2;
const SUPERBLOCK_MAGIC: &[u8; 8] = b"LOGOSFS\0";
const RECORD_MAGIC: &[u8; 4] = b"LOGR";
const SUPERBLOCK_CHECKSUM_OFFSET: usize = 64;
const RECORD_CHECKSUM_OFFSET: usize = 28;
const RECORD_HEADER_BYTES: usize = 32;
const MAX_TRANSACTION_ID: u64 = u64::MAX;

pub const MAX_RECORD_PAYLOAD_BYTES: usize = BLOCK_BYTES - RECORD_HEADER_BYTES;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FormatError {
    Block(BlockError),
    NotBlank,
    Unformatted,
    Corrupt,
    UnsupportedVersion,
    TooSmall,
    InvalidRequest,
    PayloadTooLarge,
    TransactionTooLarge,
    JournalFull,
    GenerationExhausted,
    ReplayRejected,
    ProvisionedBlank,
}

impl From<BlockError> for FormatError {
    fn from(error: BlockError) -> Self {
        Self::Block(error)
    }
}

#[derive(Clone, Copy)]
pub struct JournalRecord<'a> {
    pub kind: u16,
    pub payload: &'a [u8],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReplayError {
    Rejected,
}

pub trait ReplaySink {
    fn record(&mut self, transaction_id: u64, kind: u16, payload: &[u8])
    -> Result<(), ReplayError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RecoverySummary {
    pub committed_transactions: u64,
    pub replayed_records: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VolumeInfo {
    pub generation: u64,
    pub journal_start: u64,
    pub journal_end: u64,
    pub journal_head: u64,
    pub journal_tail: u64,
    pub root_transaction_id: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Volume {
    info: VolumeInfo,
    active_superblock: u8,
}

impl Volume {
    pub fn format<B: BlockStore>(store: &mut B) -> Result<Self, FormatError> {
        let block_count = store.block_count();
        if block_count <= JOURNAL_START {
            return Err(FormatError::TooSmall);
        }

        let mut block = Block::zero();
        for index in 0..block_count {
            store.read_block(BlockIndex::new(index), &mut block)?;
            if block.as_bytes().iter().any(|byte| *byte != 0) {
                return Err(FormatError::NotBlank);
            }
        }

        Self::format_metadata(store)
    }

    pub fn format_provisioned<B: BlockStore>(store: &mut B) -> Result<Self, FormatError> {
        if store.block_count() <= JOURNAL_START {
            return Err(FormatError::TooSmall);
        }
        let mut marker = Block::zero();
        store.read_block(SUPERBLOCK_A, &mut marker)?;
        if &marker.as_bytes()[..PROVISIONED_BLANK_MAGIC.len()] != PROVISIONED_BLANK_MAGIC {
            return Err(FormatError::NotBlank);
        }
        Self::format_metadata(store)
    }

    fn format_metadata<B: BlockStore>(store: &mut B) -> Result<Self, FormatError> {
        let block_count = store.block_count();
        if block_count <= JOURNAL_START {
            return Err(FormatError::TooSmall);
        }
        let info = VolumeInfo {
            generation: 1,
            journal_start: JOURNAL_START,
            journal_end: block_count,
            journal_head: JOURNAL_START,
            journal_tail: JOURNAL_START,
            root_transaction_id: 0,
        };

        let empty = Block::zero();
        store.write_block(BlockIndex::new(JOURNAL_START), &empty)?;
        store.flush()?;
        write_superblock(store, SUPERBLOCK_A, info)?;
        store.flush()?;
        write_superblock(store, SUPERBLOCK_B, info)?;
        store.flush()?;

        Ok(Self { info, active_superblock: 1 })
    }

    pub fn open<B: BlockStore>(store: &mut B) -> Result<Self, FormatError> {
        if store.block_count() <= JOURNAL_START {
            return Err(FormatError::TooSmall);
        }

        let first = read_superblock(store, SUPERBLOCK_A);
        let second = read_superblock(store, SUPERBLOCK_B);

        match (first, second) {
            (Err(FormatError::UnsupportedVersion), _)
            | (_, Err(FormatError::UnsupportedVersion)) => Err(FormatError::UnsupportedVersion),
            (Ok(Some(a)), Ok(Some(b))) => {
                if b.generation > a.generation {
                    Ok(Self { info: b, active_superblock: 1 })
                } else {
                    Ok(Self { info: a, active_superblock: 0 })
                }
            }
            (Ok(Some(info)), _) => Ok(Self { info, active_superblock: 0 }),
            (_, Ok(Some(info))) => Ok(Self { info, active_superblock: 1 }),
            (Ok(None), Ok(None)) => Err(FormatError::Unformatted),
            (Err(error), Ok(None)) | (Ok(None), Err(error)) => Err(error),
            (Err(error), Err(_)) => Err(error),
        }
    }

    pub const fn info(self) -> VolumeInfo {
        self.info
    }

    pub fn commit<B: BlockStore>(
        &mut self,
        store: &mut B,
        records: &[JournalRecord<'_>],
    ) -> Result<u64, FormatError> {
        if records.len() > MAX_RECORDS_PER_TRANSACTION {
            return Err(FormatError::TransactionTooLarge);
        }
        if records.iter().any(|record| record.kind == JOURNAL_COMMIT_KIND) {
            return Err(FormatError::InvalidRequest);
        }

        let mut required_blocks = records.len() as u64;
        required_blocks = required_blocks.checked_add(1).ok_or(FormatError::JournalFull)?;
        let next_head =
            self.info.journal_head.checked_add(required_blocks).ok_or(FormatError::JournalFull)?;
        if next_head > self.info.journal_end {
            return Err(FormatError::JournalFull);
        }

        let transaction_id =
            self.info.root_transaction_id.checked_add(1).ok_or(FormatError::GenerationExhausted)?;

        for (sequence, record) in records.iter().enumerate() {
            let block =
                encode_record(transaction_id, sequence as u32, record.kind, record.payload)?;
            store.write_block(BlockIndex::new(self.info.journal_head + sequence as u64), &block)?;
        }

        let commit = encode_record(transaction_id, records.len() as u32, JOURNAL_COMMIT_KIND, &[])?;
        store
            .write_block(BlockIndex::new(self.info.journal_head + records.len() as u64), &commit)?;
        store.flush()?;

        let generation =
            self.info.generation.checked_add(1).ok_or(FormatError::GenerationExhausted)?;
        let next_info = VolumeInfo {
            generation,
            journal_head: next_head,
            root_transaction_id: transaction_id,
            ..self.info
        };
        let next_slot = if self.active_superblock == 0 { SUPERBLOCK_B } else { SUPERBLOCK_A };
        write_superblock(store, next_slot, next_info)?;
        store.flush()?;

        self.info = next_info;
        self.active_superblock ^= 1;
        Ok(transaction_id)
    }

    pub fn recover<B: BlockStore, S: ReplaySink>(
        &mut self,
        store: &mut B,
        sink: &mut S,
    ) -> Result<RecoverySummary, FormatError> {
        let mut pending = [PendingRecord::EMPTY; MAX_RECORDS_PER_TRANSACTION];
        let mut pending_len = 0usize;
        let mut pending_transaction = 0u64;
        let mut committed_transactions = 0u64;
        let mut replayed_records = 0u64;
        let mut last_transaction = 0u64;
        let previous_head = self.info.journal_head;
        let mut committed_head = self.info.journal_tail;
        let mut truncated = false;
        let recovery_end = self
            .info
            .journal_head
            .saturating_add(MAX_RECORDS_PER_TRANSACTION as u64 + 1)
            .min(self.info.journal_end);

        for index in self.info.journal_tail..recovery_end {
            let mut block = Block::zero();
            store.read_block(BlockIndex::new(index), &mut block)?;
            let record = match decode_record(&block) {
                Ok(Some(record)) => record,
                Ok(None) => {
                    if index >= previous_head {
                        break;
                    }
                    // Abandon only the incomplete transaction at this gap. A
                    // later checksummed transaction may still be durable.
                    truncated = true;
                    pending_len = 0;
                    pending_transaction = 0;
                    continue;
                }
                Err(error) => {
                    if error == FormatError::UnsupportedVersion {
                        return Err(error);
                    }
                    if !remaining_journal_is_blank(store, index + 1, self.info.journal_head)? {
                        return Err(error);
                    }
                    truncated = true;
                    break;
                }
            };

            if record.transaction_id == 0 || record.transaction_id == MAX_TRANSACTION_ID {
                return Err(FormatError::Corrupt);
            }
            if record.transaction_id <= last_transaction {
                return Err(FormatError::Corrupt);
            }

            if record.kind == JOURNAL_COMMIT_KIND {
                let valid = record.payload_len == 0
                    && ((pending_len == 0 && record.sequence == 0)
                        || (pending_len > 0
                            && pending_transaction == record.transaction_id
                            && record.sequence == pending_len as u32));
                if !valid {
                    pending_len = 0;
                    pending_transaction = 0;
                    truncated = true;
                    continue;
                }
                if record.transaction_id <= last_transaction {
                    return Err(FormatError::Corrupt);
                }

                for pending_record in pending.iter().take(pending_len) {
                    sink.record(
                        record.transaction_id,
                        pending_record.kind,
                        &pending_record.payload.as_bytes()[..pending_record.payload_len as usize],
                    )
                    .map_err(|_| FormatError::ReplayRejected)?;
                    replayed_records += 1;
                }
                committed_transactions += 1;
                last_transaction = record.transaction_id;
                committed_head = index + 1;
                pending_len = 0;
                pending_transaction = 0;
                continue;
            }

            if pending_len == 0 {
                if record.sequence != 0 {
                    truncated = true;
                    continue;
                }
                pending_transaction = record.transaction_id;
            }
            if pending_transaction != record.transaction_id
                || record.sequence != pending_len as u32
                || pending_len == MAX_RECORDS_PER_TRANSACTION
            {
                pending_len = 0;
                pending_transaction = 0;
                truncated = true;
                if record.sequence != 0 {
                    continue;
                }
                pending_transaction = record.transaction_id;
            }
            pending[pending_len] = PendingRecord {
                kind: record.kind,
                payload_len: record.payload_len,
                payload: record.payload,
            };
            pending_len += 1;
        }

        if !truncated && last_transaction < self.info.root_transaction_id {
            return Err(FormatError::Corrupt);
        }

        if committed_head != previous_head || last_transaction != self.info.root_transaction_id {
            let generation =
                self.info.generation.checked_add(1).ok_or(FormatError::GenerationExhausted)?;
            let next_info = VolumeInfo {
                generation,
                journal_head: committed_head,
                root_transaction_id: last_transaction,
                ..self.info
            };
            let next_slot = if self.active_superblock == 0 { SUPERBLOCK_B } else { SUPERBLOCK_A };
            write_superblock(store, next_slot, next_info)?;
            store.flush()?;
            self.info = next_info;
            self.active_superblock ^= 1;
        }

        Ok(RecoverySummary { committed_transactions, replayed_records })
    }
}

#[derive(Clone, Copy)]
struct PendingRecord {
    kind: u16,
    payload_len: u16,
    payload: Block,
}

impl PendingRecord {
    const EMPTY: Self = Self { kind: 0, payload_len: 0, payload: Block::ZERO };
}

#[derive(Clone, Copy)]
struct DecodedRecord {
    transaction_id: u64,
    sequence: u32,
    kind: u16,
    payload_len: u16,
    payload: Block,
}

fn write_superblock<B: BlockStore>(
    store: &mut B,
    index: BlockIndex,
    info: VolumeInfo,
) -> Result<(), FormatError> {
    let mut block = Block::zero();
    let bytes = block.as_bytes_mut();
    bytes[..8].copy_from_slice(SUPERBLOCK_MAGIC);
    put_u16(bytes, 8, FORMAT_VERSION);
    put_u16(bytes, 10, 0);
    put_u64(bytes, 16, info.generation);
    put_u64(bytes, 24, info.journal_start);
    put_u64(bytes, 32, info.journal_end);
    put_u64(bytes, 40, info.journal_head);
    put_u64(bytes, 48, info.journal_tail);
    put_u64(bytes, 56, info.root_transaction_id);
    let checksum = crc32c(&*bytes);
    put_u32(bytes, SUPERBLOCK_CHECKSUM_OFFSET, checksum);
    store.write_block(index, &block)?;
    Ok(())
}

fn read_superblock<B: BlockStore>(
    store: &mut B,
    index: BlockIndex,
) -> Result<Option<VolumeInfo>, FormatError> {
    let mut block = Block::zero();
    store.read_block(index, &mut block)?;
    let bytes = block.as_bytes();
    if bytes.iter().all(|byte| *byte == 0) {
        return Ok(None);
    }
    if &bytes[..8] != SUPERBLOCK_MAGIC {
        if &bytes[..PROVISIONED_BLANK_MAGIC.len()] == PROVISIONED_BLANK_MAGIC {
            return Err(FormatError::ProvisionedBlank);
        }
        return Err(FormatError::Corrupt);
    }
    if get_u16(bytes, 8) != FORMAT_VERSION {
        return Err(FormatError::UnsupportedVersion);
    }
    let stored_checksum = get_u32(bytes, SUPERBLOCK_CHECKSUM_OFFSET);
    let mut checked = block;
    put_u32(checked.as_bytes_mut(), SUPERBLOCK_CHECKSUM_OFFSET, 0);
    if crc32c(checked.as_bytes()) != stored_checksum {
        return Err(FormatError::Corrupt);
    }

    let info = VolumeInfo {
        generation: get_u64(bytes, 16),
        journal_start: get_u64(bytes, 24),
        journal_end: get_u64(bytes, 32),
        journal_head: get_u64(bytes, 40),
        journal_tail: get_u64(bytes, 48),
        root_transaction_id: get_u64(bytes, 56),
    };
    if info.generation == 0
        || info.journal_start != JOURNAL_START
        || info.journal_end > store.block_count()
        || info.journal_end <= info.journal_start
        || info.journal_tail < info.journal_start
        || info.journal_tail > info.journal_head
        || info.journal_head > info.journal_end
    {
        return Err(FormatError::Corrupt);
    }
    Ok(Some(info))
}

fn encode_record(
    transaction_id: u64,
    sequence: u32,
    kind: u16,
    payload: &[u8],
) -> Result<Block, FormatError> {
    if payload.len() > MAX_RECORD_PAYLOAD_BYTES {
        return Err(FormatError::PayloadTooLarge);
    }
    if transaction_id == 0 || transaction_id == MAX_TRANSACTION_ID {
        return Err(FormatError::InvalidRequest);
    }

    let mut block = Block::zero();
    let bytes = block.as_bytes_mut();
    bytes[..4].copy_from_slice(RECORD_MAGIC);
    put_u16(bytes, 4, FORMAT_VERSION);
    put_u16(bytes, 6, kind);
    put_u64(bytes, 8, transaction_id);
    put_u32(bytes, 16, sequence);
    put_u16(bytes, 20, payload.len() as u16);
    bytes[RECORD_HEADER_BYTES..RECORD_HEADER_BYTES + payload.len()].copy_from_slice(payload);
    let checksum = crc32c(&*bytes);
    put_u32(bytes, RECORD_CHECKSUM_OFFSET, checksum);
    Ok(block)
}

fn decode_record(block: &Block) -> Result<Option<DecodedRecord>, FormatError> {
    let bytes = block.as_bytes();
    if bytes.iter().all(|byte| *byte == 0) {
        return Ok(None);
    }
    if &bytes[..4] != RECORD_MAGIC {
        return Err(FormatError::Corrupt);
    }
    if get_u16(bytes, 4) != FORMAT_VERSION {
        return Err(FormatError::UnsupportedVersion);
    }
    let stored_checksum = get_u32(bytes, RECORD_CHECKSUM_OFFSET);
    let mut checked = *block;
    put_u32(checked.as_bytes_mut(), RECORD_CHECKSUM_OFFSET, 0);
    if crc32c(checked.as_bytes()) != stored_checksum {
        return Err(FormatError::Corrupt);
    }
    let payload_len = get_u16(bytes, 20) as usize;
    if payload_len > MAX_RECORD_PAYLOAD_BYTES {
        return Err(FormatError::Corrupt);
    }
    let mut payload = Block::zero();
    payload.as_bytes_mut()[..payload_len]
        .copy_from_slice(&bytes[RECORD_HEADER_BYTES..RECORD_HEADER_BYTES + payload_len]);
    Ok(Some(DecodedRecord {
        transaction_id: get_u64(bytes, 8),
        sequence: get_u32(bytes, 16),
        kind: get_u16(bytes, 6),
        payload_len: payload_len as u16,
        payload,
    }))
}

fn remaining_journal_is_blank<B: BlockStore>(
    store: &mut B,
    start: u64,
    end: u64,
) -> Result<bool, FormatError> {
    let mut block = Block::zero();
    for index in start..end {
        store.read_block(BlockIndex::new(index), &mut block)?;
        if block.as_bytes().iter().any(|byte| *byte != 0) {
            return Ok(false);
        }
    }
    Ok(true)
}

fn put_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_u64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

fn get_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn get_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([bytes[offset], bytes[offset + 1], bytes[offset + 2], bytes[offset + 3]])
}

fn get_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
        bytes[offset + 4],
        bytes[offset + 5],
        bytes[offset + 6],
        bytes[offset + 7],
    ])
}

fn crc32c(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for byte in bytes {
        crc ^= *byte as u32;
        for _ in 0..8 {
            let mask = 0u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0x82f6_3b78 & mask);
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MemoryBlockStore;

    const BLOCKS: usize = 12;

    #[derive(Clone, Copy)]
    struct CrashStore<const N: usize> {
        working: [Block; N],
        durable: [Block; N],
        fail_writes_after: Option<usize>,
        fail_flush: bool,
        reads: usize,
    }

    impl<const N: usize> CrashStore<N> {
        const fn new() -> Self {
            Self {
                working: [Block::ZERO; N],
                durable: [Block::ZERO; N],
                fail_writes_after: None,
                fail_flush: false,
                reads: 0,
            }
        }

        fn power_loss(&mut self) {
            self.working = self.durable;
        }

        fn fail_write_after(&mut self, successful_writes: usize) {
            self.fail_writes_after = Some(successful_writes);
        }

        fn fail_next_flush(&mut self) {
            self.fail_flush = true;
        }

        fn reset_reads(&mut self) {
            self.reads = 0;
        }

        fn corrupt(&mut self, index: u64, offset: usize) {
            self.working[index as usize].as_bytes_mut()[offset] ^= 0xff;
            self.durable[index as usize] = self.working[index as usize];
        }
    }

    impl<const N: usize> BlockStore for CrashStore<N> {
        fn block_count(&self) -> u64 {
            N as u64
        }

        fn read_block(&mut self, index: BlockIndex, output: &mut Block) -> Result<(), BlockError> {
            self.reads += 1;
            let Some(block) = self.working.get(index.get() as usize) else {
                return Err(BlockError::OutOfBounds);
            };
            *output = *block;
            Ok(())
        }

        fn write_block(&mut self, index: BlockIndex, input: &Block) -> Result<(), BlockError> {
            if let Some(remaining) = self.fail_writes_after.as_mut() {
                if *remaining == 0 {
                    self.fail_writes_after = None;
                    return Err(BlockError::Io);
                }
                *remaining -= 1;
            }
            let Some(block) = self.working.get_mut(index.get() as usize) else {
                return Err(BlockError::OutOfBounds);
            };
            *block = *input;
            Ok(())
        }

        fn flush(&mut self) -> Result<(), BlockError> {
            if self.fail_flush {
                self.fail_flush = false;
                return Err(BlockError::Io);
            }
            self.durable = self.working;
            Ok(())
        }
    }

    struct Sink {
        records: [(u64, u16, [u8; 8], usize); 8],
        len: usize,
    }

    impl Sink {
        const fn new() -> Self {
            Self { records: [(0, 0, [0; 8], 0); 8], len: 0 }
        }
    }

    impl ReplaySink for Sink {
        fn record(
            &mut self,
            transaction_id: u64,
            kind: u16,
            payload: &[u8],
        ) -> Result<(), ReplayError> {
            if payload.len() > 8 || self.len == self.records.len() {
                return Err(ReplayError::Rejected);
            }
            let entry = &mut self.records[self.len];
            entry.0 = transaction_id;
            entry.1 = kind;
            entry.2[..payload.len()].copy_from_slice(payload);
            entry.3 = payload.len();
            self.len += 1;
            Ok(())
        }
    }

    #[test]
    fn formats_and_reopens_blank_media() {
        let mut store = MemoryBlockStore::<BLOCKS>::new();
        let volume = Volume::format(&mut store).unwrap();
        let reopened = Volume::open(&mut store).unwrap();
        assert_eq!(reopened.info(), volume.info());
    }

    #[test]
    fn provisioned_blank_media_formats_without_scanning_the_volume() {
        let mut store = CrashStore::<32>::new();
        let mut marker = Block::zero();
        marker.as_bytes_mut()[..PROVISIONED_BLANK_MAGIC.len()]
            .copy_from_slice(PROVISIONED_BLANK_MAGIC);
        store.write_block(SUPERBLOCK_A, &marker).unwrap();
        store.flush().unwrap();
        assert_eq!(Volume::open(&mut store), Err(FormatError::ProvisionedBlank));

        store.reset_reads();
        let volume = Volume::format_provisioned(&mut store).unwrap();
        assert_eq!(volume.info().journal_head, JOURNAL_START);
        assert_eq!(store.reads, 1);
    }

    #[test]
    fn unsupported_superblock_version_is_not_ignored() {
        let mut store = CrashStore::<BLOCKS>::new();
        Volume::format(&mut store).unwrap();

        let mut block = Block::zero();
        store.read_block(SUPERBLOCK_B, &mut block).unwrap();
        put_u16(block.as_bytes_mut(), 8, FORMAT_VERSION + 1);
        store.write_block(SUPERBLOCK_B, &block).unwrap();
        store.flush().unwrap();

        assert_eq!(Volume::open(&mut store), Err(FormatError::UnsupportedVersion));
    }

    #[test]
    fn unsupported_journal_version_is_not_treated_as_a_torn_tail() {
        let mut store = CrashStore::<BLOCKS>::new();
        let volume = Volume::format(&mut store).unwrap();
        let mut record = encode_record(1, 0, 7, b"future").unwrap();
        put_u16(record.as_bytes_mut(), 4, FORMAT_VERSION + 1);
        store.write_block(BlockIndex::new(JOURNAL_START), &record).unwrap();

        let mut info = volume.info();
        info.generation += 1;
        info.journal_head = JOURNAL_START + 1;
        info.root_transaction_id = 1;
        write_superblock(&mut store, SUPERBLOCK_A, info).unwrap();
        store.flush().unwrap();

        let mut reopened = Volume::open(&mut store).unwrap();
        let mut sink = Sink::new();
        assert_eq!(reopened.recover(&mut store, &mut sink), Err(FormatError::UnsupportedVersion));
    }

    #[test]
    fn rejects_nonblank_media_instead_of_formatting() {
        let mut store = MemoryBlockStore::<BLOCKS>::new();
        let mut block = Block::zero();
        block.as_bytes_mut()[17] = 1;
        store.write_block(BlockIndex::new(4), &block).unwrap();

        assert_eq!(Volume::format(&mut store), Err(FormatError::NotBlank));
    }

    #[test]
    fn falls_back_to_the_other_superblock_when_one_is_corrupt() {
        let mut store = CrashStore::<BLOCKS>::new();
        let volume = Volume::format(&mut store).unwrap();
        store.corrupt(0, 80);

        assert_eq!(Volume::open(&mut store).unwrap().info(), volume.info());
    }

    #[test]
    fn commits_and_replays_records_once() {
        let mut store = CrashStore::<BLOCKS>::new();
        let mut volume = Volume::format(&mut store).unwrap();
        let records = [JournalRecord { kind: 7, payload: b"hello" }];
        let transaction_id = volume.commit(&mut store, &records).unwrap();

        let mut reopened = Volume::open(&mut store).unwrap();
        let mut sink = Sink::new();
        let summary = reopened.recover(&mut store, &mut sink).unwrap();
        assert_eq!(transaction_id, 1);
        assert_eq!(summary, RecoverySummary { committed_transactions: 1, replayed_records: 1 });
        assert_eq!(sink.records[0].0, transaction_id);
        assert_eq!(sink.records[0].1, 7);
        assert_eq!(&sink.records[0].2[..5], b"hello");
    }

    #[test]
    fn blank_journal_hole_recovers_later_transactions() {
        let mut store = CrashStore::<BLOCKS>::new();
        let mut volume = Volume::format(&mut store).unwrap();
        volume.commit(&mut store, &[JournalRecord { kind: 1, payload: b"first" }]).unwrap();
        let later = encode_record(2, 0, 2, b"later").unwrap();
        let later_commit = encode_record(2, 1, JOURNAL_COMMIT_KIND, &[]).unwrap();
        store.write_block(BlockIndex::new(5), &later).unwrap();
        store.write_block(BlockIndex::new(6), &later_commit).unwrap();

        let mut info = volume.info();
        info.generation += 1;
        info.journal_head = 7;
        info.root_transaction_id = 2;
        write_superblock(&mut store, SUPERBLOCK_B, info).unwrap();
        store.flush().unwrap();

        let mut reopened = Volume::open(&mut store).unwrap();
        let mut sink = Sink::new();
        assert_eq!(
            reopened.recover(&mut store, &mut sink).unwrap(),
            RecoverySummary { committed_transactions: 2, replayed_records: 2 }
        );
        assert_eq!(reopened.info().journal_head, 7);
        assert_eq!(reopened.info().root_transaction_id, 2);
    }

    #[test]
    fn uncommitted_tail_disappears_after_power_loss() {
        let mut store = CrashStore::<BLOCKS>::new();
        let mut volume = Volume::format(&mut store).unwrap();
        store.fail_write_after(1);
        let records = [JournalRecord { kind: 3, payload: b"lost" }];
        assert_eq!(volume.commit(&mut store, &records), Err(FormatError::Block(BlockError::Io)));
        store.power_loss();

        let mut reopened = Volume::open(&mut store).unwrap();
        let mut sink = Sink::new();
        let summary = reopened.recover(&mut store, &mut sink).unwrap();
        assert_eq!(summary, RecoverySummary { committed_transactions: 0, replayed_records: 0 });
        assert_eq!(sink.len, 0);
    }

    #[test]
    fn torn_zero_tail_is_ignored() {
        let mut store = CrashStore::<BLOCKS>::new();
        let volume = Volume::format(&mut store).unwrap();
        let mut tampered = volume.info();
        tampered.journal_head += 1;
        write_superblock(&mut store, SUPERBLOCK_A, tampered).unwrap();
        store.flush().unwrap();

        let mut reopened = Volume::open(&mut store).unwrap();
        let mut sink = Sink::new();
        assert_eq!(reopened.recover(&mut store, &mut sink).unwrap().replayed_records, 0);
    }

    #[test]
    fn nonzero_torn_tail_is_ignored_when_remainder_is_blank() {
        let mut store = CrashStore::<BLOCKS>::new();
        let volume = Volume::format(&mut store).unwrap();
        let mut torn = Block::zero();
        torn.as_bytes_mut()[0] = 0x7f;
        store.write_block(BlockIndex::new(2), &torn).unwrap();

        let mut info = volume.info();
        info.generation += 1;
        info.journal_head += 1;
        write_superblock(&mut store, SUPERBLOCK_A, info).unwrap();
        store.flush().unwrap();

        let mut reopened = Volume::open(&mut store).unwrap();
        let mut sink = Sink::new();
        assert_eq!(reopened.recover(&mut store, &mut sink).unwrap().replayed_records, 0);
    }

    #[test]
    fn corrupt_journal_hole_with_later_data_is_rejected() {
        let mut store = CrashStore::<BLOCKS>::new();
        let volume = Volume::format(&mut store).unwrap();
        let mut corrupt = Block::zero();
        corrupt.as_bytes_mut()[0] = 0x7f;
        store.write_block(BlockIndex::new(2), &corrupt).unwrap();
        let later = encode_record(1, 0, 1, b"later").unwrap();
        store.write_block(BlockIndex::new(3), &later).unwrap();

        let mut info = volume.info();
        info.generation += 1;
        info.journal_head += 2;
        write_superblock(&mut store, SUPERBLOCK_A, info).unwrap();
        store.flush().unwrap();

        let mut reopened = Volume::open(&mut store).unwrap();
        let mut sink = Sink::new();
        assert_eq!(reopened.recover(&mut store, &mut sink), Err(FormatError::Corrupt));
    }

    #[test]
    fn corrupt_record_is_reported() {
        let mut store = CrashStore::<BLOCKS>::new();
        let mut volume = Volume::format(&mut store).unwrap();
        volume.commit(&mut store, &[JournalRecord { kind: 1, payload: b"bad" }]).unwrap();
        store.corrupt(2, RECORD_CHECKSUM_OFFSET);

        let mut reopened = Volume::open(&mut store).unwrap();
        let mut sink = Sink::new();
        assert_eq!(reopened.recover(&mut store, &mut sink), Err(FormatError::Corrupt));
    }

    #[test]
    fn flushed_commit_is_recovered_when_superblock_publish_torn() {
        let mut store = CrashStore::<BLOCKS>::new();
        let mut volume = Volume::format(&mut store).unwrap();
        store.fail_write_after(2);
        assert_eq!(
            volume.commit(&mut store, &[JournalRecord { kind: 7, payload: b"durable" }]),
            Err(FormatError::Block(BlockError::Io))
        );
        store.power_loss();

        let mut reopened = Volume::open(&mut store).unwrap();
        let mut sink = Sink::new();
        assert_eq!(
            reopened.recover(&mut store, &mut sink).unwrap(),
            RecoverySummary { committed_transactions: 1, replayed_records: 1 }
        );
        assert_eq!(reopened.info().root_transaction_id, 1);
        assert_eq!(
            reopened.commit(&mut store, &[JournalRecord { kind: 8, payload: b"next" }]),
            Ok(2)
        );
    }

    #[test]
    fn recovery_does_not_scan_the_physical_tail() {
        let mut store = CrashStore::<32>::new();
        let volume = Volume::format(&mut store).unwrap();
        store.reset_reads();
        let mut reopened = Volume::open(&mut store).unwrap();
        store.reset_reads();
        let mut sink = Sink::new();
        assert_eq!(reopened.recover(&mut store, &mut sink).unwrap().replayed_records, 0);
        assert_eq!(store.reads, 1);
        assert_eq!(reopened.info(), volume.info());
    }

    #[test]
    fn duplicate_transactions_are_rejected_during_recovery() {
        let mut store = CrashStore::<16>::new();
        let mut volume = Volume::format(&mut store).unwrap();
        volume.commit(&mut store, &[JournalRecord { kind: 1, payload: b"one" }]).unwrap();

        let duplicate = encode_record(1, 0, 1, b"one-again").unwrap();
        let duplicate_commit = encode_record(1, 1, JOURNAL_COMMIT_KIND, &[]).unwrap();
        let second = encode_record(2, 0, 2, b"two").unwrap();
        let second_commit = encode_record(2, 1, JOURNAL_COMMIT_KIND, &[]).unwrap();
        store.write_block(BlockIndex::new(4), &duplicate).unwrap();
        store.write_block(BlockIndex::new(5), &duplicate_commit).unwrap();
        store.write_block(BlockIndex::new(6), &second).unwrap();
        store.write_block(BlockIndex::new(7), &second_commit).unwrap();

        let mut info = volume.info();
        info.generation += 1;
        info.journal_head = 8;
        info.root_transaction_id = 2;
        write_superblock(&mut store, SUPERBLOCK_B, info).unwrap();
        store.flush().unwrap();

        let mut reopened = Volume::open(&mut store).unwrap();
        let mut sink = Sink::new();
        assert_eq!(reopened.recover(&mut store, &mut sink), Err(FormatError::Corrupt));
    }

    #[test]
    fn flush_failure_is_propagated_and_does_not_publish_state() {
        let mut store = CrashStore::<BLOCKS>::new();
        let mut volume = Volume::format(&mut store).unwrap();
        let before = volume.info();
        store.fail_next_flush();
        assert_eq!(
            volume.commit(&mut store, &[JournalRecord { kind: 1, payload: b"x" }]),
            Err(FormatError::Block(BlockError::Io))
        );
        assert_eq!(volume.info(), before);
    }

    #[test]
    fn journal_full_and_payload_bounds_are_explicit() {
        let mut store = CrashStore::<4>::new();
        let mut volume = Volume::format(&mut store).unwrap();
        let records = [JournalRecord { kind: 1, payload: b"x" }];
        assert_eq!(volume.commit(&mut store, &records).unwrap(), 1);
        assert_eq!(volume.commit(&mut store, &records), Err(FormatError::JournalFull));

        let mut large_store = CrashStore::<BLOCKS>::new();
        let mut large_volume = Volume::format(&mut large_store).unwrap();
        let large = [0u8; MAX_RECORD_PAYLOAD_BYTES + 1];
        assert_eq!(
            large_volume.commit(&mut large_store, &[JournalRecord { kind: 1, payload: &large }]),
            Err(FormatError::PayloadTooLarge)
        );
    }

    #[test]
    fn transaction_record_limit_is_explicit() {
        let mut store = CrashStore::<BLOCKS>::new();
        let mut volume = Volume::format(&mut store).unwrap();
        let records = [JournalRecord { kind: 1, payload: b"x" }; MAX_RECORDS_PER_TRANSACTION + 1];
        assert_eq!(volume.commit(&mut store, &records), Err(FormatError::TransactionTooLarge));
    }

    #[test]
    fn commit_marker_kind_is_reserved_for_internal_records() {
        let mut store = CrashStore::<BLOCKS>::new();
        let mut volume = Volume::format(&mut store).unwrap();
        let records = [JournalRecord { kind: JOURNAL_COMMIT_KIND, payload: b"user" }];

        assert_eq!(volume.commit(&mut store, &records), Err(FormatError::InvalidRequest));
    }
}
