#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", no_main)]
#![cfg_attr(not(target_os = "none"), allow(dead_code, unused_imports, unused_variables))]

mod common;

use core::sync::atomic::{AtomicU32, Ordering};

use logos_abi::{
    IpcBytes, IpcEndpointId, IpcStatus, MessageKind, USER_STORAGE_CHUNK_BYTES,
    USER_STORAGE_FLAG_BEGIN, USER_STORAGE_FLAG_END, UserRequest, UserResponse,
    UserStorageOperation, UserStorageRequest, UserStorageResponse, UserStorageStatus,
};
use logos_user::{EntropySource, USER_SNAPSHOT_BYTES, UserCatalogStore, UserError, UserService};

const FLOW_RECEIVE_CAPABILITY: common::CapabilitySpec = common::capability_contract(
    logos_abi::IPC_CONTRACT_BYTES,
    logos_abi::ServiceId::Flow.index() as u32,
    core::mem::size_of::<IpcBytes>(),
    logos_abi::IpcRights::Receive,
);
const FLOW_SEND_CAPABILITY: common::CapabilitySpec = common::capability_contract(
    logos_abi::IPC_CONTRACT_BYTES,
    logos_abi::ServiceId::Flow.index() as u32,
    core::mem::size_of::<IpcBytes>(),
    logos_abi::IpcRights::Send,
);
const STORAGE_SEND_CAPABILITY: common::CapabilitySpec = common::capability_contract(
    logos_abi::IPC_CONTRACT_BYTES,
    logos_abi::ServiceId::Storage.index() as u32,
    core::mem::size_of::<IpcBytes>(),
    logos_abi::IpcRights::Send,
);
const STORAGE_RECEIVE_CAPABILITY: common::CapabilitySpec = common::capability_contract(
    logos_abi::IPC_CONTRACT_BYTES,
    logos_abi::ServiceId::Storage.index() as u32,
    core::mem::size_of::<IpcBytes>(),
    logos_abi::IpcRights::Receive,
);

static NEXT_REQUEST_ID: AtomicU32 = AtomicU32::new(1);

