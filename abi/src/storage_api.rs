use super::{IPC_FLAG_MORE, IpcBytes, MAX_IPC_BYTES, MessageKind};

pub const STORAGE_API_VERSION: u8 = 2;
pub const STORAGE_API_FLAG_REPLACE: u8 = 1;
const REQUEST_HEADER_BYTES: usize = 26;
const RESPONSE_HEADER_BYTES: usize = 18;
pub const STORAGE_API_RESPONSE_DATA_BYTES: usize = MAX_IPC_BYTES - RESPONSE_HEADER_BYTES;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum StorageApiOperation {
    Begin = 1,
    Commit = 2,
    Abort = 3,
    List = 4,
    CreateFile = 5,
    Read = 6,
    Write = 7,
    Remove = 8,
    Rename = 9,
    StageWriteBegin = 10,
    StageWriteChunk = 11,
    StageWriteCommit = 12,
    StageWriteAbort = 13,
}

impl StorageApiOperation {
    pub const fn from_raw(raw: u8) -> Option<Self> {
        match raw {
            1 => Some(Self::Begin),
            2 => Some(Self::Commit),
            3 => Some(Self::Abort),
            4 => Some(Self::List),
            5 => Some(Self::CreateFile),
            6 => Some(Self::Read),
            7 => Some(Self::Write),
            8 => Some(Self::Remove),
            9 => Some(Self::Rename),
            10 => Some(Self::StageWriteBegin),
            11 => Some(Self::StageWriteChunk),
            12 => Some(Self::StageWriteCommit),
            13 => Some(Self::StageWriteAbort),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum StorageApiStatus {
    Ok = 0,
    Invalid = 1,
    Busy = 2,
    NoTransaction = 3,
    Stale = 4,
    Io = 5,
    NotFound = 6,
    AlreadyExists = 7,
    NotDirectory = 8,
    IsDirectory = 9,
    Root = 10,
    NotEmpty = 11,
    Capacity = 12,
    TooLarge = 13,
    Unsupported = 14,
}

impl StorageApiStatus {
    pub const fn from_raw(raw: u8) -> Option<Self> {
        match raw {
            0 => Some(Self::Ok),
            1 => Some(Self::Invalid),
            2 => Some(Self::Busy),
            3 => Some(Self::NoTransaction),
            4 => Some(Self::Stale),
            5 => Some(Self::Io),
            6 => Some(Self::NotFound),
            7 => Some(Self::AlreadyExists),
            8 => Some(Self::NotDirectory),
            9 => Some(Self::IsDirectory),
            10 => Some(Self::Root),
            11 => Some(Self::NotEmpty),
            12 => Some(Self::Capacity),
            13 => Some(Self::TooLarge),
            14 => Some(Self::Unsupported),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StorageApiError {
    WrongKind,
    InvalidVersion,
    Malformed,
    UnknownOperation,
    UnknownStatus,
    Oversized,
}

pub struct StorageApiRequest<'a> {
    pub operation: StorageApiOperation,
    pub flags: u8,
    pub request_id: u32,
    pub transaction_id: u64,
    pub offset: u32,
    pub path: &'a [u8],
    pub secondary_path: &'a [u8],
    pub data: &'a [u8],
}

impl<'a> StorageApiRequest<'a> {
    pub fn decode(message: &'a IpcBytes) -> Result<Self, StorageApiError> {
        if message.kind != MessageKind::StorageRequest {
            return Err(StorageApiError::WrongKind);
        }
        if message.flags != 0 {
            return Err(StorageApiError::Malformed);
        }
        let bytes = message.as_bytes().ok_or(StorageApiError::Malformed)?;
        if bytes.len() < REQUEST_HEADER_BYTES {
            return Err(StorageApiError::Malformed);
        }
        if bytes[0] != STORAGE_API_VERSION {
            return Err(StorageApiError::InvalidVersion);
        }
        let operation =
            StorageApiOperation::from_raw(bytes[1]).ok_or(StorageApiError::UnknownOperation)?;
        if bytes[3] != 0 {
            return Err(StorageApiError::Malformed);
        }
        let request_id = get_u32(bytes, 4);
        if request_id == 0 {
            return Err(StorageApiError::Malformed);
        }
        let transaction_id = get_u64(bytes, 8);
        let path_len = get_u16(bytes, 16) as usize;
        let secondary_path_len = get_u16(bytes, 18) as usize;
        let offset = get_u32(bytes, 20);
        let data_len = get_u16(bytes, 24) as usize;
        let payload_len = path_len
            .checked_add(secondary_path_len)
            .and_then(|length| length.checked_add(data_len))
            .ok_or(StorageApiError::Oversized)?;
        if REQUEST_HEADER_BYTES + payload_len != bytes.len() {
            return Err(StorageApiError::Malformed);
        }
        let path_start = REQUEST_HEADER_BYTES;
        let secondary_start = path_start + path_len;
        let data_start = secondary_start + secondary_path_len;
        let path = &bytes[path_start..secondary_start];
        let secondary_path = &bytes[secondary_start..data_start];
        let data = &bytes[data_start..];
        if !request_shape_is_valid(operation, path, secondary_path, data) {
            return Err(StorageApiError::Malformed);
        }
        Ok(Self {
            operation,
            flags: bytes[2],
            request_id,
            transaction_id,
            offset,
            path,
            secondary_path,
            data,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn encode(
        operation: StorageApiOperation,
        flags: u8,
        request_id: u32,
        transaction_id: u64,
        offset: u32,
        path: &[u8],
        secondary_path: &[u8],
        data: &[u8],
    ) -> Option<IpcBytes> {
        if request_id == 0
            || path.len() > u16::MAX as usize
            || secondary_path.len() > u16::MAX as usize
            || data.len() > u16::MAX as usize
            || !request_shape_is_valid(operation, path, secondary_path, data)
        {
            return None;
        }
        let payload_len = path.len().checked_add(secondary_path.len())?.checked_add(data.len())?;
        let total_len = REQUEST_HEADER_BYTES.checked_add(payload_len)?;
        if total_len > MAX_IPC_BYTES {
            return None;
        }
        let mut message = IpcBytes::empty(MessageKind::StorageRequest);
        let bytes = &mut message.bytes[..total_len];
        bytes[0] = STORAGE_API_VERSION;
        bytes[1] = operation as u8;
        bytes[2] = flags;
        put_u32(bytes, 4, request_id);
        put_u64(bytes, 8, transaction_id);
        put_u16(bytes, 16, path.len() as u16);
        put_u16(bytes, 18, secondary_path.len() as u16);
        put_u32(bytes, 20, offset);
        put_u16(bytes, 24, data.len() as u16);
        let mut cursor = REQUEST_HEADER_BYTES;
        bytes[cursor..cursor + path.len()].copy_from_slice(path);
        cursor += path.len();
        bytes[cursor..cursor + secondary_path.len()].copy_from_slice(secondary_path);
        cursor += secondary_path.len();
        bytes[cursor..cursor + data.len()].copy_from_slice(data);
        message.len = total_len as u16;
        Some(message)
    }
}

fn request_shape_is_valid(
    operation: StorageApiOperation,
    path: &[u8],
    secondary_path: &[u8],
    data: &[u8],
) -> bool {
    match operation {
        StorageApiOperation::Begin | StorageApiOperation::Commit | StorageApiOperation::Abort => {
            path.is_empty() && secondary_path.is_empty() && data.is_empty()
        }
        StorageApiOperation::List
        | StorageApiOperation::CreateFile
        | StorageApiOperation::Read
        | StorageApiOperation::Remove => secondary_path.is_empty() && data.is_empty(),
        StorageApiOperation::Write => secondary_path.is_empty(),
        StorageApiOperation::Rename => {
            !path.is_empty() && !secondary_path.is_empty() && data.is_empty()
        }
        StorageApiOperation::StageWriteBegin => {
            !path.is_empty() && secondary_path.is_empty() && data.is_empty()
        }
        StorageApiOperation::StageWriteChunk => {
            path.is_empty() && secondary_path.is_empty() && !data.is_empty()
        }
        StorageApiOperation::StageWriteCommit | StorageApiOperation::StageWriteAbort => {
            path.is_empty() && secondary_path.is_empty() && data.is_empty()
        }
    }
}

pub struct StorageApiResponse<'a> {
    pub status: StorageApiStatus,
    pub request_id: u32,
    pub transaction_id: u64,
    pub data: &'a [u8],
    pub more: bool,
}

impl<'a> StorageApiResponse<'a> {
    pub fn decode(message: &'a IpcBytes) -> Result<Self, StorageApiError> {
        if message.kind != MessageKind::StorageResponse {
            return Err(StorageApiError::WrongKind);
        }
        if message.flags & !IPC_FLAG_MORE != 0 {
            return Err(StorageApiError::Malformed);
        }
        let bytes = message.as_bytes().ok_or(StorageApiError::Malformed)?;
        if bytes.len() < RESPONSE_HEADER_BYTES {
            return Err(StorageApiError::Malformed);
        }
        if bytes[0] != STORAGE_API_VERSION {
            return Err(StorageApiError::InvalidVersion);
        }
        if bytes[3] != 0 {
            return Err(StorageApiError::Malformed);
        }
        let status = StorageApiStatus::from_raw(bytes[1]).ok_or(StorageApiError::UnknownStatus)?;
        let request_id = get_u32(bytes, 4);
        if request_id == 0 {
            return Err(StorageApiError::Malformed);
        }
        let data_len = get_u16(bytes, 16) as usize;
        if RESPONSE_HEADER_BYTES + data_len != bytes.len() {
            return Err(StorageApiError::Malformed);
        }
        Ok(Self {
            status,
            request_id,
            transaction_id: get_u64(bytes, 8),
            data: &bytes[RESPONSE_HEADER_BYTES..],
            more: message.flags & IPC_FLAG_MORE != 0,
        })
    }

    pub fn encode(
        status: StorageApiStatus,
        request_id: u32,
        transaction_id: u64,
        data: &[u8],
        more: bool,
    ) -> Option<IpcBytes> {
        if request_id == 0 || data.len() > u16::MAX as usize {
            return None;
        }
        let total_len = RESPONSE_HEADER_BYTES.checked_add(data.len())?;
        if total_len > MAX_IPC_BYTES {
            return None;
        }
        let mut message = IpcBytes::empty(MessageKind::StorageResponse);
        let bytes = &mut message.bytes[..total_len];
        bytes[0] = STORAGE_API_VERSION;
        bytes[1] = status as u8;
        put_u32(bytes, 4, request_id);
        put_u64(bytes, 8, transaction_id);
        put_u16(bytes, 16, data.len() as u16);
        bytes[RESPONSE_HEADER_BYTES..].copy_from_slice(data);
        message.len = total_len as u16;
        if more {
            message.flags = IPC_FLAG_MORE;
        }
        Some(message)
    }
}

fn put_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_u64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

fn get_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
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

    #[test]
    fn request_round_trips_bounded_paths_and_data() {
        let message = StorageApiRequest::encode(
            StorageApiOperation::Write,
            STORAGE_API_FLAG_REPLACE,
            7,
            9,
            12,
            b"/file",
            &[],
            b"payload",
        )
        .unwrap();
        let request = StorageApiRequest::decode(&message).unwrap();
        assert_eq!(request.operation, StorageApiOperation::Write);
        assert_eq!(request.flags, STORAGE_API_FLAG_REPLACE);
        assert_eq!(request.request_id, 7);
        assert_eq!(request.transaction_id, 9);
        assert_eq!(request.path, b"/file");
        assert_eq!(request.data, b"payload");
    }

    #[test]
    fn request_rejects_wrong_shape_and_oversized_payload() {
        assert!(
            StorageApiRequest::encode(StorageApiOperation::Rename, 0, 1, 0, 0, b"/old", &[], &[])
                .is_none()
        );
        assert!(
            StorageApiRequest::encode(
                StorageApiOperation::Write,
                0,
                1,
                0,
                0,
                b"/file",
                &[],
                &[0; MAX_IPC_BYTES]
            )
            .is_none()
        );
    }

    #[test]
    fn response_round_trips_status_and_more_flag() {
        let message =
            StorageApiResponse::encode(StorageApiStatus::Ok, 3, 11, b"data", true).unwrap();
        let response = StorageApiResponse::decode(&message).unwrap();
        assert_eq!(response.status, StorageApiStatus::Ok);
        assert_eq!(response.request_id, 3);
        assert_eq!(response.transaction_id, 11);
        assert_eq!(response.data, b"data");
        assert!(response.more);
    }

    #[test]
    fn response_rejects_truncated_header_as_malformed() {
        let mut message = IpcBytes::empty(MessageKind::StorageResponse);
        message.len = (RESPONSE_HEADER_BYTES - 1) as u16;
        assert!(matches!(StorageApiResponse::decode(&message), Err(StorageApiError::Malformed)));
    }
}
