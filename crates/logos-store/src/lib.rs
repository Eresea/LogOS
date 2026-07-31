#![no_std]

#[cfg(test)]
extern crate alloc;

#[cfg(test)]
use alloc::{vec, vec::Vec};
use logos_abi::{MAX_OBJECT_NAME, NamespaceId, PAGE_SIZE, VersionSelector};

pub const SECTOR_SIZE: usize = 512;
const SUPERBLOCKS: usize = 2;
const SUPER_MAGIC: &[u8; 4] = b"LGST";
const RECORD_MAGIC: &[u8; 4] = b"RECD";
const COMMIT_MAGIC: &[u8; 4] = b"CMIT";
const FORMAT_VERSION: u32 = 1;
const MAX_OBJECTS: usize = 32;
const SUPER_VERSION_OFFSET: usize = 4;
const RECORD_VERSION_OFFSET: usize = 96;
const CHECKSUM_OFFSET: usize = SECTOR_SIZE - 4;

pub trait SectorBackend {
    fn sectors(&self) -> usize;
    fn read(&mut self, sector: usize, output: &mut [u8; SECTOR_SIZE]) -> Result<(), Error>;
    fn write(&mut self, sector: usize, input: &[u8; SECTOR_SIZE]) -> Result<(), Error>;
    fn flush(&mut self) -> Result<(), Error>;
}

#[cfg(test)]
pub struct MemoryBackend {
    bytes: Vec<u8>,
}

#[cfg(test)]
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

#[cfg(test)]
impl SectorBackend for MemoryBackend {
    fn sectors(&self) -> usize {
        self.bytes.len() / SECTOR_SIZE
    }

    fn read(&mut self, sector: usize, output: &mut [u8; SECTOR_SIZE]) -> Result<(), Error> {
        let start = sector.checked_mul(SECTOR_SIZE).ok_or(Error::Invalid)?;
        output.copy_from_slice(self.bytes.get(start..start + SECTOR_SIZE).ok_or(Error::Invalid)?);
        Ok(())
    }

    fn write(&mut self, sector: usize, input: &[u8; SECTOR_SIZE]) -> Result<(), Error> {
        let start = sector.checked_mul(SECTOR_SIZE).ok_or(Error::Invalid)?;
        self.bytes
            .get_mut(start..start + SECTOR_SIZE)
            .ok_or(Error::Invalid)?
            .copy_from_slice(input);
        Ok(())
    }

