//! Kernel-side validation for the dedicated storage request mailbox.

use logos_abi::{StorageOperation, StorageRequest, StorageResponse, StorageStatus};

pub const STORAGE_REQUEST_ENDPOINT: usize = logos_abi::IpcEndpointId::StorageToCore as usize;
pub const STORAGE_RESPONSE_ENDPOINT: usize = logos_abi::IpcEndpointId::CoreToStorage as usize;
pub const STORAGE_MAP_REQUEST_ENDPOINT: usize = logos_abi::IpcEndpointId::StorageMapToCore as usize;
pub const STORAGE_MAP_RESPONSE_ENDPOINT: usize =
    logos_abi::IpcEndpointId::CoreToStorageMap as usize;
pub const STORAGE_MAP_OPERATION: u8 = 1;
pub const STORAGE_UNMAP_OPERATION: u8 = 2;
pub const PACKAGE_REQUEST_ENDPOINT: usize = logos_abi::IpcEndpointId::CoreToStoragePackage as usize;
pub const PACKAGE_RESPONSE_ENDPOINT: usize =
    logos_abi::IpcEndpointId::StoragePackageToCore as usize;
pub const PACKAGE_REQUEST_CAPABILITY_SLOT: usize = 6;

pub const STORAGE_CACHE_PAGES: u64 = 32;
/// Map requests address cache slots, not physical frame numbers.
pub const STORAGE_CACHE_START: u64 = 0;
pub const STORAGE_MAP_WINDOWS_PER_CLIENT: usize = 4;
pub const STORAGE_MAP_WINDOW_PAGES: u64 = 16;
pub const STORAGE_MAP_CLIENTS: usize = 2;
pub const STORAGE_MAP_TARGET_BASE: u64 = 0x0000_0000_4000_0000;

pub const fn storage_map_target(client_slot: usize, window_slot: usize) -> Option<u64> {
    if client_slot >= STORAGE_MAP_CLIENTS || window_slot >= STORAGE_MAP_WINDOWS_PER_CLIENT {
        return None;
    }
    let ordinal = (client_slot * STORAGE_MAP_WINDOWS_PER_CLIENT + window_slot) as u64;
    Some(STORAGE_MAP_TARGET_BASE + ordinal * STORAGE_MAP_WINDOW_PAGES * 0x1000)
}

