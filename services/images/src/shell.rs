#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", no_main)]
#![cfg_attr(not(target_os = "none"), allow(dead_code, unused_imports, unused_variables))]

mod common;

use core::{mem, ptr};

use logos_abi::{
    AtriumControl, AtriumControlOperation, GuiSessionContext, InputMessage, IpcBytes, IpcStatus,
    MessageKind, UserOperation, UserRequest, UserResponse, UserStatus,
};

const INPUT_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_GUI_INPUT,
    b"input",
    core::mem::size_of::<InputMessage>(),
    logos_abi::IpcRights::Receive,
);
const FLOW_CONTEXT_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_GUI_SESSION,
    b"flow",
    core::mem::size_of::<GuiSessionContext>(),
    logos_abi::IpcRights::Send,
);
const USER_SEND_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_GUI_USER_REQUEST,
    b"user",
    mem::size_of::<IpcBytes>(),
    logos_abi::IpcRights::Send,
);
const USER_RECEIVE_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_GUI_USER_RESPONSE,
    b"user",
    mem::size_of::<IpcBytes>(),
    logos_abi::IpcRights::Receive,
);
const ATRIUM_CONTROL_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_ATRIUM_CONTROL,
    b"atrium",
    mem::size_of::<AtriumControl>(),
    logos_abi::IpcRights::Receive,
);
const ATRIUM_CONTEXT_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_GUI_SESSION,
    b"atrium",
    mem::size_of::<GuiSessionContext>(),
    logos_abi::IpcRights::Send,
);
const LOCKSCREEN_REQUEST_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_GUI_USER_REQUEST,
    b"lockscreen",
    mem::size_of::<UserRequest>(),
    logos_abi::IpcRights::Receive,
);
const LOCKSCREEN_RESPONSE_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_GUI_USER_RESPONSE,
    b"lockscreen",
    mem::size_of::<UserResponse>(),
    logos_abi::IpcRights::Send,
);

