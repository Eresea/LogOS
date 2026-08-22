use crate::{ABI_VERSION, IPC_PAGE_BYTES, MAX_SERVICE_NAME_BYTES, SERVICE_HEAP_MAX_PAGES};

pub const RUNTIME_ABI_VERSION: u16 = ABI_VERSION;
pub const DIRECTORY_FLAG_MORE: u8 = 1 << 0;
pub const DIRECTORY_RECORDS_PER_PAGE: usize = 32;

macro_rules! define_handle {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        #[repr(transparent)]
        pub struct $name(u64);

        impl $name {
            pub const EMPTY: Self = Self(0);

            pub const fn new(index: u32, generation: u32) -> Option<Self> {
                if generation == 0 {
                    None
                } else {
                    Some(Self(((generation as u64) << 32) | index as u64))
                }
            }

            pub const fn from_raw(raw: u64) -> Option<Self> {
                if raw >> 32 == 0 { None } else { Some(Self(raw)) }
            }

            pub const fn raw(self) -> u64 {
                self.0
            }

            pub const fn index(self) -> u32 {
                self.0 as u32
            }

            pub const fn generation(self) -> u32 {
                (self.0 >> 32) as u32
            }

            pub const fn is_valid(self) -> bool {
                self.generation() != 0
            }
        }
    };
}