    fn flush(&mut self) -> Result<(), Error> {
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Invalid,
    Io,
    TimedOut,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Location {
    sector: usize,
    length: usize,
    version: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct Versions {
    current: Option<Location>,
    previous: Option<Location>,
}

#[derive(Clone, Copy)]
struct Entry {
    namespace: u32,
    name: [u8; MAX_OBJECT_NAME],
    name_length: u8,
    versions: Versions,
}

impl Entry {
    const EMPTY: Self = Self {
        namespace: 0,
        name: [0; MAX_OBJECT_NAME],
        name_length: 0,
        versions: Versions { current: None, previous: None },
    };

    fn matches(&self, namespace: u32, name: &[u8]) -> bool {
        self.name_length as usize == name.len()
            && self.namespace == namespace
            && self.name[..name.len()] == *name
    }
}

pub struct Store<B> {
    backend: B,
    sectors: usize,
    arena: usize,
    generation: u64,
    tail: usize,
    entries: [Entry; MAX_OBJECTS],
    recovery: Recovery,
}

#[cfg(test)]
impl Store<MemoryBackend> {
    pub fn format(sectors: usize) -> Result<Self, Error> {
        Self::format_with_backend(MemoryBackend::zeroed(sectors)?)
    }

    pub fn recover(disk: Vec<u8>) -> Result<Self, Error> {
        Self::recover_backend(MemoryBackend { bytes: disk })
    }

    pub fn into_disk(self) -> Vec<u8> {
        self.backend.into_bytes()
    }
}

impl<B: SectorBackend> Store<B> {
    pub fn format_with_backend(mut backend: B) -> Result<Self, Error> {
        validate_sectors(backend.sectors())?;
        let mut first = [0; SECTOR_SIZE];
        let mut second = [0; SECTOR_SIZE];
        backend.read(0, &mut first)?;
        backend.read(1, &mut second)?;
        if !is_zeroed(&first) || !is_zeroed(&second) {
            return Err(Error::Corrupt);
        }
        backend.write(0, &encode_superblock(1, 0))?;
        backend.flush()?;
        Self::recover_backend(backend)
    }

    pub fn recover_backend(mut backend: B) -> Result<Self, Error> {
        let sectors = backend.sectors();
        validate_sectors(sectors)?;
        let mut first = [0; SECTOR_SIZE];
        let mut second = [0; SECTOR_SIZE];
        backend.read(0, &mut first)?;
        backend.read(1, &mut second)?;
        let selected = [first, second]
            .iter()
            .filter_map(decode_superblock)
            .max_by_key(|(generation, _)| *generation)
            .ok_or(Error::Corrupt)?;
        let mut store = Self {
            backend,
            sectors,
            arena: selected.1,
            generation: selected.0,
            tail: 0,
            entries: [Entry::EMPTY; MAX_OBJECTS],
            recovery: Recovery::Clean,
        };
        store.scan()?;
        Ok(store)
    }

    pub const fn recovery(&self) -> Recovery {
        self.recovery
    }

    pub fn interruption_points(payload_length: usize) -> usize {
        record_sectors(payload_length).unwrap_or(0) + 2
    }

    pub fn replace(
        &mut self,
        namespace: NamespaceId,
        name: &[u8],
        payload: &[u8],
    ) -> Result<u64, Error> {
        self.replace_inner(namespace, name, payload, None)
    }

    #[cfg(test)]
    pub fn replace_with_cut(
        &mut self,
        namespace: NamespaceId,
        name: &[u8],
        payload: &[u8],
        cut: Option<usize>,
    ) -> Result<u64, Error> {
        self.replace_inner(namespace, name, payload, cut)
    }

    pub fn read(
        &mut self,
        namespace: NamespaceId,
        name: &[u8],
        selector: VersionSelector,
        output: &mut [u8],
    ) -> Result<(u64, usize), Error> {
        validate_name(name)?;
        if output.len() > PAGE_SIZE {
            return Err(Error::Invalid);
        }
        let entry = self
            .entries
            .iter()
            .find(|entry| entry.matches(namespace.0, name))
            .ok_or(Error::NotFound)?;
        let location = match selector {
            VersionSelector::None => return Err(Error::Invalid),
            VersionSelector::Current => entry.versions.current,
            VersionSelector::Previous => entry.versions.previous,
        }
        .ok_or(Error::NotFound)?;
        if output.len() < location.length {
            return Err(Error::Invalid);
        }
        self.read_record(location, namespace.0, name, output)?;
        Ok((location.version, location.length))
    }

    pub fn compact(&mut self, cut: Option<usize>) -> Result<(), Error> {
        if self.recovery == Recovery::Corrupt {
            return Err(Error::Corrupt);
        }
        let required = self.entries.iter().try_fold(0usize, |total, entry| {
            [entry.versions.previous, entry.versions.current].into_iter().flatten().try_fold(
                total,
                |total, location| {
                    total.checked_add(record_sectors(location.length)?).ok_or(Error::Full)
                },
            )
        })?;
        if required > self.arena_sectors() {
            return Err(Error::Full);
        }
        let target = 1 - self.arena;
        let mut cursor = self.arena_start(target);
        let mut step = 0;
        for entry in self.entries {
            for location in [entry.versions.previous, entry.versions.current].into_iter().flatten()
            {
                for offset in 0..record_sectors(location.length)? {
                    let mut sector = [0; SECTOR_SIZE];
                    self.backend.read(location.sector + offset, &mut sector)?;
                    self.backend.write(cursor, &sector)?;
                    cursor += 1;
                    checkpoint(cut, &mut step)?;
                }
            }
        }
        if required < self.arena_sectors() {
            self.backend.write(cursor, &[0; SECTOR_SIZE])?;
            checkpoint(cut, &mut step)?;
        }
        self.backend.flush()?;
        checkpoint(cut, &mut step)?;
        let generation = self.generation.checked_add(1).ok_or(Error::Full)?;
        self.backend
            .write(generation as usize % SUPERBLOCKS, &encode_superblock(generation, target))?;
        checkpoint(cut, &mut step)?;
        self.backend.flush()?;
        checkpoint(cut, &mut step)?;
        self.arena = target;
        self.generation = generation;
        self.scan()
    }

    fn replace_inner(
        &mut self,
        namespace: NamespaceId,
        name: &[u8],
        payload: &[u8],
        cut: Option<usize>,
    ) -> Result<u64, Error> {
        validate_name(name)?;
        if payload.len() > PAGE_SIZE {
            return Err(Error::Invalid);
        }
        if self.recovery == Recovery::Corrupt {
            return Err(Error::Corrupt);
        }
        let existing = self.entries.iter().position(|entry| entry.matches(namespace.0, name));
        let index = existing
            .or_else(|| self.entries.iter().position(|entry| entry.name_length == 0))
            .ok_or(Error::Full)?;
        let version = self.entries[index]
            .versions
            .current
            .map_or(Ok(1), |current| current.version.checked_add(1).ok_or(Error::Full))?;
        let sectors = record_sectors(payload.len())?;
        if self.tail.checked_add(sectors).ok_or(Error::Full)? > self.arena_sectors() {
            self.compact(None)?;
        }
        if self.tail.checked_add(sectors).ok_or(Error::Full)? > self.arena_sectors() {
            return Err(Error::Full);
        }
        let start = self.arena_start(self.arena) + self.tail;
        self.write_record(start, namespace.0, name, version, payload, cut)?;
        self.tail += sectors;
        let entry = &mut self.entries[index];
        if existing.is_none() {
            entry.namespace = namespace.0;
            entry.name[..name.len()].copy_from_slice(name);
            entry.name_length = name.len() as u8;
        }
        entry.versions.previous = entry.versions.current;
        entry.versions.current = Some(Location { sector: start, length: payload.len(), version });
        Ok(version)
    }

    fn write_record(
        &mut self,
        start: usize,
        namespace: u32,
        name: &[u8],
        version: u64,
        payload: &[u8],
        cut: Option<usize>,
    ) -> Result<(), Error> {
        let mut header = encode_header(namespace, name, version, payload.len())?;
        let mut record_crc = crc32c_state(!0, &header);
        let mut step = 0;
        self.backend.write(start, &header)?;
        checkpoint(cut, &mut step)?;
        for (index, bytes) in payload.chunks(SECTOR_SIZE).enumerate() {
            let mut sector = [0; SECTOR_SIZE];
            sector[..bytes.len()].copy_from_slice(bytes);
            record_crc = crc32c_state(record_crc, &sector);
            self.backend.write(start + 1 + index, &sector)?;
            checkpoint(cut, &mut step)?;
        }
        self.backend.flush()?;
        checkpoint(cut, &mut step)?;
        let mut commit = [0; SECTOR_SIZE];
        commit[..4].copy_from_slice(COMMIT_MAGIC);
        write_u32(&mut commit, 4, !record_crc);
        self.backend.write(start + record_sectors(payload.len())? - 1, &commit)?;
        checkpoint(cut, &mut step)?;
        self.backend.flush()?;
        checkpoint(cut, &mut step)?;
        header.fill(0);
        Ok(())
    }

    fn scan(&mut self) -> Result<(), Error> {
        self.entries = [Entry::EMPTY; MAX_OBJECTS];
        self.tail = 0;
        self.recovery = Recovery::Clean;
        let start = self.arena_start(self.arena);
        while self.tail < self.arena_sectors() {
            let sector = start + self.tail;
            let mut header = [0; SECTOR_SIZE];
            self.backend.read(sector, &mut header)?;
            if &header[..4] != RECORD_MAGIC {
                break;
            }
            if crc32c(&header[..CHECKSUM_OFFSET]) != read_u32(&header, CHECKSUM_OFFSET) {
                self.recovery = Recovery::Incomplete;
                break;
            }
            let Some((record_sectors, version, namespace, name, name_length, length)) =
                decode_header(&header)
            else {
                self.recovery = Recovery::Corrupt;
                break;
            };
            if record_sectors > self.arena_sectors() - self.tail {
                self.recovery = Recovery::Corrupt;
                break;
            }
            let mut record_crc = crc32c_state(!0, &header);
            for offset in 1..record_sectors - 1 {
                let mut payload = [0; SECTOR_SIZE];
                self.backend.read(sector + offset, &mut payload)?;
                record_crc = crc32c_state(record_crc, &payload);
            }
            let mut commit = [0; SECTOR_SIZE];
            self.backend.read(sector + record_sectors - 1, &mut commit)?;
            if &commit[..4] != COMMIT_MAGIC {
                self.recovery = Recovery::Incomplete;
                break;
            }
            if read_u32(&commit, 4) != !record_crc {
                self.recovery = Recovery::Corrupt;
                break;
            }
            let location = Location { sector, length, version };
            if !self.insert_scanned(namespace, &name[..name_length], location) {
                self.recovery = Recovery::Corrupt;
                break;
            }
            self.tail += record_sectors;
        }
        Ok(())
    }

    fn insert_scanned(&mut self, namespace: u32, name: &[u8], location: Location) -> bool {
        let index = self
            .entries
            .iter()
            .position(|entry| entry.matches(namespace, name))
            .or_else(|| self.entries.iter().position(|entry| entry.name_length == 0));
        let Some(index) = index else { return false };
        let entry = &mut self.entries[index];
        if entry.name_length == 0 {
            entry.namespace = namespace;
            entry.name[..name.len()].copy_from_slice(name);
            entry.name_length = name.len() as u8;
        }
        if entry.versions.current.is_none_or(|current| location.version > current.version) {
            entry.versions.previous = entry.versions.current;
            entry.versions.current = Some(location);
        } else if entry.versions.previous.is_none_or(|previous| location.version > previous.version)
        {
            entry.versions.previous = Some(location);
        }
        true
    }

    fn read_record(
        &mut self,
        location: Location,
        namespace: u32,
        name: &[u8],
        output: &mut [u8],
    ) -> Result<(), Error> {
        let mut header = [0; SECTOR_SIZE];
        self.backend.read(location.sector, &mut header)?;
        if crc32c(&header[..CHECKSUM_OFFSET]) != read_u32(&header, CHECKSUM_OFFSET) {
            return Err(Error::Corrupt);
        }
        let Some((sectors, version, stored_namespace, stored_name, stored_name_length, length)) =
            decode_header(&header)
        else {
            return Err(Error::Corrupt);
        };
        if version != location.version
            || length != location.length
            || stored_namespace != namespace
            || stored_name_length != name.len()
            || stored_name[..stored_name_length] != *name
        {
            return Err(Error::Corrupt);
        }
        let mut record_crc = crc32c_state(!0, &header);
        let mut copied = 0;
        for offset in 1..sectors - 1 {
            let mut payload = [0; SECTOR_SIZE];
            self.backend.read(location.sector + offset, &mut payload)?;
            record_crc = crc32c_state(record_crc, &payload);
            let count = core::cmp::min(SECTOR_SIZE, length - copied);
            output[copied..copied + count].copy_from_slice(&payload[..count]);
            copied += count;
        }
        let mut commit = [0; SECTOR_SIZE];
        self.backend.read(location.sector + sectors - 1, &mut commit)?;
        if &commit[..4] != COMMIT_MAGIC || read_u32(&commit, 4) != !record_crc {
            return Err(Error::Corrupt);
        }
        Ok(())
    }

    fn arena_start(&self, arena: usize) -> usize {
        SUPERBLOCKS + arena * self.arena_sectors()
    }

    fn arena_sectors(&self) -> usize {
        (self.sectors - SUPERBLOCKS) / 2
    }
}

fn validate_sectors(sectors: usize) -> Result<(), Error> {
    (sectors >= 10).then_some(()).ok_or(Error::Invalid)
}

fn validate_name(name: &[u8]) -> Result<(), Error> {
    if name.is_empty() || name.len() > MAX_OBJECT_NAME || core::str::from_utf8(name).is_err() {
        Err(Error::Invalid)
    } else {
        Ok(())
    }
}

fn record_sectors(payload_length: usize) -> Result<usize, Error> {
    (payload_length <= PAGE_SIZE)
        .then_some(2 + payload_length.div_ceil(SECTOR_SIZE))
        .ok_or(Error::Invalid)
}

fn encode_header(
    namespace: u32,
    name: &[u8],
    version: u64,
    length: usize,
) -> Result<[u8; SECTOR_SIZE], Error> {
    validate_name(name)?;
    let mut header = [0; SECTOR_SIZE];
    header[..4].copy_from_slice(RECORD_MAGIC);
    write_u32(&mut header, 4, record_sectors(length)? as u32);
    write_u64(&mut header, 8, version);
    write_u32(&mut header, 16, namespace);
    header[20] = name.len() as u8;
    write_u64(&mut header, 24, length as u64);
    header[32..32 + name.len()].copy_from_slice(name);
    write_u32(&mut header, RECORD_VERSION_OFFSET, FORMAT_VERSION);
    let checksum = crc32c(&header[..CHECKSUM_OFFSET]);
    write_u32(&mut header, CHECKSUM_OFFSET, checksum);
    Ok(header)
}

fn decode_header(
    header: &[u8; SECTOR_SIZE],
) -> Option<(usize, u64, u32, [u8; MAX_OBJECT_NAME], usize, usize)> {
    if read_u32(header, RECORD_VERSION_OFFSET) != FORMAT_VERSION {
        return None;
    }
    let sectors = read_u32(header, 4) as usize;
    let name_length = header[20] as usize;
    let length = read_u64(header, 24) as usize;
    if name_length == 0
        || name_length > MAX_OBJECT_NAME
        || length > PAGE_SIZE
        || sectors != record_sectors(length).ok()?
        || core::str::from_utf8(&header[32..32 + name_length]).is_err()
    {
        return None;
    }
    let mut name = [0; MAX_OBJECT_NAME];
    name[..name_length].copy_from_slice(&header[32..32 + name_length]);
    Some((sectors, read_u64(header, 8), read_u32(header, 16), name, name_length, length))
}

fn encode_superblock(generation: u64, arena: usize) -> [u8; SECTOR_SIZE] {
    let mut block = [0; SECTOR_SIZE];
    block[..4].copy_from_slice(SUPER_MAGIC);
    write_u32(&mut block, SUPER_VERSION_OFFSET, FORMAT_VERSION);
    write_u64(&mut block, 8, generation);
    block[16] = arena as u8;
    let checksum = crc32c(&block[..CHECKSUM_OFFSET]);
    write_u32(&mut block, CHECKSUM_OFFSET, checksum);
    block
}

fn decode_superblock(block: &[u8; SECTOR_SIZE]) -> Option<(u64, usize)> {
    let arena = block[16] as usize;
    (&block[..4] == SUPER_MAGIC
        && read_u32(block, SUPER_VERSION_OFFSET) == FORMAT_VERSION
        && arena < 2
        && read_u32(block, CHECKSUM_OFFSET) == crc32c(&block[..CHECKSUM_OFFSET]))
    .then_some((read_u64(block, 8), arena))
}

fn checkpoint(cut: Option<usize>, step: &mut usize) -> Result<(), Error> {
    *step += 1;
    if cut == Some(*step) { Err(Error::Interrupted) } else { Ok(()) }
}

fn is_zeroed(bytes: &[u8]) -> bool {
    bytes.iter().all(|byte| *byte == 0)
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}

fn write_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn write_u64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

fn crc32c_state(mut crc: u32, bytes: &[u8]) -> u32 {
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0x82f6_3b78 & 0u32.wrapping_sub(crc & 1));
        }
    }
    crc
}

