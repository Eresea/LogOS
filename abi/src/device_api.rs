use crate::MAX_DEVICE_NAME_BYTES;

pub const DEVICE_ABI_VERSION: u16 = 1;
pub const MAX_DEVICES: usize = 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum DeviceOperation {
    List = 1,
}

impl DeviceOperation {
    pub const fn from_raw(raw: u8) -> Option<Self> {
        match raw {
            1 => Some(Self::List),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum DeviceStatus {
    Ok = 0,
    Invalid = 1,
    Io = 2,
    Stale = 3,
    Unsupported = 4,
}

impl DeviceStatus {
    pub const fn from_raw(raw: u8) -> Option<Self> {
        match raw {
            0 => Some(Self::Ok),
            1 => Some(Self::Invalid),
            2 => Some(Self::Io),
            3 => Some(Self::Stale),
            4 => Some(Self::Unsupported),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum DeviceKind {
    Unknown = 0,
    Disk = 1,
}

impl DeviceKind {
    pub const fn from_raw(raw: u8) -> Option<Self> {
        match raw {
            0 => Some(Self::Unknown),
            1 => Some(Self::Disk),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum DeviceState {
    Absent = 0,
    Ready = 1,
    Faulted = 2,
}

impl DeviceState {
    pub const fn from_raw(raw: u8) -> Option<Self> {
        match raw {
            0 => Some(Self::Absent),
            1 => Some(Self::Ready),
            2 => Some(Self::Faulted),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct DeviceRecord {
    pub id: u8,
    pub kind: DeviceKind,
    pub state: DeviceState,
    pub flags: u8,
    pub block_size: u32,
    pub block_count: u64,
    pub name: [u8; MAX_DEVICE_NAME_BYTES],
}

impl DeviceRecord {
    pub const EMPTY: Self = Self {
        id: 0,
        kind: DeviceKind::Unknown,
        state: DeviceState::Absent,
        flags: 0,
        block_size: 0,
        block_count: 0,
        name: [0; MAX_DEVICE_NAME_BYTES],
    };

    pub fn disk(id: u8, block_count: u64, name: &[u8]) -> Option<Self> {
        if block_count == 0 || name.is_empty() || name.len() > MAX_DEVICE_NAME_BYTES {
            return None;
        }
        let mut record = Self {
            id,
            kind: DeviceKind::Disk,
            state: DeviceState::Ready,
            flags: 0,
            block_size: crate::STORAGE_BLOCK_BYTES as u32,
            block_count,
            name: [0; MAX_DEVICE_NAME_BYTES],
        };
        record.name[..name.len()].copy_from_slice(name);
        Some(record)
    }

    pub fn is_valid(self) -> bool {
        DeviceKind::from_raw(self.kind as u8).is_some()
            && DeviceState::from_raw(self.state as u8).is_some()
            && self.flags == 0
            && self
                .name
                .iter()
                .position(|byte| *byte == 0)
                .is_none_or(|end| end != 0 && self.name[end + 1..].iter().all(|byte| *byte == 0))
            && (self.kind != DeviceKind::Disk || (self.block_size != 0 && self.block_count != 0))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct DeviceRequest {
    pub abi_version: u16,
    pub operation: DeviceOperation,
    pub flags: u8,
    pub request_id: u32,
    pub reserved: u16,
    pub reserved_tail: [u8; 8],
}

impl DeviceRequest {
    pub const fn new(operation: DeviceOperation, request_id: u32) -> Self {
        Self {
            abi_version: DEVICE_ABI_VERSION,
            operation,
            flags: 0,
            request_id,
            reserved: 0,
            reserved_tail: [0; 8],
        }
    }

    pub fn is_valid(self) -> bool {
        self.abi_version == DEVICE_ABI_VERSION
            && DeviceOperation::from_raw(self.operation as u8).is_some()
            && self.flags == 0
            && self.request_id != 0
            && self.reserved == 0
            && self.reserved_tail.iter().all(|byte| *byte == 0)
    }

    pub fn wire_enums_valid(bytes: &[u8]) -> bool {
        bytes
            .get(core::mem::offset_of!(Self, operation))
            .is_some_and(|raw| *raw == DeviceOperation::List as u8)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct DeviceResponse {
    pub abi_version: u16,
    pub operation: DeviceOperation,
    pub status: DeviceStatus,
    pub request_id: u32,
    pub generation: u16,
    pub reserved: u16,
    pub service_epoch: u64,
    pub count: u8,
    pub reserved_tail: [u8; 7],
    pub records: [DeviceRecord; MAX_DEVICES],
}

impl DeviceResponse {
    pub const fn new(
        request: DeviceRequest,
        status: DeviceStatus,
        generation: u16,
        service_epoch: u64,
    ) -> Self {
        Self {
            abi_version: DEVICE_ABI_VERSION,
            operation: request.operation,
            status,
            request_id: request.request_id,
            generation,
            reserved: 0,
            service_epoch,
            count: 0,
            reserved_tail: [0; 7],
            records: [DeviceRecord::EMPTY; MAX_DEVICES],
        }
    }

    pub fn with_record(mut self, record: DeviceRecord) -> Self {
        if record.is_valid() {
            self.records[0] = record;
            self.count = 1;
        }
        self
    }

    pub fn is_valid_for(self, request: DeviceRequest) -> bool {
        self.abi_version == DEVICE_ABI_VERSION
            && self.operation as u8 == request.operation as u8
            && DeviceStatus::from_raw(self.status as u8).is_some()
            && self.request_id == request.request_id
            && self.reserved == 0
            && self.generation != 0
            && self.service_epoch != 0
            && usize::from(self.count) <= MAX_DEVICES
            && self.reserved_tail.iter().all(|byte| *byte == 0)
            && self.records[..usize::from(self.count)].iter().all(|record| record.is_valid())
            && self.records[usize::from(self.count)..]
                .iter()
                .all(|record| *record == DeviceRecord::EMPTY)
    }
}

const _: () = assert!(core::mem::size_of::<DeviceRequest>() <= crate::IPC_PAGE_BYTES);
const _: () = assert!(core::mem::size_of::<DeviceResponse>() <= crate::IPC_PAGE_BYTES);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_wire_operation_is_checked_before_decoding() {
        let mut bytes = [0u8; core::mem::size_of::<DeviceRequest>()];
        bytes[core::mem::offset_of!(DeviceRequest, operation)] = DeviceOperation::List as u8;
        assert!(DeviceRequest::wire_enums_valid(&bytes));
        bytes[core::mem::offset_of!(DeviceRequest, operation)] = u8::MAX;
        assert!(!DeviceRequest::wire_enums_valid(&bytes));
    }
}
