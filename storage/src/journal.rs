use crate::{BLOCK_BYTES, Block, BlockError, BlockIndex, BlockStore};

pub const LEGACY_FORMAT_VERSION: u16 = 1;
pub const FORMAT_VERSION: u16 = 2;
/// Reserved record kind used only for internal transaction commit markers.
pub const JOURNAL_COMMIT_KIND: u16 = u16::MAX;
/// Durable marker describing the exact transaction window after the published head.
pub const JOURNAL_INTENT_KIND: u16 = u16::MAX - 1;
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
    format_version: u16,
}

#[derive(Clone, Copy)]
struct Superblock {
    info: VolumeInfo,
    format_version: u16,
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

        let info = VolumeInfo {
            generation: 1,
            journal_start: JOURNAL_START,
            journal_end: block_count,
            journal_head: JOURNAL_START,
            journal_tail: JOURNAL_START,
            root_transaction_id: 0,
        };

        write_superblock(store, SUPERBLOCK_A, info, FORMAT_VERSION)?;
        store.flush()?;
        write_superblock(store, SUPERBLOCK_B, info, FORMAT_VERSION)?;
        store.flush()?;

        Ok(Self { info, active_superblock: 1, format_version: FORMAT_VERSION })
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
                if b.info.generation > a.info.generation {
                    Ok(Self {
                        info: b.info,
                        active_superblock: 1,
                        format_version: b.format_version,
                    })
                } else {
                    Ok(Self {
                        info: a.info,
                        active_superblock: 0,
                        format_version: a.format_version,
                    })
                }
            }
            (Ok(Some(superblock)), _) => Ok(Self {
                info: superblock.info,
                active_superblock: 0,
                format_version: superblock.format_version,
            }),
            (_, Ok(Some(superblock))) => Ok(Self {
                info: superblock.info,
                active_superblock: 1,
                format_version: superblock.format_version,
            }),
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
        if records
            .iter()
            .any(|record| matches!(record.kind, JOURNAL_COMMIT_KIND | JOURNAL_INTENT_KIND))
        {
            return Err(FormatError::InvalidRequest);
        }

        if self.format_version == LEGACY_FORMAT_VERSION {
            return self.commit_legacy(store, records);
        }

        self.commit_with_intent(store, records)
    }

    fn commit_legacy<B: BlockStore>(
        &mut self,
        store: &mut B,
        records: &[JournalRecord<'_>],
    ) -> Result<u64, FormatError> {
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
            let block = encode_record(
                LEGACY_FORMAT_VERSION,
                transaction_id,
                sequence as u32,
                record.kind,
                record.payload,
            )?;
            store.write_block(BlockIndex::new(self.info.journal_head + sequence as u64), &block)?;
        }

        let commit = encode_record(
            LEGACY_FORMAT_VERSION,
            transaction_id,
            records.len() as u32,
            JOURNAL_COMMIT_KIND,
            &[],
        )?;
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
        write_superblock(store, next_slot, next_info, LEGACY_FORMAT_VERSION)?;
        store.flush()?;

        self.info = next_info;
        self.active_superblock ^= 1;
        Ok(transaction_id)
    }

    fn commit_with_intent<B: BlockStore>(
        &mut self,
        store: &mut B,
        records: &[JournalRecord<'_>],
    ) -> Result<u64, FormatError> {
        let required_blocks =
            (records.len() as u64).checked_add(2).ok_or(FormatError::JournalFull)?;
        let next_head =
            self.info.journal_head.checked_add(required_blocks).ok_or(FormatError::JournalFull)?;
        if next_head > self.info.journal_end {
            return Err(FormatError::JournalFull);
        }

        let transaction_id =
            self.info.root_transaction_id.checked_add(1).ok_or(FormatError::GenerationExhausted)?;
        let mut intent_payload = [0; 10];
        put_u16(&mut intent_payload, 0, records.len() as u16);
        put_u64(&mut intent_payload, 2, next_head);
        let intent =
            encode_record(FORMAT_VERSION, transaction_id, 0, JOURNAL_INTENT_KIND, &intent_payload)?;
        store.write_block(BlockIndex::new(self.info.journal_head), &intent)?;

        for (sequence, record) in records.iter().enumerate() {
            let block = encode_record(
                FORMAT_VERSION,
                transaction_id,
                sequence as u32,
                record.kind,
                record.payload,
            )?;
            store.write_block(
                BlockIndex::new(self.info.journal_head + 1 + sequence as u64),
                &block,
            )?;
        }

        let commit = encode_record(
            FORMAT_VERSION,
            transaction_id,
            records.len() as u32,
            JOURNAL_COMMIT_KIND,
            &[],
        )?;
        store.write_block(
            BlockIndex::new(self.info.journal_head + 1 + records.len() as u64),
            &commit,
        )?;
        store.flush()?;

        self.publish(store, next_head, transaction_id, FORMAT_VERSION)
    }

    fn publish<B: BlockStore>(
        &mut self,
        store: &mut B,
        journal_head: u64,
        root_transaction_id: u64,
        format_version: u16,
    ) -> Result<u64, FormatError> {
        let generation =
            self.info.generation.checked_add(1).ok_or(FormatError::GenerationExhausted)?;
        let next_info = VolumeInfo { generation, journal_head, root_transaction_id, ..self.info };
        let next_slot = if self.active_superblock == 0 { SUPERBLOCK_B } else { SUPERBLOCK_A };
        write_superblock(store, next_slot, next_info, format_version)?;
        store.flush()?;

        self.info = next_info;
        self.active_superblock ^= 1;
        self.format_version = format_version;
        Ok(root_transaction_id)
    }

    pub fn recover<B: BlockStore, S: ReplaySink>(
        &mut self,
        store: &mut B,
        sink: &mut S,
    ) -> Result<RecoverySummary, FormatError> {
        if self.format_version == LEGACY_FORMAT_VERSION {
            let summary = self.recover_legacy(store, sink)?;
            self.publish(
                store,
                self.info.journal_head,
                self.info.root_transaction_id,
                FORMAT_VERSION,
            )?;
            return Ok(summary);
        }

        self.recover_current(store, sink)
    }

    fn recover_legacy<B: BlockStore, S: ReplaySink>(
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

        for index in self.info.journal_tail..self.info.journal_end {
            let mut block = Block::zero();
            store.read_block(BlockIndex::new(index), &mut block)?;
            let record = match decode_record(&block) {
                Ok(Some(record)) => record,
                Ok(None) => {
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

            if record.format_version != LEGACY_FORMAT_VERSION {
                return Err(FormatError::UnsupportedVersion);
            }

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
            write_superblock(store, next_slot, next_info, LEGACY_FORMAT_VERSION)?;
            store.flush()?;
            self.info = next_info;
            self.active_superblock ^= 1;
        }

        Ok(RecoverySummary { committed_transactions, replayed_records })
    }

    fn recover_current<B: BlockStore, S: ReplaySink>(
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
        let mut index = self.info.journal_tail;

        while index < self.info.journal_head {
            let mut block = Block::zero();
            store.read_block(BlockIndex::new(index), &mut block)?;
            let Some(record) = decode_record(&block)? else {
                pending_len = 0;
                pending_transaction = 0;
                index += 1;
                continue;
            };

            if record.kind == JOURNAL_INTENT_KIND {
                if pending_len != 0 || record.format_version != FORMAT_VERSION {
                    return Err(FormatError::Corrupt);
                }
                let Some((next_index, transaction_id, count)) =
                    replay_intent(store, index, record, sink, last_transaction, true)?
                else {
                    return Err(FormatError::Corrupt);
                };
                last_transaction = transaction_id;
                committed_transactions += 1;
                replayed_records += count;
                index = next_index;
                continue;
            }

            if record.format_version != LEGACY_FORMAT_VERSION {
                return Err(FormatError::Corrupt);
            }
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
                    index += 1;
                    continue;
                }
                replay_pending(
                    sink,
                    record.transaction_id,
                    &pending,
                    pending_len,
                    &mut replayed_records,
                )?;
                committed_transactions += 1;
                last_transaction = record.transaction_id;
                pending_len = 0;
                pending_transaction = 0;
                index += 1;
                continue;
            }

            if pending_len == 0 {
                if record.sequence != 0 {
                    index += 1;
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
                if record.sequence != 0 {
                    index += 1;
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
            index += 1;
        }

        if pending_len != 0 || last_transaction < self.info.root_transaction_id {
            return Err(FormatError::Corrupt);
        }

        if self.info.journal_head < self.info.journal_end {
            let mut block = Block::zero();
            store.read_block(BlockIndex::new(self.info.journal_head), &mut block)?;
            match decode_record(&block) {
                Ok(Some(record)) if record.kind == JOURNAL_INTENT_KIND => {
                    if record.format_version != FORMAT_VERSION {
                        return Err(FormatError::UnsupportedVersion);
                    }
                    if let Some((next_index, transaction_id, count)) = replay_intent(
                        store,
                        self.info.journal_head,
                        record,
                        sink,
                        self.info.root_transaction_id,
                        false,
                    )? {
                        self.publish(store, next_index, transaction_id, FORMAT_VERSION)?;
                        committed_transactions += 1;
                        replayed_records += count;
                    }
                }
                Err(FormatError::UnsupportedVersion) => {
                    return Err(FormatError::UnsupportedVersion);
                }
                _ => {}
            }
        }

        Ok(RecoverySummary { committed_transactions, replayed_records })
    }
}

fn replay_pending<S: ReplaySink>(
    sink: &mut S,
    transaction_id: u64,
    pending: &[PendingRecord; MAX_RECORDS_PER_TRANSACTION],
    pending_len: usize,
    replayed_records: &mut u64,
) -> Result<(), FormatError> {
    for pending_record in pending.iter().take(pending_len) {
        sink.record(
            transaction_id,
            pending_record.kind,
            &pending_record.payload.as_bytes()[..pending_record.payload_len as usize],
        )
        .map_err(|_| FormatError::ReplayRejected)?;
        *replayed_records += 1;
    }
    Ok(())
}

fn replay_intent<B: BlockStore, S: ReplaySink>(
    store: &mut B,
    start: u64,
    intent: DecodedRecord,
    sink: &mut S,
    last_transaction: u64,
    strict: bool,
) -> Result<Option<(u64, u64, u64)>, FormatError> {
    if intent.transaction_id == 0
        || intent.transaction_id == MAX_TRANSACTION_ID
        || intent.transaction_id <= last_transaction
        || intent.sequence != 0
        || intent.payload_len != 10
    {
        return if strict { Err(FormatError::Corrupt) } else { Ok(None) };
    }
    let payload = intent.payload.as_bytes();
    let record_count = get_u16(payload, 0) as usize;
    if record_count > MAX_RECORDS_PER_TRANSACTION {
        return if strict { Err(FormatError::TransactionTooLarge) } else { Ok(None) };
    }
    let next_index = get_u64(payload, 2);
    let expected_next = start.checked_add(record_count as u64 + 2).ok_or(FormatError::Corrupt)?;
    if next_index != expected_next {
        return if strict { Err(FormatError::Corrupt) } else { Ok(None) };
    }
    let mut pending = [PendingRecord::EMPTY; MAX_RECORDS_PER_TRANSACTION];
    for (sequence, pending_slot) in pending.iter_mut().enumerate().take(record_count) {
        let mut block = Block::zero();
        store.read_block(BlockIndex::new(start + 1 + sequence as u64), &mut block)?;
        let Some(record) = decode_record(&block)? else {
            return if strict { Err(FormatError::Corrupt) } else { Ok(None) };
        };
        if record.format_version != FORMAT_VERSION
            || record.transaction_id != intent.transaction_id
            || record.sequence != sequence as u32
            || record.kind == JOURNAL_COMMIT_KIND
            || record.kind == JOURNAL_INTENT_KIND
        {
            return if strict { Err(FormatError::Corrupt) } else { Ok(None) };
        }
        *pending_slot = PendingRecord {
            kind: record.kind,
            payload_len: record.payload_len,
            payload: record.payload,
        };
    }
    let mut block = Block::zero();
    store.read_block(BlockIndex::new(start + 1 + record_count as u64), &mut block)?;
    let Some(commit) = decode_record(&block)? else {
        return if strict { Err(FormatError::Corrupt) } else { Ok(None) };
    };
    if commit.format_version != FORMAT_VERSION
        || commit.transaction_id != intent.transaction_id
        || commit.sequence != record_count as u32
        || commit.kind != JOURNAL_COMMIT_KIND
        || commit.payload_len != 0
    {
        return if strict { Err(FormatError::Corrupt) } else { Ok(None) };
    }
    let mut replayed_records = 0;
    replay_pending(sink, intent.transaction_id, &pending, record_count, &mut replayed_records)?;
    Ok(Some((next_index, intent.transaction_id, replayed_records)))
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
    format_version: u16,
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
    format_version: u16,
) -> Result<(), FormatError> {
    let mut block = Block::zero();
    let bytes = block.as_bytes_mut();
    bytes[..8].copy_from_slice(SUPERBLOCK_MAGIC);
    put_u16(bytes, 8, format_version);
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
) -> Result<Option<Superblock>, FormatError> {
    let mut block = Block::zero();
    store.read_block(index, &mut block)?;
    let bytes = block.as_bytes();
    if bytes.iter().all(|byte| *byte == 0) {
        return Ok(None);
    }
    if &bytes[..8] != SUPERBLOCK_MAGIC {
        return Err(FormatError::Corrupt);
    }
    let format_version = get_u16(bytes, 8);
    if !matches!(format_version, LEGACY_FORMAT_VERSION | FORMAT_VERSION) {
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
    Ok(Some(Superblock { info, format_version }))
}

fn encode_record(
    format_version: u16,
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
    put_u16(bytes, 4, format_version);
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
    let format_version = get_u16(bytes, 4);
    if !matches!(format_version, LEGACY_FORMAT_VERSION | FORMAT_VERSION) {
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
        format_version,
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

    fn downgrade_to_legacy<B: BlockStore>(store: &mut B, info: VolumeInfo) {
        write_superblock(store, SUPERBLOCK_A, info, LEGACY_FORMAT_VERSION).unwrap();
        store.flush().unwrap();
        write_superblock(store, SUPERBLOCK_B, info, LEGACY_FORMAT_VERSION).unwrap();
        store.flush().unwrap();
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
        let mut record = encode_record(LEGACY_FORMAT_VERSION, 1, 0, 7, b"future").unwrap();
        put_u16(record.as_bytes_mut(), 4, FORMAT_VERSION + 1);
        store.write_block(BlockIndex::new(JOURNAL_START), &record).unwrap();

        let mut info = volume.info();
        info.generation += 1;
        info.journal_head = JOURNAL_START + 1;
        info.root_transaction_id = 1;
        write_superblock(&mut store, SUPERBLOCK_A, info, LEGACY_FORMAT_VERSION).unwrap();
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
        let formatted = Volume::format(&mut store).unwrap();
        downgrade_to_legacy(&mut store, formatted.info());
        let mut volume = Volume::open(&mut store).unwrap();
        volume.commit(&mut store, &[JournalRecord { kind: 1, payload: b"first" }]).unwrap();
        let later = encode_record(LEGACY_FORMAT_VERSION, 2, 0, 2, b"later").unwrap();
        let later_commit =
            encode_record(LEGACY_FORMAT_VERSION, 2, 1, JOURNAL_COMMIT_KIND, &[]).unwrap();
        store.write_block(BlockIndex::new(5), &later).unwrap();
        store.write_block(BlockIndex::new(6), &later_commit).unwrap();

        let mut info = volume.info();
        info.generation += 1;
        info.journal_head = 7;
        info.root_transaction_id = 2;
        write_superblock(&mut store, SUPERBLOCK_B, info, LEGACY_FORMAT_VERSION).unwrap();
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
        write_superblock(&mut store, SUPERBLOCK_A, tampered, LEGACY_FORMAT_VERSION).unwrap();
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
        write_superblock(&mut store, SUPERBLOCK_A, info, LEGACY_FORMAT_VERSION).unwrap();
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
        let later = encode_record(LEGACY_FORMAT_VERSION, 1, 0, 1, b"later").unwrap();
        store.write_block(BlockIndex::new(3), &later).unwrap();

        let mut info = volume.info();
        info.generation += 1;
        info.journal_head += 2;
        write_superblock(&mut store, SUPERBLOCK_A, info, LEGACY_FORMAT_VERSION).unwrap();
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
        store.fail_write_after(3);
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
    fn current_recovery_probes_only_the_active_transaction_window() {
        let mut store = CrashStore::<32>::new();
        let mut volume = Volume::format(&mut store).unwrap();
        store.reset_reads();
        store.fail_write_after(3);
        assert_eq!(
            volume.commit(&mut store, &[JournalRecord { kind: 7, payload: b"durable" }]),
            Err(FormatError::Block(BlockError::Io))
        );
        store.power_loss();

        let mut reopened = Volume::open(&mut store).unwrap();
        store.reset_reads();
        let mut sink = Sink::new();
        assert_eq!(
            reopened.recover(&mut store, &mut sink).unwrap(),
            RecoverySummary { committed_transactions: 1, replayed_records: 1 }
        );
        assert!(store.reads <= 4, "recovery read {} blocks", store.reads);
    }

    #[test]
    fn duplicate_transactions_are_rejected_during_recovery() {
        let mut store = CrashStore::<16>::new();
        let formatted = Volume::format(&mut store).unwrap();
        downgrade_to_legacy(&mut store, formatted.info());
        let mut volume = Volume::open(&mut store).unwrap();
        volume.commit(&mut store, &[JournalRecord { kind: 1, payload: b"one" }]).unwrap();

        let duplicate = encode_record(LEGACY_FORMAT_VERSION, 1, 0, 1, b"one-again").unwrap();
        let duplicate_commit =
            encode_record(LEGACY_FORMAT_VERSION, 1, 1, JOURNAL_COMMIT_KIND, &[]).unwrap();
        let second = encode_record(LEGACY_FORMAT_VERSION, 2, 0, 2, b"two").unwrap();
        let second_commit =
            encode_record(LEGACY_FORMAT_VERSION, 2, 1, JOURNAL_COMMIT_KIND, &[]).unwrap();
        store.write_block(BlockIndex::new(4), &duplicate).unwrap();
        store.write_block(BlockIndex::new(5), &duplicate_commit).unwrap();
        store.write_block(BlockIndex::new(6), &second).unwrap();
        store.write_block(BlockIndex::new(7), &second_commit).unwrap();

        let mut info = volume.info();
        info.generation += 1;
        info.journal_head = 8;
        info.root_transaction_id = 2;
        write_superblock(&mut store, SUPERBLOCK_B, info, LEGACY_FORMAT_VERSION).unwrap();
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
        let mut store = CrashStore::<5>::new();
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
