#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", no_main)]
#![cfg_attr(not(target_os = "none"), allow(dead_code, unused_imports, unused_variables))]

mod common;

use core::mem;

use logos_abi::{
    AtriumApp, AtriumSurfaceInput, AtriumSurfaceRequest, AtriumSurfaceResponse, GuiDrawBatch,
    GuiDrawCommand, GuiRect, IpcStatus, ManagerOperation, ManagerRequest, ManagerResponse,
    ManagerState, SurfaceHandle,
};

const ATRIUM_REQUEST: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_ATRIUM_SURFACE_REQUEST,
    b"atrium",
    mem::size_of::<AtriumSurfaceRequest>(),
    logos_abi::IpcRights::Send,
);
const ATRIUM_RESPONSE: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_ATRIUM_SURFACE_RESPONSE,
    b"atrium",
    mem::size_of::<AtriumSurfaceResponse>(),
    logos_abi::IpcRights::Receive,
);
const ATRIUM_INPUT: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_ATRIUM_SURFACE_INPUT,
    b"atrium",
    mem::size_of::<AtriumSurfaceInput>(),
    logos_abi::IpcRights::Receive,
);
const ATRIUM_DRAW: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_ATRIUM_SURFACE_DRAW,
    b"atrium",
    mem::size_of::<logos_abi::GuiSceneOp>(),
    logos_abi::IpcRights::Send,
);

fn next_id(next: &mut u32) -> u32 {
    let id = *next;
    *next = next.wrapping_add(1).max(1);
    id
}

fn state_name(state: ManagerState) -> &'static [u8] {
    match state {
        ManagerState::Vacant => b"vacant",
        ManagerState::Disabled => b"disabled",
        ManagerState::Stopped => b"stopped",
        ManagerState::Starting => b"starting",
        ManagerState::Running => b"running",
        ManagerState::Stopping => b"stopping",
        ManagerState::Failed => b"failed",
        ManagerState::Exited => b"exited",
        ManagerState::Faulted => b"faulted",
    }
}

fn push_text(batch: &mut GuiDrawBatch, x: i32, y: i32, color: u32, text: &[u8]) {
    if let Some(command) = GuiDrawCommand::glyph_run(x, y, color, text) {
        let _ = batch.push(command);
    }
}

fn draw_status(draw: logos_abi::CapabilityHandle, surface: SurfaceHandle, sequence: u32) {
    let mut batch = GuiDrawBatch::new(surface, sequence, GuiRect::SURFACE);
    let _ = batch.push(GuiDrawCommand::fill_surface(0x101820));
    push_text(&mut batch, 20, 24, 0xffffff, b"System");
    push_text(&mut batch, 20, 48, 0x7890aa, b"Service manager status");

    let mut cursor = 0;
    let mut row = 0i32;
    loop {
        let request_id = sequence.wrapping_add(row as u32).max(1);
        let request =
            ManagerRequest { cursor, ..ManagerRequest::new(ManagerOperation::List, request_id) };
        let mut response = ManagerResponse::new(
            ManagerOperation::List,
            logos_abi::ManagerStatus::Malformed,
            request_id,
        );
        if common::manager_call(&request, &mut response) != IpcStatus::Ok
            || response.status != logos_abi::ManagerStatus::Ok
        {
            break;
        }
        let record = response.record;
        let name_len = usize::from(record.name_len).min(record.name.len());
        let y = 76 + row.saturating_mul(16);
        push_text(&mut batch, 20, y, 0xd9e5f5, &record.name[..name_len]);
        push_text(&mut batch, 190, y, 0x7ee787, state_name(record.state));
        row += 1;
        if response.cursor == u64::MAX || response.cursor <= cursor || row >= 14 {
            break;
        }
        cursor = response.cursor;
    }
    let _ = common::ipc_send_scene_batch(draw, &batch, 1);
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    common::init_service_allocator();
    let request_cap = common::capability_handle(ATRIUM_REQUEST).unwrap_or_else(|_| common::idle());
    let response_cap =
        common::capability_handle(ATRIUM_RESPONSE).unwrap_or_else(|_| common::idle());
    let input_cap = common::capability_handle(ATRIUM_INPUT).unwrap_or_else(|_| common::idle());
    let draw_cap = common::capability_handle(ATRIUM_DRAW).unwrap_or_else(|_| common::idle());

    let mut next_request = 1u32;
    let mut sequence = 0u32;
    let mut surface = SurfaceHandle::EMPTY;
    let mut request_pending = false;
    let mut heartbeat_ticks = 0u16;
    let mut response = AtriumSurfaceResponse::new(
        AtriumSurfaceRequest::new(AtriumApp::System, common::bootstrap_page().service, 1),
        logos_abi::GuiStatus::Malformed,
    );
    let mut input = AtriumSurfaceInput::new(
        SurfaceHandle::EMPTY,
        logos_abi::InputMessage::key(logos_abi::KeyCode::Unknown, logos_abi::KeyState::Released, 0),
    );

    loop {
        common::heartbeat_tick(&mut heartbeat_ticks);
        if !surface.is_valid() && !request_pending {
            let request = AtriumSurfaceRequest::new(
                AtriumApp::System,
                common::bootstrap_page().service,
                next_id(&mut next_request),
            );
            if common::ipc_send_handle(request_cap, &request) == IpcStatus::Ok {
                request_pending = true;
            }
        }

        while common::ipc_receive_handle(response_cap, &mut response) == IpcStatus::Ok {
            request_pending = false;
            if response.status == logos_abi::GuiStatus::Ok && response.surface.is_valid() {
                surface = response.surface;
                sequence = sequence.wrapping_add(1).max(1);
                draw_status(draw_cap, surface, sequence);
            } else if response.is_revoke() || response.status == logos_abi::GuiStatus::NotFound {
                surface = SurfaceHandle::EMPTY;
            }
        }
        while common::ipc_receive_handle(input_cap, &mut input) == IpcStatus::Ok {
            if input.surface == surface && input.is_valid() && surface.is_valid() {
                sequence = sequence.wrapping_add(1).max(1);
                draw_status(draw_cap, surface, sequence);
            }
        }

        common::wait_on_capabilities(&[request_cap, response_cap, input_cap]);
    }
}

#[cfg(target_os = "none")]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo<'_>) -> ! {
    common::idle()
}

#[cfg(not(target_os = "none"))]
fn main() {}
