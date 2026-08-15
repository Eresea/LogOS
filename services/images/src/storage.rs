#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", no_main)]
#![cfg_attr(not(target_os = "none"), allow(dead_code, unused_imports, unused_variables))]

mod common;

use logos_abi::{
    IpcCapability, IpcStatus, StorageOperation, StorageRequest, StorageResponse, StorageStatus,
};
use logos_storage::Block;
use logos_storage_service::{DurableNamespace, IpcBlockStore, KernelStorageIpc, NamespaceError};

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

struct StorageTransport {
    operation: Option<StorageOperation>,
}

impl StorageTransport {
    const fn new() -> Self {
        Self { operation: None }
    }
}

impl KernelStorageIpc for StorageTransport {
    fn send(
        &mut self,
        _capability: IpcCapability,
        request: StorageRequest,
        staging: &mut Block,
    ) -> IpcStatus {
        self.operation = Some(request.operation);
        if request.operation == StorageOperation::Write {
            unsafe { *(logos_abi::STORAGE_DATA_BASE as *mut Block) = *staging };
        }
        common::ipc_send(REQUEST_CAPABILITY, &request)
    }

    fn receive(
        &mut self,
        _capability: IpcCapability,
        response: &mut StorageResponse,
        staging: &mut Block,
    ) -> IpcStatus {
        let status = common::ipc_receive(RESPONSE_CAPABILITY, response);
        if status == IpcStatus::Ok && self.operation == Some(StorageOperation::Read) {
            *staging = unsafe { *(logos_abi::STORAGE_DATA_BASE as *const Block) };
        }
        status
    }
}

fn new_store(capability: IpcCapability, blocks: u64) -> Option<IpcBlockStore<StorageTransport>> {
    IpcBlockStore::new_with_slot(
        StorageTransport::new(),
        capability,
        REQUEST_CAPABILITY as u16,
        capability.generation,
        capability.service_epoch,
        blocks,
    )
    .ok()
}

fn discover(capability: IpcCapability) -> Option<u64> {
    let request = StorageRequest::new(
        StorageOperation::Reopen,
        1,
        capability.generation,
        REQUEST_CAPABILITY as u16,
        capability.service_epoch,
        0,
        0,
        0,
        0,
    )?;
    let mut transport = StorageTransport::new();
    let mut staging = Block::zero();
    if transport.send(capability, request, &mut staging) != IpcStatus::Ok {
        return None;
    }
    let mut response = StorageResponse::new(1, StorageStatus::Invalid, 1, 0, 0, 0);
    if transport.receive(capability, &mut response, &mut staging) != IpcStatus::Ok
        || response.status != StorageStatus::Ok
    {
        return None;
    }
    (response.block_count > 2).then_some(response.block_count)
}

fn stop_on_storage_error<T>(_error: T) -> ! {
    let mut heartbeat_ticks = 0u16;
    loop {
        common::heartbeat_tick(&mut heartbeat_ticks, logos_abi::ServiceId::Storage);
        common::wait(0, logos_abi::ServiceId::Storage);
    }
}

fn run_filesystem(capability: IpcCapability, blocks: u64) -> ! {
    let Some(store) = new_store(capability, blocks) else {
        common::idle();
    };
    let mut filesystem = match DurableNamespace::open(store) {
        Ok(filesystem) => filesystem,
        Err(NamespaceError::Format(logos_storage::FormatError::Unformatted)) => {
            let Some(store) = new_store(capability, blocks) else {
                common::idle();
            };
            DurableNamespace::format(store).unwrap_or_else(|error| stop_on_storage_error(error))
        }
        Err(error) => stop_on_storage_error(error),
    };
    if filesystem.open_file(b"/marker").is_err() {
        let marker = filesystem
            .create_file(filesystem.root(), b"marker")
            .unwrap_or_else(|error| stop_on_storage_error(error));
        filesystem
            .write(marker, 0, b"LogOS storage marker")
            .unwrap_or_else(|error| stop_on_storage_error(error));
    }
    filesystem.flush().unwrap_or_else(|error| stop_on_storage_error(error));

    let mut heartbeat_ticks = 0u16;
    loop {
        common::heartbeat_tick(&mut heartbeat_ticks, logos_abi::ServiceId::Storage);
        common::wait(0, logos_abi::ServiceId::Storage);
    }
}

/// The storage image owns the durable format and namespace. It discovers the
/// whole raw volume, reopens valid media, or formats only blank media.
#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    let Some(capability) = common::capability(REQUEST_CAPABILITY) else {
        stop_on_storage_error(())
    };
    let Some(blocks) = discover(capability) else {
        stop_on_storage_error(())
    };
    run_filesystem(capability, blocks)
}

#[cfg(target_os = "none")]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo<'_>) -> ! {
    common::idle()
}

#[cfg(not(target_os = "none"))]
fn main() {}
