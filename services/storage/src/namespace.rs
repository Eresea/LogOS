use crate::packages::{
    MAX_PACKAGE_EXTENTS, MAX_PACKAGE_RECORDS, PACKAGE_INSTALL_KIND, PACKAGE_RECORD_BYTES,
    PACKAGE_SNAPSHOT_BYTES, PackageCatalog, PackageCatalogError, PackageExtent, PackageHandle,
    PackageInfo, PackageInstall, encode_install_record, validate_install_on_store,
};
use logos_abi::ServiceId;
use logos_package::{
    PACKAGE_FORMAT_VERSION_V2, PACKAGE_HEADER_BYTES, PACKAGE_HEADER_V2_BYTES, PackageHeaderV2,
    PackageTarget, ServicePackageHeader,
};
use logos_storage::{
    BLOCK_BYTES, Block, BlockError, BlockIndex, BlockStore, CHECKPOINT_PAYLOAD_BYTES, CowError,
    CowExtent, CowTransaction, CowVolume, FormatError, JournalRecord, MAX_RECORD_PAYLOAD_BYTES,
    ReadMap,
};

pub const MAX_OBJECTS: usize = 4;
pub const MAX_COMPONENT_BYTES: usize = 255;
pub const MAX_PATH_DEPTH: usize = 32;
pub const MAX_FILE_EXTENTS: usize = 8;
pub const MAX_FILE_BLOCKS: usize = 2;
pub const MAX_FILE_BYTES: usize = MAX_FILE_BLOCKS * BLOCK_BYTES;
const MAX_WRITE_RECORDS: usize = MAX_FILE_BLOCKS + 1;
const WRITE_HEADER_BYTES: usize = 12;
const MAX_WRITE_BYTES: usize = MAX_RECORD_PAYLOAD_BYTES - WRITE_HEADER_BYTES;
const CREATE_KIND: u16 = 1;
const RENAME_KIND: u16 = 2;
const UNLINK_KIND: u16 = 3;
const WRITE_KIND: u16 = 4;
const TRUNCATE_KIND: u16 = 5;
const SNAPSHOT_FIXED_BYTES: usize = 4 + 2 + 4 + 1 + 1 + 2 + MAX_COMPONENT_BYTES + 4;
const EXTENT_WIRE_BYTES: usize = 8 + 4;
const SNAPSHOT_RECORD_BYTES: usize =
    SNAPSHOT_FIXED_BYTES + 1 + MAX_FILE_EXTENTS * EXTENT_WIRE_BYTES + MAX_FILE_BYTES;
const SNAPSHOT_BYTES: usize = MAX_OBJECTS * SNAPSHOT_RECORD_BYTES;
const STORAGE_SNAPSHOT_BYTES: usize = SNAPSHOT_BYTES + PACKAGE_SNAPSHOT_BYTES;
const STORAGE_SNAPSHOT_BUFFER_BYTES: usize = STORAGE_SNAPSHOT_BYTES + BLOCK_BYTES;
const STORAGE_SNAPSHOT_MAX_BLOCKS: usize = STORAGE_SNAPSHOT_BUFFER_BYTES.div_ceil(BLOCK_BYTES);

const _: () = assert!(STORAGE_SNAPSHOT_BYTES <= CHECKPOINT_PAYLOAD_BYTES);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ObjectId {
    slot: u16,
    generation: u32,
}

impl ObjectId {
    pub const ROOT: Self = Self { slot: 0, generation: 1 };

    pub const fn new(slot: u16, generation: u32) -> Option<Self> {
        if generation == 0 { None } else { Some(Self { slot, generation }) }
    }

    pub const fn slot(self) -> u16 {
        self.slot
    }

