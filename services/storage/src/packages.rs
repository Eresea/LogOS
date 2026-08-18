use logos_abi::ServiceId;
use logos_package::{
    MAX_PACKAGE_BYTES, PACKAGE_HEADER_BYTES, PackageError as FormatPackageError, PackageReader,
    ServicePackageHeader, validate_package,
};
use logos_storage::{BLOCK_BYTES, Block, BlockError};

pub const MAX_PACKAGE_RECORDS: usize = 8;
pub const MAX_PACKAGE_EXTENTS: usize = 8;
pub const MAX_PACKAGE_BLOCKS: usize = MAX_PACKAGE_BYTES.div_ceil(BLOCK_BYTES);
pub const PACKAGE_INSTALL_KIND: u16 = 0x0100;
pub const PACKAGE_RECORD_BYTES: usize = 152;
pub const PACKAGE_SNAPSHOT_BYTES: usize = 2 + MAX_PACKAGE_RECORDS * PACKAGE_RECORD_BYTES;

const _: () = assert!(PACKAGE_SNAPSHOT_BYTES <= logos_storage::CHECKPOINT_PAYLOAD_BYTES);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PackageCatalogError {
    Unsupported,
    Capacity,
    TooLarge,
    InvalidRequest,
    Stale,
    NoSpace,
    InvalidRecord,
    Block(BlockError),
    Format(FormatPackageError),
}

