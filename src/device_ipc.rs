use logos_abi::{DeviceRequest, DeviceStatus, IpcRights, ServiceId};

pub const DEVICE_REQUEST_ENDPOINT: usize = logos_abi::IpcEndpointId::DeviceToCore as usize;
pub const DEVICE_RESPONSE_ENDPOINT: usize = logos_abi::IpcEndpointId::CoreToDevice as usize;

pub fn validate_request(
    request: DeviceRequest,
    capability_slot: usize,
    generation: u16,
    service_epoch: u64,
) -> Result<(), DeviceStatus> {
    if capability_slot
        != logos_abi::ipc_capability_slot(
            ServiceId::Device,
            logos_abi::IpcEndpointId::DeviceToCore,
            IpcRights::Send,
        )
        .unwrap_or(usize::MAX)
    {
        return Err(DeviceStatus::Invalid);
    }
    if generation == 0 || service_epoch == 0 || !request.is_valid() {
        return Err(DeviceStatus::Stale);
    }
    Ok(())
}