    pub const fn generation(self) -> u32 {
        self.generation
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ObjectKind {
    File = 1,
    Directory = 2,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NamespaceError {
    Format(FormatError),
    Block(BlockError),
    Capacity,
    InvalidName,
    InvalidPath,
    NotFound,
    NotDirectory,
    AlreadyExists,
    IsDirectory,
    Root,
    NotEmpty,
    Stale,
    TooLarge,
    InvalidRecord,
    GenerationExhausted,
    Recovery,
    Unsupported,
    Package(PackageCatalogError),
}

impl From<FormatError> for NamespaceError {
    fn from(error: FormatError) -> Self {
        Self::Format(error)
    }
}

impl From<CowError> for NamespaceError {
    fn from(error: CowError) -> Self {
        match error {
            CowError::UnsupportedVersion => Self::Format(FormatError::UnsupportedVersion),
            CowError::Unformatted => Self::Format(FormatError::Unformatted),
            CowError::ProvisionedBlank => Self::Format(FormatError::ProvisionedBlank),
            CowError::NotBlank => Self::Format(FormatError::NotBlank),
            CowError::TooSmall => Self::Format(FormatError::TooSmall),
            CowError::Block(error) => Self::Block(error),
            CowError::GenerationExhausted => Self::GenerationExhausted,
            CowError::OutOfSpace | CowError::RetiredExtentCapacity => Self::Capacity,
            _ => Self::Recovery,
        }
    }
}

impl From<BlockError> for NamespaceError {
    fn from(error: BlockError) -> Self {
        Self::Block(error)
    }
}

impl From<PackageCatalogError> for NamespaceError {
    fn from(error: PackageCatalogError) -> Self {
        Self::Package(error)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ObjectInfo {
    pub id: ObjectId,
    pub parent: ObjectId,
    pub kind: ObjectKind,
    pub length: u32,
    pub name: [u8; MAX_COMPONENT_BYTES],
    pub name_length: u16,
}

impl ObjectInfo {
    pub fn name_bytes(&self) -> &[u8] {
        &self.name[..self.name_length as usize]
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ObjectList {
    ids: [Option<ObjectId>; MAX_OBJECTS],
    count: usize,
}

impl ObjectList {
    const fn empty() -> Self {
        Self { ids: [None; MAX_OBJECTS], count: 0 }
    }

    pub const fn len(self) -> usize {
        self.count
    }

    pub const fn is_empty(self) -> bool {
        self.count == 0
    }

    pub const fn get(self, index: usize) -> Option<ObjectId> {
        if index >= self.count { None } else { self.ids[index] }
    }
}

#[derive(Clone, Copy)]
struct ObjectRecord {
    generation: u32,
    parent: ObjectId,
    kind: ObjectKind,
    alive: bool,
    name_length: u16,
    name: [u8; MAX_COMPONENT_BYTES],
    length: u32,
    extent_count: u8,
    extents: [CowExtent; MAX_FILE_EXTENTS],
    data: [u8; MAX_FILE_BYTES],
}

impl ObjectRecord {
    const EMPTY: Self = Self {
        generation: 0,
        parent: ObjectId { slot: u16::MAX, generation: 1 },
        kind: ObjectKind::File,
        alive: false,
        name_length: 0,
        name: [0; MAX_COMPONENT_BYTES],
        length: 0,
        extent_count: 0,
        extents: [CowExtent::EMPTY; MAX_FILE_EXTENTS],
        data: [0; MAX_FILE_BYTES],
    };
}

#[derive(Clone, Copy)]
pub struct ObjectNamespace {
    records: [ObjectRecord; MAX_OBJECTS],
}

impl ObjectNamespace {
    pub const fn new() -> Self {
        let mut records = [ObjectRecord::EMPTY; MAX_OBJECTS];
        records[0] = ObjectRecord {
            generation: ObjectId::ROOT.generation,
            parent: ObjectId { slot: u16::MAX, generation: 1 },
            kind: ObjectKind::Directory,
            alive: true,
            name_length: 0,
            name: [0; MAX_COMPONENT_BYTES],
            length: 0,
            extent_count: 0,
            extents: [CowExtent::EMPTY; MAX_FILE_EXTENTS],
            data: [0; MAX_FILE_BYTES],
        };
        Self { records }
    }

    pub const fn root(&self) -> ObjectId {
        ObjectId::ROOT
    }

    fn object_record(&self, id: ObjectId) -> Result<&ObjectRecord, NamespaceError> {
        let Some(record) = self.records.get(id.slot as usize) else {
            return Err(NamespaceError::Stale);
        };
        if !record.alive || record.generation != id.generation {
            return Err(NamespaceError::Stale);
        }
        Ok(record)
    }

    fn object_record_mut(&mut self, id: ObjectId) -> Result<&mut ObjectRecord, NamespaceError> {
        let Some(record) = self.records.get_mut(id.slot as usize) else {
            return Err(NamespaceError::Stale);
        };
        if !record.alive || record.generation != id.generation {
            return Err(NamespaceError::Stale);
        }
        Ok(record)
    }

    fn validate_name(name: &[u8]) -> Result<(), NamespaceError> {
        if name.is_empty()
            || name.len() > MAX_COMPONENT_BYTES
            || name.iter().any(|byte| *byte == 0 || *byte == b'/')
        {
            return Err(NamespaceError::InvalidName);
        }
        Ok(())
    }

    fn find_child(
        &self,
        parent: ObjectId,
        name: &[u8],
    ) -> Result<Option<ObjectId>, NamespaceError> {
        self.object_record(parent)?;
        for (slot, record) in self.records.iter().enumerate() {
            if record.alive
                && record.parent == parent
                && record.name_length as usize == name.len()
                && record.name[..name.len()] == *name
            {
                return Ok(Some(ObjectId { slot: slot as u16, generation: record.generation }));
            }
        }
        Ok(None)
    }

    fn plan_create(
        &self,
        parent: ObjectId,
        kind: ObjectKind,
        name: &[u8],
    ) -> Result<(ObjectId, [u8; 2 + 4 + 2 + 4 + 1 + 2 + MAX_COMPONENT_BYTES]), NamespaceError> {
        if self.object_record(parent)?.kind != ObjectKind::Directory {
            return Err(NamespaceError::NotDirectory);
        }
        Self::validate_name(name)?;
        if self.find_child(parent, name)?.is_some() {
            return Err(NamespaceError::AlreadyExists);
        }
        let Some((slot, record)) =
            self.records.iter().enumerate().find(|(_, record)| !record.alive)
        else {
            return Err(NamespaceError::Capacity);
        };
        let generation =
            record.generation.checked_add(1).ok_or(NamespaceError::GenerationExhausted)?;
        let id = ObjectId { slot: slot as u16, generation };
        let mut payload = [0; 2 + 4 + 2 + 4 + 1 + 2 + MAX_COMPONENT_BYTES];
        put_u16(&mut payload, 0, id.slot);
        put_u32(&mut payload, 2, id.generation);
        put_u16(&mut payload, 6, parent.slot);
        put_u32(&mut payload, 8, parent.generation);
        payload[12] = kind as u8;
        put_u16(&mut payload, 13, name.len() as u16);
        payload[15..15 + name.len()].copy_from_slice(name);
        Ok((id, payload))
    }

    fn apply_create(&mut self, payload: &[u8]) -> Result<(), NamespaceError> {
        if payload.len() != 2 + 4 + 2 + 4 + 1 + 2 + MAX_COMPONENT_BYTES {
            return Err(NamespaceError::InvalidRecord);
        }
        let id = ObjectId::new(get_u16(payload, 0), get_u32(payload, 2))
            .ok_or(NamespaceError::InvalidRecord)?;
        let parent = ObjectId::new(get_u16(payload, 6), get_u32(payload, 8))
            .ok_or(NamespaceError::InvalidRecord)?;
        let kind = match payload[12] {
            1 => ObjectKind::File,
            2 => ObjectKind::Directory,
            _ => return Err(NamespaceError::InvalidRecord),
        };
        let name_length = get_u16(payload, 13) as usize;
        if name_length == 0 || name_length > MAX_COMPONENT_BYTES {
            return Err(NamespaceError::InvalidRecord);
        }
        Self::validate_name(&payload[15..15 + name_length])?;
        if self.object_record(parent)?.kind != ObjectKind::Directory {
            return Err(NamespaceError::NotDirectory);
        }
        if self.find_child(parent, &payload[15..15 + name_length])?.is_some() {
            return Err(NamespaceError::AlreadyExists);
        }
        let Some(record) = self.records.get_mut(id.slot as usize) else {
            return Err(NamespaceError::InvalidRecord);
        };
        if record.alive || record.generation >= id.generation {
            return Err(NamespaceError::InvalidRecord);
        }
        record.generation = id.generation;
        record.parent = parent;
        record.kind = kind;
        record.alive = true;
        record.name_length = name_length as u16;
        record.name.fill(0);
        record.name[..name_length].copy_from_slice(&payload[15..15 + name_length]);
        record.length = 0;
        record.extent_count = 0;
        record.extents.fill(CowExtent::EMPTY);
        record.data.fill(0);
        Ok(())
    }

    fn apply_rename(&mut self, payload: &[u8]) -> Result<(), NamespaceError> {
        if payload.len() != 2 + 4 + 2 + 4 + 2 + MAX_COMPONENT_BYTES {
            return Err(NamespaceError::InvalidRecord);
        }
        let id = ObjectId::new(get_u16(payload, 0), get_u32(payload, 2))
            .ok_or(NamespaceError::InvalidRecord)?;
        if id == ObjectId::ROOT {
            return Err(NamespaceError::Root);
        }
        let parent = ObjectId::new(get_u16(payload, 6), get_u32(payload, 8))
            .ok_or(NamespaceError::InvalidRecord)?;
        let name_length = get_u16(payload, 12) as usize;
        if name_length == 0 || name_length > MAX_COMPONENT_BYTES {
            return Err(NamespaceError::InvalidRecord);
        }
        Self::validate_name(&payload[14..14 + name_length])?;
        if self.object_record(parent)?.kind != ObjectKind::Directory {
            return Err(NamespaceError::NotDirectory);
        }
        if let Some(child) = self.find_child(parent, &payload[14..14 + name_length])? {
            if child != id {
                return Err(NamespaceError::AlreadyExists);
            }
        }
        if self.is_descendant(id, parent)? {
            return Err(NamespaceError::InvalidPath);
        }
        let record = self.object_record_mut(id)?;
        record.parent = parent;
        record.name_length = name_length as u16;
        record.name.fill(0);
        record.name[..name_length].copy_from_slice(&payload[14..14 + name_length]);
        Ok(())
    }

    fn is_descendant(&self, object: ObjectId, parent: ObjectId) -> Result<bool, NamespaceError> {
        let mut current = parent;
        for _ in 0..=MAX_OBJECTS {
            if current == object {
                return Ok(true);
            }
            if current == ObjectId::ROOT {
                return Ok(false);
            }
            current = self.object_record(current)?.parent;
        }
        Err(NamespaceError::InvalidRecord)
    }

    fn apply_unlink(&mut self, payload: &[u8]) -> Result<(), NamespaceError> {
        if payload.len() != 6 {
            return Err(NamespaceError::InvalidRecord);
        }
        let id = ObjectId::new(get_u16(payload, 0), get_u32(payload, 2))
            .ok_or(NamespaceError::InvalidRecord)?;
        if id == ObjectId::ROOT {
            return Err(NamespaceError::Root);
        }
        let record = self.object_record(id)?;
        if record.kind == ObjectKind::Directory
            && self.records.iter().any(|child| child.alive && child.parent == id)
        {
            return Err(NamespaceError::NotEmpty);
        }
        self.records[id.slot as usize].alive = false;
        Ok(())
    }

    fn apply_write(&mut self, payload: &[u8]) -> Result<(), NamespaceError> {
        if payload.len() < WRITE_HEADER_BYTES {
            return Err(NamespaceError::InvalidRecord);
        }
        let id = ObjectId::new(get_u16(payload, 0), get_u32(payload, 2))
            .ok_or(NamespaceError::InvalidRecord)?;
        let offset = get_u32(payload, 6) as usize;
        let length = get_u16(payload, 10) as usize;
        if length != payload.len() - WRITE_HEADER_BYTES
            || offset.checked_add(length).is_none()
            || offset + length > MAX_FILE_BYTES
        {
            return Err(NamespaceError::InvalidRecord);
        }
        let record = self.object_record_mut(id)?;
        if record.kind != ObjectKind::File {
            return Err(NamespaceError::IsDirectory);
        }
        if record.extent_count != 0 {
            return Err(NamespaceError::Unsupported);
        }
        record.data[offset..offset + length].copy_from_slice(&payload[WRITE_HEADER_BYTES..]);
        record.length = record.length.max((offset + length) as u32);
        Ok(())
    }

    fn apply_truncate(&mut self, payload: &[u8]) -> Result<(), NamespaceError> {
        if payload.len() != 10 {
            return Err(NamespaceError::InvalidRecord);
        }
        let id = ObjectId::new(get_u16(payload, 0), get_u32(payload, 2))
            .ok_or(NamespaceError::InvalidRecord)?;
        let length = get_u32(payload, 6) as usize;
        if length > MAX_FILE_BYTES {
            return Err(NamespaceError::InvalidRecord);
        }
        let record = self.object_record_mut(id)?;
        if record.kind != ObjectKind::File {
            return Err(NamespaceError::IsDirectory);
        }
        if record.extent_count != 0 {
            return Err(NamespaceError::Unsupported);
        }
        record.data[length..].fill(0);
        record.length = length as u32;
        Ok(())
    }

    fn apply_record(&mut self, kind: u16, payload: &[u8]) -> Result<(), NamespaceError> {
        match kind {
            CREATE_KIND => self.apply_create(payload),
            RENAME_KIND => self.apply_rename(payload),
            UNLINK_KIND => self.apply_unlink(payload),
            WRITE_KIND => self.apply_write(payload),
            TRUNCATE_KIND => self.apply_truncate(payload),
            _ => Err(NamespaceError::InvalidRecord),
        }
    }

    fn parent_and_name<'a>(&self, path: &'a [u8]) -> Result<(ObjectId, &'a [u8]), NamespaceError> {
        if path.is_empty() || path[0] != b'/' || path == b"/" {
            return Err(NamespaceError::InvalidPath);
        }
        let slash = path.iter().rposition(|byte| *byte == b'/').unwrap_or(0);
        let name = &path[slash + 1..];
        if name.is_empty() {
            return Err(NamespaceError::InvalidPath);
        }
        let parent_path = if slash == 0 { b"/" } else { &path[..slash] };
        let parent = self.resolve_path(parent_path)?;
        Ok((parent, name))
    }

    pub fn stat(&self, id: ObjectId) -> Result<ObjectInfo, NamespaceError> {
        let record = self.object_record(id)?;
        let mut name = [0; MAX_COMPONENT_BYTES];
        name[..record.name_length as usize]
            .copy_from_slice(&record.name[..record.name_length as usize]);
        Ok(ObjectInfo {
            id,
            parent: record.parent,
            kind: record.kind,
            length: record.length,
            name,
            name_length: record.name_length,
        })
    }

    pub fn resolve_path(&self, path: &[u8]) -> Result<ObjectId, NamespaceError> {
        if path == b"/" {
            return Ok(ObjectId::ROOT);
        }
        if path.is_empty() || path[0] != b'/' {
            return Err(NamespaceError::InvalidPath);
        }
        let mut current = ObjectId::ROOT;
        let mut depth = 0;
        let mut start = 1;
        while start < path.len() {
            let end = path[start..]
                .iter()
                .position(|byte| *byte == b'/')
                .map_or(path.len(), |offset| start + offset);
            let component = &path[start..end];
            if component.is_empty() {
                return Err(NamespaceError::InvalidPath);
            }
            depth += 1;
            if depth > MAX_PATH_DEPTH {
                return Err(NamespaceError::InvalidPath);
            }
            current = self.find_child(current, component)?.ok_or(NamespaceError::NotFound)?;
            start = end + 1;
        }
        Ok(current)
    }

    pub fn read(
        &self,
        id: ObjectId,
        offset: usize,
        output: &mut [u8],
    ) -> Result<usize, NamespaceError> {
        let record = self.object_record(id)?;
        if record.kind != ObjectKind::File {
            return Err(NamespaceError::IsDirectory);
        }
        if record.extent_count != 0 {
            return Err(NamespaceError::Unsupported);
        }
        if offset > record.length as usize {
            return Err(NamespaceError::TooLarge);
        }
        let length = output.len().min(record.length as usize - offset);
        if record.extent_count != 0 {
            return Err(NamespaceError::Unsupported);
        }
        output[..length].copy_from_slice(&record.data[offset..offset + length]);
        Ok(length)
    }

    fn file_extents(
        &self,
        id: ObjectId,
    ) -> Result<([CowExtent; MAX_FILE_EXTENTS], usize), NamespaceError> {
        let record = self.object_record(id)?;
        let count = usize::from(record.extent_count);
        if count > MAX_FILE_EXTENTS {
            return Err(NamespaceError::InvalidRecord);
        }
        if count == 0 {
            return Ok(([CowExtent::EMPTY; MAX_FILE_EXTENTS], 0));
        }
        if record.extents[..count].iter().any(|extent| extent.blocks == 0)
            || record.extents[count..].iter().any(|extent| extent.blocks != 0)
        {
            return Err(NamespaceError::InvalidRecord);
        }
        Ok((record.extents, count))
    }

    fn set_file_extents(
        &mut self,
        id: ObjectId,
        extents: &[CowExtent],
        length: usize,
    ) -> Result<(), NamespaceError> {
        let record = self.object_record_mut(id)?;
        if record.kind != ObjectKind::File
            || extents.is_empty()
            || extents.len() > MAX_FILE_EXTENTS
            || extents.iter().any(|extent| extent.blocks == 0)
            || length
                > extents.iter().map(|extent| extent.blocks as usize).sum::<usize>() * BLOCK_BYTES
        {
            return Err(NamespaceError::InvalidRecord);
        }
        record.length = length as u32;
        record.extent_count = extents.len() as u8;
        record.extents.fill(CowExtent::EMPTY);
        record.extents[..extents.len()].copy_from_slice(extents);
        record.data.fill(0);
        Ok(())
    }

    pub fn list(&self, parent: ObjectId) -> Result<ObjectList, NamespaceError> {
        if self.object_record(parent)?.kind != ObjectKind::Directory {
            return Err(NamespaceError::NotDirectory);
        }
        let mut list = ObjectList::empty();
        for (slot, record) in self.records.iter().enumerate() {
            if record.alive && record.parent == parent {
                list.ids[list.count] =
                    Some(ObjectId { slot: slot as u16, generation: record.generation });
                list.count += 1;
            }
        }
        Ok(list)
    }
}

impl Default for ObjectNamespace {
    fn default() -> Self {
        Self::new()
    }
}

pub struct DurableNamespace<B> {
    store: B,
    volume: CowVolume,
    namespace: ObjectNamespace,
    packages: PackageCatalog,
    retired_file_extents: [CowExtent; MAX_OBJECTS * MAX_FILE_EXTENTS],
    retired_file_extent_count: usize,
    retired_package_extents: [CowExtent; MAX_PACKAGE_EXTENTS],
    retired_package_extent_count: usize,
    active_package_install: Option<u32>,
    next_package_install: u32,
}

struct SnapshotSource<'a> {
    namespace: &'a ObjectNamespace,
    packages: &'a [u8; PACKAGE_SNAPSHOT_BYTES],
    offset: usize,
    pending: Option<u8>,
}

impl<'a> SnapshotSource<'a> {
    fn new(namespace: &'a ObjectNamespace, packages: &'a [u8; PACKAGE_SNAPSHOT_BYTES]) -> Self {
        Self { namespace, packages, offset: 0, pending: None }
    }

    fn next(&mut self) -> Option<u8> {
        if let Some(byte) = self.pending.take() {
            return Some(byte);
        }
        if self.offset >= STORAGE_SNAPSHOT_BYTES {
            return None;
        }
        let value = if self.offset < SNAPSHOT_BYTES {
            let record_index = self.offset / SNAPSHOT_RECORD_BYTES;
            let record_offset = self.offset % SNAPSHOT_RECORD_BYTES;
            let record = &self.namespace.records[record_index];
            if record_offset < 4 {
                record.generation.to_le_bytes()[record_offset]
            } else if record_offset < 6 {
                record.parent.slot.to_le_bytes()[record_offset - 4]
            } else if record_offset < 10 {
                record.parent.generation.to_le_bytes()[record_offset - 6]
            } else if record_offset == 10 {
                record.kind as u8
            } else if record_offset == 11 {
                u8::from(record.alive)
            } else if record_offset < 14 {
                record.name_length.to_le_bytes()[record_offset - 12]
            } else if record_offset < 269 {
                record.name[record_offset - 14]
            } else if record_offset < 273 {
                record.length.to_le_bytes()[record_offset - 269]
            } else if record_offset == 273 {
                record.extent_count
            } else if record_offset
                < SNAPSHOT_FIXED_BYTES + 1 + MAX_FILE_EXTENTS * EXTENT_WIRE_BYTES
            {
                let offset = record_offset - (SNAPSHOT_FIXED_BYTES + 1);
                let extent = &record.extents[offset / EXTENT_WIRE_BYTES];
                if offset % EXTENT_WIRE_BYTES < 8 {
                    extent.start.get().to_le_bytes()[offset % EXTENT_WIRE_BYTES]
                } else {
                    extent.blocks.to_le_bytes()[offset % EXTENT_WIRE_BYTES - 8]
                }
            } else {
                record.data[record_offset
                    - (SNAPSHOT_FIXED_BYTES + 1 + MAX_FILE_EXTENTS * EXTENT_WIRE_BYTES)]
            }
        } else {
            self.packages[self.offset - SNAPSHOT_BYTES]
        };
        self.offset += 1;
        Some(value)
    }

    fn run(&mut self, first: u8, output: &mut [u8; 256]) -> usize {
        let zero = first == 0;
        let mut length = 1;
        output[0] = first;
        while length < output.len() {
            let Some(next) = self.next() else { break };
            if (next == 0) != zero {
                self.pending = Some(next);
                break;
            }
            output[length] = next;
            length += 1;
        }
        length
    }
}

fn encoded_snapshot_length(source: &mut SnapshotSource<'_>) -> usize {
    let mut input = source.next();
    let mut run = [0; 256];
    let mut output_length = 8;
    while let Some(byte) = input {
        let zero = byte == 0;
        let length = source.run(byte, &mut run);
        output_length += 3 + if zero { 0 } else { length };
        input = source.next();
    }
    output_length
}

struct SnapshotWriter<'a, B: BlockStore> {
    transaction: &'a CowTransaction,
    store: &'a mut B,
    extent: CowExtent,
    block: Block,
    block_number: u32,
    offset: usize,
    total: usize,
}

impl<'a, B: BlockStore> SnapshotWriter<'a, B> {
    fn new(transaction: &'a CowTransaction, store: &'a mut B, extent: CowExtent) -> Self {
        Self {
            transaction,
            store,
            extent,
            block: Block::zero(),
            block_number: 0,
            offset: 0,
            total: 0,
        }
    }

    fn push(&mut self, byte: u8) -> Result<(), NamespaceError> {
        if self.block_number >= self.extent.blocks {
            return Err(NamespaceError::TooLarge);
        }
        self.block.as_bytes_mut()[self.offset] = byte;
        self.offset += 1;
        self.total += 1;
        if self.offset == BLOCK_BYTES {
            self.flush_block()?;
        }
        Ok(())
    }

    fn push_bytes(&mut self, bytes: &[u8]) -> Result<(), NamespaceError> {
        for byte in bytes {
            self.push(*byte)?;
        }
        Ok(())
    }

    fn flush_block(&mut self) -> Result<(), NamespaceError> {
        self.transaction
            .write_block(
                self.store,
                BlockIndex::new(self.extent.start.get() + self.block_number as u64),
                &self.block,
            )
            .map_err(NamespaceError::from)?;
        self.block = Block::zero();
        self.block_number += 1;
        self.offset = 0;
        Ok(())
    }

    fn finish(mut self) -> Result<usize, NamespaceError> {
        if self.offset != 0 {
            self.flush_block()?;
        }
        Ok(self.total)
    }
}

fn write_snapshot_stream<B: BlockStore>(
    transaction: &CowTransaction,
    store: &mut B,
    extent: CowExtent,
    source: &mut SnapshotSource<'_>,
    encoded_length: usize,
) -> Result<(), NamespaceError> {
    let mut writer = SnapshotWriter::new(transaction, store, extent);
    writer.push_bytes(&(STORAGE_SNAPSHOT_BYTES as u32).to_le_bytes())?;
    writer.push_bytes(&(encoded_length as u32).to_le_bytes())?;
    let mut input = source.next();
    let mut run = [0; 256];
    while let Some(byte) = input {
        let zero = byte == 0;
        let length = source.run(byte, &mut run);
        writer.push(u8::from(!zero))?;
        writer.push_bytes(&(length as u16).to_le_bytes())?;
        if !zero {
            writer.push_bytes(&run[..length])?;
        }
        input = source.next();
    }
    if writer.finish()? != encoded_length {
        return Err(NamespaceError::Recovery);
    }
    Ok(())
}

struct SnapshotReader<'a, B: BlockStore> {
    volume: &'a CowVolume,
    store: &'a mut B,
    blocks: u32,
    block: Block,
    block_number: u32,
    offset: usize,
}

impl<'a, B: BlockStore> SnapshotReader<'a, B> {
    fn new(volume: &'a CowVolume, store: &'a mut B) -> Result<Self, NamespaceError> {
        let blocks = volume.root().metadata_blocks;
        if blocks == 0 || blocks as usize > STORAGE_SNAPSHOT_MAX_BLOCKS {
            return Err(NamespaceError::Recovery);
        }
        Ok(Self {
            volume,
            store,
            blocks,
            block: Block::zero(),
            block_number: 0,
            offset: BLOCK_BYTES,
        })
    }

    fn read_byte(&mut self) -> Result<u8, NamespaceError> {
        if self.offset == BLOCK_BYTES {
            if self.block_number == self.blocks {
                return Err(NamespaceError::InvalidRecord);
            }
            self.volume.read_metadata_block(self.store, self.block_number, &mut self.block)?;
            self.block_number += 1;
            self.offset = 0;
        }
        let byte = self.block.as_bytes()[self.offset];
        self.offset += 1;
        Ok(byte)
    }

    fn read_bytes(&mut self, output: &mut [u8]) -> Result<(), NamespaceError> {
        for byte in output {
            *byte = self.read_byte()?;
        }
        Ok(())
    }
}

struct SnapshotDecoder<'a, B: BlockStore> {
    reader: SnapshotReader<'a, B>,
    encoded_length: usize,
    consumed: usize,
    output_offset: usize,
    run_remaining: usize,
    run_literal: bool,
}

impl<'a, B: BlockStore> SnapshotDecoder<'a, B> {
    fn new(volume: &'a CowVolume, store: &'a mut B) -> Result<Self, NamespaceError> {
        let mut reader = SnapshotReader::new(volume, store)?;
        let mut header = [0; 8];
        reader.read_bytes(&mut header)?;
        let original_length = u32::from_le_bytes(header[..4].try_into().unwrap()) as usize;
        let encoded_length = u32::from_le_bytes(header[4..].try_into().unwrap()) as usize;
        if original_length != STORAGE_SNAPSHOT_BYTES
            || encoded_length < 8
            || encoded_length > volume.root().metadata_blocks as usize * BLOCK_BYTES
        {
            return Err(NamespaceError::InvalidRecord);
        }
        Ok(Self {
            reader,
            encoded_length,
            consumed: 8,
            output_offset: 0,
            run_remaining: 0,
            run_literal: false,
        })
    }

    fn next(&mut self) -> Result<Option<u8>, NamespaceError> {
        if self.run_remaining == 0 {
            if self.consumed == self.encoded_length {
                return Ok(None);
            }
            if self.consumed + 3 > self.encoded_length {
                return Err(NamespaceError::InvalidRecord);
            }
            self.run_literal = self.reader.read_byte()? != 0;
            let length =
                u16::from_le_bytes([self.reader.read_byte()?, self.reader.read_byte()?]) as usize;
            if length == 0 {
                return Err(NamespaceError::InvalidRecord);
            }
            self.consumed += 3;
            self.run_remaining = length;
        }
        if self.output_offset == STORAGE_SNAPSHOT_BYTES {
            return Err(NamespaceError::InvalidRecord);
        }
        let byte = if self.run_literal { self.reader.read_byte()? } else { 0 };
        if self.run_literal {
            self.consumed += 1;
        }
        self.run_remaining -= 1;
        self.output_offset += 1;
        Ok(Some(byte))
    }

    fn read_exact(&mut self, output: &mut [u8]) -> Result<(), NamespaceError> {
        for byte in output {
            *byte = self.next()?.ok_or(NamespaceError::InvalidRecord)?;
        }
        Ok(())
    }

    fn finish(&mut self) -> Result<(), NamespaceError> {
        if self.next()?.is_some()
            || self.consumed != self.encoded_length
            || self.output_offset != STORAGE_SNAPSHOT_BYTES
        {
            return Err(NamespaceError::InvalidRecord);
        }
        Ok(())
    }
}

fn restore_record_from_decoder<B: BlockStore>(
    decoder: &mut SnapshotDecoder<'_, B>,
    record: &mut ObjectRecord,
) -> Result<(), NamespaceError> {
    let mut bytes = [0; 4];
    decoder.read_exact(&mut bytes)?;
    record.generation = u32::from_le_bytes(bytes);
    let mut bytes = [0; 2];
    decoder.read_exact(&mut bytes)?;
    let slot = u16::from_le_bytes(bytes);
    let mut bytes = [0; 4];
    decoder.read_exact(&mut bytes)?;
    record.parent =
        ObjectId::new(slot, u32::from_le_bytes(bytes)).ok_or(NamespaceError::InvalidRecord)?;
    let kind = match decoder.next()?.ok_or(NamespaceError::InvalidRecord)? {
        1 => ObjectKind::File,
        2 => ObjectKind::Directory,
        _ => return Err(NamespaceError::InvalidRecord),
    };
    let alive = decoder.next()?.ok_or(NamespaceError::InvalidRecord)?;
    if alive > 1 {
        return Err(NamespaceError::InvalidRecord);
    }
    let mut bytes = [0; 2];
    decoder.read_exact(&mut bytes)?;
    let name_length = u16::from_le_bytes(bytes) as usize;
    if name_length > MAX_COMPONENT_BYTES {
        return Err(NamespaceError::InvalidRecord);
    }
    decoder.read_exact(&mut record.name)?;
    let mut bytes = [0; 4];
    decoder.read_exact(&mut bytes)?;
    let length = u32::from_le_bytes(bytes) as usize;
    let extent_count = decoder.next()?.ok_or(NamespaceError::InvalidRecord)? as usize;
    if extent_count > MAX_FILE_EXTENTS {
        return Err(NamespaceError::InvalidRecord);
    }
    record.extent_count = extent_count as u8;
    record.extents.fill(CowExtent::EMPTY);
    for (index, extent) in record.extents.iter_mut().enumerate() {
        let mut bytes = [0; 8];
        decoder.read_exact(&mut bytes)?;
        let start = u64::from_le_bytes(bytes);
        let mut bytes = [0; 4];
        decoder.read_exact(&mut bytes)?;
        let blocks = u32::from_le_bytes(bytes);
        if index < extent_count {
            *extent = CowExtent::new(BlockIndex::new(start), blocks)
                .ok_or(NamespaceError::InvalidRecord)?;
        } else if start != 0 || blocks != 0 {
            return Err(NamespaceError::InvalidRecord);
        }
    }
    if extent_count == 0 && length > MAX_FILE_BYTES {
        return Err(NamespaceError::InvalidRecord);
    }
    let extent_capacity =
        record.extents[..extent_count].iter().map(|extent| extent.blocks as usize).sum::<usize>()
            * BLOCK_BYTES;
    if extent_count != 0 && (record.kind != ObjectKind::File || length > extent_capacity) {
        return Err(NamespaceError::InvalidRecord);
    }
    decoder.read_exact(&mut record.data)?;
    record.kind = kind;
    record.alive = alive != 0;
    record.name_length = name_length as u16;
    record.length = length as u32;
    Ok(())
}

fn restore_snapshot_from_store<B: BlockStore>(
    volume: &CowVolume,
    store: &mut B,
    namespace: &mut ObjectNamespace,
    packages: &mut PackageCatalog,
) -> Result<(), NamespaceError> {
    if volume.root().metadata_blocks == 0 {
        return Ok(());
    }
    if volume.root().metadata_blocks == 1 {
        let mut block = Block::zero();
        volume.read_metadata_root(store, &mut block)?;
        if block.as_bytes()[..8].iter().all(|byte| *byte == 0) {
            return Ok(());
        }
    }
    let mut decoder = SnapshotDecoder::new(volume, store)?;
    for record in &mut namespace.records {
        restore_record_from_decoder(&mut decoder, record)?;
    }
    if namespace.records[0].generation != ObjectId::ROOT.generation
        || !namespace.records[0].alive
        || namespace.records[0].kind != ObjectKind::Directory
    {
        return Err(NamespaceError::InvalidRecord);
    }
    for (index, record) in namespace.records.iter().enumerate().skip(1) {
        if record.alive {
            ObjectNamespace::validate_name(&record.name[..record.name_length as usize])?;
            let parent = namespace.object_record(record.parent)?;
            if parent.kind != ObjectKind::Directory
                || namespace
                    .find_child(record.parent, &record.name[..record.name_length as usize])?
                    != Some(ObjectId { slot: index as u16, generation: record.generation })
            {
                return Err(NamespaceError::InvalidRecord);
            }
            let mut current = ObjectId { slot: index as u16, generation: record.generation };
            let mut reaches_root = false;
            for _ in 0..=MAX_OBJECTS {
                if current == ObjectId::ROOT {
                    reaches_root = true;
                    break;
                }
                current = namespace.object_record(current)?.parent;
            }
            if !reaches_root {
                return Err(NamespaceError::InvalidRecord);
            }
        }
    }
    let (data_start, data_end) = volume.data_arena();
    for (index, record) in namespace.records.iter().enumerate() {
        let extent_count = usize::from(record.extent_count);
        if !record.alive || extent_count == 0 {
            continue;
        }
        if record.kind != ObjectKind::File || extent_count > MAX_FILE_EXTENTS {
            return Err(NamespaceError::InvalidRecord);
        }
        let total_blocks = record.extents[..extent_count]
            .iter()
            .map(|extent| extent.blocks as usize)
            .sum::<usize>();
        if record.length as usize > total_blocks * BLOCK_BYTES {
            return Err(NamespaceError::InvalidRecord);
        }
        for (extent_index, extent) in record.extents[..extent_count].iter().enumerate() {
            let end = extent
                .start
                .get()
                .checked_add(extent.blocks as u64)
                .ok_or(NamespaceError::InvalidRecord)?;
            if extent.start.get() < data_start || end > data_end {
                return Err(NamespaceError::InvalidRecord);
            }
            for other_extent in record.extents[..extent_index].iter() {
                let other_end = other_extent
                    .start
                    .get()
                    .checked_add(other_extent.blocks as u64)
                    .ok_or(NamespaceError::InvalidRecord)?;
                if extent.start.get() < other_end && other_extent.start.get() < end {
                    return Err(NamespaceError::InvalidRecord);
                }
            }
            for other in namespace.records[..index]
                .iter()
                .filter(|other| other.alive && other.extent_count != 0)
            {
                for other_extent in other.extents[..usize::from(other.extent_count)].iter() {
                    let other_end = other_extent
                        .start
                        .get()
                        .checked_add(other_extent.blocks as u64)
                        .ok_or(NamespaceError::InvalidRecord)?;
                    if extent.start.get() < other_end && other_extent.start.get() < end {
                        return Err(NamespaceError::InvalidRecord);
                    }
                }
            }
        }
    }
    let mut package_snapshot = [0; PACKAGE_SNAPSHOT_BYTES];
    decoder.read_exact(&mut package_snapshot)?;
    decoder.finish()?;
    packages.restore_snapshot(&package_snapshot, volume.package_arena())?;
    let mut package_extents =
        [PackageExtent { start: 0, blocks: 0 }; MAX_PACKAGE_RECORDS * MAX_PACKAGE_EXTENTS];
    let package_extent_count = packages.used_extents(&mut package_extents);
    for record in namespace.records.iter().filter(|record| record.alive && record.extent_count != 0)
    {
        for file_extent in record.extents[..usize::from(record.extent_count)].iter() {
            let end = file_extent
                .start
                .get()
                .checked_add(file_extent.blocks as u64)
                .ok_or(NamespaceError::InvalidRecord)?;
            if package_extents[..package_extent_count].iter().any(|extent| {
                let Some(package_end) = extent.start.checked_add(extent.blocks as u64) else {
                    return true;
                };
                file_extent.start.get() < package_end && extent.start < end
            }) {
                return Err(NamespaceError::InvalidRecord);
            }
        }
    }
    Ok(())
}

impl<B: BlockStore> DurableNamespace<B> {
    pub fn format(mut store: B) -> Result<Self, NamespaceError> {
        let volume = CowVolume::format(&mut store)?;
        let filesystem = Self {
            store,
            volume,
            namespace: ObjectNamespace::new(),
            packages: PackageCatalog::new(),
            retired_file_extents: [CowExtent::EMPTY; MAX_OBJECTS * MAX_FILE_EXTENTS],
            retired_file_extent_count: 0,
            retired_package_extents: [CowExtent::EMPTY; MAX_PACKAGE_EXTENTS],
            retired_package_extent_count: 0,
            active_package_install: None,
            next_package_install: 1,
        };
        Ok(filesystem)
    }

    pub fn format_provisioned(mut store: B) -> Result<Self, NamespaceError> {
        let volume = CowVolume::format_provisioned(&mut store)?;
        let filesystem = Self {
            store,
            volume,
            namespace: ObjectNamespace::new(),
            packages: PackageCatalog::new(),
            retired_file_extents: [CowExtent::EMPTY; MAX_OBJECTS * MAX_FILE_EXTENTS],
            retired_file_extent_count: 0,
            retired_package_extents: [CowExtent::EMPTY; MAX_PACKAGE_EXTENTS],
            retired_package_extent_count: 0,
            active_package_install: None,
            next_package_install: 1,
        };
        Ok(filesystem)
    }

    fn reopen(&mut self) -> Result<(), NamespaceError> {
        let volume = CowVolume::open(&mut self.store)?;
        let mut namespace = ObjectNamespace::new();
        let mut packages = PackageCatalog::new();
        restore_snapshot_from_store(&volume, &mut self.store, &mut namespace, &mut packages)?;
        self.volume = volume;
        self.namespace = namespace;
        self.packages = packages;
        self.retired_file_extent_count = 0;
        self.retired_package_extent_count = 0;
        self.active_package_install = None;
        Ok(())
    }

    pub fn open(mut store: B) -> Result<Self, NamespaceError> {
        let volume = CowVolume::open(&mut store)?;
        let mut namespace = ObjectNamespace::new();
        let mut packages = PackageCatalog::new();
        restore_snapshot_from_store(&volume, &mut store, &mut namespace, &mut packages)?;
        Ok(Self {
            store,
            volume,
            namespace,
            packages,
            retired_file_extents: [CowExtent::EMPTY; MAX_OBJECTS * MAX_FILE_EXTENTS],
            retired_file_extent_count: 0,
            retired_package_extents: [CowExtent::EMPTY; MAX_PACKAGE_EXTENTS],
            retired_package_extent_count: 0,
            active_package_install: None,
            next_package_install: 1,
        })
    }

    pub fn root(&self) -> ObjectId {
        self.namespace.root()
    }

    pub fn into_store(self) -> B {
        self.store
    }

    pub fn block_store_mut(&mut self) -> &mut B {
        &mut self.store
    }

    pub fn resolve_path(&self, path: &[u8]) -> Result<ObjectId, NamespaceError> {
        self.namespace.resolve_path(path)
    }

    pub fn open_file(&self, path: &[u8]) -> Result<ObjectId, NamespaceError> {
        let id = self.resolve_path(path)?;
        if self.stat(id)?.kind != ObjectKind::File {
            return Err(NamespaceError::IsDirectory);
        }
        Ok(id)
    }

    pub fn stat(&self, id: ObjectId) -> Result<ObjectInfo, NamespaceError> {
        self.namespace.stat(id)
    }

    pub fn list(&self, parent: ObjectId) -> Result<ObjectList, NamespaceError> {
        self.namespace.list(parent)
    }

    pub fn read(
        &mut self,
        id: ObjectId,
        offset: usize,
        output: &mut [u8],
    ) -> Result<usize, NamespaceError> {
        self.read_handle(id, offset, output)
    }

    pub fn read_handle(
        &mut self,
        id: ObjectId,
        offset: usize,
        output: &mut [u8],
    ) -> Result<usize, NamespaceError> {
        let record = *self.namespace.object_record(id)?;
        let (extents, extent_count) = self.namespace.file_extents(id)?;
        if extent_count == 0 {
            return self.namespace.read(id, offset, output);
        }
        if offset > record.length as usize {
            return Err(NamespaceError::TooLarge);
        }
        let length = output.len().min(record.length as usize - offset);
        self.read_extent_range(&extents[..extent_count], offset, length, output)?;
        Ok(length)
    }

    pub fn map_read(
        &mut self,
        id: ObjectId,
        offset: usize,
        length: usize,
    ) -> Result<ReadMap, NamespaceError> {
        let record = *self.namespace.object_record(id)?;
        if record.kind != ObjectKind::File {
            return Err(NamespaceError::IsDirectory);
        }
        let (extents, extent_count) = self.namespace.file_extents(id)?;
        if extent_count == 0 {
            return Err(NamespaceError::Unsupported);
        }
        if length == 0
            || offset % BLOCK_BYTES != 0
            || length % BLOCK_BYTES != 0
            || length > 16 * BLOCK_BYTES
            || offset.checked_add(length).is_none()
            || offset + length > record.length as usize
        {
            return Err(NamespaceError::InvalidRecord);
        }
        let start_block = offset / BLOCK_BYTES;
        let blocks = length / BLOCK_BYTES;
        let mut remaining = start_block;
        let mut selected = None;
        for extent in extents[..extent_count].iter() {
            if remaining < extent.blocks as usize {
                let local_blocks = extent.blocks as usize - remaining;
                if blocks > local_blocks {
                    return Err(NamespaceError::Unsupported);
                }
                selected = Some((extent, remaining));
                break;
            }
            remaining -= extent.blocks as usize;
        }
        let Some((extent, local_start)) = selected else {
            return Err(NamespaceError::InvalidRecord);
        };
        self.store
            .map_read_blocks(
                BlockIndex::new(extent.start.get() + local_start as u64),
                blocks as u32,
            )
            .map_err(|error| match error {
                BlockError::InvalidRequest => NamespaceError::Unsupported,
                other => NamespaceError::Block(other),
            })
    }

    pub fn unmap_read(&mut self, mapping: ReadMap) -> Result<(), NamespaceError> {
        self.store.unmap_read(mapping).map_err(NamespaceError::Block)
    }

    fn read_extent_range(
        &mut self,
        extents: &[CowExtent],
        offset: usize,
        length: usize,
        output: &mut [u8],
    ) -> Result<(), NamespaceError> {
        let mut copied = 0;
        while copied < length {
            let absolute = offset + copied;
            let block_offset = absolute % BLOCK_BYTES;
            let block_number = absolute / BLOCK_BYTES;
            let mut remaining = block_number;
            let mut physical = None;
            for extent in extents {
                if remaining < extent.blocks as usize {
                    physical = Some(extent.start.get() + remaining as u64);
                    break;
                }
                remaining -= extent.blocks as usize;
            }
            let physical = physical.ok_or(NamespaceError::InvalidRecord)?;
            let amount = (length - copied).min(BLOCK_BYTES - block_offset);
            let mut block = Block::zero();
            self.store.read_block(BlockIndex::new(physical), &mut block)?;
            output[copied..copied + amount]
                .copy_from_slice(&block.as_bytes()[block_offset..block_offset + amount]);
            copied += amount;
        }
        Ok(())
    }

    pub fn begin_transaction(&self) -> NamespaceTransaction {
        NamespaceTransaction::new()
    }

    pub(crate) fn transaction_base(&self) -> &ObjectNamespace {
        &self.namespace
    }

    fn queue_retired_file_extents(&mut self, extents: &[CowExtent]) -> Result<(), NamespaceError> {
        if self.retired_file_extent_count + extents.len() > self.retired_file_extents.len() {
            return Err(NamespaceError::Capacity);
        }
        let end = self.retired_file_extent_count + extents.len();
        self.retired_file_extents[self.retired_file_extent_count..end].copy_from_slice(extents);
        self.retired_file_extent_count = end;
        Ok(())
    }

    fn queue_retired_package_extents(
        &mut self,
        extents: &[CowExtent],
    ) -> Result<(), NamespaceError> {
        if self.retired_package_extent_count + extents.len() > self.retired_package_extents.len() {
            return Err(NamespaceError::Capacity);
        }
        let end = self.retired_package_extent_count + extents.len();
        self.retired_package_extents[self.retired_package_extent_count..end]
            .copy_from_slice(extents);
        self.retired_package_extent_count = end;
        Ok(())
    }

    fn persist_snapshot(&mut self) -> Result<u64, NamespaceError> {
        let transaction = self.volume.begin(&mut self.store)?;
        self.persist_snapshot_with(transaction)
    }

    fn persist_snapshot_with(
        &mut self,
        mut transaction: CowTransaction,
    ) -> Result<u64, NamespaceError> {
        let mut package_snapshot = [0; PACKAGE_SNAPSHOT_BYTES];
        self.packages.encode_snapshot(&mut package_snapshot)?;
        let mut source = SnapshotSource::new(&self.namespace, &package_snapshot);
        let compressed_length = encoded_snapshot_length(&mut source);
        let metadata_blocks = compressed_length.div_ceil(BLOCK_BYTES);
        if metadata_blocks > STORAGE_SNAPSHOT_MAX_BLOCKS {
            return Err(NamespaceError::TooLarge);
        }
        let mut package_extents =
            [PackageExtent { start: 0, blocks: 0 }; MAX_PACKAGE_RECORDS * MAX_PACKAGE_EXTENTS];
        let package_extent_count = self.packages.used_extents(&mut package_extents);
        for extent in package_extents[..package_extent_count].iter() {
            if extent.blocks != 0 {
                transaction.reserve_extent(
                    CowExtent::new(BlockIndex::new(extent.start), extent.blocks)
                        .ok_or(NamespaceError::Recovery)?,
                )?;
            }
        }
        let extent = transaction.allocate_blocks(&mut self.store, metadata_blocks as u32)?;
        let mut source = SnapshotSource::new(&self.namespace, &package_snapshot);
        write_snapshot_stream(
            &transaction,
            &mut self.store,
            extent,
            &mut source,
            compressed_length,
        )?;
        let previous = self.volume.root();
        for extent in self.retired_file_extents[..self.retired_file_extent_count].iter() {
            transaction.retire_extent(*extent)?;
        }
        for extent in self.retired_package_extents[..self.retired_package_extent_count].iter() {
            transaction.retire_extent(*extent)?;
        }
        transaction.retire_extent(
            CowExtent::new(previous.metadata_root, previous.metadata_blocks)
                .ok_or(NamespaceError::Recovery)?,
        )?;
        let generation = self.volume.commit(&mut self.store, transaction, extent)?;
        self.retired_file_extent_count = 0;
        self.retired_package_extent_count = 0;
        Ok(generation)
    }

    fn commit_one(&mut self, kind: u16, payload: &[u8]) -> Result<(), NamespaceError> {
        let mut retired = [CowExtent::EMPTY; MAX_FILE_EXTENTS];
        let mut retired_count = 0;
        if kind == UNLINK_KIND && payload.len() == 6 {
            let id = ObjectId::new(get_u16(payload, 0), get_u32(payload, 2))
                .ok_or(NamespaceError::InvalidRecord)?;
            let (extents, count) = self.namespace.file_extents(id)?;
            retired[..count].copy_from_slice(&extents[..count]);
            retired_count = count;
        }
        if self.retired_file_extent_count + retired_count > self.retired_file_extents.len() {
            return Err(NamespaceError::Capacity);
        }
        self.namespace.apply_record(kind, payload)?;
        self.queue_retired_file_extents(&retired[..retired_count])?;
        if let Err(error) = self.persist_snapshot() {
            return Err(match self.reopen() {
                Ok(()) => error,
                Err(recovery) => recovery,
            });
        }
        Ok(())
    }

    pub fn create(
        &mut self,
        parent: ObjectId,
        kind: ObjectKind,
        name: &[u8],
    ) -> Result<ObjectId, NamespaceError> {
        let (id, payload) = self.namespace.plan_create(parent, kind, name)?;
        self.commit_one(CREATE_KIND, &payload)?;
        Ok(id)
    }

    pub fn create_file(
        &mut self,
        parent: ObjectId,
        name: &[u8],
    ) -> Result<ObjectId, NamespaceError> {
        self.create(parent, ObjectKind::File, name)
    }

    pub fn create_directory(
        &mut self,
        parent: ObjectId,
        name: &[u8],
    ) -> Result<ObjectId, NamespaceError> {
        self.create(parent, ObjectKind::Directory, name)
    }

    pub fn mkdir_path(&mut self, path: &[u8]) -> Result<ObjectId, NamespaceError> {
        let (parent, name) = self.namespace.parent_and_name(path)?;
        self.create_directory(parent, name)
    }

    pub fn rename(
        &mut self,
        id: ObjectId,
        parent: ObjectId,
        name: &[u8],
    ) -> Result<(), NamespaceError> {
        if id == ObjectId::ROOT {
            return Err(NamespaceError::Root);
        }
        self.namespace.object_record(id)?;
        if self.namespace.object_record(parent)?.kind != ObjectKind::Directory {
            return Err(NamespaceError::NotDirectory);
        }
        ObjectNamespace::validate_name(name)?;
        if self.namespace.find_child(parent, name)?.is_some_and(|child| child != id) {
            return Err(NamespaceError::AlreadyExists);
        }
        if self.namespace.is_descendant(id, parent)? {
            return Err(NamespaceError::InvalidPath);
        }
        let mut payload = [0; 2 + 4 + 2 + 4 + 2 + MAX_COMPONENT_BYTES];
        put_u16(&mut payload, 0, id.slot);
        put_u32(&mut payload, 2, id.generation);
        put_u16(&mut payload, 6, parent.slot);
        put_u32(&mut payload, 8, parent.generation);
        put_u16(&mut payload, 12, name.len() as u16);
        payload[14..14 + name.len()].copy_from_slice(name);
        self.commit_one(RENAME_KIND, &payload)
    }

    pub fn unlink(&mut self, id: ObjectId) -> Result<(), NamespaceError> {
        self.namespace.object_record(id)?;
        let mut payload = [0; 6];
        put_u16(&mut payload, 0, id.slot);
        put_u32(&mut payload, 2, id.generation);
        self.commit_one(UNLINK_KIND, &payload)
    }

    pub fn write(
        &mut self,
        id: ObjectId,
        offset: usize,
        input: &[u8],
    ) -> Result<usize, NamespaceError> {
        let record = self.namespace.object_record(id)?;
        if record.kind != ObjectKind::File {
            return Err(NamespaceError::IsDirectory);
        }
        if input.is_empty()
            || offset.checked_add(input.len()).is_none()
            || offset + input.len() > MAX_FILE_BYTES
        {
            return Err(NamespaceError::TooLarge);
        }
        if record.extent_count != 0 {
            return Err(NamespaceError::Unsupported);
        }
        let count = input.len().div_ceil(MAX_WRITE_BYTES);
        if count > MAX_WRITE_RECORDS {
            return Err(NamespaceError::TooLarge);
        }
        let mut payloads = [[0; MAX_RECORD_PAYLOAD_BYTES]; MAX_WRITE_RECORDS];
        let mut lengths = [0usize; MAX_WRITE_RECORDS];
        let mut records = [JournalRecord { kind: 0, payload: &[] }; MAX_WRITE_RECORDS];
        let mut copied = 0;
        for index in 0..count {
            let length = (input.len() - copied).min(MAX_WRITE_BYTES);
            let payload = &mut payloads[index][..WRITE_HEADER_BYTES + length];
            put_u16(payload, 0, id.slot);
            put_u32(payload, 2, id.generation);
            put_u32(payload, 6, (offset + copied) as u32);
            put_u16(payload, 10, length as u16);
            payload[WRITE_HEADER_BYTES..].copy_from_slice(&input[copied..copied + length]);
            lengths[index] = payload.len();
            copied += length;
        }
        for index in 0..count {
            records[index] =
                JournalRecord { kind: WRITE_KIND, payload: &payloads[index][..lengths[index]] };
        }
        for record in records.iter().take(count) {
            self.namespace.apply_record(WRITE_KIND, record.payload)?;
        }
        if let Err(error) = self.persist_snapshot() {
            return Err(match self.reopen() {
                Ok(()) => error,
                Err(recovery) => recovery,
            });
        }
        Ok(input.len())
    }

    pub fn write_handle(
        &mut self,
        id: ObjectId,
        offset: usize,
        input: &[u8],
    ) -> Result<usize, NamespaceError> {
        if input.is_empty() {
            return Err(NamespaceError::TooLarge);
        }
        let record = *self.namespace.object_record(id)?;
        if record.kind != ObjectKind::File {
            return Err(NamespaceError::IsDirectory);
        }
        let end = offset.checked_add(input.len()).ok_or(NamespaceError::TooLarge)?;
        if record.extent_count == 0 && end <= MAX_FILE_BYTES {
            return self.write(id, offset, input);
        }
        self.write_handle_extent(id, offset, input, record, end)
    }

    fn write_handle_extent(
        &mut self,
        id: ObjectId,
        offset: usize,
        input: &[u8],
        record: ObjectRecord,
        end: usize,
    ) -> Result<usize, NamespaceError> {
        let length = record.length as usize;
        let new_length = length.max(end);
        let mut remaining_blocks = new_length.div_ceil(BLOCK_BYTES);
        let (old_extents, old_extent_count) = self.namespace.file_extents(id)?;
        let mut new_extents = [CowExtent::EMPTY; MAX_FILE_EXTENTS];
        let mut new_extent_count = 0;
        let mut transaction = self.volume.begin(&mut self.store)?;
        while remaining_blocks != 0 {
            if new_extent_count == MAX_FILE_EXTENTS {
                return Err(NamespaceError::TooLarge);
            }
            let mut requested = remaining_blocks.min(u32::MAX as usize) as u32;
            let extent = loop {
                match transaction.allocate_blocks(&mut self.store, requested) {
                    Ok(extent) => break extent,
                    Err(CowError::OutOfSpace) if requested > 1 => {
                        requested = requested.div_ceil(2);
                    }
                    Err(error) => return Err(error.into()),
                }
            };
            new_extents[new_extent_count] = extent;
            new_extent_count += 1;
            remaining_blocks -= extent.blocks as usize;
        }
        let old_blocks = if old_extent_count == 0 {
            length.div_ceil(BLOCK_BYTES)
        } else {
            old_extents[..old_extent_count]
                .iter()
                .map(|extent| extent.blocks as usize)
                .sum::<usize>()
        };
        let mut logical_block = 0;
        for extent in new_extents[..new_extent_count].iter() {
            for extent_block in 0..extent.blocks as usize {
                let block_number = logical_block;
                logical_block += 1;
                let block_start = block_number * BLOCK_BYTES;
                let block_end = block_start + BLOCK_BYTES;
                let mut block = Block::zero();
                if block_number < old_blocks {
                    if old_extent_count == 0 {
                        let amount = (length - block_start).min(BLOCK_BYTES);
                        block.as_bytes_mut()[..amount]
                            .copy_from_slice(&record.data[block_start..block_start + amount]);
                    } else {
                        self.read_logical_block(
                            &old_extents[..old_extent_count],
                            block_number,
                            &mut block,
                        )?;
                    }
                }
                let input_start = offset.max(block_start);
                let input_end = end.min(block_end);
                if input_start < input_end {
                    let source_start = input_start - offset;
                    block.as_bytes_mut()[input_start - block_start..input_end - block_start]
                        .copy_from_slice(
                            &input[source_start..source_start + input_end - input_start],
                        );
                }
                transaction.write_block(
                    &mut self.store,
                    BlockIndex::new(extent.start.get() + extent_block as u64),
                    &block,
                )?;
            }
        }
        self.namespace.set_file_extents(id, &new_extents[..new_extent_count], new_length)?;
        self.queue_retired_file_extents(&old_extents[..old_extent_count])?;
        if let Err(error) = self.persist_snapshot_with(transaction) {
            return Err(match self.reopen() {
                Ok(()) => error,
                Err(recovery) => recovery,
            });
        }
        Ok(input.len())
    }

    fn read_logical_block(
        &mut self,
        extents: &[CowExtent],
        logical_block: usize,
        output: &mut Block,
    ) -> Result<(), NamespaceError> {
        let mut remaining = logical_block;
        for extent in extents {
            if remaining < extent.blocks as usize {
                self.store
                    .read_block(BlockIndex::new(extent.start.get() + remaining as u64), output)?;
                return Ok(());
            }
            remaining -= extent.blocks as usize;
        }
        Err(NamespaceError::InvalidRecord)
    }

    pub fn flush(&mut self) -> Result<(), NamespaceError> {
        self.store.flush().map_err(NamespaceError::Block)
    }

    pub fn lookup_package(&mut self, service: ServiceId) -> Result<PackageInfo, NamespaceError> {
        match self.packages.lookup_with_store(&mut self.store, service) {
            Err(PackageCatalogError::Stale) => Err(NamespaceError::NotFound),
            result => result.map_err(NamespaceError::Package),
        }
    }

    pub fn package_at(&mut self, index: usize) -> Result<Option<PackageInfo>, NamespaceError> {
        let Some(service) = self.packages.service_at(index) else {
            return Ok(None);
        };
        self.lookup_package(service).map(Some)
    }

    pub fn lookup_package_name(&mut self, name: &[u8]) -> Result<PackageInfo, NamespaceError> {
        let Ok(name) = logos_package::PackageName::parse(name) else {
            return Err(NamespaceError::NotFound);
        };
        for index in 0..crate::packages::MAX_PACKAGE_RECORDS {
            let Some(info) = self.package_at(index)? else {
                break;
            };
            if info.manifest.is_some_and(|manifest| manifest.name == name) {
                return Ok(info);
            }
        }
        Err(NamespaceError::NotFound)
    }

    pub fn install_package_file(&mut self, path: &[u8]) -> Result<PackageHandle, NamespaceError> {
        let id = self.namespace.resolve_path(path)?;
        let info = self.namespace.stat(id)?;
        if info.kind != ObjectKind::File || info.length == 0 {
            return Err(NamespaceError::InvalidRecord);
        }
        let length = info.length as usize;
        let mut prefix = [0; 10];
        if self.namespace.read(id, 0, &mut prefix)? != prefix.len() {
            return Err(NamespaceError::InvalidRecord);
        }
        let service = if u16::from_le_bytes([prefix[8], prefix[9]]) == PACKAGE_FORMAT_VERSION_V2 {
            let mut header = [0; PACKAGE_HEADER_V2_BYTES];
            if self.namespace.read(id, 0, &mut header)? != header.len() {
                return Err(NamespaceError::InvalidRecord);
            }
            let header =
                PackageHeaderV2::decode(&header).map_err(|_| NamespaceError::InvalidRecord)?;
            match header.manifest.target {
                PackageTarget::Service(service) => service,
                PackageTarget::None => return Err(NamespaceError::InvalidRecord),
            }
        } else {
            let mut header = [0; PACKAGE_HEADER_BYTES];
            if self.namespace.read(id, 0, &mut header)? != header.len() {
                return Err(NamespaceError::InvalidRecord);
            }
            ServicePackageHeader::decode(&header)
                .map_err(|_| NamespaceError::InvalidRecord)?
                .service
        };
        let mut install = self.begin_package_install(service, length)?;
        let result = (|| {
            for offset in (0..length).step_by(BLOCK_BYTES) {
                let amount = (length - offset).min(BLOCK_BYTES);
                let mut block = logos_storage::Block::zero();
                if self.namespace.read(id, offset, &mut block.as_bytes_mut()[..amount])? != amount {
                    return Err(NamespaceError::InvalidRecord);
                }
                self.write_package_chunk(&mut install, offset, &block.as_bytes()[..amount])?;
            }
            self.commit_package_install(install)
        })();
        if result.is_err() {
            self.abort_package_install(install);
        }
        result
    }

    pub fn begin_package_install(
        &mut self,
        service: ServiceId,
        bytes: usize,
    ) -> Result<PackageInstall, NamespaceError> {
        if self.active_package_install.is_some() {
            return Err(NamespaceError::Package(PackageCatalogError::Busy));
        }
        let arena = self.volume.package_arena().ok_or(NamespaceError::Unsupported)?;
        let install_id = self.next_package_install;
        self.next_package_install = self
            .next_package_install
            .checked_add(1)
            .ok_or(NamespaceError::Package(PackageCatalogError::InvalidRequest))?;
        let mut install = self.packages.plan_install(arena, service, bytes)?;
        install.install_id = install_id;
        self.active_package_install = Some(install_id);
        Ok(install)
    }

    pub fn write_package_chunk(
        &mut self,
        install: &mut PackageInstall,
        offset: usize,
        input: &[u8],
    ) -> Result<(), NamespaceError> {
        if self.active_package_install != Some(install.install_id) {
            return Err(NamespaceError::InvalidRecord);
        }
        if !install.write_offset_valid(offset, input.len())
            || (offset + input.len() < install.bytes() && input.len() != BLOCK_BYTES)
        {
            return Err(NamespaceError::InvalidRecord);
        }
        let block_index =
            install.logical_block(offset / BLOCK_BYTES).ok_or(NamespaceError::InvalidRecord)?;
        let mut block = logos_storage::Block::zero();
        block.as_bytes_mut()[..input.len()].copy_from_slice(input);
        self.store.write_block(logos_storage::BlockIndex::new(block_index), &block)?;
        install.mark_written(offset);
        Ok(())
    }

    pub fn abort_package_install(&mut self, install: PackageInstall) {
        if self.active_package_install == Some(install.install_id) {
            self.active_package_install = None;
        }
    }

    pub fn commit_package_install(
        &mut self,
        install: PackageInstall,
    ) -> Result<PackageHandle, NamespaceError> {
        if self.active_package_install != Some(install.install_id) {
            return Err(NamespaceError::InvalidRecord);
        }
        let result = self.commit_package_install_inner(install);
        self.active_package_install = None;
        result
    }

    fn commit_package_install_inner(
        &mut self,
        install: PackageInstall,
    ) -> Result<PackageHandle, NamespaceError> {
        let arena = self.volume.package_arena().ok_or(NamespaceError::Unsupported)?;
        if !install.complete() {
            return Err(NamespaceError::InvalidRecord);
        }
        self.store.flush()?;
        let mut scratch = [0; BLOCK_BYTES];
        let package = validate_install_on_store(&mut self.store, &install, &mut scratch)?;
        if package.payload_length as usize + package.header_bytes != install.bytes() {
            return Err(NamespaceError::InvalidRecord);
        }
        self.packages.validate_install_policy(
            &mut self.store,
            install.service,
            package,
            &mut scratch,
        )?;
        let generation = self.packages.next_generation(install.service)?;
        let mut payload = [0; PACKAGE_RECORD_BYTES];
        encode_install_record(&install, package, generation, &mut payload)?;
        let old_package = self.packages.lookup(install.service);
        self.packages.apply_record(PACKAGE_INSTALL_KIND, &payload, Some(arena))?;
        if let Some(old_package) = old_package {
            let mut retired = [CowExtent::EMPTY; MAX_PACKAGE_EXTENTS];
            let mut retired_count = 0;
            for extent in old_package.extents[..old_package.extent_count as usize].iter() {
                retired[retired_count] =
                    CowExtent::new(BlockIndex::new(extent.start), extent.blocks)
                        .ok_or(NamespaceError::Recovery)?;
                retired_count += 1;
            }
            self.queue_retired_package_extents(&retired[..retired_count])?;
        }
        if let Err(error) = self.persist_snapshot() {
            return Err(match self.reopen() {
                Ok(()) => error,
                Err(recovery) => recovery,
            });
        }
        Ok(PackageHandle { service: install.service, generation })
    }

    pub fn read_package(
        &mut self,
        handle: PackageHandle,
        offset: usize,
        output: &mut [u8],
    ) -> Result<usize, NamespaceError> {
        let info = self.packages.validate_handle(handle)?;
        if offset > info.bytes as usize {
            return Err(NamespaceError::TooLarge);
        }
        let length = output.len().min(info.bytes as usize - offset);
        let mut copied = 0;
        while copied < length {
            let absolute = offset + copied;
            let block_index = absolute / BLOCK_BYTES;
            let block_offset = absolute % BLOCK_BYTES;
            let amount = (length - copied).min(BLOCK_BYTES - block_offset);
            let mut block = logos_storage::Block::zero();
            let mut remaining = block_index as u64;
            let mut physical = None;
            for extent in info.extents[..info.extent_count as usize].iter() {
                if remaining < extent.blocks as u64 {
                    physical = Some(extent.start + remaining);
                    break;
                }
                remaining -= extent.blocks as u64;
            }
            let physical = physical.ok_or(NamespaceError::InvalidRecord)?;
            self.store.read_block(logos_storage::BlockIndex::new(physical), &mut block)?;
            output[copied..copied + amount]
                .copy_from_slice(&block.as_bytes()[block_offset..block_offset + amount]);
            copied += amount;
        }
        Ok(length)
    }
}

const MAX_TRANSACTION_RECORDS: usize = logos_storage::MAX_RECORDS_PER_TRANSACTION;

#[derive(Clone, Copy)]
struct PendingRecord {
    kind: u16,
    len: u16,
    payload: [u8; MAX_RECORD_PAYLOAD_BYTES],
}

impl PendingRecord {
    const EMPTY: Self = Self { kind: 0, len: 0, payload: [0; MAX_RECORD_PAYLOAD_BYTES] };
}

struct NamespaceView<'a> {
    base: &'a ObjectNamespace,
    changes: &'a [Option<ObjectRecord>; MAX_OBJECTS],
}

impl<'a> NamespaceView<'a> {
    fn record_at(&self, slot: usize) -> Option<&ObjectRecord> {
        let change = self.changes.get(slot)?;
        change.as_ref().or_else(|| self.base.records.get(slot))
    }

