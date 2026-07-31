#![no_std]

extern crate alloc;

use alloc::{collections::BTreeMap, vec, vec::Vec};
use logos_abi::{MAX_OBJECT_NAME, NamespaceId, VersionSelector};

pub const SECTOR_SIZE: usize = 512;
const SUPERBLOCKS: usize = 2;
const SUPER_MAGIC: &[u8; 4] = b"LGST";
const RECORD_MAGIC: &[u8; 4] = b"RECD";
const COMMIT_MAGIC: &[u8; 4] = b"CMIT";

pub trait SectorBackend {
    fn sectors(&self) -> usize;
    fn read(&self, sector: usize, output: &mut [u8; SECTOR_SIZE]) -> Result<(), Error>;
    fn write(&mut self, sector: usize, input: &[u8; SECTOR_SIZE]) -> Result<(), Error>;
    fn flush(&mut self) -> Result<(), Error>;
}

pub struct MemoryBackend {
    bytes: Vec<u8>,
}

impl MemoryBackend {
    pub fn zeroed(sectors: usize) -> Result<Self, Error> {
        (sectors >= 10)
            .then(|| Self { bytes: vec![0; sectors * SECTOR_SIZE] })
            .ok_or(Error::Invalid)
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

impl SectorBackend for MemoryBackend {
    fn sectors(&self) -> usize {
        self.bytes.len() / SECTOR_SIZE
    }

    fn read(&self, sector: usize, output: &mut [u8; SECTOR_SIZE]) -> Result<(), Error> {
        let start = sector.checked_mul(SECTOR_SIZE).ok_or(Error::Invalid)?;
        let bytes = self.bytes.get(start..start + SECTOR_SIZE).ok_or(Error::Invalid)?;
        output.copy_from_slice(bytes);
        Ok(())
    }

    fn write(&mut self, sector: usize, input: &[u8; SECTOR_SIZE]) -> Result<(), Error> {
        let start = sector.checked_mul(SECTOR_SIZE).ok_or(Error::Invalid)?;
        let bytes = self.bytes.get_mut(start..start + SECTOR_SIZE).ok_or(Error::Invalid)?;
        bytes.copy_from_slice(input);
        Ok(())
    }

    fn flush(&mut self) -> Result<(), Error> {
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Invalid,
    Corrupt,
    Full,
    NotFound,
    Interrupted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Recovery {
    Clean,
    Incomplete,
    Corrupt,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct Key {
    namespace: u32,
    name: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Location {
    offset: usize,
    length: usize,
    version: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct Versions {
    current: Option<Location>,
    previous: Option<Location>,
}

pub struct Store<B = MemoryBackend> {
    disk: Vec<u8>,
    backend: B,
    arena: usize,
    generation: u64,
    tail: usize,
    versions: BTreeMap<Key, Versions>,
    recovery: Recovery,
}

impl Store<MemoryBackend> {
    pub fn format(sectors: usize) -> Result<Self, Error> {
        Self::format_with_backend(MemoryBackend::zeroed(sectors)?)
    }

    pub fn recover(disk: Vec<u8>) -> Result<Self, Error> {
        let backend = MemoryBackend { bytes: disk };
        Self::recover_backend(backend)
    }

    pub fn into_disk(self) -> Vec<u8> {
        self.backend.into_bytes()
    }
}

impl<B: SectorBackend> Store<B> {
    pub fn format_with_backend(mut backend: B) -> Result<Self, Error> {
        if backend.sectors() < 10 {
            return Err(Error::Invalid);
        }
        backend.write(0, &encode_superblock(1, 0))?;
        backend.flush()?;
        Self::recover_backend(backend)
    }

    pub fn recover_backend(backend: B) -> Result<Self, Error> {
        if backend.sectors() < 10 {
            return Err(Error::Invalid);
        }
        let mut disk = vec![0; backend.sectors() * SECTOR_SIZE];
        for sector in 0..backend.sectors() {
            let mut block = [0; SECTOR_SIZE];
            backend.read(sector, &mut block)?;
            disk[sector * SECTOR_SIZE..][..SECTOR_SIZE].copy_from_slice(&block);
        }
        let selected = (0..SUPERBLOCKS)
            .filter_map(|slot| decode_superblock(&disk[slot * SECTOR_SIZE..][..SECTOR_SIZE]))
            .max_by_key(|(generation, _)| *generation)
            .ok_or(Error::Corrupt)?;
        let (generation, arena) = selected;
        let mut store = Self {
            disk,
            backend,
            arena,
            generation,
            tail: 0,
            versions: BTreeMap::new(),
            recovery: Recovery::Clean,
        };
        store.scan()?;
        Ok(store)
    }

    pub fn disk(&self) -> &[u8] {
        &self.disk
    }

    pub const fn recovery(&self) -> Recovery {
        self.recovery
    }

    pub fn interruption_points(payload_length: usize) -> usize {
        record_sectors(payload_length) + 2
    }

    pub fn replace(
        &mut self,
        namespace: NamespaceId,
        name: &[u8],
        payload: &[u8],
    ) -> Result<u64, Error> {
        self.replace_with_cut(namespace, name, payload, None)
    }

    pub fn replace_with_cut(
        &mut self,
        namespace: NamespaceId,
        name: &[u8],
        payload: &[u8],
        cut: Option<usize>,
    ) -> Result<u64, Error> {
        validate_name(name)?;
        let key = Key { namespace: namespace.0, name: name.into() };
        let version = self
            .versions
            .get(&key)
            .and_then(|versions| versions.current)
            .map_or(1, |current| current.version.saturating_add(1));
        let record = encode_record(namespace.0, name, version, payload)?;
        if self.tail + record.len() > self.arena_len() {
            self.compact(None)?;
        }
        if self.tail + record.len() > self.arena_len() {
            return Err(Error::Full);
        }
        let start = self.arena_start() + self.tail;
        let mut step = 0;
        let sectors = record.len() / SECTOR_SIZE;
        for (index, sector) in record.chunks_exact(SECTOR_SIZE).enumerate() {
            self.write_sector(start / SECTOR_SIZE + index, sector.try_into().unwrap())?;
            step += 1;
            if cut == Some(step) {
                return Err(Error::Interrupted);
            }
            if index + 2 == sectors {
                self.backend.flush()?;
                step += 1; // payload flush
                if cut == Some(step) {
                    return Err(Error::Interrupted);
                }
            }
        }
        self.backend.flush()?;
        step += 1; // commit flush
        if cut == Some(step) {
            return Err(Error::Interrupted);
        }
        self.scan()?;
        Ok(version)
    }

    pub fn read(
        &self,
        namespace: NamespaceId,
        name: &[u8],
        selector: VersionSelector,
    ) -> Result<&[u8], Error> {
        validate_name(name)?;
        let key = Key { namespace: namespace.0, name: name.into() };
        let versions = self.versions.get(&key).ok_or(Error::NotFound)?;
        let location = match selector {
            VersionSelector::None => return Err(Error::Invalid),
            VersionSelector::Current => versions.current,
            VersionSelector::Previous => versions.previous,
        }
        .ok_or(Error::NotFound)?;
        Ok(&self.disk[location.offset..location.offset + location.length])
    }

    pub fn compact(&mut self, cut: Option<usize>) -> Result<(), Error> {
        let target = 1 - self.arena;
        let target_start = self.arena_start_for(target);
        let arena_len = self.arena_len();
        for sector in target_start / SECTOR_SIZE..(target_start + arena_len) / SECTOR_SIZE {
            self.write_sector(sector, &[0; SECTOR_SIZE])?;
        }
        let mut records = Vec::new();
        for (key, versions) in &self.versions {
            for location in [versions.previous, versions.current].into_iter().flatten() {
                records.push(encode_record(
                    key.namespace,
                    &key.name,
                    location.version,
                    &self.disk[location.offset..location.offset + location.length],
                )?);
            }
        }
        let required: usize = records.iter().map(Vec::len).sum();
        if required > arena_len {
            return Err(Error::Full);
        }
        let mut cursor = target_start;
        let mut step = 0;
        for record in records {
            for sector in record.chunks_exact(SECTOR_SIZE) {
                self.write_sector(cursor / SECTOR_SIZE, sector.try_into().unwrap())?;
                cursor += SECTOR_SIZE;
                step += 1;
                if cut == Some(step) {
                    return Err(Error::Interrupted);
                }
            }
        }
        self.backend.flush()?;
        step += 1; // arena flush
        if cut == Some(step) {
            return Err(Error::Interrupted);
        }
        let generation = self.generation.saturating_add(1);
        let superblock = encode_superblock(generation, target);
        let slot = generation as usize % SUPERBLOCKS;
        let offset = slot * SECTOR_SIZE;
        self.write_sector(offset / SECTOR_SIZE, &superblock)?;
        step += 1;
        if cut == Some(step) {
            return Err(Error::Interrupted);
        }
        self.backend.flush()?;
        step += 1; // superblock flush
        if cut == Some(step) {
            return Err(Error::Interrupted);
        }
        self.arena = target;
        self.generation = generation;
        self.scan()
    }

    fn scan(&mut self) -> Result<(), Error> {
        self.versions.clear();
        self.tail = 0;
        self.recovery = Recovery::Clean;
        let start = self.arena_start();
        let end = start + self.arena_len();
        while start + self.tail + SECTOR_SIZE <= end {
            let offset = start + self.tail;
            let header = &self.disk[offset..offset + SECTOR_SIZE];
            if &header[..4] != RECORD_MAGIC {
                break;
            }
            let sectors = read_u32(header, 4)? as usize;
            if sectors < 2 || offset + sectors * SECTOR_SIZE > end {
                self.recovery = Recovery::Incomplete;
                break;
            }
            if crc32c(&header[..SECTOR_SIZE - 4]) != read_u32(header, SECTOR_SIZE - 4)? {
                self.recovery = Recovery::Incomplete;
                break;
            }
            let name_len = usize::from(header[20]);
            let payload_len = read_u64(header, 24)? as usize;
            if name_len == 0 || name_len > MAX_OBJECT_NAME {
                self.recovery = Recovery::Corrupt;
                break;
            }
            let commit_offset = offset + (sectors - 1) * SECTOR_SIZE;
            let commit = &self.disk[commit_offset..commit_offset + SECTOR_SIZE];
            if &commit[..4] != COMMIT_MAGIC {
                self.recovery = Recovery::Incomplete;
                break;
            }
            if crc32c(&self.disk[offset..commit_offset]) != read_u32(commit, 4)? {
                self.recovery = Recovery::Corrupt;
                break;
            }
            let payload_offset = offset + SECTOR_SIZE;
            if payload_offset + payload_len > commit_offset {
                self.recovery = Recovery::Corrupt;
                break;
            }
            let key =
                Key { namespace: read_u32(header, 16)?, name: header[32..32 + name_len].into() };
            let location = Location {
                offset: payload_offset,
                length: payload_len,
                version: read_u64(header, 8)?,
            };
            let versions = self.versions.entry(key).or_default();
            if versions.current.is_none_or(|current| location.version > current.version) {
                versions.previous = versions.current;
                versions.current = Some(location);
            }
            self.tail += sectors * SECTOR_SIZE;
        }
        Ok(())
    }

    fn arena_start(&self) -> usize {
        self.arena_start_for(self.arena)
    }

    fn arena_start_for(&self, arena: usize) -> usize {
        SUPERBLOCKS * SECTOR_SIZE + arena * self.arena_len()
    }

    fn arena_len(&self) -> usize {
        ((self.disk.len() - SUPERBLOCKS * SECTOR_SIZE) / 2 / SECTOR_SIZE) * SECTOR_SIZE
    }

    fn write_sector(&mut self, sector: usize, block: &[u8; SECTOR_SIZE]) -> Result<(), Error> {
        let offset = sector.checked_mul(SECTOR_SIZE).ok_or(Error::Invalid)?;
        self.disk
            .get_mut(offset..offset + SECTOR_SIZE)
            .ok_or(Error::Invalid)?
            .copy_from_slice(block);
        self.backend.write(sector, block)
    }
}

fn validate_name(name: &[u8]) -> Result<(), Error> {
    if name.is_empty() || name.len() > MAX_OBJECT_NAME || core::str::from_utf8(name).is_err() {
        Err(Error::Invalid)
    } else {
        Ok(())
    }
}

fn record_sectors(payload_length: usize) -> usize {
    2 + payload_length.div_ceil(SECTOR_SIZE)
}

fn encode_record(
    namespace: u32,
    name: &[u8],
    version: u64,
    payload: &[u8],
) -> Result<Vec<u8>, Error> {
    validate_name(name)?;
    let sectors = record_sectors(payload.len());
    let mut record = vec![0; sectors.checked_mul(SECTOR_SIZE).ok_or(Error::Full)?];
    record[..4].copy_from_slice(RECORD_MAGIC);
    write_u32(&mut record, 4, sectors as u32);
    write_u64(&mut record, 8, version);
    write_u32(&mut record, 16, namespace);
    record[20] = name.len() as u8;
    write_u64(&mut record, 24, payload.len() as u64);
    record[32..32 + name.len()].copy_from_slice(name);
    let header_crc = crc32c(&record[..SECTOR_SIZE - 4]);
    write_u32(&mut record, SECTOR_SIZE - 4, header_crc);
    record[SECTOR_SIZE..SECTOR_SIZE + payload.len()].copy_from_slice(payload);
    let commit_offset = (sectors - 1) * SECTOR_SIZE;
    let record_crc = crc32c(&record[..commit_offset]);
    record[commit_offset..commit_offset + 4].copy_from_slice(COMMIT_MAGIC);
    write_u32(&mut record, commit_offset + 4, record_crc);
    Ok(record)
}

fn encode_superblock(generation: u64, arena: usize) -> [u8; SECTOR_SIZE] {
    let mut block = [0; SECTOR_SIZE];
    block[..4].copy_from_slice(SUPER_MAGIC);
    write_u64(&mut block, 8, generation);
    block[16] = arena as u8;
    let crc = crc32c(&block[..SECTOR_SIZE - 4]);
    write_u32(&mut block, SECTOR_SIZE - 4, crc);
    block
}

fn decode_superblock(block: &[u8]) -> Option<(u64, usize)> {
    let arena = usize::from(*block.get(16)?);
    if &block[..4] != SUPER_MAGIC
        || arena >= 2
        || read_u32(block, SECTOR_SIZE - 4).ok()? != crc32c(&block[..SECTOR_SIZE - 4])
    {
        return None;
    }
    Some((read_u64(block, 8).ok()?, arena))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, Error> {
    Ok(u32::from_le_bytes(bytes.get(offset..offset + 4).ok_or(Error::Corrupt)?.try_into().unwrap()))
}

fn read_u64(bytes: &[u8], offset: usize) -> Result<u64, Error> {
    Ok(u64::from_le_bytes(bytes.get(offset..offset + 8).ok_or(Error::Corrupt)?.try_into().unwrap()))
}

fn write_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn write_u64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

pub fn crc32c(bytes: &[u8]) -> u32 {
    !bytes.iter().fold(!0u32, |mut crc, byte| {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0x82f6_3b78 & 0u32.wrapping_sub(crc & 1));
        }
        crc
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const NS: NamespaceId = NamespaceId(7);

    #[test]
    fn interrupted_replace_recovers_old_or_new() {
        let mut base = Store::format(32).unwrap();
        base.replace(NS, b"history", b"old").unwrap();
        let disk = base.into_disk();
        for cut in 1..=Store::<MemoryBackend>::interruption_points(700) {
            let mut store = Store::recover(disk.clone()).unwrap();
            assert_eq!(
                store.replace_with_cut(NS, b"history", &vec![b'n'; 700], Some(cut)),
                Err(Error::Interrupted)
            );
            let recovered = Store::recover(store.into_disk()).unwrap();
            let value = recovered.read(NS, b"history", VersionSelector::Current).unwrap();
            assert!(value == b"old" || value == vec![b'n'; 700]);
        }
    }

    #[test]
    fn keeps_current_and_previous_across_compaction() {
        let mut store = Store::format(32).unwrap();
        store.replace(NS, b"history", b"one").unwrap();
        store.replace(NS, b"history", b"two").unwrap();
        store.compact(None).unwrap();
        assert_eq!(store.read(NS, b"history", VersionSelector::Current).unwrap(), b"two");
        assert_eq!(store.read(NS, b"history", VersionSelector::Previous).unwrap(), b"one");
    }

    #[test]
    fn rejects_bad_names_and_detects_committed_corruption() {
        let mut store = Store::format(32).unwrap();
        assert_eq!(store.replace(NS, b"", b"value"), Err(Error::Invalid));
        store.replace(NS, b"object", b"value").unwrap();
        let mut disk = store.into_disk();
        disk[3 * SECTOR_SIZE] ^= 1;
        let recovered = Store::recover(disk).unwrap();
        assert_eq!(recovered.recovery(), Recovery::Corrupt);
        assert_eq!(recovered.read(NS, b"object", VersionSelector::Current), Err(Error::NotFound));
    }

    #[test]
    fn interrupted_compaction_keeps_selected_arena() {
        let mut base = Store::format(32).unwrap();
        base.replace(NS, b"history", b"one").unwrap();
        base.replace(NS, b"history", b"two").unwrap();
        let disk = base.into_disk();
        let mut completed = false;
        for cut in 1..20 {
            let mut store = Store::recover(disk.clone()).unwrap();
            match store.compact(Some(cut)) {
                Err(Error::Interrupted) => {
                    let recovered = Store::recover(store.into_disk()).unwrap();
                    assert_eq!(
                        recovered.read(NS, b"history", VersionSelector::Current).unwrap(),
                        b"two"
                    );
                }
                Ok(()) => {
                    completed = true;
                    break;
                }
                result => panic!("unexpected compaction result: {result:?}"),
            }
        }
        assert!(completed);
    }

    #[test]
    fn crc32c_matches_standard_vector() {
        assert_eq!(crc32c(b"123456789"), 0xe306_9283);
    }
}