pub fn map_request_from_descriptor(
    generation: u64,
    client: u16,
    target_page: u64,
    window_generation: u32,
    descriptor: &[u8],
) -> Option<StorageMapRequest> {
    if descriptor.len() != logos_abi::STORAGE_API_MAP_DESCRIPTOR_BYTES {
        return None;
    }
    let source_page = u64::from_le_bytes(descriptor[..8].try_into().ok()?);
    Some(StorageMapRequest {
        generation,
        client,
        source_page,
        target_page,
        pages: descriptor[8],
        window_generation,
        flags: 0,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StorageMapRequest {
    pub generation: u64,
    pub client: u16,
    pub source_page: u64,
    pub target_page: u64,
    pub pages: u8,
    pub window_generation: u32,
    pub flags: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StorageMapWindow {
    pub target_page: u64,
    pub pages: u8,
    pub generation: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StorageMapResponse {
    pub generation: u64,
    pub window_generation: u32,
    pub target_page: u64,
    pub pages: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StorageMapRelease {
    pub generation: u64,
    pub client: u16,
    pub target_page: u64,
    pub window_generation: u32,
}

pub const fn storage_map_client_slot(client: u16) -> Option<usize> {
    match client {
        value if value == logos_abi::ServiceId::Flow as u16 => Some(0),
        value if value == logos_abi::ServiceId::Fetch as u16 => Some(1),
        _ => None,
    }
}

impl StorageMapResponse {
    pub const fn accepted(request: StorageMapRequest) -> Self {
        Self {
            generation: request.generation,
            window_generation: request.window_generation,
            target_page: request.target_page,
            pages: request.pages,
        }
    }
}

pub fn validate_map_request(
    request: StorageMapRequest,
    expected_generation: u64,
    expected_client: u16,
    cache_start: u64,
    windows: &[Option<StorageMapWindow>; STORAGE_MAP_WINDOWS_PER_CLIENT],
) -> Result<(), StorageStatus> {
    if request.generation != expected_generation || request.window_generation == 0 {
        return Err(StorageStatus::Stale);
    }
    if request.client != expected_client || request.flags != 0 {
        return Err(StorageStatus::Unauthorized);
    }
    if request.pages == 0 || u64::from(request.pages) > STORAGE_MAP_WINDOW_PAGES {
        return Err(StorageStatus::Invalid);
    }
    let source_end = request.source_page.checked_add(u64::from(request.pages));
    let target_bytes = u64::from(request.pages) * 0x1000;
    let target_end = request.target_page.checked_add(target_bytes);
    let Some(cache_end) = cache_start.checked_add(STORAGE_CACHE_PAGES) else {
        return Err(StorageStatus::Invalid);
    };
    if request.source_page < cache_start
        || source_end.is_none_or(|end| end > cache_end)
        || request.target_page == 0
        || request.target_page & 0xfff != 0
        || target_end.is_none_or(|end| end > 0x0000_8000_0000_0000)
    {
        return Err(StorageStatus::Invalid);
    }
    if windows.iter().flatten().any(|window| {
        let end = request.target_page + u64::from(request.pages) * 0x1000;
        let existing_end = window.target_page + u64::from(window.pages) * 0x1000;
        request.target_page < existing_end && window.target_page < end
    }) {
        return Err(StorageStatus::Invalid);
    }
    Ok(())
}

pub fn validate_map_descriptor(
    generation: u64,
    client: u16,
    source_page: u64,
    pages: u8,
    flags: u8,
    expected_generation: u64,
) -> Result<(), StorageStatus> {
    if generation != expected_generation {
        return Err(StorageStatus::Stale);
    }
    if storage_map_client_slot(client).is_none() || flags != 0 {
        return Err(StorageStatus::Unauthorized);
    }
    if pages == 0 || u64::from(pages) > STORAGE_MAP_WINDOW_PAGES {
        return Err(StorageStatus::Invalid);
    }
    let Some(end) = source_page.checked_add(u64::from(pages)) else {
        return Err(StorageStatus::Invalid);
    };
    if end > STORAGE_CACHE_START + STORAGE_CACHE_PAGES {
        return Err(StorageStatus::Invalid);
    }
    Ok(())
}

pub fn validate_request(
    request: StorageRequest,
    capability_slot: usize,
    generation: u16,
    service_epoch: u64,
) -> Result<(), StorageStatus> {
    if request.capability_slot as usize != capability_slot {
        return Err(StorageStatus::Unauthorized);
    }
    if request.generation != generation || request.service_epoch != service_epoch {
        return Err(StorageStatus::Stale);
    }
    if request.request_id == 0
        || request.flags != 0
        || request.blocks > logos_abi::STORAGE_MAX_BLOCKS_PER_REQUEST
        || request.payload_bytes as usize > logos_abi::IPC_PAGE_BYTES
    {
        return Err(StorageStatus::Invalid);
    }
    if matches!(request.operation, StorageOperation::Read | StorageOperation::Write) {
        if request.blocks != 1 || request.payload_bytes != logos_abi::STORAGE_BLOCK_BYTES {
            return Err(StorageStatus::Invalid);
        }
    } else if request.operation == StorageOperation::AppendRecord {
        if request.blocks != 0 || request.payload_bytes == 0 {
            return Err(StorageStatus::Invalid);
        }
    } else if request.blocks != 0 || request.payload_bytes != 0 {
        return Err(StorageStatus::Invalid);
    }
    Ok(())
}

pub const fn unsupported_response(request: StorageRequest) -> StorageResponse {
    StorageResponse::new(
        request.request_id,
        StorageStatus::Unsupported,
        request.generation,
        0,
        0,
        request.transaction_id,
    )
}

pub fn validate_package_request(
    request: logos_abi::PackageRequest,
    capability_slot: usize,
    generation: u16,
    service_epoch: u64,
) -> Result<logos_abi::ServiceId, logos_abi::PackageStatus> {
    request.validate(capability_slot, generation, service_epoch)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mailbox_validation_is_generation_and_capability_bound() {
        let request =
            StorageRequest::new(StorageOperation::Reopen, 1, 7, 0, 11, 0, 0, 0, 9).unwrap();
        assert_eq!(validate_request(request, 0, 7, 11), Ok(()));
        assert_eq!(validate_request(request, 1, 7, 11), Err(StorageStatus::Unauthorized));
        assert_eq!(validate_request(request, 0, 8, 11), Err(StorageStatus::Stale));
    }

    #[test]
    fn mailbox_returns_a_typed_bounded_response() {
        let request = StorageRequest::new(StorageOperation::Flush, 4, 1, 0, 2, 0, 0, 0, 3).unwrap();
        let response = unsupported_response(request);
        assert_eq!(response.request_id, 4);
        assert_eq!(response.status, StorageStatus::Unsupported);
        assert_eq!(response.transaction_id, 3);
    }

    #[test]
    fn mailbox_rejects_flags_and_wrong_block_payload_shapes() {
        let mut request = StorageRequest::new(
            StorageOperation::Read,
            4,
            1,
            0,
            2,
            0,
            1,
            logos_abi::STORAGE_BLOCK_BYTES,
            3,
        )
        .unwrap();
        request.flags = 1;
        assert_eq!(validate_request(request, 0, 1, 2), Err(StorageStatus::Invalid));
    }

    #[test]
    fn package_mailbox_validates_request_identity_and_shape() {
        let request = logos_abi::PackageRequest::new(
            logos_abi::PackageOperation::Lookup,
            logos_abi::ServiceId::Storage,
            1,
            7,
            6,
            11,
            0,
            0,
            0,
        )
        .unwrap();
        assert_eq!(validate_package_request(request, 6, 7, 11), Ok(logos_abi::ServiceId::Storage));
        assert_eq!(
            validate_package_request(request, 6, 8, 11),
            Err(logos_abi::PackageStatus::Stale)
        );
    }

    #[test]
    fn map_requests_are_read_only_bounded_and_non_overlapping() {
        let request = StorageMapRequest {
            generation: 9,
            client: 2,
            source_page: 100,
            target_page: 0x40_000,
            pages: 4,
            window_generation: 1,
            flags: 0,
        };
        let windows = [None; STORAGE_MAP_WINDOWS_PER_CLIENT];
        assert_eq!(validate_map_request(request, 9, 2, 96, &windows), Ok(()));
        assert_eq!(
            StorageMapResponse::accepted(request),
            StorageMapResponse {
                generation: 9,
                window_generation: 1,
                target_page: 0x40_000,
                pages: 4,
            }
        );

        let mut occupied = windows;
        occupied[0] = Some(StorageMapWindow { target_page: 0x40_000, pages: 2, generation: 1 });
        assert_eq!(validate_map_request(request, 9, 2, 96, &occupied), Err(StorageStatus::Invalid));
        assert_eq!(
            validate_map_request(StorageMapRequest { flags: 1, ..request }, 9, 2, 96, &windows),
            Err(StorageStatus::Unauthorized)
        );
        assert_eq!(storage_map_client_slot(logos_abi::ServiceId::Flow as u16), Some(0));
        assert_eq!(storage_map_client_slot(logos_abi::ServiceId::Fetch as u16), Some(1));
        assert_eq!(storage_map_client_slot(logos_abi::ServiceId::Storage as u16), None);
        assert_eq!(storage_map_target(0, 0), Some(STORAGE_MAP_TARGET_BASE));
        assert_eq!(storage_map_target(STORAGE_MAP_CLIENTS, 0), None);
        assert_eq!(
            map_request_from_descriptor(9, 2, 0x40_000, 1, &[100, 0, 0, 0, 0, 0, 0, 0, 4]),
            Some(request)
        );
    }
}