    fn object_record(&self, id: ObjectId) -> Result<&ObjectRecord, NamespaceError> {
        let Some(record) = self.record_at(id.slot as usize) else {
            return Err(NamespaceError::Stale);
        };
        if !record.alive || record.generation != id.generation {
            return Err(NamespaceError::Stale);
        }
        Ok(record)
    }

    fn find_child(
        &self,
        parent: ObjectId,
        name: &[u8],
    ) -> Result<Option<ObjectId>, NamespaceError> {
        self.object_record(parent)?;
        for slot in 0..MAX_OBJECTS {
            let Some(record) = self.record_at(slot) else { continue };
            if record.alive
                && record.parent == parent
                && record.name_length as usize == name.len()
                && record.name[..name.len()] == *name
            {
                return Ok(Some(ObjectId { slot: slot as u16, generation: record.generation }));
            }
        }
        Ok(None)
    }

    fn plan_create(
        &self,
        parent: ObjectId,
        kind: ObjectKind,
        name: &[u8],
    ) -> Result<(ObjectId, [u8; 2 + 4 + 2 + 4 + 1 + 2 + MAX_COMPONENT_BYTES]), NamespaceError> {
        if self.object_record(parent)?.kind != ObjectKind::Directory {
            return Err(NamespaceError::NotDirectory);
        }
        ObjectNamespace::validate_name(name)?;
        if self.find_child(parent, name)?.is_some() {
            return Err(NamespaceError::AlreadyExists);
        }
        let Some((slot, record)) = (0..MAX_OBJECTS)
            .filter_map(|slot| self.record_at(slot).map(|record| (slot, record)))
            .find(|(_, record)| !record.alive)
        else {
            return Err(NamespaceError::Capacity);
        };
        let generation =
            record.generation.checked_add(1).ok_or(NamespaceError::GenerationExhausted)?;
        let id = ObjectId { slot: slot as u16, generation };
        let mut payload = [0; 2 + 4 + 2 + 4 + 1 + 2 + MAX_COMPONENT_BYTES];
        put_u16(&mut payload, 0, id.slot);
        put_u32(&mut payload, 2, id.generation);
        put_u16(&mut payload, 6, parent.slot);
        put_u32(&mut payload, 8, parent.generation);
        payload[12] = kind as u8;
        put_u16(&mut payload, 13, name.len() as u16);
        payload[15..15 + name.len()].copy_from_slice(name);
        Ok((id, payload))
    }