impl From<BlockError> for PackageCatalogError {
    fn from(error: BlockError) -> Self {
        Self::Block(error)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PackageExtent {
    pub start: u64,
    pub blocks: u32,
}

impl PackageExtent {
    const EMPTY: Self = Self { start: 0, blocks: 0 };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PackageHandle {
    pub service: ServiceId,
    pub generation: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PackageInfo {
    pub handle: PackageHandle,
    pub package_version: u32,
    pub bytes: u32,
    pub crc32c: u32,
    pub extents: [PackageExtent; MAX_PACKAGE_EXTENTS],
    pub extent_count: u8,
}

#[derive(Clone, Copy)]
struct PackageRecord {
    alive: bool,
    service: ServiceId,
    generation: u32,
    package_version: u32,
    bytes: u32,
    crc32c: u32,
    extents: [PackageExtent; MAX_PACKAGE_EXTENTS],
    extent_count: u8,
}

impl PackageRecord {
    const EMPTY: Self = Self {
        alive: false,
        service: ServiceId::Input,
        generation: 0,
        package_version: 0,
        bytes: 0,
        crc32c: 0,
        extents: [PackageExtent::EMPTY; MAX_PACKAGE_EXTENTS],
        extent_count: 0,
    };

    const fn info(self) -> PackageInfo {
        PackageInfo {
            handle: PackageHandle { service: self.service, generation: self.generation },
            package_version: self.package_version,
            bytes: self.bytes,
            crc32c: self.crc32c,
            extents: self.extents,
            extent_count: self.extent_count,
        }
    }
}

#[derive(Clone, Copy)]
pub struct PackageCatalog {
    records: [PackageRecord; MAX_PACKAGE_RECORDS],
}

impl PackageCatalog {
    pub const fn new() -> Self {
        Self { records: [PackageRecord::EMPTY; MAX_PACKAGE_RECORDS] }
    }

    pub fn lookup(&self, service: ServiceId) -> Option<PackageInfo> {
        self.records
            .iter()
            .find(|record| record.alive && record.service == service)
            .copied()
            .map(PackageRecord::info)
    }

    pub fn validate_handle(
        &self,
        handle: PackageHandle,
    ) -> Result<PackageInfo, PackageCatalogError> {
        let Some(info) = self.lookup(handle.service) else {
            return Err(PackageCatalogError::Stale);
        };
        if info.handle != handle {
            return Err(PackageCatalogError::Stale);
        }
        Ok(info)
    }

    pub(crate) fn next_generation(&self, service: ServiceId) -> Result<u32, PackageCatalogError> {
        self.lookup(service).map_or(Ok(1), |info| {
            info.handle.generation.checked_add(1).ok_or(PackageCatalogError::InvalidRequest)
        })
    }

    pub fn plan_install(
        &self,
        arena: (u64, u64),
        service: ServiceId,
        bytes: usize,
    ) -> Result<PackageInstall, PackageCatalogError> {
        if !(PACKAGE_HEADER_BYTES..=MAX_PACKAGE_BYTES).contains(&bytes) {
            return Err(PackageCatalogError::TooLarge);
        }
        if self.lookup(service).is_none() && !self.records.iter().any(|record| !record.alive) {
            return Err(PackageCatalogError::Capacity);
        }
        let blocks = bytes.div_ceil(BLOCK_BYTES);
        let mut install = PackageInstall {
            service,
            bytes: bytes as u32,
            blocks: blocks as u32,
            extents: [PackageExtent::EMPTY; MAX_PACKAGE_EXTENTS],
            extent_count: 0,
            written: [false; MAX_PACKAGE_BLOCKS],
        };
        let mut cursor = arena.0;
        let mut remaining = blocks as u64;
        while remaining != 0 && cursor < arena.1 {
            while cursor < arena.1 && self.block_used(cursor) {
                cursor += 1;
            }
            let start = cursor;
            while cursor < arena.1 && !self.block_used(cursor) && cursor - start < remaining {
                cursor += 1;
            }
            let length = cursor - start;
            if length == 0 {
                continue;
            }
            if install.extent_count as usize == MAX_PACKAGE_EXTENTS {
                return Err(PackageCatalogError::Capacity);
            }
            install.extents[install.extent_count as usize] =
                PackageExtent { start, blocks: length as u32 };
            install.extent_count += 1;
            remaining -= length;
        }
        if remaining != 0 {
            return Err(PackageCatalogError::NoSpace);
        }
        Ok(install)
    }

    fn block_used(&self, block: u64) -> bool {
        self.records.iter().any(|record| {
            record.alive
                && record.extents[..record.extent_count as usize].iter().any(|extent| {
                    block >= extent.start && block < extent.start + extent.blocks as u64
                })
        })
    }

    pub(crate) fn apply_record(
        &mut self,
        kind: u16,
        payload: &[u8],
        arena: Option<(u64, u64)>,
    ) -> Result<(), PackageCatalogError> {
        if kind != PACKAGE_INSTALL_KIND || payload.len() != PACKAGE_RECORD_BYTES {
            return Err(PackageCatalogError::InvalidRecord);
        }
        let record = decode_record(payload)?;
        let slot = self
            .records
            .iter()
            .position(|current| current.alive && current.service == record.service)
            .or_else(|| self.records.iter().position(|current| !current.alive))
            .ok_or(PackageCatalogError::Capacity)?;
        let arena = arena.ok_or(PackageCatalogError::Unsupported)?;
        validate_record_layout(&record, arena, &self.records, Some(slot))?;
        self.records[slot] = record;
        Ok(())
    }

    pub(crate) fn encode_snapshot(&self, output: &mut [u8]) -> Result<usize, PackageCatalogError> {
        if output.len() < PACKAGE_SNAPSHOT_BYTES {
            return Err(PackageCatalogError::InvalidRequest);
        }
        output[..PACKAGE_SNAPSHOT_BYTES].fill(0);
        output[0] = 1;
        output[1] = MAX_PACKAGE_RECORDS as u8;
        for (index, record) in self.records.iter().enumerate() {
            encode_record(record, &mut output[2 + index * PACKAGE_RECORD_BYTES..]);
        }
        Ok(PACKAGE_SNAPSHOT_BYTES)
    }

    pub(crate) fn restore_snapshot(
        &mut self,
        input: &[u8],
        arena: Option<(u64, u64)>,
    ) -> Result<(), PackageCatalogError> {
        if input.len() != PACKAGE_SNAPSHOT_BYTES
            || input[0] != 1
            || input[1] as usize != MAX_PACKAGE_RECORDS
        {
            return Err(PackageCatalogError::InvalidRecord);
        }
        let mut records = [PackageRecord::EMPTY; MAX_PACKAGE_RECORDS];
        for (index, record) in records.iter_mut().enumerate() {
            *record = decode_record(&input[2 + index * PACKAGE_RECORD_BYTES..])?;
        }
        for (index, record) in records.iter().enumerate() {
            if record.alive {
                let arena = arena.ok_or(PackageCatalogError::Unsupported)?;
                validate_record_layout(record, arena, &records, Some(index))?;
            }
        }
        *self = Self { records };
        Ok(())
    }
}

impl Default for PackageCatalog {
    fn default() -> Self {
        Self::new()
    }
}

pub struct PackageInstall {
    pub(crate) service: ServiceId,
    pub(crate) bytes: u32,
    pub(crate) blocks: u32,
    pub(crate) extents: [PackageExtent; MAX_PACKAGE_EXTENTS],
    pub(crate) extent_count: u8,
    written: [bool; MAX_PACKAGE_BLOCKS],
}

impl PackageInstall {
    pub fn service(&self) -> ServiceId {
        self.service
    }

    pub fn bytes(&self) -> usize {
        self.bytes as usize
    }

    pub fn write_offset_valid(&self, offset: usize, length: usize) -> bool {
        offset % BLOCK_BYTES == 0
            && length != 0
            && length <= BLOCK_BYTES
            && offset.checked_add(length).is_some_and(|end| end <= self.bytes as usize)
    }

    pub fn mark_written(&mut self, offset: usize) {
        self.written[offset / BLOCK_BYTES] = true;
    }

    pub fn complete(&self) -> bool {
        (0..self.blocks as usize).all(|index| self.written[index])
    }

    fn record(&self, header: ServicePackageHeader) -> PackageRecord {
        PackageRecord {
            alive: true,
            service: self.service,
            generation: 0,
            package_version: header.package_version,
            bytes: self.bytes,
            crc32c: header.payload_crc32c,
            extents: self.extents,
            extent_count: self.extent_count,
        }
    }

    pub(crate) fn logical_block(&self, index: usize) -> Option<u64> {
        if index >= self.blocks as usize {
            return None;
        }
        let mut remaining = index as u64;
        for extent in self.extents[..self.extent_count as usize].iter() {
            if remaining < extent.blocks as u64 {
                return Some(extent.start + remaining);
            }
            remaining -= extent.blocks as u64;
        }
        None
    }
}

impl PackageRecord {
    fn with_generation(mut self, generation: u32) -> Self {
        self.generation = generation;
        self
    }
}

pub(crate) fn encode_install_record(
    install: &PackageInstall,
    header: ServicePackageHeader,
    generation: u32,
    output: &mut [u8],
) -> Result<(), PackageCatalogError> {
    if output.len() < PACKAGE_RECORD_BYTES {
        return Err(PackageCatalogError::InvalidRequest);
    }
    encode_record(&install.record(header).with_generation(generation), output);
    Ok(())
}

pub(crate) fn validate_install<R: PackageReader>(
    reader: &mut R,
    service: ServiceId,
    scratch: &mut [u8],
) -> Result<ServicePackageHeader, PackageCatalogError> {
    validate_package(reader, service, logos_abi::ABI_VERSION, scratch)
        .map_err(PackageCatalogError::Format)
}

fn encode_record(record: &PackageRecord, output: &mut [u8]) {
    output[..PACKAGE_RECORD_BYTES].fill(0);
    output[0] = u8::from(record.alive);
    output[1] = record.service as u8;
    put_u32(output, 4, record.generation);
    put_u32(output, 8, record.package_version);
    put_u32(output, 12, record.bytes);
    put_u32(output, 16, record.crc32c);
    output[20] = record.extent_count;
    for (index, extent) in record.extents.iter().enumerate() {
        let offset = 24 + index * 16;
        put_u64(output, offset, extent.start);
        put_u32(output, offset + 8, extent.blocks);
    }
}

fn decode_record(input: &[u8]) -> Result<PackageRecord, PackageCatalogError> {
    if input.len() < PACKAGE_RECORD_BYTES || input[0] > 1 {
        return Err(PackageCatalogError::InvalidRecord);
    }
    let service = input[1]
        .checked_sub(1)
        .and_then(|raw| ServiceId::from_index(raw as usize))
        .ok_or(PackageCatalogError::InvalidRecord)?;
    let extent_count = input[20] as usize;
    if extent_count > MAX_PACKAGE_EXTENTS {
        return Err(PackageCatalogError::InvalidRecord);
    }
    let mut extents = [PackageExtent::EMPTY; MAX_PACKAGE_EXTENTS];
    for (index, extent) in extents.iter_mut().enumerate() {
        let offset = 24 + index * 16;
        *extent =
            PackageExtent { start: get_u64(input, offset), blocks: get_u32(input, offset + 8) };
        if index >= extent_count && *extent != PackageExtent::EMPTY {
            return Err(PackageCatalogError::InvalidRecord);
        }
        if extent.blocks == 0 && index < extent_count {
            return Err(PackageCatalogError::InvalidRecord);
        }
    }
    let generation = get_u32(input, 4);
    let bytes = get_u32(input, 12);
    if input[0] != 0
        && (generation == 0
            || !(PACKAGE_HEADER_BYTES..=MAX_PACKAGE_BYTES).contains(&(bytes as usize)))
    {
        return Err(PackageCatalogError::InvalidRecord);
    }
    if input[0] == 0
        && (generation != 0
            || bytes != 0
            || get_u32(input, 16) != 0
            || extent_count != 0
            || extents.iter().any(|extent| *extent != PackageExtent::EMPTY))
    {
        return Err(PackageCatalogError::InvalidRecord);
    }
    Ok(PackageRecord {
        alive: input[0] != 0,
        service,
        generation,
        package_version: get_u32(input, 8),
        bytes,
        crc32c: get_u32(input, 16),
        extents,
        extent_count: extent_count as u8,
    })
}

fn validate_record_layout(
    record: &PackageRecord,
    arena: (u64, u64),
    records: &[PackageRecord; MAX_PACKAGE_RECORDS],
    skip: Option<usize>,
) -> Result<(), PackageCatalogError> {
    if arena.1 <= arena.0 || !record.alive {
        return Err(PackageCatalogError::InvalidRecord);
    }
    let expected_blocks = (record.bytes as usize).div_ceil(BLOCK_BYTES) as u64;
    let mut total_blocks = 0u64;
    for (index, extent) in record.extents[..record.extent_count as usize].iter().enumerate() {
        let end = extent
            .start
            .checked_add(extent.blocks as u64)
            .ok_or(PackageCatalogError::InvalidRecord)?;
        if extent.start < arena.0 || end > arena.1 {
            return Err(PackageCatalogError::InvalidRecord);
        }
        total_blocks = total_blocks
            .checked_add(extent.blocks as u64)
            .ok_or(PackageCatalogError::InvalidRecord)?;
        for previous in &record.extents[..index] {
            let previous_end = previous
                .start
                .checked_add(previous.blocks as u64)
                .ok_or(PackageCatalogError::InvalidRecord)?;
            if extent.start < previous_end && previous.start < end {
                return Err(PackageCatalogError::InvalidRecord);
            }
        }
        for (other_index, other) in records.iter().enumerate() {
            if Some(other_index) == skip || !other.alive {
                continue;
            }
            for other_extent in &other.extents[..other.extent_count as usize] {
                let other_end = other_extent
                    .start
                    .checked_add(other_extent.blocks as u64)
                    .ok_or(PackageCatalogError::InvalidRecord)?;
                if extent.start < other_end && other_extent.start < end {
                    return Err(PackageCatalogError::InvalidRecord);
                }
            }
        }
    }
    if total_blocks != expected_blocks {
        return Err(PackageCatalogError::InvalidRecord);
    }
    Ok(())
}

struct InstallReader<'a, B> {
    store: &'a mut B,
    install: &'a PackageInstall,
}

impl<B: logos_storage::BlockStore> PackageReader for InstallReader<'_, B> {
    fn len(&self) -> usize {
        self.install.bytes()
    }

    fn read(&mut self, offset: usize, output: &mut [u8]) -> Result<usize, FormatPackageError> {
        if offset.checked_add(output.len()).is_none() || offset + output.len() > self.len() {
            return Err(FormatPackageError::Reader);
        }
        let mut copied = 0;
        while copied < output.len() {
            let absolute = offset + copied;
            let block_index = absolute / BLOCK_BYTES;
            let block_offset = absolute % BLOCK_BYTES;
            let amount = (output.len() - copied).min(BLOCK_BYTES - block_offset);
            let Some(physical) = self.install.logical_block(block_index) else {
                return Err(FormatPackageError::Reader);
            };
            let mut block = Block::zero();
            self.store
                .read_block(logos_storage::BlockIndex::new(physical), &mut block)
                .map_err(|_| FormatPackageError::Reader)?;
            output[copied..copied + amount]
                .copy_from_slice(&block.as_bytes()[block_offset..block_offset + amount]);
            copied += amount;
        }
        Ok(output.len())
    }
}

pub(crate) fn validate_install_on_store<B: logos_storage::BlockStore>(
    store: &mut B,
    install: &PackageInstall,
    scratch: &mut [u8],
) -> Result<ServicePackageHeader, PackageCatalogError> {
    let mut reader = InstallReader { store, install };
    validate_install(&mut reader, install.service, scratch)
}

fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_u64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
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

#[cfg(test)]
mod tests {
    use super::*;

    fn record(start: u64, blocks: u32, bytes: u32) -> PackageRecord {
        let mut record = PackageRecord::EMPTY;
        record.alive = true;
        record.service = ServiceId::Input;
        record.generation = 1;
        record.bytes = bytes;
        record.extent_count = 1;
        record.extents[0] = PackageExtent { start, blocks };
        record
    }

    #[test]
    fn restore_rejects_invalid_package_geometry() {
        let mut catalog = PackageCatalog::new();
        let mut snapshot = [0; PACKAGE_SNAPSHOT_BYTES];
        let mut records = [PackageRecord::EMPTY; MAX_PACKAGE_RECORDS];
        records[0] = record(9, 1, PACKAGE_HEADER_BYTES as u32);
        catalog.records = records;
        catalog.encode_snapshot(&mut snapshot).unwrap();
        assert_eq!(
            PackageCatalog::new().restore_snapshot(&snapshot, Some((10, 20))),
            Err(PackageCatalogError::InvalidRecord)
        );

        records[0] = record(10, 2, PACKAGE_HEADER_BYTES as u32);
        catalog.records = records;
        catalog.encode_snapshot(&mut snapshot).unwrap();
        assert_eq!(
            PackageCatalog::new().restore_snapshot(&snapshot, Some((10, 20))),
            Err(PackageCatalogError::InvalidRecord)
        );
    }

    #[test]
    fn restore_and_replay_reject_overlapping_or_overflowing_extents() {
        let mut catalog = PackageCatalog::new();
        let mut snapshot = [0; PACKAGE_SNAPSHOT_BYTES];
        let mut records = [PackageRecord::EMPTY; MAX_PACKAGE_RECORDS];
        records[0] = record(10, 1, PACKAGE_HEADER_BYTES as u32);
        records[1] = record(10, 1, PACKAGE_HEADER_BYTES as u32);
        catalog.records = records;
        catalog.encode_snapshot(&mut snapshot).unwrap();
        assert_eq!(
            PackageCatalog::new().restore_snapshot(&snapshot, Some((10, 20))),
            Err(PackageCatalogError::InvalidRecord)
        );

        let overflowing = record(u64::MAX, 1, PACKAGE_HEADER_BYTES as u32);
        let mut payload = [0; PACKAGE_RECORD_BYTES];
        encode_record(&overflowing, &mut payload);
        assert_eq!(
            PackageCatalog::new().apply_record(PACKAGE_INSTALL_KIND, &payload, Some((10, 20))),
            Err(PackageCatalogError::InvalidRecord)
        );
    }
}
