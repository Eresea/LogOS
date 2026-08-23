use logos_abi::{DeviceRequest, DeviceStatus};

pub const DEVICE_REQUEST_ENDPOINT: usize = logos_abi::IpcEndpointId::DeviceToCore as usize;
pub const DEVICE_RESPONSE_ENDPOINT: usize = logos_abi::IpcEndpointId::CoreToDevice as usize;

pub fn validate_request(
    request: DeviceRequest,
    generation: u16,
    service_epoch: u64,
) -> Result<(), DeviceStatus> {
    if generation == 0 || service_epoch == 0 || !request.is_valid() {
        return Err(DeviceStatus::Stale);
    }
    Ok(())
}

pub fn validate_dynamic_request(
    request: DeviceRequest,
    generation: u16,
    service_epoch: u64,
) -> Result<(), DeviceStatus> {
    if generation == 0 || service_epoch == 0 || !request.is_valid() {
        return Err(DeviceStatus::Stale);
    }
    Ok(())
}