    fn is_descendant(&self, object: ObjectId, parent: ObjectId) -> Result<bool, NamespaceError> {
        let mut current = parent;
        for _ in 0..=MAX_OBJECTS {
            if current == object {
                return Ok(true);
            }
            if current == ObjectId::ROOT {
                return Ok(false);
            }
            current = self.object_record(current)?.parent;
        }
        Err(NamespaceError::InvalidRecord)
    }

    fn parent_and_name<'b>(&self, path: &'b [u8]) -> Result<(ObjectId, &'b [u8]), NamespaceError> {
        if path.is_empty() || path[0] != b'/' || path == b"/" {
            return Err(NamespaceError::InvalidPath);
        }
        let slash = path.iter().rposition(|byte| *byte == b'/').unwrap_or(0);
        let name = &path[slash + 1..];
        if name.is_empty() {
            return Err(NamespaceError::InvalidPath);
        }
        let parent_path = if slash == 0 { b"/" } else { &path[..slash] };
        let parent = self.resolve_path(parent_path)?;
        Ok((parent, name))
    }

    fn resolve_path(&self, path: &[u8]) -> Result<ObjectId, NamespaceError> {
        if path == b"/" {
            return Ok(ObjectId::ROOT);
        }
        if path.is_empty() || path[0] != b'/' {
            return Err(NamespaceError::InvalidPath);
        }
        let mut current = ObjectId::ROOT;
        let mut depth = 0;
        let mut start = 1;
        while start < path.len() {
            let end = path[start..]
                .iter()
                .position(|byte| *byte == b'/')
                .map_or(path.len(), |offset| start + offset);
            let component = &path[start..end];
            if component.is_empty() {
                return Err(NamespaceError::InvalidPath);
            }
            depth += 1;
            if depth > MAX_PATH_DEPTH {
                return Err(NamespaceError::InvalidPath);
            }
            current = self.find_child(current, component)?.ok_or(NamespaceError::NotFound)?;
            start = end + 1;
        }
        Ok(current)
    }

