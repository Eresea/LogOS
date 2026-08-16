use crate::{MAX_MANAGER_SERVICES, MAX_SERVICE_NAME_BYTES};

pub const MANAGER_ABI_VERSION: u16 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ManagerOperation {
    List = 1,
    Status = 2,
    Start = 3,
    Stop = 4,
    Restart = 5,
}

impl ManagerOperation {
    pub const fn from_raw(raw: u8) -> Option<Self> {
        match raw {
            1 => Some(Self::List),
            2 => Some(Self::Status),
            3 => Some(Self::Start),
            4 => Some(Self::Stop),
            5 => Some(Self::Restart),
            _ => None,
        }
    }

    pub const fn requires_lifecycle(self) -> bool {
        !matches!(self, Self::List | Self::Status)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ManagerStatus {
    Ok = 0,
    Accepted = 1,
    Malformed = 2,
    Unauthorized = 3,
    NotFound = 4,
    Stale = 5,
    InvalidState = 6,
    Dependency = 7,
    Busy = 8,
    Capacity = 9,
    Unsupported = 10,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ManagerState {
    Vacant = 0,
    Stopped = 1,
    Starting = 2,
    Running = 3,
    Stopping = 4,
    Failed = 5,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct ManagerRights(pub u8);

impl ManagerRights {
    pub const NONE: Self = Self(0);
    pub const INSPECT: Self = Self(1 << 0);
    pub const LIFECYCLE: Self = Self(1 << 1);
    pub const ALL: Self = Self(Self::INSPECT.0 | Self::LIFECYCLE.0);

    pub const fn contains(self, required: Self) -> bool {
        self.0 & required.0 == required.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct ManagerCapability {
    pub generation: u32,
    pub rights: ManagerRights,
    pub reserved: [u8; 3],
    pub service_epoch: u64,
}

impl ManagerCapability {
    pub const EMPTY: Self =
        Self { generation: 0, rights: ManagerRights::NONE, reserved: [0; 3], service_epoch: 0 };

    pub const fn new(generation: u32, rights: ManagerRights, service_epoch: u64) -> Option<Self> {
        if generation == 0
            || rights.0 == 0
            || rights.0 & !ManagerRights::ALL.0 != 0
            || service_epoch == 0
        {
            return None;
        }
        Some(Self { generation, rights, reserved: [0; 3], service_epoch })
    }

    pub const fn is_empty(self) -> bool {
        self.generation == 0
    }
}

#[repr(C, align(16))]
pub struct ManagerCapabilityPage {
    pub capability: ManagerCapability,
}

impl ManagerCapabilityPage {
    pub const fn empty() -> Self {
        Self { capability: ManagerCapability::EMPTY }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct ManagerRequest {
    pub abi_version: u16,
    pub operation: ManagerOperation,
    pub reserved: u8,
    pub request_id: u32,
    pub slot: u8,
    pub cursor: u8,
    pub reserved_tail: [u8; 2],
    pub generation: u32,
}

impl ManagerRequest {
    pub const fn new(operation: ManagerOperation, request_id: u32) -> Self {
        Self {
            abi_version: MANAGER_ABI_VERSION,
            operation,
            reserved: 0,
            request_id,
            slot: u8::MAX,
            cursor: 0,
            reserved_tail: [0; 2],
            generation: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct ServiceManagerRecord {
    pub slot: u8,
    pub state: ManagerState,
    pub restarts: u8,
    pub name_len: u8,
    pub generation: u32,
    pub dependencies: u8,
    pub reserved: [u8; 3],
    pub name: [u8; MAX_SERVICE_NAME_BYTES],
}

impl ServiceManagerRecord {
    pub const EMPTY: Self = Self {
        slot: u8::MAX,
        state: ManagerState::Vacant,
        restarts: 0,
        name_len: 0,
        generation: 0,
        dependencies: 0,
        reserved: [0; 3],
        name: [0; MAX_SERVICE_NAME_BYTES],
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct ManagerResponse {
    pub abi_version: u16,
    pub operation: ManagerOperation,
    pub status: ManagerStatus,
    pub request_id: u32,
    pub cursor: u8,
    pub reserved: [u8; 3],
    pub record: ServiceManagerRecord,
}

impl ManagerResponse {
    pub const fn new(operation: ManagerOperation, status: ManagerStatus, request_id: u32) -> Self {
        Self {
            abi_version: MANAGER_ABI_VERSION,
            operation,
            status,
            request_id,
            cursor: 0,
            reserved: [0; 3],
            record: ServiceManagerRecord::EMPTY,
        }
    }
}

const _: () = assert!(MAX_MANAGER_SERVICES <= u8::MAX as usize);
const _: () = assert!(core::mem::size_of::<ManagerCapability>() == 16);
const _: () = assert!(core::mem::align_of::<ManagerCapability>() == 8);
const _: () = assert!(core::mem::size_of::<ManagerCapabilityPage>() == 16);
const _: () = assert!(core::mem::align_of::<ManagerCapabilityPage>() == 16);
const _: () = assert!(core::mem::size_of::<ManagerRequest>() == 16);
const _: () = assert!(core::mem::align_of::<ManagerRequest>() == 4);
const _: () = assert!(core::mem::size_of::<ServiceManagerRecord>() == 28);
const _: () = assert!(core::mem::align_of::<ServiceManagerRecord>() == 4);
const _: () = assert!(core::mem::size_of::<ManagerResponse>() == 40);
const _: () = assert!(core::mem::align_of::<ManagerResponse>() == 4);
const _: () = assert!(core::mem::size_of::<ManagerCapabilityPage>() <= crate::IPC_PAGE_BYTES);
const _: () = assert!(core::mem::size_of::<ManagerRequest>() <= crate::IPC_PAGE_BYTES);
const _: () = assert!(core::mem::size_of::<ManagerResponse>() <= crate::IPC_PAGE_BYTES);
