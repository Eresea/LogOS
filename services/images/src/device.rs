#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", no_main)]
#![cfg_attr(not(target_os = "none"), allow(dead_code, unused_imports, unused_variables))]

mod common;

use logos_abi::{
    DeviceOperation, DeviceRequest, DeviceResponse, DeviceStatus, IpcEndpointId, IpcStatus,
    ServiceId,
};

const FLOW_RECEIVE_CAPABILITY: usize = common::capability_slot(
    ServiceId::Device,
    IpcEndpointId::FlowToDevice,
    logos_abi::IpcRights::Receive,
);
const FLOW_SEND_CAPABILITY: usize = common::capability_slot(
    ServiceId::Device,
    IpcEndpointId::DeviceToFlow,
    logos_abi::IpcRights::Send,
);
const CORE_SEND_CAPABILITY: usize = common::capability_slot(
    ServiceId::Device,
    IpcEndpointId::DeviceToCore,
    logos_abi::IpcRights::Send,
);
const CORE_RECEIVE_CAPABILITY: usize = common::capability_slot(
    ServiceId::Device,
    IpcEndpointId::CoreToDevice,
    logos_abi::IpcRights::Receive,
);

fn identity() -> (u16, u64) {
    common::capability(CORE_SEND_CAPABILITY)
        .map(|capability| (capability.generation, capability.service_epoch))
        .unwrap_or((1, 1))
}

fn error_response(request: DeviceRequest, status: DeviceStatus) -> DeviceResponse {
    let (generation, service_epoch) = identity();
    DeviceResponse::new(request, status, generation, service_epoch)
}

fn run() -> ! {
    let mut flow_request: Option<DeviceRequest> = None;
    let mut core_sent = false;
    let mut pending_response: Option<DeviceResponse> = None;
    let mut heartbeat_ticks = 0u16;
    let mut manager = logos_device::DeviceManager::new();

    loop {
        common::heartbeat_tick(&mut heartbeat_ticks, ServiceId::Device);
        let mut progressed = false;

        if let Some(response) = pending_response {
            match common::ipc_send(FLOW_SEND_CAPABILITY, &response) {
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
                    match common::ipc_send(CORE_SEND_CAPABILITY, &request) {
                        IpcStatus::Ok => core_sent = true,
                        IpcStatus::Full => {}
                        _ => {
                            pending_response = Some(error_response(request, DeviceStatus::Io));
                        }
                    }
                    progressed = true;
                } else {
                    let mut response = DeviceResponse::new(request, DeviceStatus::Invalid, 1, 1);
                    match common::ipc_receive(CORE_RECEIVE_CAPABILITY, &mut response) {
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
                match common::ipc_receive(FLOW_RECEIVE_CAPABILITY, &mut request) {
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
            common::wait(
                common::ipc_read_event(IpcEndpointId::FlowToDevice)
                    | common::ipc_write_event(IpcEndpointId::DeviceToFlow)
                    | common::ipc_write_event(IpcEndpointId::DeviceToCore)
                    | common::ipc_read_event(IpcEndpointId::CoreToDevice),
                ServiceId::Device,
            );
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    run()
}

#[cfg(target_os = "none")]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo<'_>) -> ! {
    common::idle()
}

#[cfg(not(target_os = "none"))]
fn main() {}