    fn stat(&self, id: ObjectId) -> Result<ObjectInfo, NamespaceError> {
        let record = self.object_record(id)?;
        let mut name = [0; MAX_COMPONENT_BYTES];
        name[..record.name_length as usize]
            .copy_from_slice(&record.name[..record.name_length as usize]);
        Ok(ObjectInfo {
            id,
            parent: record.parent,
            kind: record.kind,
            length: record.length,
            name,
            name_length: record.name_length,
        })
    }

    fn read(
        &self,
        id: ObjectId,
        offset: usize,
        output: &mut [u8],
    ) -> Result<usize, NamespaceError> {
        let record = self.object_record(id)?;
        if record.kind != ObjectKind::File {
            return Err(NamespaceError::IsDirectory);
        }
        if record.extent_count != 0 {
            return Err(NamespaceError::Unsupported);
        }
        if offset > record.length as usize {
            return Err(NamespaceError::TooLarge);
        }
        let length = output.len().min(record.length as usize - offset);
        output[..length].copy_from_slice(&record.data[offset..offset + length]);
        Ok(length)
    }

    fn file_extents(
        &self,
        id: ObjectId,
    ) -> Result<([CowExtent; MAX_FILE_EXTENTS], usize), NamespaceError> {
        let record = self.object_record(id)?;
        let count = usize::from(record.extent_count);
        if count > MAX_FILE_EXTENTS {
            return Err(NamespaceError::InvalidRecord);
        }
        if count == 0 {
            return Ok(([CowExtent::EMPTY; MAX_FILE_EXTENTS], 0));
        }
        if record.extents[..count].iter().any(|extent| extent.blocks == 0)
            || record.extents[count..].iter().any(|extent| extent.blocks != 0)
        {
            return Err(NamespaceError::InvalidRecord);
        }
        Ok((record.extents, count))
    }

