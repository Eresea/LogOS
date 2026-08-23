#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", no_main)]
#![cfg_attr(not(target_os = "none"), allow(dead_code, unused_imports, unused_variables))]

mod common;

use logos_abi::{DeviceOperation, DeviceRequest, DeviceResponse, DeviceStatus, IpcStatus};

const FLOW_RECEIVE_CAPABILITY: common::CapabilitySpec = common::capability_contract(
    logos_abi::IPC_CONTRACT_DEVICE_REQUEST,
    logos_abi::ServiceId::Flow.index() as u32,
    core::mem::size_of::<DeviceRequest>(),
    logos_abi::IpcRights::Receive,
);
const FLOW_SEND_CAPABILITY: common::CapabilitySpec = common::capability_contract(
    logos_abi::IPC_CONTRACT_DEVICE_RESPONSE,
    logos_abi::ServiceId::Flow.index() as u32,
    core::mem::size_of::<DeviceResponse>(),
    logos_abi::IpcRights::Send,
);
const CORE_SEND_CAPABILITY: common::CapabilitySpec = common::capability_contract(
    logos_abi::IPC_CONTRACT_DEVICE_REQUEST,
    common::CORE_PEER_INDEX,
    core::mem::size_of::<DeviceRequest>(),
    logos_abi::IpcRights::Send,
);
const CORE_RECEIVE_CAPABILITY: common::CapabilitySpec = common::capability_contract(
    logos_abi::IPC_CONTRACT_DEVICE_RESPONSE,
    common::CORE_PEER_INDEX,
    core::mem::size_of::<DeviceResponse>(),
    logos_abi::IpcRights::Receive,
);
fn identity() -> (u16, u64) {
    let bootstrap = common::bootstrap_page();
    (bootstrap.service.generation() as u16, bootstrap.service_epoch)
}

fn error_response(request: DeviceRequest, status: DeviceStatus) -> DeviceResponse {
    let (generation, service_epoch) = identity();
    DeviceResponse::new(request, status, generation, service_epoch)
}

fn run(
    flow_receive_capability: logos_abi::CapabilityHandle,
    flow_send_capability: logos_abi::CapabilityHandle,
    core_send_capability: logos_abi::CapabilityHandle,
    core_receive_capability: logos_abi::CapabilityHandle,
) -> ! {
    let mut flow_request: Option<DeviceRequest> = None;
    let mut core_sent = false;
    let mut pending_response: Option<DeviceResponse> = None;
    let mut heartbeat_ticks = 0u16;
    let mut manager = logos_device::DeviceManager::new();

    loop {
        common::heartbeat_tick(&mut heartbeat_ticks);
        let mut progressed = false;

        if let Some(response) = pending_response {
            match common::ipc_send_handle(flow_send_capability, &response) {
                IpcStatus::Ok | IpcStatus::Stale | IpcStatus::Disconnected => {
                    pending_response = None;
                    flow_request = None;
                    core_sent = false;
                    progressed = true;
                }
                IpcStatus::Full => {}
                _ => {
                    pending_response = None;
                    flow_request = None;
                    core_sent = false;
                    progressed = true;
                }
            }
        }

        if pending_response.is_none() {
            if let Some(request) = flow_request {
                if !core_sent {
                    match common::ipc_send_handle(core_send_capability, &request) {
                        IpcStatus::Ok => core_sent = true,
                        IpcStatus::Full => {}
                        _ => {
                            pending_response = Some(error_response(request, DeviceStatus::Io));
                        }
                    }
                    progressed = true;
                } else {
                    let mut response = DeviceResponse::new(request, DeviceStatus::Invalid, 1, 1);
                    match common::ipc_receive_handle(core_receive_capability, &mut response) {
                        IpcStatus::Ok if response.is_valid_for(request) => {
                            if response.status == DeviceStatus::Ok
                                && manager.publish(response).is_err()
                            {
                                pending_response =
                                    Some(error_response(request, DeviceStatus::Invalid));
                            } else {
                                pending_response = Some(response);
                            }
                            progressed = true;
                        }
                        IpcStatus::Empty => {}
                        _ => {
                            pending_response = Some(error_response(request, DeviceStatus::Io));
                            progressed = true;
                        }
                    }
                }
            } else {
                let mut request = DeviceRequest::new(DeviceOperation::List, 1);
                match common::ipc_receive_handle(flow_receive_capability, &mut request) {
                    IpcStatus::Ok if request.is_valid() => {
                        flow_request = Some(request);
                        progressed = true;
                    }
                    IpcStatus::Empty => {}
                    _ => progressed = true,
                }
            }
        }

        if !progressed {
            common::wait_on_capabilities(&[
                flow_receive_capability,
                flow_send_capability,
                core_send_capability,
                core_receive_capability,
            ]);
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    common::init_service_allocator();
    let flow_receive_capability = match common::capability_handle(FLOW_RECEIVE_CAPABILITY) {
        Ok(capability) => capability,
        Err(_) => common::idle(),
    };
    let flow_send_capability = match common::capability_handle(FLOW_SEND_CAPABILITY) {
        Ok(capability) => capability,
        Err(_) => common::idle(),
    };
    let core_send_capability = match common::capability_handle(CORE_SEND_CAPABILITY) {
        Ok(capability) => capability,
        Err(_) => common::idle(),
    };
    let core_receive_capability = match common::capability_handle(CORE_RECEIVE_CAPABILITY) {
        Ok(capability) => capability,
        Err(_) => common::idle(),
    };
    run(
        flow_receive_capability,
        flow_send_capability,
        core_send_capability,
        core_receive_capability,
    )
}

#[cfg(target_os = "none")]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo<'_>) -> ! {
    common::idle()
}

#[cfg(not(target_os = "none"))]
fn main() {}