pub fn crc32c(bytes: &[u8]) -> u32 {
    !crc32c_state(!0, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::{rc::Rc, vec::Vec};
    use core::cell::Cell;

    const NS: NamespaceId = NamespaceId(7);

    fn read_value(
        store: &mut Store<MemoryBackend>,
        name: &[u8],
        selector: VersionSelector,
    ) -> Vec<u8> {
        let mut output = [0; PAGE_SIZE];
        let (_, length) = store.read(NS, name, selector, &mut output).unwrap();
        output[..length].to_vec()
    }

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
            let mut recovered = Store::recover(store.into_disk()).unwrap();
            let value = read_value(&mut recovered, b"history", VersionSelector::Current);
            assert!(value == b"old" || value == vec![b'n'; 700]);
        }
    }

    #[test]
    fn keeps_current_and_previous_across_compaction() {
        let mut store = Store::format(32).unwrap();
        store.replace(NS, b"history", b"one").unwrap();
        store.replace(NS, b"history", b"two").unwrap();
        store.compact(None).unwrap();
        assert_eq!(read_value(&mut store, b"history", VersionSelector::Current), b"two");
        assert_eq!(read_value(&mut store, b"history", VersionSelector::Previous), b"one");
    }

    #[test]
    fn interrupted_compaction_keeps_selected_arena() {
        let mut base = Store::format(32).unwrap();
        base.replace(NS, b"history", b"one").unwrap();
        base.replace(NS, b"history", b"two").unwrap();
        let disk = base.into_disk();
        let mut completed = false;
        for cut in 1..32 {
            let mut store = Store::recover(disk.clone()).unwrap();
            match store.compact(Some(cut)) {
                Err(Error::Interrupted) => {
                    let mut recovered = Store::recover(store.into_disk()).unwrap();
                    assert_eq!(
                        read_value(&mut recovered, b"history", VersionSelector::Current),
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
    fn blank_disk_formats_and_nonblank_disk_is_untouched() {
        assert_eq!(Store::format(10).unwrap().recovery(), Recovery::Clean);
        let writes = Rc::new(Cell::new(0));
        let mut bytes = vec![0; 10 * SECTOR_SIZE];
        bytes[0] = 1;
        let backend = CountingBackend { bytes, writes: writes.clone() };
        assert!(matches!(Store::format_with_backend(backend), Err(Error::Corrupt)));
        assert_eq!(writes.get(), 0);
    }

    #[test]
    fn rejects_limits_and_corrupt_records() {
        let mut store = Store::format(512).unwrap();
        for number in 0..MAX_OBJECTS {
            let mut name = [0; 2];
            name[0] = b'a' + (number / 10) as u8;
            name[1] = b'0' + (number % 10) as u8;
            store.replace(NS, &name, b"value").unwrap();
        }
        assert_eq!(store.replace(NS, b"extra", b"value"), Err(Error::Full));
        assert_eq!(store.replace(NS, b"a0", &vec![0; PAGE_SIZE + 1]), Err(Error::Invalid));
        store.replace(NS, b"a0", &vec![0; PAGE_SIZE]).unwrap();
        let mut disk = store.into_disk();
        disk[2 * SECTOR_SIZE + RECORD_VERSION_OFFSET] = 2;
        let checksum = crc32c(&disk[2 * SECTOR_SIZE..2 * SECTOR_SIZE + CHECKSUM_OFFSET]);
        write_u32(&mut disk, 2 * SECTOR_SIZE + CHECKSUM_OFFSET, checksum);
        let mut recovered = Store::recover(disk).unwrap();
        assert_eq!(recovered.recovery(), Recovery::Corrupt);
        assert_eq!(recovered.replace(NS, b"new", b"value"), Err(Error::Corrupt));
    }

    #[test]
    fn recovery_classifies_incomplete_and_checksum_failures() {
        let mut store = Store::format(32).unwrap();
        store.replace(NS, b"object", b"value").unwrap();
        let disk = store.into_disk();
        let mut torn = disk.clone();
        torn[2 * SECTOR_SIZE + CHECKSUM_OFFSET] ^= 1;
        assert_eq!(Store::recover(torn).unwrap().recovery(), Recovery::Incomplete);
        let mut corrupt = disk;
        corrupt[3 * SECTOR_SIZE] ^= 1;
        assert_eq!(Store::recover(corrupt).unwrap().recovery(), Recovery::Corrupt);
    }

    #[test]
    fn recovery_does_not_allocate_by_device_size() {
        let store = Store::recover_backend(HugeBackend).unwrap();
        assert_eq!(store.recovery(), Recovery::Clean);
    }

    #[test]
    fn crc32c_matches_standard_vector() {
        assert_eq!(crc32c(b"123456789"), 0xe306_9283);
    }

    struct CountingBackend {
        bytes: Vec<u8>,
        writes: Rc<Cell<usize>>,
    }

    impl SectorBackend for CountingBackend {
        fn sectors(&self) -> usize {
            self.bytes.len() / SECTOR_SIZE
        }
        fn read(&mut self, sector: usize, output: &mut [u8; SECTOR_SIZE]) -> Result<(), Error> {
            let start = sector * SECTOR_SIZE;
            output.copy_from_slice(&self.bytes[start..start + SECTOR_SIZE]);
            Ok(())
        }
        fn write(&mut self, sector: usize, input: &[u8; SECTOR_SIZE]) -> Result<(), Error> {
            self.writes.set(self.writes.get() + 1);
            let start = sector * SECTOR_SIZE;
            self.bytes[start..start + SECTOR_SIZE].copy_from_slice(input);
            Ok(())
        }
        fn flush(&mut self) -> Result<(), Error> {
            Ok(())
        }
    }

    struct HugeBackend;

    impl SectorBackend for HugeBackend {
        fn sectors(&self) -> usize {
            1_000_000_000
        }
        fn read(&mut self, sector: usize, output: &mut [u8; SECTOR_SIZE]) -> Result<(), Error> {
            output.fill(0);
            if sector == 0 {
                *output = encode_superblock(1, 0);
            }
            Ok(())
        }
        fn write(&mut self, _: usize, _: &[u8; SECTOR_SIZE]) -> Result<(), Error> {
            Err(Error::Io)
        }
        fn flush(&mut self) -> Result<(), Error> {
            Err(Error::Io)
        }
    }
}