fn next_request_id() -> u32 {
    loop {
        let current = NEXT_REQUEST_ID.load(Ordering::Relaxed);
        let next = current.wrapping_add(1).max(1);
        if NEXT_REQUEST_ID
            .compare_exchange(current, next, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
        {
            return current;
        }
    }
}

struct Entropy;

impl Entropy {
    const fn new() -> Self {
        Self
    }

    #[cfg(target_arch = "x86_64")]
    fn hardware_word() -> Option<u64> {
        for _ in 0..8 {
            let mut value: u64;
            let mut ready: u8;
            unsafe {
                core::arch::asm!(
                    "rdrand {value}",
                    "setc {ready}",
                    value = lateout(reg) value,
                    ready = lateout(reg_byte) ready,
                    options(nostack),
                );
            }
            if ready != 0 {
                return Some(value);
            }
        }
        None
    }
}

impl EntropySource for Entropy {
    fn fill(&mut self, output: &mut [u8]) -> Result<(), UserError> {
        let mut offset = 0;
        while offset < output.len() {
            let word = Self::hardware_word().ok_or(UserError::Entropy)?.to_le_bytes();
            let length = (output.len() - offset).min(word.len());
            output[offset..offset + length].copy_from_slice(&word[..length]);
            offset += length;
        }
        Ok(())
    }
}

struct CatalogTransport {
    send_capability: logos_abi::CapabilityHandle,
    receive_capability: logos_abi::CapabilityHandle,
}

impl CatalogTransport {
    fn exchange(&mut self, request: UserStorageRequest) -> Result<UserStorageResponse, UserError> {
        let message = IpcBytes::from_bytes(MessageKind::UserStorageRequest, unsafe {
            core::slice::from_raw_parts(
                (&request as *const UserStorageRequest).cast::<u8>(),
                core::mem::size_of::<UserStorageRequest>(),
            )
        })
        .ok_or(UserError::Persistence)?;
        loop {
            match common::ipc_send_handle(self.send_capability, &message) {
                IpcStatus::Ok => break,
                IpcStatus::Full => common::wait(
                    common::ipc_write_event(IpcEndpointId::UserToStorage),
                    logos_abi::ServiceId::User,
                ),
                _ => return Err(UserError::Persistence),
            }
        }
        loop {
            let mut response = IpcBytes::empty(MessageKind::UserStorageResponse);
            match common::ipc_receive_handle(self.receive_capability, &mut response) {
                IpcStatus::Ok => {
                    if response.kind != MessageKind::UserStorageResponse {
                        return Err(UserError::Persistence);
                    }
                    let bytes = response.as_bytes().ok_or(UserError::Persistence)?;
                    if bytes.len() != core::mem::size_of::<UserStorageResponse>() {
                        return Err(UserError::Persistence);
                    }
                    let value: UserStorageResponse =
                        unsafe { core::ptr::read_unaligned(bytes.as_ptr().cast()) };
                    if !value.is_valid_for(request) {
                        return Err(UserError::Persistence);
                    }
                    return Ok(value);
                }
                IpcStatus::Empty => common::wait(
                    common::ipc_read_event(IpcEndpointId::StorageToUser),
                    logos_abi::ServiceId::User,
                ),
                _ => return Err(UserError::Persistence),
            }
        }
    }
}

impl UserCatalogStore for CatalogTransport {
    fn load(&mut self, output: &mut [u8]) -> Result<usize, UserError> {
        let mut offset = 0usize;
        let mut total = None;
        loop {
            let request = UserStorageRequest::new(
                UserStorageOperation::Load,
                next_request_id(),
                offset as u32,
            );
            let response = self.exchange(request)?;
            if response.status == UserStorageStatus::NotFound {
                return Err(UserError::NotFound);
            }
            if response.status != UserStorageStatus::Ok
                || response.offset as usize != offset
                || response.data_len as usize > USER_STORAGE_CHUNK_BYTES
            {
                return Err(UserError::Persistence);
            }
            let length = response.data_len as usize;
            let complete = response.total_len as usize;
            if complete == 0 || complete > output.len() || offset + length > complete {
                return Err(UserError::Persistence);
            }
            if total.replace(complete).is_some_and(|value| value != complete) {
                return Err(UserError::Persistence);
            }
            output[offset..offset + length].copy_from_slice(&response.data[..length]);
            offset += length;
            if offset == complete {
                return Ok(complete);
            }
            if length == 0 {
                return Err(UserError::Persistence);
            }
        }
    }

    fn save(&mut self, snapshot: &[u8]) -> Result<(), UserError> {
        if snapshot.is_empty() || snapshot.len() > USER_SNAPSHOT_BYTES {
            return Err(UserError::Persistence);
        }
        let mut offset = 0;
        while offset < snapshot.len() {
            let length = (snapshot.len() - offset).min(USER_STORAGE_CHUNK_BYTES);
            let mut request = UserStorageRequest::new(
                UserStorageOperation::Save,
                next_request_id(),
                offset as u32,
            );
            request.total_len = snapshot.len() as u32;
            if offset == 0 {
                request.flags |= USER_STORAGE_FLAG_BEGIN;
            }
            if offset + length == snapshot.len() {
                request.flags |= USER_STORAGE_FLAG_END;
            }
            if !request.set_data(&snapshot[offset..offset + length]) {
                return Err(UserError::Persistence);
            }
            let response = self.exchange(request)?;
            if response.status != UserStorageStatus::Ok {
                return Err(UserError::Persistence);
            }
            offset += length;
        }
        Ok(())
    }
}

fn response_message(response: UserResponse) -> IpcBytes {
    let bytes = unsafe {
        core::slice::from_raw_parts(
            (&response as *const UserResponse).cast::<u8>(),
            core::mem::size_of::<UserResponse>(),
        )
    };
    IpcBytes::from_bytes(MessageKind::UserResponse, bytes)
        .unwrap_or_else(|| IpcBytes::empty(MessageKind::UserResponse))
}

static mut SERVICE: UserService<Entropy> = UserService::new(Entropy::new());
static mut SNAPSHOT: [u8; USER_SNAPSHOT_BYTES] = [0; USER_SNAPSHOT_BYTES];

fn load_catalog(
    storage_send_capability: logos_abi::CapabilityHandle,
    storage_receive_capability: logos_abi::CapabilityHandle,
) -> bool {
    let mut transport = CatalogTransport {
        send_capability: storage_send_capability,
        receive_capability: storage_receive_capability,
    };
    unsafe {
        let service = core::ptr::addr_of_mut!(SERVICE).as_mut().unwrap();
        let buffer = core::ptr::addr_of_mut!(SNAPSHOT).as_mut().unwrap();
        service.catalog_mut().load_from(&mut transport, buffer).is_ok()
    }
}

fn persist_catalog(
    storage_send_capability: logos_abi::CapabilityHandle,
    storage_receive_capability: logos_abi::CapabilityHandle,
) -> bool {
    let mut transport = CatalogTransport {
        send_capability: storage_send_capability,
        receive_capability: storage_receive_capability,
    };
    unsafe {
        let service = core::ptr::addr_of_mut!(SERVICE).as_ref().unwrap();
        let buffer = core::ptr::addr_of_mut!(SNAPSHOT).as_mut().unwrap();
        service.catalog().save_to(&mut transport, buffer).is_ok()
    }
}

fn durable(operation: logos_abi::UserOperation) -> bool {
    matches!(
        operation,
        logos_abi::UserOperation::Claim
            | logos_abi::UserOperation::Create
            | logos_abi::UserOperation::Rename
            | logos_abi::UserOperation::SetPassword
            | logos_abi::UserOperation::CreateRole
            | logos_abi::UserOperation::AssignRole
    )
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    common::heartbeat(logos_abi::ServiceId::User);
    common::init_service_allocator();
    let flow_receive_capability = match common::capability_handle(FLOW_RECEIVE_CAPABILITY) {
        Ok(capability) => capability,
        Err(_) => common::idle(),
    };
    let flow_send_capability = match common::capability_handle(FLOW_SEND_CAPABILITY) {
        Ok(capability) => capability,
        Err(_) => common::idle(),
    };
    let storage_send_capability = match common::capability_handle(STORAGE_SEND_CAPABILITY) {
        Ok(capability) => capability,
        Err(_) => common::idle(),
    };
    let storage_receive_capability = match common::capability_handle(STORAGE_RECEIVE_CAPABILITY) {
        Ok(capability) => capability,
        Err(_) => common::idle(),
    };
    while !load_catalog(storage_send_capability, storage_receive_capability) {
        common::heartbeat(logos_abi::ServiceId::User);
        common::wait(0, logos_abi::ServiceId::User);
    }
    let mut heartbeat_ticks = 0u16;
    loop {
        common::heartbeat_tick(&mut heartbeat_ticks, logos_abi::ServiceId::User);
        let mut request = IpcBytes::empty(MessageKind::UserRequest);
        match common::ipc_receive_handle(flow_receive_capability, &mut request) {
            IpcStatus::Ok => {
                let response = request
                    .as_bytes()
                    .filter(|bytes| bytes.len() == core::mem::size_of::<UserRequest>())
                    .map(|bytes| unsafe { core::ptr::read_unaligned(bytes.as_ptr().cast()) })
                    .map(|request: UserRequest| {
                        let mut response = unsafe {
                            core::ptr::addr_of_mut!(SERVICE).as_mut().unwrap().handle(request)
                        };
                        if response.status == logos_abi::UserStatus::Ok
                            && durable(response.operation)
                            && !persist_catalog(storage_send_capability, storage_receive_capability)
                        {
                            response.status = logos_abi::UserStatus::Invalid;
                        }
                        response_message(response)
                    })
                    .unwrap_or_else(|| IpcBytes::empty(MessageKind::UserResponse));
                loop {
                    match common::ipc_send_handle(flow_send_capability, &response) {
                        IpcStatus::Ok => break,
                        IpcStatus::Full => common::wait(
                            common::ipc_write_event(IpcEndpointId::UserToFlow),
                            logos_abi::ServiceId::User,
                        ),
                        _ => break,
                    }
                }
            }
            IpcStatus::Empty => common::wait(
                common::ipc_read_event(IpcEndpointId::FlowToUser)
                    | common::ipc_read_event(IpcEndpointId::StorageToUser),
                logos_abi::ServiceId::User,
            ),
            _ => common::wait(0, logos_abi::ServiceId::User),
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
