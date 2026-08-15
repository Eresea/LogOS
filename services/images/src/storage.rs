#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", no_main)]
#![cfg_attr(not(target_os = "none"), allow(dead_code, unused_imports, unused_variables))]

mod common;

use logos_abi::{IpcStatus, StorageOperation, StorageRequest, StorageResponse, StorageStatus};

const REQUEST_CAPABILITY: usize = common::capability_slot(
    logos_abi::ServiceId::Storage,
    logos_abi::IpcEndpointId::StorageToCore,
    logos_abi::IpcRights::Send,
);
const RESPONSE_CAPABILITY: usize = common::capability_slot(
    logos_abi::ServiceId::Storage,
    logos_abi::IpcEndpointId::CoreToStorage,
    logos_abi::IpcRights::Receive,
);

/// The storage image is admitted as a fixed service endpoint. Block requests
/// remain kernel-mediated; this bounded lifecycle image exercises the request
/// boundary while the hardware-backed completion path is still being added.
#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    let mut heartbeat_ticks = 0u16;
    let mut request_id = 1u32;
    let mut waiting = false;
    loop {
        common::heartbeat_tick(&mut heartbeat_ticks, logos_abi::ServiceId::Storage);
        if !waiting {
            let Some(request) = StorageRequest::new(
                StorageOperation::Reopen,
                request_id,
                1,
                REQUEST_CAPABILITY as u16,
                1,
                0,
                0,
                0,
                0,
            ) else {
                common::idle();
            };
            match common::ipc_send(REQUEST_CAPABILITY, &request) {
                IpcStatus::Ok => waiting = true,
                IpcStatus::Full => common::wait(
                    common::ipc_write_event(logos_abi::IpcEndpointId::StorageToCore),
                    logos_abi::ServiceId::Storage,
                ),
                IpcStatus::Stale
                | IpcStatus::Disconnected
                | IpcStatus::Unauthorized
                | IpcStatus::Malformed
                | IpcStatus::Empty => common::wait(0, logos_abi::ServiceId::Storage),
            }
            continue;
        }

        let mut response = StorageResponse::new(0, StorageStatus::Invalid, 0, 0, 0, 0);
        match common::ipc_receive(RESPONSE_CAPABILITY, &mut response) {
            IpcStatus::Ok => {
                waiting = false;
                request_id = request_id.wrapping_add(1).max(1);
            }
            IpcStatus::Empty => common::wait(
                common::ipc_read_event(logos_abi::IpcEndpointId::CoreToStorage),
                logos_abi::ServiceId::Storage,
            ),
            IpcStatus::Stale
            | IpcStatus::Disconnected
            | IpcStatus::Unauthorized
            | IpcStatus::Malformed
            | IpcStatus::Full => {
                waiting = false;
            }
        }
    }
}

#[cfg(target_os = "none")]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo<'_>) -> ! {
    common::idle()
}

#[cfg(not(target_os = "none"))]
fn main() {}
