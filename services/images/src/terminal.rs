#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", no_main)]
#![cfg_attr(not(target_os = "none"), allow(dead_code, unused_imports, unused_variables))]

mod common;

use logos_abi::{
    AtriumApp, AtriumSurfaceInput, AtriumSurfaceRequest, AtriumSurfaceResponse, InputMessage,
    IpcBytes, IpcStatus, KeyCode, KeyState, MessageKind, SurfaceHandle,
};

const INPUT_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_ATRIUM_SURFACE_INPUT,
    b"atrium",
    core::mem::size_of::<AtriumSurfaceInput>(),
    logos_abi::IpcRights::Receive,
);
const ATRIUM_RENDER_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_RENDER,
    b"atrium",
    core::mem::size_of::<logos_abi::RenderMessage>(),
    logos_abi::IpcRights::Send,
);
const ATRIUM_SURFACE_REQUEST_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_ATRIUM_SURFACE_REQUEST,
    b"atrium",
    core::mem::size_of::<AtriumSurfaceRequest>(),
    logos_abi::IpcRights::Send,
);
const ATRIUM_SURFACE_RESPONSE_CAPABILITY: common::CapabilitySpec =
    common::capability_contract_named(
        logos_abi::IPC_CONTRACT_ATRIUM_SURFACE_RESPONSE,
        b"atrium",
        core::mem::size_of::<AtriumSurfaceResponse>(),
        logos_abi::IpcRights::Receive,
    );
const SESSION_INPUT_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_BYTES,
    b"session",
    core::mem::size_of::<IpcBytes>(),
    logos_abi::IpcRights::Send,
);
const SESSION_OUTPUT_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_BYTES,
    b"session",
    core::mem::size_of::<IpcBytes>(),
    logos_abi::IpcRights::Receive,
);