    fn list(&self, parent: ObjectId) -> Result<ObjectList, NamespaceError> {
        if self.object_record(parent)?.kind != ObjectKind::Directory {
            return Err(NamespaceError::NotDirectory);
        }
        let mut list = ObjectList::empty();
        for slot in 0..MAX_OBJECTS {
            let Some(record) = self.record_at(slot) else { continue };
            if record.alive && record.parent == parent {
                list.ids[list.count] =
                    Some(ObjectId { slot: slot as u16, generation: record.generation });
                list.count += 1;
            }
        }
        Ok(list)
    }
}

pub struct NamespaceTransaction {
    records: [PendingRecord; MAX_TRANSACTION_RECORDS],
    count: usize,
    changes: [Option<ObjectRecord>; MAX_OBJECTS],
    retired_extents: [CowExtent; MAX_OBJECTS * MAX_FILE_EXTENTS],
    retired_extent_count: usize,
}

impl NamespaceTransaction {
    fn new() -> Self {
        Self {
            records: [PendingRecord::EMPTY; MAX_TRANSACTION_RECORDS],
            count: 0,
            changes: [None; MAX_OBJECTS],
            retired_extents: [CowExtent::EMPTY; MAX_OBJECTS * MAX_FILE_EXTENTS],
            retired_extent_count: 0,
        }
    }

    pub const fn record_count(&self) -> usize {
        self.count
    }

    fn view<'a>(&'a self, base: &'a ObjectNamespace) -> NamespaceView<'a> {
        NamespaceView { base, changes: &self.changes }
    }

    fn record_copy(
        &self,
        base: &ObjectNamespace,
        id: ObjectId,
    ) -> Result<ObjectRecord, NamespaceError> {
        Ok(*self.view(base).object_record(id)?)
    }

    fn apply_create(
        &mut self,
        base: &ObjectNamespace,
        payload: &[u8],
    ) -> Result<(), NamespaceError> {
        if payload.len() != 2 + 4 + 2 + 4 + 1 + 2 + MAX_COMPONENT_BYTES {
            return Err(NamespaceError::InvalidRecord);
        }
        let id = ObjectId::new(get_u16(payload, 0), get_u32(payload, 2))
            .ok_or(NamespaceError::InvalidRecord)?;
        let parent = ObjectId::new(get_u16(payload, 6), get_u32(payload, 8))
            .ok_or(NamespaceError::InvalidRecord)?;
        let kind = match payload[12] {
            1 => ObjectKind::File,
            2 => ObjectKind::Directory,
            _ => return Err(NamespaceError::InvalidRecord),
        };
        let name_length = get_u16(payload, 13) as usize;
        if name_length == 0 || name_length > MAX_COMPONENT_BYTES {
            return Err(NamespaceError::InvalidRecord);
        }
        ObjectNamespace::validate_name(&payload[15..15 + name_length])?;
        let view = self.view(base);
        if view.object_record(parent)?.kind != ObjectKind::Directory {
            return Err(NamespaceError::NotDirectory);
        }
        if view.find_child(parent, &payload[15..15 + name_length])?.is_some() {
            return Err(NamespaceError::AlreadyExists);
        }
        let slot = usize::from(id.slot);
        let Some(previous) = view.record_at(slot) else {
            return Err(NamespaceError::InvalidRecord);
        };
        if previous.alive || previous.generation >= id.generation {
            return Err(NamespaceError::InvalidRecord);
        }
        let mut record = *previous;
        record.generation = id.generation;
        record.parent = parent;
        record.kind = kind;
        record.alive = true;
        record.name_length = name_length as u16;
        record.name.fill(0);
        record.name[..name_length].copy_from_slice(&payload[15..15 + name_length]);
        record.length = 0;
        record.extent_count = 0;
        record.extents.fill(CowExtent::EMPTY);
        record.data.fill(0);
        self.changes[slot] = Some(record);
        Ok(())
    }

    fn apply_rename(
        &mut self,
        base: &ObjectNamespace,
        payload: &[u8],
    ) -> Result<(), NamespaceError> {
        if payload.len() != 2 + 4 + 2 + 4 + 2 + MAX_COMPONENT_BYTES {
            return Err(NamespaceError::InvalidRecord);
        }
        let id = ObjectId::new(get_u16(payload, 0), get_u32(payload, 2))
            .ok_or(NamespaceError::InvalidRecord)?;
        if id == ObjectId::ROOT {
            return Err(NamespaceError::Root);
        }
        let parent = ObjectId::new(get_u16(payload, 6), get_u32(payload, 8))
            .ok_or(NamespaceError::InvalidRecord)?;
        let name_length = get_u16(payload, 12) as usize;
        if name_length == 0 || name_length > MAX_COMPONENT_BYTES {
            return Err(NamespaceError::InvalidRecord);
        }
        ObjectNamespace::validate_name(&payload[14..14 + name_length])?;
        let view = self.view(base);
        if view.object_record(parent)?.kind != ObjectKind::Directory {
            return Err(NamespaceError::NotDirectory);
        }
        if let Some(child) = view.find_child(parent, &payload[14..14 + name_length])?
            && child != id
        {
            return Err(NamespaceError::AlreadyExists);
        }
        if view.is_descendant(id, parent)? {
            return Err(NamespaceError::InvalidPath);
        }
        let mut record = *view.object_record(id)?;
        record.parent = parent;
        record.name_length = name_length as u16;
        record.name.fill(0);
        record.name[..name_length].copy_from_slice(&payload[14..14 + name_length]);
        self.changes[usize::from(id.slot)] = Some(record);
        Ok(())
    }

    fn apply_unlink(
        &mut self,
        base: &ObjectNamespace,
        payload: &[u8],
    ) -> Result<(), NamespaceError> {
        if payload.len() != 6 {
            return Err(NamespaceError::InvalidRecord);
        }
        let id = ObjectId::new(get_u16(payload, 0), get_u32(payload, 2))
            .ok_or(NamespaceError::InvalidRecord)?;
        if id == ObjectId::ROOT {
            return Err(NamespaceError::Root);
        }
        let view = self.view(base);
        let record = view.object_record(id)?;
        if record.kind == ObjectKind::Directory
            && (0..MAX_OBJECTS).any(|slot| {
                view.record_at(slot).is_some_and(|child| child.alive && child.parent == id)
            })
        {
            return Err(NamespaceError::NotEmpty);
        }
        let mut record = *record;
        record.alive = false;
        self.changes[usize::from(id.slot)] = Some(record);
        Ok(())
    }

    fn apply_write(
        &mut self,
        base: &ObjectNamespace,
        payload: &[u8],
    ) -> Result<(), NamespaceError> {
        if payload.len() < WRITE_HEADER_BYTES {
            return Err(NamespaceError::InvalidRecord);
        }
        let id = ObjectId::new(get_u16(payload, 0), get_u32(payload, 2))
            .ok_or(NamespaceError::InvalidRecord)?;
        let offset = get_u32(payload, 6) as usize;
        let length = get_u16(payload, 10) as usize;
        if length != payload.len() - WRITE_HEADER_BYTES
            || offset.checked_add(length).is_none()
            || offset + length > MAX_FILE_BYTES
        {
            return Err(NamespaceError::InvalidRecord);
        }
        let mut record = self.record_copy(base, id)?;
        if record.kind != ObjectKind::File {
            return Err(NamespaceError::IsDirectory);
        }
        if record.extent_count != 0 {
            return Err(NamespaceError::Unsupported);
        }
        record.data[offset..offset + length].copy_from_slice(&payload[WRITE_HEADER_BYTES..]);
        record.length = record.length.max((offset + length) as u32);
        self.changes[usize::from(id.slot)] = Some(record);
        Ok(())
    }

    fn apply_truncate(
        &mut self,
        base: &ObjectNamespace,
        payload: &[u8],
    ) -> Result<(), NamespaceError> {
        if payload.len() != 10 {
            return Err(NamespaceError::InvalidRecord);
        }
        let id = ObjectId::new(get_u16(payload, 0), get_u32(payload, 2))
            .ok_or(NamespaceError::InvalidRecord)?;
        let length = get_u32(payload, 6) as usize;
        if length > MAX_FILE_BYTES {
            return Err(NamespaceError::InvalidRecord);
        }
        let mut record = self.record_copy(base, id)?;
        if record.kind != ObjectKind::File {
            return Err(NamespaceError::IsDirectory);
        }
        if record.extent_count != 0 {
            return Err(NamespaceError::Unsupported);
        }
        record.data[length..].fill(0);
        record.length = length as u32;
        self.changes[usize::from(id.slot)] = Some(record);
        Ok(())
    }

    fn apply_record(
        &mut self,
        base: &ObjectNamespace,
        kind: u16,
        payload: &[u8],
    ) -> Result<(), NamespaceError> {
        match kind {
            CREATE_KIND => self.apply_create(base, payload),
            RENAME_KIND => self.apply_rename(base, payload),
            UNLINK_KIND => self.apply_unlink(base, payload),
            WRITE_KIND => self.apply_write(base, payload),
            TRUNCATE_KIND => self.apply_truncate(base, payload),
            _ => Err(NamespaceError::InvalidRecord),
        }
    }

    fn push_record(
        &mut self,
        base: &ObjectNamespace,
        kind: u16,
        payload: &[u8],
    ) -> Result<(), NamespaceError> {
        if self.count == MAX_TRANSACTION_RECORDS {
            return Err(NamespaceError::Capacity);
        }
        if payload.len() > MAX_RECORD_PAYLOAD_BYTES {
            return Err(NamespaceError::TooLarge);
        }
        let mut record = PendingRecord::EMPTY;
        record.kind = kind;
        record.len = payload.len() as u16;
        record.payload[..payload.len()].copy_from_slice(payload);
        self.apply_record(base, kind, &record.payload[..payload.len()])?;
        self.records[self.count] = record;
        self.count += 1;
        Ok(())
    }

    pub fn create_file(
        &mut self,
        base: &ObjectNamespace,
        path: &[u8],
    ) -> Result<ObjectId, NamespaceError> {
        let (id, payload) = {
            let view = self.view(base);
            let (parent, name) = view.parent_and_name(path)?;
            let (id, payload) = view.plan_create(parent, ObjectKind::File, name)?;
            (id, payload)
        };
        self.push_record(base, CREATE_KIND, &payload)?;
        Ok(id)
    }

    pub fn list(&self, base: &ObjectNamespace, path: &[u8]) -> Result<ObjectList, NamespaceError> {
        let view = self.view(base);
        let parent = view.resolve_path(path)?;
        view.list(parent)
    }

    pub fn stat(&self, base: &ObjectNamespace, path: &[u8]) -> Result<ObjectInfo, NamespaceError> {
        let view = self.view(base);
        let id = view.resolve_path(path)?;
        view.stat(id)
    }

    pub fn stat_id(
        &self,
        base: &ObjectNamespace,
        id: ObjectId,
    ) -> Result<ObjectInfo, NamespaceError> {
        self.view(base).stat(id)
    }

    pub fn read(
        &self,
        base: &ObjectNamespace,
        path: &[u8],
        offset: usize,
        output: &mut [u8],
    ) -> Result<usize, NamespaceError> {
        let view = self.view(base);
        let id = view.resolve_path(path)?;
        view.read(id, offset, output)
    }

