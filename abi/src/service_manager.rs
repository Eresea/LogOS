use crate::{MAX_PACKAGE_NAME_BYTES, ServiceHandle};

pub const MANAGER_ABI_VERSION: u16 = 5;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ManagerTargetKind {
    Service = 0,
    Program = 1,
}

impl ManagerTargetKind {
    pub const fn from_raw(raw: u8) -> Option<Self> {
        match raw {
            0 => Some(Self::Service),
            1 => Some(Self::Program),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ManagerOperation {
    List = 1,
    Status = 2,
    Start = 3,
    Stop = 4,
    Restart = 5,
    ProgramStart = 6,
    ProgramStatus = 7,
    ProgramStop = 8,
}

impl ManagerOperation {
    pub const fn from_raw(raw: u8) -> Option<Self> {
        match raw {
            1 => Some(Self::List),
            2 => Some(Self::Status),
            3 => Some(Self::Start),
            4 => Some(Self::Stop),
            5 => Some(Self::Restart),
            6 => Some(Self::ProgramStart),
            7 => Some(Self::ProgramStatus),
            8 => Some(Self::ProgramStop),
            _ => None,
        }
    }

    pub const fn requires_lifecycle(self) -> bool {
        !matches!(self, Self::List | Self::Status | Self::ProgramStatus)
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
    Disabled = 1,
    Stopped = 2,
    Starting = 3,
    Running = 4,
    Stopping = 5,
    Failed = 6,
    Exited = 7,
    Faulted = 8,
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
pub struct ManagerRequest {
    pub abi_version: u16,
    pub operation: ManagerOperation,
    pub target_kind: ManagerTargetKind,
    pub request_id: u32,
    pub service: ServiceHandle,
    pub cursor: u64,
    pub program_slot: u8,
    pub name_len: u8,
    pub reserved_tail: [u8; 2],
    pub program_generation: u32,
    pub name: [u8; MAX_PACKAGE_NAME_BYTES],
}

impl ManagerRequest {
    pub const fn new(operation: ManagerOperation, request_id: u32) -> Self {
        Self {
            abi_version: MANAGER_ABI_VERSION,
            operation,
            target_kind: ManagerTargetKind::Service,
            request_id,
            service: ServiceHandle::EMPTY,
            cursor: 0,
            program_slot: u8::MAX,
            name_len: 0,
            reserved_tail: [0; 2],
            program_generation: 0,
            name: [0; MAX_PACKAGE_NAME_BYTES],
        }
    }

    pub fn with_program_name(mut self, name: &[u8]) -> Option<Self> {
        crate::PackageTarget::program(name)?;
        self.target_kind = ManagerTargetKind::Program;
        self.name_len = name.len() as u8;
        self.name[..name.len()].copy_from_slice(name);
        Some(self)
    }

    pub const fn program_target(self) -> bool {
        matches!(self.target_kind, ManagerTargetKind::Program)
    }

    pub fn name(&self) -> &[u8] {
        &self.name[..self.name_len as usize]
    }

    pub fn wire_enums_valid(bytes: &[u8]) -> bool {
        bytes
            .get(core::mem::offset_of!(Self, operation))
            .and_then(|raw| ManagerOperation::from_raw(*raw))
            .is_some()
            && bytes
                .get(core::mem::offset_of!(Self, target_kind))
                .and_then(|raw| ManagerTargetKind::from_raw(*raw))
                .is_some()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct ServiceManagerRecord {
    pub service: ServiceHandle,
    pub state: ManagerState,
    pub restarts: u8,
    pub name_len: u8,
    pub dependencies: u16,
    pub reserved: [u8; 2],
    pub program_slot: u8,
    pub reserved_program: [u8; 3],
    pub program_generation: u32,
    pub name: [u8; MAX_PACKAGE_NAME_BYTES],
}

impl ServiceManagerRecord {
    pub const EMPTY: Self = Self {
        service: ServiceHandle::EMPTY,
        state: ManagerState::Vacant,
        restarts: 0,
        name_len: 0,
        dependencies: 0,
        reserved: [0; 2],
        program_slot: u8::MAX,
        reserved_program: [0; 3],
        program_generation: 0,
        name: [0; MAX_PACKAGE_NAME_BYTES],
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct ManagerResponse {
    pub abi_version: u16,
    pub operation: ManagerOperation,
    pub status: ManagerStatus,
    pub request_id: u32,
    pub cursor: u64,
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
            record: ServiceManagerRecord::EMPTY,
        }
    }

    pub fn is_valid_for(self, request: ManagerRequest) -> bool {
        let cursor_valid = match request.operation {
            ManagerOperation::List => true,
            ManagerOperation::Status
            | ManagerOperation::Start
            | ManagerOperation::Stop
            | ManagerOperation::Restart
            | ManagerOperation::ProgramStatus
            | ManagerOperation::ProgramStart
            | ManagerOperation::ProgramStop => self.cursor == 0,
        };
        self.abi_version == MANAGER_ABI_VERSION
            && self.operation == request.operation
            && request.request_id != 0
            && self.request_id == request.request_id
            && ManagerTargetKind::from_raw(request.target_kind as u8).is_some()
            && request.reserved_tail == [0; 2]
            && (request.target_kind == ManagerTargetKind::Service
                || crate::PackageTarget::program(request.name()).is_some())
            && request.name_len as usize <= request.name.len()
            && request.name[request.name_len as usize..].iter().all(|byte| *byte == 0)
            && self.record.reserved == [0; 2]
            && self.record.reserved_program == [0; 3]
            && usize::from(self.record.name_len) <= self.record.name.len()
            && cursor_valid
    }
}

const _: () = assert!(core::mem::size_of::<ManagerRequest>() <= crate::IPC_PAGE_BYTES);
const _: () = assert!(core::mem::align_of::<ManagerRequest>() == 8);
const _: () = assert!(core::mem::offset_of!(ManagerRequest, abi_version) == 0);
const _: () = assert!(core::mem::offset_of!(ManagerRequest, operation) == 2);
const _: () = assert!(core::mem::offset_of!(ManagerRequest, target_kind) == 3);
const _: () = assert!(core::mem::offset_of!(ManagerRequest, request_id) == 4);
const _: () = assert!(core::mem::offset_of!(ManagerRequest, service) == 8);
const _: () = assert!(core::mem::offset_of!(ManagerRequest, cursor) == 16);
const _: () = assert!(core::mem::offset_of!(ManagerRequest, program_slot) == 24);
const _: () = assert!(core::mem::offset_of!(ManagerRequest, name_len) == 25);
const _: () = assert!(core::mem::offset_of!(ManagerRequest, reserved_tail) == 26);
const _: () = assert!(core::mem::offset_of!(ManagerRequest, program_generation) == 28);
const _: () = assert!(core::mem::offset_of!(ManagerRequest, name) == 32);
const _: () = assert!(core::mem::size_of::<ServiceManagerRecord>() == 56);
const _: () = assert!(core::mem::align_of::<ServiceManagerRecord>() == 8);
const _: () = assert!(core::mem::offset_of!(ServiceManagerRecord, service) == 0);
const _: () = assert!(core::mem::offset_of!(ServiceManagerRecord, state) == 8);
const _: () = assert!(core::mem::offset_of!(ServiceManagerRecord, restarts) == 9);
const _: () = assert!(core::mem::offset_of!(ServiceManagerRecord, name_len) == 10);
const _: () = assert!(core::mem::offset_of!(ServiceManagerRecord, dependencies) == 12);
const _: () = assert!(core::mem::offset_of!(ServiceManagerRecord, reserved) == 14);
const _: () = assert!(core::mem::offset_of!(ServiceManagerRecord, program_slot) == 16);
const _: () = assert!(core::mem::offset_of!(ServiceManagerRecord, program_generation) == 20);
const _: () = assert!(core::mem::offset_of!(ServiceManagerRecord, name) == 24);
const _: () = assert!(core::mem::size_of::<ManagerResponse>() == 72);
const _: () = assert!(core::mem::align_of::<ManagerResponse>() == 8);
const _: () = assert!(core::mem::offset_of!(ManagerResponse, abi_version) == 0);
const _: () = assert!(core::mem::offset_of!(ManagerResponse, operation) == 2);
const _: () = assert!(core::mem::offset_of!(ManagerResponse, status) == 3);
const _: () = assert!(core::mem::offset_of!(ManagerResponse, request_id) == 4);
const _: () = assert!(core::mem::offset_of!(ManagerResponse, cursor) == 8);
const _: () = assert!(core::mem::offset_of!(ManagerResponse, record) == 16);
const _: () = assert!(core::mem::size_of::<ManagerRequest>() <= crate::IPC_PAGE_BYTES);
const _: () = assert!(core::mem::size_of::<ManagerResponse>() <= crate::IPC_PAGE_BYTES);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_enum_bytes_are_validated_before_typed_decoding() {
        let mut bytes = [0u8; core::mem::size_of::<ManagerRequest>()];
        bytes[core::mem::offset_of!(ManagerRequest, operation)] = ManagerOperation::List as u8;
        bytes[core::mem::offset_of!(ManagerRequest, target_kind)] =
            ManagerTargetKind::Service as u8;
        assert!(ManagerRequest::wire_enums_valid(&bytes));
        bytes[core::mem::offset_of!(ManagerRequest, target_kind)] = u8::MAX;
        assert!(!ManagerRequest::wire_enums_valid(&bytes));
    }
}