static mut TERMINAL: logos_terminal::TerminalService = logos_terminal::TerminalService::new();
static mut PENDING_RENDER: Option<logos_abi::RenderMessage> = None;
static mut PENDING_SESSION_INPUT: Option<IpcBytes> = None;

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    common::init_service_allocator();
    let terminal = unsafe { &mut *core::ptr::addr_of_mut!(TERMINAL) };
    let pending_render = unsafe { &mut *core::ptr::addr_of_mut!(PENDING_RENDER) };
    let pending_session_input = unsafe { &mut *core::ptr::addr_of_mut!(PENDING_SESSION_INPUT) };
    let input_capability = match common::capability_handle(INPUT_CAPABILITY) {
        Ok(capability) => capability,
        Err(_) => common::idle(),
    };
    let atrium_render_capability = match common::capability_handle(ATRIUM_RENDER_CAPABILITY) {
        Ok(capability) => capability,
        Err(_) => common::idle(),
    };
    let atrium_surface_request_capability =
        match common::capability_handle(ATRIUM_SURFACE_REQUEST_CAPABILITY) {
            Ok(capability) => capability,
            Err(_) => common::idle(),
        };
    let atrium_surface_response_capability =
        match common::capability_handle(ATRIUM_SURFACE_RESPONSE_CAPABILITY) {
            Ok(capability) => capability,
            Err(_) => common::idle(),
        };
    let session_input_capability = match common::capability_handle(SESSION_INPUT_CAPABILITY) {
        Ok(capability) => capability,
        Err(_) => common::idle(),
    };
    let session_output_capability = match common::capability_handle(SESSION_OUTPUT_CAPABILITY) {
        Ok(capability) => capability,
        Err(_) => common::idle(),
    };
    let mut heartbeat_ticks = 0u16;
    let mut render_more = false;
    let client = common::bootstrap_page().service;
    let surface_request = AtriumSurfaceRequest::new(AtriumApp::Terminal, client, 1);
    let mut surface_request_sent = false;
    let mut terminal_surface = SurfaceHandle::EMPTY;
    loop {
        if render_more || pending_render.is_some() {
            common::heartbeat();
        } else {
            common::heartbeat_tick(&mut heartbeat_ticks);
        }
        if !surface_request_sent && !terminal_surface.is_valid() {
            match common::ipc_send_handle(atrium_surface_request_capability, &surface_request) {
                IpcStatus::Ok => surface_request_sent = true,
                IpcStatus::Full => {}
                IpcStatus::Stale
                | IpcStatus::Disconnected
                | IpcStatus::Unauthorized
                | IpcStatus::Malformed
                | IpcStatus::Empty => {}
            }
        }
        let mut surface_response =
            AtriumSurfaceResponse::new(surface_request, logos_abi::GuiStatus::Malformed);
        while common::ipc_receive_handle(atrium_surface_response_capability, &mut surface_response)
            == IpcStatus::Ok
        {
            if surface_response.is_valid_for(surface_request)
                && surface_response.status == logos_abi::GuiStatus::Ok
                && surface_response.surface.is_valid()
            {
                terminal_surface = surface_response.surface;
            } else if surface_response.is_revoke() && surface_response.surface == terminal_surface {
                terminal_surface = SurfaceHandle::EMPTY;
                *pending_render = None;
                render_more = false;
            }
        }
        if let Some(message) = *pending_session_input {
            match common::ipc_send_handle(session_input_capability, &message) {
                IpcStatus::Ok => {
                    *pending_session_input = None;
                }
                IpcStatus::Full => {}
                IpcStatus::Stale
                | IpcStatus::Disconnected
                | IpcStatus::Unauthorized
                | IpcStatus::Malformed
                | IpcStatus::Empty => *pending_session_input = None,
            }
        }
        if pending_session_input.is_none() {
            let mut event = AtriumSurfaceInput::new(
                SurfaceHandle::EMPTY,
                InputMessage::key(KeyCode::Unknown, KeyState::Released, 0),
            );
            while common::ipc_receive_handle(input_capability, &mut event) == IpcStatus::Ok {
                if !event.is_valid() || event.surface != terminal_surface {
                    continue;
                }
                if let Some(message) = terminal.input(&event.input) {
                    match common::ipc_send_handle(session_input_capability, &message) {
                        IpcStatus::Ok => {}
                        IpcStatus::Full => {
                            *pending_session_input = Some(message);
                            break;
                        }
                        IpcStatus::Stale
                        | IpcStatus::Disconnected
                        | IpcStatus::Unauthorized
                        | IpcStatus::Malformed
                        | IpcStatus::Empty => {}
                    }
                }
            }
            if pending_session_input.is_none() {}
        }
        let mut message = IpcBytes::empty(MessageKind::SessionOutput);
        while common::ipc_receive_handle(session_output_capability, &mut message) == IpcStatus::Ok {
            if let Some(bytes) = message.as_bytes() {
                terminal.session_output_bytes(bytes);
            }
        }
        if pending_render.is_none() {
            *pending_render = terminal.next_render();
        }
        if let Some(mut render) = *pending_render {
            if !terminal_surface.is_valid() {
                common::wait_on_capabilities(&[
                    input_capability,
                    session_input_capability,
                    session_output_capability,
                    atrium_render_capability,
                    atrium_surface_request_capability,
                    atrium_surface_response_capability,
                ]);
                continue;
            }
            render.surface = terminal_surface;
            match common::ipc_send_handle(atrium_render_capability, &render) {
                IpcStatus::Ok => {
                    *pending_render = None;
                    render_more = render.flags & logos_abi::RENDER_FLAG_MORE != 0;
                }
                IpcStatus::Full => *pending_render = Some(render),
                IpcStatus::Stale
                | IpcStatus::Disconnected
                | IpcStatus::Unauthorized
                | IpcStatus::Malformed
                | IpcStatus::Empty => {
                    *pending_render = None;
                    render_more = false;
                }
            }
        }
        if pending_render.is_none() && render_more {
            continue;
        }
        common::wait_on_capabilities(&[
            input_capability,
            session_input_capability,
            session_output_capability,
            atrium_render_capability,
            atrium_surface_request_capability,
            atrium_surface_response_capability,
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