    pub fn write(
        &mut self,
        base: &ObjectNamespace,
        path: &[u8],
        offset: usize,
        input: &[u8],
        replace: bool,
    ) -> Result<usize, NamespaceError> {
        let id = self.view(base).resolve_path(path)?;
        let record = self.record_copy(base, id)?;
        if record.kind != ObjectKind::File {
            return Err(NamespaceError::IsDirectory);
        }
        if input.is_empty() && !replace {
            return Err(NamespaceError::TooLarge);
        }
        if offset.checked_add(input.len()).is_none() || offset + input.len() > MAX_FILE_BYTES {
            return Err(NamespaceError::TooLarge);
        }
        let count = input.len().div_ceil(MAX_WRITE_BYTES);
        let truncate_count = usize::from(replace && record.length != 0);
        if count > MAX_WRITE_RECORDS
            || self.count + truncate_count + count > MAX_TRANSACTION_RECORDS
        {
            return Err(NamespaceError::Capacity);
        }
        if replace && record.length != 0 {
            let mut truncate = [0; 10];
            put_u16(&mut truncate, 0, id.slot);
            put_u32(&mut truncate, 2, id.generation);
            put_u32(&mut truncate, 6, 0);
            self.push_record(base, TRUNCATE_KIND, &truncate)?;
        }
        if input.is_empty() {
            return Ok(0);
        }
        let mut copied = 0;
        for _ in 0..count {
            let length = (input.len() - copied).min(MAX_WRITE_BYTES);
            let mut payload = [0; MAX_RECORD_PAYLOAD_BYTES];
            put_u16(&mut payload, 0, id.slot);
            put_u32(&mut payload, 2, id.generation);
            put_u32(&mut payload, 6, (offset + copied) as u32);
            put_u16(&mut payload, 10, length as u16);
            payload[WRITE_HEADER_BYTES..WRITE_HEADER_BYTES + length]
                .copy_from_slice(&input[copied..copied + length]);
            self.push_record(base, WRITE_KIND, &payload[..WRITE_HEADER_BYTES + length])?;
            copied += length;
        }
        Ok(input.len())
    }

    pub fn remove(&mut self, base: &ObjectNamespace, path: &[u8]) -> Result<(), NamespaceError> {
        let view = self.view(base);
        let id = view.resolve_path(path)?;
        let (extents, extent_count) = view.file_extents(id)?;
        let mut payload = [0; 6];
        put_u16(&mut payload, 0, id.slot);
        put_u32(&mut payload, 2, id.generation);
        self.push_record(base, UNLINK_KIND, &payload)?;
        if self.retired_extent_count + extent_count > self.retired_extents.len() {
            return Err(NamespaceError::Capacity);
        }
        let end = self.retired_extent_count + extent_count;
        self.retired_extents[self.retired_extent_count..end]
            .copy_from_slice(&extents[..extent_count]);
        self.retired_extent_count = end;
        Ok(())
    }

    pub fn rename(
        &mut self,
        base: &ObjectNamespace,
        from: &[u8],
        to: &[u8],
    ) -> Result<(), NamespaceError> {
        let view = self.view(base);
        let id = view.resolve_path(from)?;
        let (parent, name) = view.parent_and_name(to)?;
        if id == ObjectId::ROOT {
            return Err(NamespaceError::Root);
        }
        let mut payload = [0; 2 + 4 + 2 + 4 + 2 + MAX_COMPONENT_BYTES];
        put_u16(&mut payload, 0, id.slot);
        put_u32(&mut payload, 2, id.generation);
        put_u16(&mut payload, 6, parent.slot);
        put_u32(&mut payload, 8, parent.generation);
        put_u16(&mut payload, 12, name.len() as u16);
        payload[14..14 + name.len()].copy_from_slice(name);
        self.push_record(base, RENAME_KIND, &payload)
    }

    pub fn commit<B: BlockStore>(
        self,
        namespace: &mut DurableNamespace<B>,
    ) -> Result<u64, NamespaceError> {
        for slot in 0..MAX_OBJECTS {
            if let Some(record) = self.changes[slot] {
                namespace.namespace.records[slot] = record;
            }
        }
        namespace.retired_file_extents = self.retired_extents;
        namespace.retired_file_extent_count = self.retired_extent_count;
        match namespace.persist_snapshot() {
            Ok(generation) => Ok(generation.saturating_sub(1)),
            Err(error) => Err(match namespace.reopen() {
                Ok(()) => error,
                Err(recovery) => recovery,
            }),
        }
    }

    pub fn abort(self) {}
}

fn put_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn get_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn get_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([bytes[offset], bytes[offset + 1], bytes[offset + 2], bytes[offset + 3]])
}

#[cfg(test)]
mod tests {
    use super::*;
    use logos_abi::ServiceId;
    use logos_package::{
        PACKAGE_HEADER_BYTES, PACKAGE_HEADER_V2_BYTES, PackageDependency, PackageHeaderV2,
        PackageManifest, PackageName, SemanticVersion, ServicePackageHeader, crc32c,
    };
    use logos_storage::{Block, BlockError, BlockIndex, BlockStore, MemoryBlockStore};
    use std::boxed::Box;
    use std::vec;

    struct HeapStore(Box<MemoryBlockStore<96>>);

    impl BlockStore for HeapStore {
        fn block_count(&self) -> u64 {
            self.0.block_count()
        }

        fn read_block(&mut self, index: BlockIndex, output: &mut Block) -> Result<(), BlockError> {
            self.0.read_block(index, output)
        }

        fn write_block(&mut self, index: BlockIndex, input: &Block) -> Result<(), BlockError> {
            self.0.write_block(index, input)
        }

        fn flush(&mut self) -> Result<(), BlockError> {
            self.0.flush()
        }
    }

    fn heap_store() -> HeapStore {
        HeapStore(Box::new(MemoryBlockStore::new()))
    }

    fn install(
        filesystem: &mut DurableNamespace<HeapStore>,
        service: ServiceId,
        payload: &[u8],
        version: u32,
    ) -> PackageHandle {
        let header =
            ServicePackageHeader::new(service, version, payload.len(), crc32c(payload)).unwrap();
        let mut package = vec![0; PACKAGE_HEADER_BYTES + payload.len()];
        header.encode(&mut package).unwrap();
        package[PACKAGE_HEADER_BYTES..].copy_from_slice(payload);
        let mut transaction = filesystem.begin_package_install(service, package.len()).unwrap();
        for (offset, chunk) in package.chunks(BLOCK_BYTES).enumerate() {
            filesystem.write_package_chunk(&mut transaction, offset * BLOCK_BYTES, chunk).unwrap();
        }
        filesystem.commit_package_install(transaction).unwrap()
    }

    fn install_v2(
        filesystem: &mut DurableNamespace<HeapStore>,
        service: ServiceId,
        name: &[u8],
    ) -> PackageHandle {
        let manifest = PackageManifest::for_service(
            PackageName::parse(name).unwrap(),
            SemanticVersion::new(1, 2, 3),
            service,
        );
        install_v2_manifest(filesystem, manifest, b"managed-elf").unwrap()
    }

    fn install_v2_manifest(
        filesystem: &mut DurableNamespace<HeapStore>,
        manifest: PackageManifest,
        payload: &[u8],
    ) -> Result<PackageHandle, NamespaceError> {
        let service = match manifest.target {
            logos_package::PackageTarget::Service(service) => service,
            logos_package::PackageTarget::None => panic!("test package must target a service"),
        };
        let header = PackageHeaderV2::new(manifest, payload.len(), crc32c(payload)).unwrap();
        let mut package = vec![0; PACKAGE_HEADER_V2_BYTES + payload.len()];
        header.encode(&mut package).unwrap();
        package[PACKAGE_HEADER_V2_BYTES..].copy_from_slice(payload);
        let mut transaction = filesystem.begin_package_install(service, package.len()).unwrap();
        for (offset, chunk) in package.chunks(BLOCK_BYTES).enumerate() {
            filesystem.write_package_chunk(&mut transaction, offset * BLOCK_BYTES, chunk).unwrap();
        }
        filesystem.commit_package_install(transaction)
    }

    #[test]
    fn object_lifecycle_is_bounded_and_generation_safe() {
        let mut namespace = ObjectNamespace::new();
        let (id, payload) =
            namespace.plan_create(ObjectId::ROOT, ObjectKind::File, b"one").unwrap();
        namespace.apply_record(CREATE_KIND, &payload).unwrap();
        assert_eq!(namespace.resolve_path(b"/one").unwrap(), id);
        assert_eq!(namespace.list(ObjectId::ROOT).unwrap().len(), 1);
        let stale = id;
        let mut unlink = [0; 6];
        put_u16(&mut unlink, 0, id.slot);
        put_u32(&mut unlink, 2, id.generation);
        namespace.apply_record(UNLINK_KIND, &unlink).unwrap();
        assert_eq!(namespace.stat(stale), Err(NamespaceError::Stale));
    }

    #[test]
    fn durable_namespace_recovers_write_rename_and_unlink() {
        let store = MemoryBlockStore::<16>::new();
        let mut fs = DurableNamespace::format(store).unwrap();
        let file = fs.create(fs.root(), ObjectKind::File, b"old").unwrap();
        let mut input = [0; BLOCK_BYTES * 2];
        input[0] = 0x5a;
        input[BLOCK_BYTES + 1] = 0xa5;
        assert_eq!(fs.write(file, 0, &input).unwrap(), input.len());
        fs.rename(file, fs.root(), b"new").unwrap();

        let store = fs.into_store();
        let mut reopened = DurableNamespace::open(store).unwrap();
        let reopened_file = reopened.resolve_path(b"/new").unwrap();
        let mut output = [0; BLOCK_BYTES * 2];
        assert_eq!(reopened.read(reopened_file, 0, &mut output).unwrap(), input.len());
        assert_eq!(output, input);
        reopened.unlink(reopened_file).unwrap();
        let store = reopened.into_store();
        let reopened = DurableNamespace::open(store).unwrap();
        assert_eq!(reopened.resolve_path(b"/new"), Err(NamespaceError::NotFound));
    }

    #[test]
    fn multi_extent_files_reopen_and_reclaim_old_space() {
        let mut fs = DurableNamespace::format(heap_store()).unwrap();
        let file = fs.create_file(fs.root(), b"multi").unwrap();
        let mut transaction = fs.volume.begin(&mut fs.store).unwrap();
        let first = transaction.allocate_blocks(&mut fs.store, 1).unwrap();
        let _blocker = transaction.allocate_blocks(&mut fs.store, 1).unwrap();
        let second = transaction.allocate_blocks(&mut fs.store, 1).unwrap();
        let mut first_block = Block::zero();
        first_block.as_bytes_mut().fill(0x11);
        let mut second_block = Block::zero();
        second_block.as_bytes_mut().fill(0x22);
        transaction.write_block(&mut fs.store, first.start, &first_block).unwrap();
        transaction.write_block(&mut fs.store, second.start, &second_block).unwrap();
        let mut next = fs.namespace;
        next.set_file_extents(file, &[first, second], BLOCK_BYTES * 2).unwrap();
        fs.namespace = next;
        fs.persist_snapshot_with(transaction).unwrap();

        let (old_extents, old_extent_count) = fs.namespace.file_extents(file).unwrap();
        assert_eq!(old_extent_count, 2);
        let mut output = vec![0; BLOCK_BYTES * 2];
        assert_eq!(fs.read(file, 0, &mut output).unwrap(), output.len());
        assert_eq!(&output[..BLOCK_BYTES], &[0x11; BLOCK_BYTES]);
        assert_eq!(&output[BLOCK_BYTES..], &[0x22; BLOCK_BYTES]);

        let patch = [0x44; 32];
        fs.write_handle(file, BLOCK_BYTES, &patch).unwrap();
        let mut output = vec![0; BLOCK_BYTES * 2];
        fs.read(file, 0, &mut output).unwrap();
        assert_eq!(&output[..BLOCK_BYTES], &[0x11; BLOCK_BYTES]);
        assert_eq!(&output[BLOCK_BYTES..BLOCK_BYTES + patch.len()], &patch);
        assert!(output[BLOCK_BYTES + patch.len()..].iter().all(|byte| *byte == 0x22));

        let store = fs.into_store();
        let mut reopened = DurableNamespace::open(store).unwrap();
        let reopened_file = reopened.resolve_path(b"/multi").unwrap();
        let mut reopened_output = vec![0; BLOCK_BYTES * 2];
        reopened.read(reopened_file, 0, &mut reopened_output).unwrap();
        assert_eq!(reopened_output, output);

        reopened.create_file(reopened.root(), b"reclaim").unwrap();
        let root = reopened.volume.root();
        let mut bitmap = Block::zero();
        reopened
            .store
            .read_block(
                BlockIndex::new(root.bitmap_start.get() + root.bitmap_slot as u64),
                &mut bitmap,
            )
            .unwrap();
        for extent in old_extents[..old_extent_count].iter() {
            for index in extent.start.get()..extent.start.get() + extent.blocks as u64 {
                assert_eq!(bitmap.as_bytes()[index as usize / 8] & (1 << (index % 8)), 0);
            }
        }
    }

    #[test]
    fn same_name_rename_is_recoverable() {
        let store = MemoryBlockStore::<16>::new();
        let mut fs = DurableNamespace::format(store).unwrap();
        let file = fs.create_file(fs.root(), b"same").unwrap();
        fs.rename(file, fs.root(), b"same").unwrap();

        let store = fs.into_store();
        let reopened = DurableNamespace::open(store).unwrap();
        assert_eq!(reopened.resolve_path(b"/same"), Ok(file));
    }

    #[test]
    fn path_depth_and_component_bounds_are_rejected() {
        let namespace = ObjectNamespace::new();
        assert_eq!(namespace.resolve_path(b"relative"), Err(NamespaceError::InvalidPath));
        assert_eq!(
            namespace.resolve_path(&[b'/'; MAX_COMPONENT_BYTES + 2]),
            Err(NamespaceError::InvalidPath)
        );
    }

    #[test]
    fn file_open_and_slot_reuse_reject_stale_ids() {
        let store = MemoryBlockStore::<16>::new();
        let mut fs = DurableNamespace::format(store).unwrap();
        let first = fs.create_file(fs.root(), b"first").unwrap();
        assert_eq!(fs.open_file(b"/first"), Ok(first));
        assert_eq!(fs.open_file(b"/"), Err(NamespaceError::IsDirectory));
        fs.unlink(first).unwrap();
        let replacement = fs.create_file(fs.root(), b"second").unwrap();
        assert_eq!(replacement.slot(), first.slot());
        assert_ne!(replacement.generation(), first.generation());
        assert_eq!(fs.stat(first), Err(NamespaceError::Stale));
    }

    #[test]
    fn root_and_generation_invariants_are_rejected() {
        let store = MemoryBlockStore::<16>::new();
        let mut fs = DurableNamespace::format(store).unwrap();
        assert_eq!(fs.rename(fs.root(), fs.root(), b"root"), Err(NamespaceError::Root));

        let mut namespace = ObjectNamespace::new();
        namespace.records[1].generation = u32::MAX;
        assert_eq!(
            namespace.plan_create(ObjectId::ROOT, ObjectKind::File, b"file"),
            Err(NamespaceError::GenerationExhausted)
        );
    }

