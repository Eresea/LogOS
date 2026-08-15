//! Kernel-side validation for the dedicated storage request mailbox.

use logos_abi::{StorageOperation, StorageRequest, StorageResponse, StorageStatus};

pub const STORAGE_REQUEST_ENDPOINT: usize = logos_abi::IpcEndpointId::StorageToCore as usize;
pub const STORAGE_RESPONSE_ENDPOINT: usize = logos_abi::IpcEndpointId::CoreToStorage as usize;

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
        || request.blocks > logos_abi::STORAGE_MAX_BLOCKS_PER_REQUEST
        || request.payload_bytes as usize > logos_abi::IPC_PAGE_BYTES
    {
        return Err(StorageStatus::Invalid);
    }
    if matches!(request.operation, StorageOperation::Read | StorageOperation::Write)
        != (request.blocks != 0)
    {
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
}
