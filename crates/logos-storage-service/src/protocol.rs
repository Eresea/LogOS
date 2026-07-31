#![allow(dead_code)]

use logos_abi::{MAX_OBJECT_NAME, NamespaceId, PAGE_SIZE, StoreRequest, VersionSelector};

pub const CALLER: u32 = 1;

pub struct ReplaceTransaction {
    caller: u32,
    namespace: NamespaceId,
    name: [u8; MAX_OBJECT_NAME],
    name_length: u8,
    length: usize,
    written: usize,
    bytes: [u8; PAGE_SIZE],
}

impl ReplaceTransaction {
    pub fn begin(request: StoreRequest) -> Option<Self> {
        (request.length != 0 && request.length as usize <= PAGE_SIZE).then_some(Self {
            caller: CALLER,
            namespace: request.namespace,
            name: request.name,
            name_length: request.name_length,
            length: request.length as usize,
            written: 0,
            bytes: [0; PAGE_SIZE],
        })
    }

    pub fn write(&mut self, request: StoreRequest, page: &[u8]) -> bool {
        if request.id == 0
            || self.caller != CALLER
            || request.offset != self.written as u64
            || request.length == 0
            || !request.offset.checked_add(request.length as u64).is_some_and(|end| {
                end <= self.length as u64 && request.length as usize <= page.len()
            })
        {
            return false;
        }
        let length = request.length as usize;
        self.bytes[self.written..self.written + length].copy_from_slice(&page[..length]);
        self.written += length;
        true
    }

    pub fn complete(&self) -> bool {
        self.written == self.length
    }

    pub fn namespace(&self) -> NamespaceId {
        self.namespace
    }

    pub fn name(&self) -> &[u8] {
        &self.name[..self.name_length as usize]
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes[..self.length]
    }
}

#[derive(Clone, Copy)]
pub struct ReadSelection {
    caller: u32,
    namespace: NamespaceId,
    name: [u8; MAX_OBJECT_NAME],
    name_length: u8,
    version: VersionSelector,
    length: usize,
}

impl ReadSelection {
    pub fn new(request: StoreRequest, length: usize) -> Self {
        Self {
            caller: CALLER,
            namespace: request.namespace,
            name: request.name,
            name_length: request.name_length,
            version: request.version,
            length,
        }
    }

    pub fn valid_for(&self, request: StoreRequest) -> bool {
        self.caller == CALLER && request.id != 0
    }

    pub fn namespace(&self) -> NamespaceId {
        self.namespace
    }

    pub fn name(&self) -> &[u8] {
        &self.name[..self.name_length as usize]
    }

    pub fn version(&self) -> VersionSelector {
        self.version
    }

    pub fn length(&self) -> usize {
        self.length
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(operation: logos_abi::StoreOperation, offset: u64, length: u32) -> StoreRequest {
        let mut name = [0; MAX_OBJECT_NAME];
        name[0] = b'x';
        StoreRequest {
            id: 1,
            operation,
            namespace: NamespaceId(1),
            name,
            name_length: 1,
            version: VersionSelector::None,
            offset,
            length,
            page: logos_abi::PageHandle(1),
            deadline: 0,
        }
    }

    #[test]
    fn replace_requires_contiguous_complete_chunks() {
        let mut begin = request(logos_abi::StoreOperation::BeginReplace, 0, 3);
        begin.page = logos_abi::PageHandle(0);
        let mut replace = ReplaceTransaction::begin(begin).unwrap();
        assert!(!replace.write(request(logos_abi::StoreOperation::WriteChunk, 1, 1), b"a"));
        assert!(replace.write(request(logos_abi::StoreOperation::WriteChunk, 0, 2), b"ab"));
        assert!(!replace.write(request(logos_abi::StoreOperation::WriteChunk, 0, 1), b"c"));
        assert!(replace.write(request(logos_abi::StoreOperation::WriteChunk, 2, 1), b"c"));
        assert!(replace.complete());
        assert_eq!(replace.bytes(), b"abc");
    }

    #[test]
    fn replace_rejects_empty_payloads() {
        let request = request(logos_abi::StoreOperation::BeginReplace, 0, 0);
        assert!(ReplaceTransaction::begin(request).is_none());
    }

    #[test]
    fn read_selection_preserves_namespace_version_and_length() {
        let mut open = request(logos_abi::StoreOperation::OpenRead, 0, 0);
        open.version = VersionSelector::Previous;
        open.page = logos_abi::PageHandle(0);
        let selection = ReadSelection::new(open, 12);
        assert_eq!(selection.namespace(), NamespaceId(1));
        assert_eq!(selection.name(), b"x");
        assert_eq!(selection.version(), VersionSelector::Previous);
        assert_eq!(selection.length(), 12);
    }
}