    #[test]
    fn replay_rejects_directory_cycles() {
        let mut namespace = ObjectNamespace::new();
        let (parent, create_parent) =
            namespace.plan_create(ObjectId::ROOT, ObjectKind::Directory, b"parent").unwrap();
        namespace.apply_record(CREATE_KIND, &create_parent).unwrap();
        let (child, create_child) =
            namespace.plan_create(parent, ObjectKind::Directory, b"child").unwrap();
        namespace.apply_record(CREATE_KIND, &create_child).unwrap();

        let mut rename = [0; 2 + 4 + 2 + 4 + 2 + MAX_COMPONENT_BYTES];
        put_u16(&mut rename, 0, parent.slot);
        put_u32(&mut rename, 2, parent.generation);
        put_u16(&mut rename, 6, child.slot);
        put_u32(&mut rename, 8, child.generation);
        put_u16(&mut rename, 12, 5);
        rename[14..19].copy_from_slice(b"moved");

        assert_eq!(namespace.apply_record(RENAME_KIND, &rename), Err(NamespaceError::InvalidPath));
    }

    #[test]
    fn transaction_supports_read_your_writes_and_atomic_reopen() {
        let store = MemoryBlockStore::<32>::new();
        let mut fs = DurableNamespace::format(store).unwrap();
        let mut transaction = fs.begin_transaction();
        transaction.create_file(fs.transaction_base(), b"/proof").unwrap();
        transaction.write(fs.transaction_base(), b"/proof", 0, b"durable", true).unwrap();
        let mut output = [0; 7];
        assert_eq!(transaction.read(fs.transaction_base(), b"/proof", 0, &mut output).unwrap(), 7);
        assert_eq!(&output, b"durable");
        transaction.rename(fs.transaction_base(), b"/proof", b"/committed").unwrap();
        let transaction_id = transaction.commit(&mut fs).unwrap();
        assert_eq!(transaction_id, 1);

        let store = fs.into_store();
        let mut reopened = DurableNamespace::open(store).unwrap();
        let id = reopened.resolve_path(b"/committed").unwrap();
        let mut output = [0; 7];
        assert_eq!(reopened.read(id, 0, &mut output).unwrap(), 7);
        assert_eq!(&output, b"durable");
    }

    #[test]
    fn checkpointed_namespace_reopens_without_losing_files() {
        let store = MemoryBlockStore::<32>::new();
        let mut fs = DurableNamespace::format(store).unwrap();
        let file = fs.create_file(fs.root(), b"proof").unwrap();
        fs.write(file, 0, b"durable").unwrap();

        fs.reopen().unwrap();
        let file = fs.resolve_path(b"/proof").unwrap();
        let mut output = [0; 7];
        assert_eq!(fs.read(file, 0, &mut output).unwrap(), 7);
        assert_eq!(&output, b"durable");
    }

    #[test]
    fn replace_write_capacity_check_does_not_stage_truncate() {
        let store = MemoryBlockStore::<32>::new();
        let mut fs = DurableNamespace::format(store).unwrap();
        let file = fs.create_file(fs.root(), b"file").unwrap();
        fs.write(file, 0, b"x").unwrap();

        let mut transaction = fs.begin_transaction();
        for offset in 1..MAX_TRANSACTION_RECORDS {
            transaction.write(fs.transaction_base(), b"/file", offset, b"x", false).unwrap();
        }
        assert_eq!(
            transaction.write(fs.transaction_base(), b"/file", 0, b"replacement", true),
            Err(NamespaceError::Capacity)
        );
        assert_eq!(transaction.record_count(), MAX_TRANSACTION_RECORDS - 1);
        assert_eq!(
            transaction.stat(fs.transaction_base(), b"/file").unwrap().length,
            MAX_TRANSACTION_RECORDS as u32
        );
    }

    #[test]
    fn aborted_transaction_does_not_change_namespace() {
        let store = MemoryBlockStore::<16>::new();
        let fs = DurableNamespace::format(store).unwrap();
        let mut transaction = fs.begin_transaction();
        transaction.create_file(fs.transaction_base(), b"/discarded").unwrap();
        transaction.abort();
        assert_eq!(fs.resolve_path(b"/discarded"), Err(NamespaceError::NotFound));
    }

    #[test]
    fn package_install_has_one_owner_and_rejects_stale_handles() {
        let mut fs = DurableNamespace::format(heap_store()).unwrap();
        let first = fs.begin_package_install(ServiceId::Flow, PACKAGE_HEADER_BYTES).unwrap();
        let mut stale = first;
        assert!(matches!(
            fs.begin_package_install(ServiceId::Storage, PACKAGE_HEADER_BYTES),
            Err(NamespaceError::Package(PackageCatalogError::Busy))
        ));
        fs.abort_package_install(first);

        let second = fs.begin_package_install(ServiceId::Storage, PACKAGE_HEADER_BYTES).unwrap();
        assert_eq!(
            fs.write_package_chunk(&mut stale, 0, &[0; PACKAGE_HEADER_BYTES]),
            Err(NamespaceError::InvalidRecord)
        );
        fs.abort_package_install(second);
    }

    #[test]
    fn packages_use_actual_extents_and_reuse_aborted_space() {
        let mut fs = DurableNamespace::format(heap_store()).unwrap();
        let first_payload = [0x11; 100];
        let first = install(&mut fs, ServiceId::Storage, &first_payload, 1);
        let first_info = fs.lookup_package(ServiceId::Storage).unwrap();
        assert_eq!(first_info.bytes as usize, PACKAGE_HEADER_BYTES + first_payload.len());
        assert_eq!(first_info.extent_count, 1);
        assert_eq!(first_info.extents[0].blocks, 1);

        let mut abandoned = fs.begin_package_install(ServiceId::Flow, BLOCK_BYTES * 2 + 1).unwrap();
        let abandoned_start = abandoned.extents[0].start;
        fs.write_package_chunk(&mut abandoned, 0, &[0; BLOCK_BYTES]).unwrap();
        fs.abort_package_install(abandoned);

        let second_payload = [0x22; BLOCK_BYTES + 32];
        let second = install(&mut fs, ServiceId::Flow, &second_payload, 2);
        let second_info = fs.lookup_package(ServiceId::Flow).unwrap();
        assert_eq!(second_info.extents[0].blocks, 2);
        assert_eq!(second_info.extents[0].start, abandoned_start);
        assert_eq!(fs.lookup_package(ServiceId::Storage).unwrap().handle, first);
        assert_eq!(fs.lookup_package(ServiceId::Flow).unwrap().handle, second);
    }

    #[test]
    fn v2_manifest_is_read_from_durable_package_header() {
        let mut fs = DurableNamespace::format(heap_store()).unwrap();
        let handle = install_v2(&mut fs, ServiceId::Flow, b"flow-managed");
        let info = fs.lookup_package(ServiceId::Flow).unwrap();
        assert_eq!(info.handle, handle);
        assert_eq!(
            info.manifest.map(|manifest| manifest.version),
            Some(SemanticVersion::new(1, 2, 3))
        );

        fs.reopen().unwrap();
        assert_eq!(
            fs.lookup_package(ServiceId::Flow).unwrap().manifest.map(|manifest| manifest.name),
            Some(PackageName::parse(b"flow-managed").unwrap())
        );
    }

    #[test]
    fn v2_updates_require_newer_versions_and_preserve_dependency_ranges() {
        let mut fs = DurableNamespace::format(heap_store()).unwrap();
        let base_name = PackageName::parse(b"base").unwrap();
        let app_name = PackageName::parse(b"app").unwrap();
        install_v2_manifest(
            &mut fs,
            PackageManifest::for_service(
                base_name,
                SemanticVersion::new(1, 0, 0),
                ServiceId::Storage,
            ),
            b"base",
        )
        .unwrap();

        let mut app =
            PackageManifest::for_service(app_name, SemanticVersion::new(1, 0, 0), ServiceId::Flow);
        app.add_dependency(PackageDependency::new(base_name, b"^1.0.0").unwrap()).unwrap();
        install_v2_manifest(&mut fs, app, b"app").unwrap();

        let same = PackageManifest::for_service(
            base_name,
            SemanticVersion::new(1, 0, 0),
            ServiceId::Storage,
        );
        assert_eq!(
            install_v2_manifest(&mut fs, same, b"same"),
            Err(NamespaceError::Package(PackageCatalogError::VersionConflict))
        );
        let incompatible = PackageManifest::for_service(
            base_name,
            SemanticVersion::new(2, 0, 0),
            ServiceId::Storage,
        );
        assert_eq!(
            install_v2_manifest(&mut fs, incompatible, b"incompatible"),
            Err(NamespaceError::Package(PackageCatalogError::DependencyConflict))
        );
        let renamed = PackageManifest::for_service(
            PackageName::parse(b"renamed").unwrap(),
            SemanticVersion::new(2, 0, 0),
            ServiceId::Storage,
        );
        assert_eq!(
            install_v2_manifest(&mut fs, renamed, b"renamed"),
            Err(NamespaceError::Package(PackageCatalogError::DependencyConflict))
        );
        let compatible = PackageManifest::for_service(
            base_name,
            SemanticVersion::new(1, 1, 0),
            ServiceId::Storage,
        );
        install_v2_manifest(&mut fs, compatible, b"compatible").unwrap();
    }

    #[test]
    fn legacy_package_cannot_replace_a_v2_manifest() {
        let mut fs = DurableNamespace::format(heap_store()).unwrap();
        install_v2(&mut fs, ServiceId::Storage, b"storage");

        let header =
            ServicePackageHeader::new(ServiceId::Storage, 99, 6, crc32c(b"legacy")).unwrap();
        let mut package = vec![0; PACKAGE_HEADER_BYTES + 6];
        header.encode(&mut package).unwrap();
        package[PACKAGE_HEADER_BYTES..].copy_from_slice(b"legacy");
        let mut transaction = fs.begin_package_install(ServiceId::Storage, package.len()).unwrap();
        for (offset, chunk) in package.chunks(BLOCK_BYTES).enumerate() {
            fs.write_package_chunk(&mut transaction, offset * BLOCK_BYTES, chunk).unwrap();
        }
        assert_eq!(
            fs.commit_package_install(transaction),
            Err(NamespaceError::Package(PackageCatalogError::VersionConflict))
        );
        assert_eq!(
            fs.lookup_package(ServiceId::Storage).unwrap().manifest.unwrap().name,
            PackageName::parse(b"storage").unwrap()
        );
    }

    #[test]
    fn package_file_import_reuses_validation_and_publishes_generation() {
        let mut fs = DurableNamespace::format(heap_store()).unwrap();
        let payload = b"elf";
        let header =
            ServicePackageHeader::new(ServiceId::Flow, 7, payload.len(), crc32c(payload)).unwrap();
        let mut package = vec![0; PACKAGE_HEADER_BYTES + payload.len()];
        header.encode(&mut package).unwrap();
        package[PACKAGE_HEADER_BYTES..].copy_from_slice(payload);
        let source = fs.create_file(fs.root(), b"import.pkg").unwrap();
        fs.write(source, 0, &package).unwrap();

        let handle = fs.install_package_file(b"/import.pkg").unwrap();
        assert_eq!(fs.lookup_package(ServiceId::Flow).unwrap().handle, handle);
        let mut output = [0; 35];
        assert_eq!(fs.read_package(handle, 0, &mut output).unwrap(), output.len());
        assert_eq!(&output[PACKAGE_HEADER_BYTES..], payload);
    }

    #[test]
    fn incomplete_package_write_is_not_published_after_reopen() {
        let mut fs = DurableNamespace::format(heap_store()).unwrap();
        let incomplete_start = {
            let mut incomplete =
                fs.begin_package_install(ServiceId::Flow, BLOCK_BYTES * 2 + 1).unwrap();
            let incomplete_start = incomplete.extents[0].start;
            fs.write_package_chunk(&mut incomplete, 0, &[0; BLOCK_BYTES]).unwrap();
            incomplete_start
        };

        let store = fs.into_store();
        let mut reopened = DurableNamespace::open(store).unwrap();
        assert_eq!(reopened.lookup_package(ServiceId::Flow), Err(NamespaceError::NotFound));

        let payload = [0x55; BLOCK_BYTES + 32];
        let handle = install(&mut reopened, ServiceId::Flow, &payload, 1);
        let info = reopened.lookup_package(ServiceId::Flow).unwrap();
        assert_eq!(info.handle, handle);
        assert_eq!(info.extents[0].start, incomplete_start);
    }

    #[test]
    fn replacement_is_generation_safe_and_reopens_from_journal() {
        let mut fs = DurableNamespace::format(heap_store()).unwrap();
        let old_payload = [0x33; 100];
        let old = install(&mut fs, ServiceId::Storage, &old_payload, 1);
        let new_payload = [0x44; BLOCK_BYTES + 100];
        let new = install(&mut fs, ServiceId::Storage, &new_payload, 2);
        assert_ne!(old.generation, new.generation);
        let mut output = vec![0; PACKAGE_HEADER_BYTES + new_payload.len()];
        assert_eq!(fs.read_package(new, 0, &mut output).unwrap(), output.len());
        assert_eq!(&output[PACKAGE_HEADER_BYTES..], &new_payload);
        assert_eq!(
            fs.read_package(old, 0, &mut output),
            Err(NamespaceError::Package(PackageCatalogError::Stale))
        );

        let store = fs.into_store();
        let mut reopened = DurableNamespace::open(store).unwrap();
        let info = reopened.lookup_package(ServiceId::Storage).unwrap();
        assert_eq!(info.handle, new);
        let mut reopened_output = vec![0; output.len()];
        assert_eq!(reopened.read_package(new, 0, &mut reopened_output).unwrap(), output.len());
        assert_eq!(reopened_output, output);
    }

    #[test]
    fn ordinary_files_keep_the_8k_limit_and_v2_rejects_package_operations() {
        let mut fs = DurableNamespace::format(heap_store()).unwrap();
        let file = fs.create_file(fs.root(), b"ordinary").unwrap();
        assert_eq!(fs.write(file, 0, &[0; MAX_FILE_BYTES]).unwrap(), MAX_FILE_BYTES);
        assert_eq!(fs.write(file, MAX_FILE_BYTES, &[1]), Err(NamespaceError::TooLarge));

        let mut legacy_store = heap_store();
        logos_storage::Volume::format_as(&mut legacy_store, logos_storage::V2_FORMAT_VERSION)
            .unwrap();
        assert!(matches!(
            DurableNamespace::open(legacy_store),
            Err(NamespaceError::Format(logos_storage::FormatError::UnsupportedVersion))
        ));
    }
}
