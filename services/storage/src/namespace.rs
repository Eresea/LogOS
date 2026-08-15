use logos_storage::{
    BLOCK_BYTES, BlockError, BlockStore, FormatError, JournalRecord, MAX_RECORD_PAYLOAD_BYTES,
    ReplayError, ReplaySink, Volume,
};

pub const MAX_OBJECTS: usize = 4;
pub const MAX_COMPONENT_BYTES: usize = 255;
pub const MAX_PATH_DEPTH: usize = 32;
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
}

impl From<FormatError> for NamespaceError {
    fn from(error: FormatError) -> Self {
        Self::Format(error)
    }
}

impl From<BlockError> for NamespaceError {
    fn from(error: BlockError) -> Self {
        Self::Block(error)
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
        if offset > record.length as usize {
            return Err(NamespaceError::TooLarge);
        }
        let length = output.len().min(record.length as usize - offset);
        output[..length].copy_from_slice(&record.data[offset..offset + length]);
        Ok(length)
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

impl ReplaySink for ObjectNamespace {
    fn record(
        &mut self,
        _transaction_id: u64,
        kind: u16,
        payload: &[u8],
    ) -> Result<(), ReplayError> {
        self.apply_record(kind, payload).map_err(|_| ReplayError::Rejected)
    }
}

pub struct DurableNamespace<B> {
    store: B,
    volume: Volume,
    namespace: ObjectNamespace,
}

impl<B: BlockStore> DurableNamespace<B> {
    pub fn format(mut store: B) -> Result<Self, NamespaceError> {
        let volume = Volume::format(&mut store)?;
        Ok(Self { store, volume, namespace: ObjectNamespace::new() })
    }

    pub fn open(mut store: B) -> Result<Self, NamespaceError> {
        let mut volume = Volume::open(&mut store)?;
        let mut namespace = ObjectNamespace::new();
        volume.recover(&mut store, &mut namespace)?;
        Ok(Self { store, volume, namespace })
    }

    fn reopen(&mut self) -> Result<(), NamespaceError> {
        let mut volume = Volume::open(&mut self.store)?;
        let mut namespace = ObjectNamespace::new();
        volume.recover(&mut self.store, &mut namespace)?;
        self.volume = volume;
        self.namespace = namespace;
        Ok(())
    }

    pub fn root(&self) -> ObjectId {
        self.namespace.root()
    }

    pub fn into_store(self) -> B {
        self.store
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
        &self,
        id: ObjectId,
        offset: usize,
        output: &mut [u8],
    ) -> Result<usize, NamespaceError> {
        self.namespace.read(id, offset, output)
    }

    pub fn begin_transaction(&self) -> NamespaceTransaction {
        NamespaceTransaction::new(self.namespace)
    }

    fn commit_one(&mut self, kind: u16, payload: &[u8]) -> Result<(), NamespaceError> {
        self.volume.commit(&mut self.store, &[JournalRecord { kind, payload }])?;
        self.namespace.apply_record(kind, payload)
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
        self.volume.commit(&mut self.store, &records[..count])?;
        for record in records.iter().take(count) {
            self.namespace.apply_record(WRITE_KIND, record.payload)?;
        }
        Ok(input.len())
    }

    pub fn flush(&mut self) -> Result<(), NamespaceError> {
        self.store.flush().map_err(NamespaceError::Block)
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

pub struct NamespaceTransaction {
    shadow: ObjectNamespace,
    records: [PendingRecord; MAX_TRANSACTION_RECORDS],
    count: usize,
}

impl NamespaceTransaction {
    fn new(namespace: ObjectNamespace) -> Self {
        Self {
            shadow: namespace,
            records: [PendingRecord::EMPTY; MAX_TRANSACTION_RECORDS],
            count: 0,
        }
    }

    pub const fn record_count(&self) -> usize {
        self.count
    }

    fn push_record(&mut self, kind: u16, payload: &[u8]) -> Result<(), NamespaceError> {
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
        self.shadow.apply_record(kind, &record.payload[..payload.len()])?;
        self.records[self.count] = record;
        self.count += 1;
        Ok(())
    }

    pub fn create_file(&mut self, path: &[u8]) -> Result<ObjectId, NamespaceError> {
        let (parent, name) = self.shadow.parent_and_name(path)?;
        let (id, payload) = self.shadow.plan_create(parent, ObjectKind::File, name)?;
        self.push_record(CREATE_KIND, &payload)?;
        Ok(id)
    }

    pub fn list(&self, path: &[u8]) -> Result<ObjectList, NamespaceError> {
        let parent = self.shadow.resolve_path(path)?;
        self.shadow.list(parent)
    }

    pub fn stat(&self, path: &[u8]) -> Result<ObjectInfo, NamespaceError> {
        let id = self.shadow.resolve_path(path)?;
        self.shadow.stat(id)
    }

    pub fn stat_id(&self, id: ObjectId) -> Result<ObjectInfo, NamespaceError> {
        self.shadow.stat(id)
    }

    pub fn read(
        &self,
        path: &[u8],
        offset: usize,
        output: &mut [u8],
    ) -> Result<usize, NamespaceError> {
        let id = self.shadow.resolve_path(path)?;
        self.shadow.read(id, offset, output)
    }

    pub fn write(
        &mut self,
        path: &[u8],
        offset: usize,
        input: &[u8],
        replace: bool,
    ) -> Result<usize, NamespaceError> {
        let id = self.shadow.resolve_path(path)?;
        let record = self.shadow.object_record(id)?;
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
            self.push_record(TRUNCATE_KIND, &truncate)?;
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
            self.push_record(WRITE_KIND, &payload[..WRITE_HEADER_BYTES + length])?;
            copied += length;
        }
        Ok(input.len())
    }

    pub fn remove(&mut self, path: &[u8]) -> Result<(), NamespaceError> {
        let id = self.shadow.resolve_path(path)?;
        let mut payload = [0; 6];
        put_u16(&mut payload, 0, id.slot);
        put_u32(&mut payload, 2, id.generation);
        self.push_record(UNLINK_KIND, &payload)
    }

    pub fn rename(&mut self, from: &[u8], to: &[u8]) -> Result<(), NamespaceError> {
        let id = self.shadow.resolve_path(from)?;
        let (parent, name) = self.shadow.parent_and_name(to)?;
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
        self.push_record(RENAME_KIND, &payload)
    }

    pub fn commit<B: BlockStore>(
        self,
        namespace: &mut DurableNamespace<B>,
    ) -> Result<u64, NamespaceError> {
        let mut records = [JournalRecord { kind: 0, payload: &[] }; MAX_TRANSACTION_RECORDS];
        for (index, record) in self.records[..self.count].iter().enumerate() {
            records[index] = JournalRecord {
                kind: record.kind,
                payload: &record.payload[..record.len as usize],
            };
        }
        let transaction_id =
            match namespace.volume.commit(&mut namespace.store, &records[..self.count]) {
                Ok(transaction_id) => transaction_id,
                Err(error) => {
                    let _ = namespace.reopen();
                    return Err(error.into());
                }
            };
        namespace.namespace = self.shadow;
        Ok(transaction_id)
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
    use logos_storage::MemoryBlockStore;

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
        transaction.create_file(b"/proof").unwrap();
        transaction.write(b"/proof", 0, b"durable", true).unwrap();
        let mut output = [0; 7];
        assert_eq!(transaction.read(b"/proof", 0, &mut output).unwrap(), 7);
        assert_eq!(&output, b"durable");
        transaction.rename(b"/proof", b"/committed").unwrap();
        let transaction_id = transaction.commit(&mut fs).unwrap();
        assert_eq!(transaction_id, 1);

        let store = fs.into_store();
        let reopened = DurableNamespace::open(store).unwrap();
        let id = reopened.resolve_path(b"/committed").unwrap();
        let mut output = [0; 7];
        assert_eq!(reopened.read(id, 0, &mut output).unwrap(), 7);
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
            transaction.write(b"/file", offset, b"x", false).unwrap();
        }
        assert_eq!(
            transaction.write(b"/file", 0, b"replacement", true),
            Err(NamespaceError::Capacity)
        );
        assert_eq!(transaction.record_count(), MAX_TRANSACTION_RECORDS - 1);
        assert_eq!(transaction.stat(b"/file").unwrap().length, MAX_TRANSACTION_RECORDS as u32);
    }

    #[test]
    fn aborted_transaction_does_not_change_namespace() {
        let store = MemoryBlockStore::<16>::new();
        let fs = DurableNamespace::format(store).unwrap();
        let mut transaction = fs.begin_transaction();
        transaction.create_file(b"/discarded").unwrap();
        transaction.abort();
        assert_eq!(fs.resolve_path(b"/discarded"), Err(NamespaceError::NotFound));
    }
}
