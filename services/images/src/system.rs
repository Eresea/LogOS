#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", no_main)]
#![cfg_attr(not(target_os = "none"), allow(dead_code, unused_imports, unused_variables))]

mod common;

use core::mem;

use logos_abi::{
    AtriumApp, AtriumSurfaceInput, AtriumSurfaceRequest, AtriumSurfaceResponse, GUI_DRAW_FLAG_MORE,
    GuiDrawCommand, GuiSceneOp, IpcStatus, MAX_GUI_NODES, ManagerOperation, ManagerRequest,
    ManagerResponse, ManagerState, SurfaceHandle,
};
use logos_atrium::{FULLSCREEN_SURFACE_BOUNDS, STATUS_BAR_BOUNDS, STATUS_BAR_CLOSE_BOUNDS};

const MAX_SYSTEM_SCENE_OPS: usize = MAX_GUI_NODES + 2;

#[derive(Clone, Copy)]
struct SystemScene {
    ops: [GuiSceneOp; MAX_SYSTEM_SCENE_OPS],
    len: u8,
}

impl SystemScene {
    const EMPTY_OP: GuiSceneOp = GuiSceneOp::commit(SurfaceHandle::EMPTY, 1);

    const fn new() -> Self {
        Self { ops: [Self::EMPTY_OP; MAX_SYSTEM_SCENE_OPS], len: 0 }
    }

    const fn len(&self) -> usize {
        self.len as usize
    }

    fn push(&mut self, mut op: GuiSceneOp) -> bool {
        let index = self.len();
        if index >= MAX_SYSTEM_SCENE_OPS {
            return false;
        }
        op.flags = GUI_DRAW_FLAG_MORE;
        self.ops[index] = op;
        self.len += 1;
        true
    }

    fn finish(&mut self) {
        if self.len != 0 {
            self.ops[self.len() - 1].flags = 0;
        }
    }
}

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

#[cfg(feature = "qemu-proof")]
fn proof_line(message: &[u8]) {
    common::proof_line(message);
}

#[cfg(not(feature = "qemu-proof"))]
fn proof_line(_message: &[u8]) {}

#[allow(clippy::too_many_arguments)]
fn push_text(
    scene: &mut SystemScene,
    surface: SurfaceHandle,
    sequence: u32,
    node_id: u32,
    x: i32,
    y: i32,
    color: u32,
    text: &[u8],
) -> bool {
    let Some(command) = GuiDrawCommand::glyph_run(x, y, color, text) else { return false };
    scene.push(GuiSceneOp::upsert(surface, sequence, node_id, command))
}

fn build_status(surface: SurfaceHandle, sequence: u32) -> Option<SystemScene> {
    let mut scene = SystemScene::new();
    if !scene.push(GuiSceneOp::clear(surface, sequence))
        || !scene.push(GuiSceneOp::upsert(
            surface,
            sequence,
            1,
            GuiDrawCommand::fill_rect(FULLSCREEN_SURFACE_BOUNDS, 0x101820),
        ))
        || !scene.push(GuiSceneOp::upsert(
            surface,
            sequence,
            2,
            GuiDrawCommand::fill_rect(STATUS_BAR_BOUNDS, 0x182535),
        ))
        || !push_text(&mut scene, surface, sequence, 3, 16, 10, 0xffffff, b"System")
        || !scene.push(GuiSceneOp::upsert(
            surface,
            sequence,
            4,
            GuiDrawCommand::fill_rounded_rect(STATUS_BAR_CLOSE_BOUNDS, 0x9f3b3b, 6),
        ))
        || !push_text(&mut scene, surface, sequence, 5, 616, 10, 0xffffff, b"X")
        || !push_text(&mut scene, surface, sequence, 6, 20, 48, 0x7890aa, b"Service manager status")
    {
        return None;
    }

    let mut cursor = 0;
    let mut row = 0u32;
    loop {
        let request_id = sequence.wrapping_add(row).max(1);
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
        let y = 76 + (row as i32).saturating_mul(16);
        if !push_text(
            &mut scene,
            surface,
            sequence,
            7 + row * 2,
            20,
            y,
            0xd9e5f5,
            &record.name[..name_len],
        ) || !push_text(
            &mut scene,
            surface,
            sequence,
            8 + row * 2,
            190,
            y,
            0x7ee787,
            state_name(record.state),
        ) {
            return None;
        }
        row += 1;
        if response.cursor == u64::MAX || response.cursor <= cursor || row >= 4 {
            break;
        }
        cursor = response.cursor;
    }
    scene.finish();
    proof_line(b"LogOS vNext: System scene built");
    Some(scene)
}

fn flush_scene(
    draw: logos_abi::CapabilityHandle,
    scene: &SystemScene,
    index: &mut usize,
) -> IpcStatus {
    while *index < scene.len() {
        match common::ipc_send_handle(draw, &scene.ops[*index]) {
            IpcStatus::Ok => *index += 1,
            IpcStatus::Full => return IpcStatus::Full,
            status => {
                *index = scene.len();
                return status;
            }
        }
    }
    IpcStatus::Ok
}

fn queue_status(
    draw: logos_abi::CapabilityHandle,
    surface: SurfaceHandle,
    sequence: u32,
    pending: &mut SystemScene,
    pending_index: &mut usize,
) {
    if *pending_index < pending.len() {
        return;
    }
    let Some(scene) = build_status(surface, sequence) else { return };
    *pending = scene;
    *pending_index = 0;
    let _ = flush_scene(draw, pending, pending_index);
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
    let mut pending_scene = SystemScene::new();
    let mut pending_scene_index = 0usize;
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
        if pending_scene_index < pending_scene.len() {
            let _ = flush_scene(draw_cap, &pending_scene, &mut pending_scene_index);
            if pending_scene_index < pending_scene.len() {
                common::heartbeat();
                continue;
            }
        }
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
                queue_status(
                    draw_cap,
                    surface,
                    sequence,
                    &mut pending_scene,
                    &mut pending_scene_index,
                );
            } else if response.is_revoke() || response.status == logos_abi::GuiStatus::NotFound {
                surface = SurfaceHandle::EMPTY;
            }
        }
        while common::ipc_receive_handle(input_cap, &mut input) == IpcStatus::Ok {
            if input.surface == surface
                && input.is_valid()
                && surface.is_valid()
                && pending_scene_index == pending_scene.len()
            {
                sequence = sequence.wrapping_add(1).max(1);
                queue_status(
                    draw_cap,
                    surface,
                    sequence,
                    &mut pending_scene,
                    &mut pending_scene_index,
                );
            }
        }

        if pending_scene_index < pending_scene.len() {
            common::heartbeat();
            continue;
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