define_handle!(ServiceHandle);
define_handle!(EndpointHandle);
define_handle!(CapabilityHandle);
define_handle!(EventHandle);
define_handle!(EventSetHandle);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C, align(16))]
pub struct ServiceBootstrapPage {
    pub abi_version: u16,
    pub flags: u16,
    pub service: ServiceHandle,
    pub control: CapabilityHandle,
    pub directory: CapabilityHandle,
    pub heap: CapabilityHandle,
    pub heap_base: u64,
    pub heap_pages: u32,
    pub heap_quota_pages: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum EventOperation {
    Create = 1,
    Destroy = 2,
    CreateSet = 3,
    Add = 4,
    Remove = 5,
    Wait = 6,
    Signal = 7,
}

impl EventOperation {
    pub const fn from_raw(raw: u8) -> Option<Self> {
        match raw {
            1 => Some(Self::Create),
            2 => Some(Self::Destroy),
            3 => Some(Self::CreateSet),
            4 => Some(Self::Add),
            5 => Some(Self::Remove),
            6 => Some(Self::Wait),
            7 => Some(Self::Signal),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum EventStatus {
    Ok = 0,
    Ready = 1,
    Pending = 2,
    Timeout = 3,
    Stale = 4,
    Unauthorized = 5,
    Capacity = 6,
    Duplicate = 7,
    NotMember = 8,
    InvalidDeadline = 9,
    Malformed = 10,
}

impl EventStatus {
    pub const fn from_raw(raw: usize) -> Option<Self> {
        match raw {
            0 => Some(Self::Ok),
            1 => Some(Self::Ready),
            2 => Some(Self::Pending),
            3 => Some(Self::Timeout),
            4 => Some(Self::Stale),
            5 => Some(Self::Unauthorized),
            6 => Some(Self::Capacity),
            7 => Some(Self::Duplicate),
            8 => Some(Self::NotMember),
            9 => Some(Self::InvalidDeadline),
            10 => Some(Self::Malformed),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C, align(16))]
pub struct EventRequest {
    pub abi_version: u16,
    pub operation: EventOperation,
    pub reserved: u8,
    pub request_id: u32,
    pub event_set: EventSetHandle,
    pub event: EventHandle,
    pub deadline: u64,
}

impl EventRequest {
    pub const fn new(operation: EventOperation, request_id: u32) -> Self {
        Self {
            abi_version: RUNTIME_ABI_VERSION,
            operation,
            reserved: 0,
            request_id,
            event_set: EventSetHandle::EMPTY,
            event: EventHandle::EMPTY,
            deadline: u64::MAX,
        }
    }

    pub const fn is_valid(self) -> bool {
        self.abi_version == RUNTIME_ABI_VERSION
            && EventOperation::from_raw(self.operation as u8).is_some()
            && self.reserved == 0
            && self.request_id != 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C, align(16))]
pub struct EventResponse {
    pub abi_version: u16,
    pub status: EventStatus,
    pub reserved: u8,
    pub request_id: u32,
    pub event: EventHandle,
}

impl EventResponse {
    pub const fn empty(status: EventStatus, request_id: u32) -> Self {
        Self {
            abi_version: RUNTIME_ABI_VERSION,
            status,
            reserved: 0,
            request_id,
            event: EventHandle::EMPTY,
        }
    }

    pub const fn is_valid_for(self, request: EventRequest) -> bool {
        self.abi_version == RUNTIME_ABI_VERSION
            && self.reserved == 0
            && self.request_id == request.request_id
            && EventStatus::from_raw(self.status as usize).is_some()
    }
}

impl ServiceBootstrapPage {
    pub const fn empty() -> Self {
        Self {
            abi_version: RUNTIME_ABI_VERSION,
            flags: 0,
            service: ServiceHandle::EMPTY,
            control: CapabilityHandle::EMPTY,
            directory: CapabilityHandle::EMPTY,
            heap: CapabilityHandle::EMPTY,
            heap_base: 0,
            heap_pages: 0,
            heap_quota_pages: 0,
        }
    }

    pub const fn is_valid(self) -> bool {
        self.abi_version == RUNTIME_ABI_VERSION
            && self.flags == 0
            && self.service.is_valid()
            && self.control.is_valid()
            && self.directory.is_valid()
            && self.heap.is_valid()
            && self.heap_base != 0
            && self.heap_base % IPC_PAGE_BYTES as u64 == 0
            && self.heap_pages != 0
            && (self.heap_pages as usize) <= SERVICE_HEAP_MAX_PAGES
            && self.heap_quota_pages >= self.heap_pages
            && (self.heap_quota_pages as usize) <= SERVICE_HEAP_MAX_PAGES
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum DirectoryOperation {
    Services = 1,
    Capabilities = 2,
    Endpoints = 3,
}

impl DirectoryOperation {
    pub const fn from_raw(raw: u8) -> Option<Self> {
        match raw {
            1 => Some(Self::Services),
            2 => Some(Self::Capabilities),
            3 => Some(Self::Endpoints),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum DirectoryStatus {
    Ok = 0,
    Malformed = 1,
    Unauthorized = 2,
    Stale = 3,
    NotFound = 4,
    Capacity = 5,
}

impl DirectoryStatus {
    pub const fn from_raw(raw: usize) -> Option<Self> {
        match raw {
            0 => Some(Self::Ok),
            1 => Some(Self::Malformed),
            2 => Some(Self::Unauthorized),
            3 => Some(Self::Stale),
            4 => Some(Self::NotFound),
            5 => Some(Self::Capacity),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum DirectoryRecordKind {
    Empty = 0,
    Service = 1,
    Capability = 2,
    Endpoint = 3,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct DirectoryRequest {
    pub abi_version: u16,
    pub operation: DirectoryOperation,
    pub reserved: u8,
    pub request_id: u32,
    pub cursor: u64,
    pub subject: ServiceHandle,
}

impl DirectoryRequest {
    pub const fn new(operation: DirectoryOperation, request_id: u32) -> Self {
        Self {
            abi_version: RUNTIME_ABI_VERSION,
            operation,
            reserved: 0,
            request_id,
            cursor: 0,
            subject: ServiceHandle::EMPTY,
        }
    }

    pub const fn is_valid(self) -> bool {
        self.abi_version == RUNTIME_ABI_VERSION
            && DirectoryOperation::from_raw(self.operation as u8).is_some()
            && self.reserved == 0
            && self.request_id != 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct DirectoryRecord {
    pub kind: DirectoryRecordKind,
    pub rights: u8,
    pub flags: u16,
    pub handle: u64,
    pub peer: ServiceHandle,
    pub message_bytes: u16,
    pub queue_capacity: u16,
    pub name_len: u8,
    pub reserved: [u8; 3],
    pub name: [u8; MAX_SERVICE_NAME_BYTES],
}

impl DirectoryRecord {
    pub const EMPTY: Self = Self {
        kind: DirectoryRecordKind::Empty,
        rights: 0,
        flags: 0,
        handle: 0,
        peer: ServiceHandle::EMPTY,
        message_bytes: 0,
        queue_capacity: 0,
        name_len: 0,
        reserved: [0; 3],
        name: [0; MAX_SERVICE_NAME_BYTES],
    };

    pub fn service(handle: ServiceHandle, name: &[u8]) -> Option<Self> {
        if !handle.is_valid() || name.is_empty() || name.len() > MAX_SERVICE_NAME_BYTES {
            return None;
        }
        let mut record = Self::EMPTY;
        record.kind = DirectoryRecordKind::Service;
        record.handle = handle.raw();
        record.name_len = name.len() as u8;
        record.name[..name.len()].copy_from_slice(name);
        Some(record)
    }

    pub const fn is_empty(self) -> bool {
        matches!(self.kind, DirectoryRecordKind::Empty)
    }

    pub fn is_valid(self) -> bool {
        if self.is_empty() {
            return self == Self::EMPTY;
        }
        self.reserved == [0; 3]
            && self.handle != 0
            && self.name_len as usize <= self.name.len()
            && self.name[self.name_len as usize..].iter().all(|byte| *byte == 0)
    }

    pub fn name(&self) -> &[u8] {
        &self.name[..self.name_len as usize]
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C, align(16))]
pub struct DirectoryResponse {
    pub abi_version: u16,
    pub operation: DirectoryOperation,
    pub status: DirectoryStatus,
    pub request_id: u32,
    pub cursor: u64,
    pub count: u8,
    pub flags: u8,
    pub reserved: [u8; 2],
    pub records: [DirectoryRecord; DIRECTORY_RECORDS_PER_PAGE],
}

impl DirectoryResponse {
    pub const fn empty(
        operation: DirectoryOperation,
        status: DirectoryStatus,
        request_id: u32,
    ) -> Self {
        Self {
            abi_version: RUNTIME_ABI_VERSION,
            operation,
            status,
            request_id,
            cursor: 0,
            count: 0,
            flags: 0,
            reserved: [0; 2],
            records: [DirectoryRecord::EMPTY; DIRECTORY_RECORDS_PER_PAGE],
        }
    }

    pub fn is_valid_for(self, request: DirectoryRequest) -> bool {
        self.abi_version == RUNTIME_ABI_VERSION
            && self.operation == request.operation
            && request.is_valid()
            && self.request_id == request.request_id
            && self.flags & !DIRECTORY_FLAG_MORE == 0
            && (self.flags & DIRECTORY_FLAG_MORE == 0 || self.cursor != 0)
            && self.reserved == [0; 2]
            && usize::from(self.count) <= DIRECTORY_RECORDS_PER_PAGE
            && self.records[..self.count as usize].iter().all(|record| record.is_valid())
            && self.records[self.count as usize..].iter().all(|record| record.is_empty())
    }
}

const _: () = assert!(core::mem::size_of::<ServiceBootstrapPage>() <= IPC_PAGE_BYTES);
const _: () = assert!(core::mem::size_of::<DirectoryRequest>() <= IPC_PAGE_BYTES);
const _: () = assert!(core::mem::size_of::<DirectoryResponse>() <= IPC_PAGE_BYTES);
const _: () = assert!(core::mem::size_of::<EventRequest>() <= IPC_PAGE_BYTES);
const _: () = assert!(core::mem::size_of::<EventResponse>() <= IPC_PAGE_BYTES);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SERVICE_HEAP_INITIAL_PAGES;

    #[test]
    fn handles_encode_generation_and_reject_empty_generation() {
        let handle = ServiceHandle::new(7, 3).unwrap();
        assert_eq!(handle.index(), 7);
        assert_eq!(handle.generation(), 3);
        assert_eq!(ServiceHandle::from_raw(handle.raw()), Some(handle));
        assert!(ServiceHandle::new(7, 0).is_none());
        assert!(ServiceHandle::from_raw(7).is_none());
        assert!(!ServiceHandle::EMPTY.is_valid());
    }

    #[test]
    fn bootstrap_requires_all_runtime_grants() {
        let mut page = ServiceBootstrapPage::empty();
        assert!(!page.is_valid());
        page.service = ServiceHandle::new(1, 1).unwrap();
        page.control = CapabilityHandle::new(2, 1).unwrap();
        page.directory = CapabilityHandle::new(3, 1).unwrap();
        page.heap = CapabilityHandle::new(4, 1).unwrap();
        page.heap_base = 0x0000_0200_0000_0000;
        page.heap_pages = SERVICE_HEAP_INITIAL_PAGES as u32;
        page.heap_quota_pages = SERVICE_HEAP_MAX_PAGES as u32;
        assert!(page.is_valid());
        page.flags = 1;
        assert!(!page.is_valid());
    }

    #[test]
    fn directory_response_is_bounded_and_cursored() {
        let request = DirectoryRequest::new(DirectoryOperation::Services, 9);
        let service = DirectoryRecord::service(ServiceHandle::new(4, 2).unwrap(), b"flow").unwrap();
        let mut response = DirectoryResponse::empty(
            DirectoryOperation::Services,
            DirectoryStatus::Ok,
            request.request_id,
        );
        response.records[0] = service;
        response.count = 1;
        assert!(response.is_valid_for(request));
        response.flags = DIRECTORY_FLAG_MORE;
        response.cursor = 11;
        assert!(response.is_valid_for(request));
        response.cursor = 0;
        assert!(!response.is_valid_for(request));
        response.flags = 0;
        response.count = (DIRECTORY_RECORDS_PER_PAGE + 1) as u8;
        assert!(!response.is_valid_for(request));
    }

    #[test]
    fn event_requests_and_responses_are_generation_safe_envelopes() {
        let request = EventRequest::new(EventOperation::Wait, 11);
        assert!(request.is_valid());
        let mut response = EventResponse::empty(EventStatus::Pending, request.request_id);
        assert!(response.is_valid_for(request));
        response.request_id = 12;
        assert!(!response.is_valid_for(request));
        assert_eq!(
            EventStatus::from_raw(EventStatus::Malformed as usize),
            Some(EventStatus::Malformed)
        );
        assert!(EventOperation::from_raw(99).is_none());
    }
}