static mut SHELL: logos_shell::Shell = logos_shell::Shell::new();
fn flush_pending<T: Copy>(capability: logos_abi::CapabilityHandle, pending: &mut Option<T>) {
    let Some(message) = *pending else { return };
    match common::ipc_send_handle(capability, &message) {
        IpcStatus::Ok => *pending = None,
        IpcStatus::Full => {}
        IpcStatus::Stale
        | IpcStatus::Disconnected
        | IpcStatus::Unauthorized
        | IpcStatus::Malformed
        | IpcStatus::Empty => *pending = None,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    common::init_service_allocator();
    let input = match common::capability_handle(INPUT_CAPABILITY) {
        Ok(value) => value,
        Err(_) => common::idle(),
    };
    let flow = match common::capability_handle(FLOW_CONTEXT_CAPABILITY) {
        Ok(value) => value,
        Err(_) => common::idle(),
    };
    let user_send = match common::capability_handle(USER_SEND_CAPABILITY) {
        Ok(value) => value,
        Err(_) => common::idle(),
    };
    let user_receive = match common::capability_handle(USER_RECEIVE_CAPABILITY) {
        Ok(value) => value,
        Err(_) => common::idle(),
    };
    let atrium_control = match common::capability_handle(ATRIUM_CONTROL_CAPABILITY) {
        Ok(value) => value,
        Err(_) => common::idle(),
    };
    let atrium_context = match common::capability_handle(ATRIUM_CONTEXT_CAPABILITY) {
        Ok(value) => value,
        Err(_) => common::idle(),
    };
    let lockscreen_request = match common::capability_handle(LOCKSCREEN_REQUEST_CAPABILITY) {
        Ok(value) => value,
        Err(_) => common::idle(),
    };
    let lockscreen_response = match common::capability_handle(LOCKSCREEN_RESPONSE_CAPABILITY) {
        Ok(value) => value,
        Err(_) => common::idle(),
    };
    let shell = unsafe { &mut *core::ptr::addr_of_mut!(SHELL) };
    let context = GuiSessionContext::EMPTY;
    let mut pending_flow_context = Some(context);
    let mut pending_atrium_context = Some(context);
    let mut pending_user: Option<IpcBytes> = None;
    let mut pending_probe: Option<UserRequest> = None;
    let mut pending_lock_request: Option<UserRequest> = None;
    let mut pending_lock_response: Option<UserResponse> = None;
    let mut response = IpcBytes::empty(MessageKind::UserResponse);
    let mut lock_request = UserRequest::new(UserOperation::Login, 1);
    let mut atrium_command = AtriumControl::new(AtriumControlOperation::Reset, 1);
    let mut message =
        InputMessage::key(logos_abi::KeyCode::Unknown, logos_abi::KeyState::Released, 0);
    let mut heartbeat_ticks = 0u16;
    if let Ok(mut request) = shell.begin_user_request(UserOperation::Login, b"Probe", b"probe") {
        let request_id = request.request_id;
        let bytes = unsafe {
            core::slice::from_raw_parts(
                (&request as *const UserRequest).cast::<u8>(),
                mem::size_of::<UserRequest>(),
            )
        };
        pending_user = IpcBytes::from_bytes(MessageKind::UserRequest, bytes);
        pending_probe = Some(UserRequest::new(UserOperation::Login, request_id));
        logos_shell::Shell::acknowledge_sent(&mut request);
    }
    loop {
        common::heartbeat_tick(&mut heartbeat_ticks);
        flush_pending(flow, &mut pending_flow_context);
        flush_pending(atrium_context, &mut pending_atrium_context);
        flush_pending(lockscreen_response, &mut pending_lock_response);
        while common::ipc_receive_handle(atrium_control, &mut atrium_command) == IpcStatus::Ok {
            if atrium_command.operation == AtriumControlOperation::Logout && pending_user.is_none()
            {
                if let Ok(mut request) = shell.logout() {
                    pending_flow_context = Some(GuiSessionContext::EMPTY);
                    pending_atrium_context = Some(GuiSessionContext::EMPTY);
                    let bytes = unsafe {
                        core::slice::from_raw_parts(
                            (&request as *const UserRequest).cast::<u8>(),
                            mem::size_of::<UserRequest>(),
                        )
                    };
                    pending_user = IpcBytes::from_bytes(MessageKind::UserRequest, bytes);
                    logos_shell::Shell::acknowledge_sent(&mut request);
                }
            }
        }
        while common::ipc_receive_handle(lockscreen_request, &mut lock_request) == IpcStatus::Ok {
            if pending_user.is_some()
                || pending_probe.is_some()
                || pending_lock_request.is_some()
                || pending_lock_response.is_some()
                || !lock_request.is_valid()
                || !matches!(lock_request.operation, UserOperation::Claim | UserOperation::Login)
            {
                continue;
            }
            let name = &lock_request.name[..lock_request.name_len as usize];
            let password = &lock_request.password[..lock_request.password_len as usize];
            if let Ok(mut request) =
                shell.begin_user_request(lock_request.operation, name, password)
            {
                let bytes = unsafe {
                    core::slice::from_raw_parts(
                        (&request as *const UserRequest).cast::<u8>(),
                        mem::size_of::<UserRequest>(),
                    )
                };
                pending_user = IpcBytes::from_bytes(MessageKind::UserRequest, bytes);
                pending_lock_request = Some(lock_request);
                logos_shell::Shell::acknowledge_sent(&mut request);
            }
        }
        while common::ipc_receive_handle(input, &mut message) == IpcStatus::Ok {
            // Atrium routes locked input to the standalone LockScreen image.
        }
        if let Some(request) = pending_user {
            match common::ipc_send_handle(user_send, &request) {
                IpcStatus::Ok => pending_user = None,
                IpcStatus::Full => {}
                _ => pending_user = None,
            }
        }
        while common::ipc_receive_handle(user_receive, &mut response) == IpcStatus::Ok {
            if let Some(bytes) =
                response.as_bytes().filter(|bytes| bytes.len() == mem::size_of::<UserResponse>())
            {
                let result: UserResponse = unsafe { ptr::read_unaligned(bytes.as_ptr().cast()) };
                let probe = pending_probe.filter(|request| result.is_valid_for(*request));
                let applied = shell.apply_user_response(result);
                if let Some(request) = probe {
                    pending_probe = None;
                    if applied.is_ok() && result.status == UserStatus::Unclaimed {
                        pending_lock_response = Some(UserResponse::new(request, result.status));
                    }
                } else if let Some(request) = pending_lock_request.take() {
                    let status = if applied.is_ok() { result.status } else { UserStatus::Stale };
                    let mut lock_response = UserResponse::new(request, status);
                    if status == UserStatus::Ok {
                        lock_response.user = result.user;
                        lock_response.role = result.role;
                        lock_response.session = result.session;
                        lock_response.capability = result.capability;
                        lock_response.root = result.root;
                        lock_response.rights = result.rights;
                    }
                    pending_lock_response = Some(lock_response);
                }
                if applied.is_ok() {
                    let context = shell.context();
                    pending_flow_context = Some(context);
                    pending_atrium_context = Some(context);
                }
            }
        }
        common::wait_on_capabilities(&[
            input,
            user_send,
            user_receive,
            flow,
            atrium_control,
            atrium_context,
            lockscreen_request,
            lockscreen_response,
        ]);
    }
}

#[cfg(target_os = "none")]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo<'_>) -> ! {
    common::idle()
}

#[cfg(not(target_os = "none"))]
fn main() {}
