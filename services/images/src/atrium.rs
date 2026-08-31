#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", no_main)]
#![cfg_attr(not(target_os = "none"), allow(dead_code, unused_imports, unused_variables))]

mod common;

use logos_abi::{
    AtriumApp, AtriumControl, AtriumControlOperation, AtriumSurfaceInput, AtriumSurfaceRequest,
    AtriumSurfaceResponse, GuiDrawBatch, GuiDrawCommand, GuiHook, GuiHookKind, GuiRect, GuiSceneOp,
    GuiSessionContext, GuiSurfaceOperation, GuiSurfaceRequest, GuiSurfaceResponse, InputMessage,
    IpcStatus, KeyCode, KeyState, MessageKind, PointerState, RenderMessage, SurfaceHandle,
};

const INPUT_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_GUI_INPUT,
    b"input",
    core::mem::size_of::<InputMessage>(),
    logos_abi::IpcRights::Receive,
);
const DISPLAY_DRAW_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_GUI_DRAW,
    b"display",
    core::mem::size_of::<logos_abi::GuiSceneOp>(),
    logos_abi::IpcRights::Send,
);
const TERMINAL_RENDER_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_RENDER,
    b"terminal",
    core::mem::size_of::<RenderMessage>(),
    logos_abi::IpcRights::Receive,
);
const DISPLAY_RENDER_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_RENDER,
    b"display",
    core::mem::size_of::<RenderMessage>(),
    logos_abi::IpcRights::Send,
);
const DISPLAY_CONTROL_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_GUI_SURFACE,
    b"display",
    core::mem::size_of::<GuiSurfaceRequest>(),
    logos_abi::IpcRights::Send,
);
const DISPLAY_RESPONSE_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_GUI_SURFACE,
    b"display",
    core::mem::size_of::<GuiSurfaceResponse>(),
    logos_abi::IpcRights::Receive,
);
const TERMINAL_SURFACE_REQUEST_CAPABILITY: common::CapabilitySpec =
    common::capability_contract_named(
        logos_abi::IPC_CONTRACT_ATRIUM_SURFACE_REQUEST,
        b"terminal",
        core::mem::size_of::<AtriumSurfaceRequest>(),
        logos_abi::IpcRights::Receive,
    );
const TERMINAL_SURFACE_RESPONSE_CAPABILITY: common::CapabilitySpec =
    common::capability_contract_named(
        logos_abi::IPC_CONTRACT_ATRIUM_SURFACE_RESPONSE,
        b"terminal",
        core::mem::size_of::<AtriumSurfaceResponse>(),
        logos_abi::IpcRights::Send,
    );
const TERMINAL_SURFACE_INPUT_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_ATRIUM_SURFACE_INPUT,
    b"terminal",
    core::mem::size_of::<AtriumSurfaceInput>(),
    logos_abi::IpcRights::Send,
);
const SYSTEM_SURFACE_REQUEST_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_ATRIUM_SURFACE_REQUEST,
    b"system",
    core::mem::size_of::<AtriumSurfaceRequest>(),
    logos_abi::IpcRights::Receive,
);
const SYSTEM_SURFACE_RESPONSE_CAPABILITY: common::CapabilitySpec =
    common::capability_contract_named(
        logos_abi::IPC_CONTRACT_ATRIUM_SURFACE_RESPONSE,
        b"system",
        core::mem::size_of::<AtriumSurfaceResponse>(),
        logos_abi::IpcRights::Send,
    );
const SYSTEM_SURFACE_INPUT_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_ATRIUM_SURFACE_INPUT,
    b"system",
    core::mem::size_of::<AtriumSurfaceInput>(),
    logos_abi::IpcRights::Send,
);
const SYSTEM_SURFACE_DRAW_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_ATRIUM_SURFACE_DRAW,
    b"system",
    core::mem::size_of::<logos_abi::GuiSceneOp>(),
    logos_abi::IpcRights::Receive,
);
const SHELL_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_ATRIUM_CONTROL,
    b"shell",
    core::mem::size_of::<AtriumControl>(),
    logos_abi::IpcRights::Send,
);
const SHELL_CONTEXT_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_GUI_SESSION,
    b"shell",
    core::mem::size_of::<GuiSessionContext>(),
    logos_abi::IpcRights::Receive,
);
const LOCKSCREEN_INPUT_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_GUI_INPUT,
    b"lockscreen",
    core::mem::size_of::<InputMessage>(),
    logos_abi::IpcRights::Send,
);
const LOCKSCREEN_CONTROL_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_GUI_HOOK,
    b"lockscreen",
    core::mem::size_of::<GuiHook>(),
    logos_abi::IpcRights::Send,
);
const MAX_PENDING_SURFACE_COMMANDS: usize = logos_atrium::MAX_ATRIUM_SURFACES * 2;
const CURSOR_BOUNDS: GuiRect = GuiRect::new(0, 0, 640, 400);

#[derive(Clone, Copy)]
struct ProgramSurfaceCapabilities {
    client: logos_abi::ServiceHandle,
    input: logos_abi::CapabilityHandle,
    render: logos_abi::CapabilityHandle,
    draw: logos_abi::CapabilityHandle,
}

static mut ATRIUM: logos_atrium::Atrium = logos_atrium::Atrium::new();
static mut CALCULATOR: logos_atrium::Calculator = logos_atrium::Calculator::new();

fn push_text(batch: &mut GuiDrawBatch, x: i32, y: i32, color: u32, text: &[u8]) {
    if let Some(command) = GuiDrawCommand::glyph_run(x, y, color, text) {
        let _ = batch.push(command);
    }
}

#[cfg(feature = "qemu-proof")]
fn proof_line(message: &[u8]) {
    common::proof_line(message);
}

#[cfg(not(feature = "qemu-proof"))]
fn proof_line(_message: &[u8]) {}

fn push_surface_text(
    batch: &mut GuiDrawBatch,
    bounds: GuiRect,
    x: i32,
    y: i32,
    color: u32,
    text: &[u8],
) {
    push_text(batch, bounds.x.saturating_add(x), bounds.y.saturating_add(y), color, text);
}

fn draw_home(
    display: logos_abi::CapabilityHandle,
    surface: SurfaceHandle,
    launcher_index: usize,
    sequence: u32,
) {
    let mut batch = GuiDrawBatch::new(surface, sequence, GuiRect::SURFACE);
    batch.flags = logos_abi::GUI_DRAW_FLAG_MORE;
    let _ = batch.push(GuiDrawCommand::fill_surface(0x101820));
    let _ = batch.push(GuiDrawCommand::shadow(
        GuiRect::new(120, 48, 400, 320),
        0x55000000,
        16,
        3,
        0,
        4,
    ));
    let _ = batch.push(GuiDrawCommand::fill_rounded_rect(
        logos_atrium::COMMAND_MENU_BOUNDS,
        0x182535,
        16,
    ));
    let _ = common::ipc_send_scene_batch(display, &batch, 1);

    let mut header = GuiDrawBatch::new(surface, sequence, GuiRect::new(128, 56, 384, 48));
    header.flags = logos_abi::GUI_DRAW_FLAG_MORE;
    push_text(&mut header, 160, 82, 0xffffff, b"Command menu");
    push_text(&mut header, 160, 100, 0xb8c7da, b"Choose an app to open");
    let _ = common::ipc_send_scene_batch(display, &header, 4);

    let labels: [&[u8]; 4] = [b"Calculator", b"Files", b"Terminal", b"System"];
    for (index, label) in labels.into_iter().enumerate() {
        let bounds = logos_atrium::command_menu_item_bounds(index);
        let mut item = GuiDrawBatch::new(surface, sequence, bounds);
        item.flags = logos_abi::GUI_DRAW_FLAG_MORE;
        let selected = index == launcher_index;
        let _ = item.push(GuiDrawCommand::fill_rounded_rect(
            bounds,
            if selected { 0x2f6f66 } else { 0x203040 },
            8,
        ));
        push_text(
            &mut item,
            bounds.x + 16,
            bounds.y + 11,
            if selected { 0xffffff } else { 0xd9e5f5 },
            label,
        );
        let _ = common::ipc_send_scene_batch(display, &item, 6 + index as u32 * 2);
    }

    let mut footer = GuiDrawBatch::new(surface, sequence, GuiRect::new(128, 324, 384, 36));
    push_text(&mut footer, 160, 346, 0xb8c7da, b"Up/Down select   Enter open");
    let _ = common::ipc_send_scene_batch(display, &footer, 14);
}

fn draw_calculator_ui(
    display: logos_abi::CapabilityHandle,
    surface: logos_atrium::Surface,
    calculator: &logos_atrium::Calculator,
    sequence: u32,
) {
    let bounds = surface.bounds;
    let panel_bounds = GuiRect::new(
        bounds.x.saturating_add(12),
        bounds.y.saturating_add(40),
        bounds.width.saturating_sub(24),
        bounds.height.saturating_sub(52),
    );
    let mut base = GuiDrawBatch::new(surface.reference, sequence, bounds);
    base.flags = logos_abi::GUI_DRAW_FLAG_MORE;
    let _ = base.push(GuiDrawCommand::fill_rounded_rect(panel_bounds, 0x182535, 16));
    let display_bounds =
        GuiRect::new(bounds.x.saturating_add(20), bounds.y.saturating_add(52), 260, 40);
    let _ = base.push(GuiDrawCommand::fill_rounded_rect(display_bounds, 0x263548, 8));
    push_surface_text(&mut base, bounds, 32, 64, 0xffffff, calculator.display());
    let _ = common::ipc_send_scene_batch(display, &base, 6);

    let rows: [&[u8]; 4] = [
        b"[ 7 ]   [ 8 ]   [ 9 ]   [ / ]",
        b"[ 4 ]   [ 5 ]   [ 6 ]   [ * ]",
        b"[ 1 ]   [ 2 ]   [ 3 ]   [ - ]",
        b"[ 0 ]   [ . ]   [ = ]   [ + ]",
    ];
    for (row, labels) in rows.into_iter().enumerate() {
        let mut keypad = GuiDrawBatch::new(surface.reference, sequence, bounds);
        if row < 3 {
            keypad.flags = logos_abi::GUI_DRAW_FLAG_MORE;
        }
        push_surface_text(
            &mut keypad,
            bounds,
            20,
            logos_atrium::CALCULATOR_BUTTON_TOP + row as i32 * 28,
            0xffffff,
            labels,
        );
        let _ = common::ipc_send_scene_batch(display, &keypad, 9 + row as u32);
    }
}

fn draw_surface_chrome(
    display: logos_abi::CapabilityHandle,
    surface: logos_atrium::Surface,
    sequence: u32,
    title: &[u8],
    more: bool,
    opaque: bool,
) {
    let bounds = surface.bounds;
    let mut base = GuiDrawBatch::new(surface.reference, sequence, bounds);
    base.flags = logos_abi::GUI_DRAW_FLAG_MORE;
    let status_bar = GuiRect::new(bounds.x, bounds.y, bounds.width, 32);
    if opaque {
        let _ = base.push(GuiDrawCommand::fill_surface(0x101820));
    }
    let _ = base.push(GuiDrawCommand::fill_rect(status_bar, 0x182535));
    push_surface_text(&mut base, bounds, 16, 10, 0xffffff, title);
    let _ = common::ipc_send_scene_batch(display, &base, 1);

    let mut close = GuiDrawBatch::new(surface.reference, sequence, bounds);
    if more {
        close.flags = logos_abi::GUI_DRAW_FLAG_MORE;
    }
    let close_bounds = GuiRect::new(
        bounds.x.saturating_add(logos_atrium::STATUS_BAR_CLOSE_BOUNDS.x),
        bounds.y.saturating_add(logos_atrium::STATUS_BAR_CLOSE_BOUNDS.y),
        logos_atrium::STATUS_BAR_CLOSE_BOUNDS.width,
        logos_atrium::STATUS_BAR_CLOSE_BOUNDS.height,
    );
    let _ = close.push(GuiDrawCommand::fill_rounded_rect(close_bounds, 0x9f3b3b, 6));
    push_surface_text(&mut close, bounds, 616, 10, 0xffffff, b"X");
    let _ = common::ipc_send_scene_batch(display, &close, 4);
}

fn draw_app(
    display: logos_abi::CapabilityHandle,
    surface: logos_atrium::Surface,
    calculator: &logos_atrium::Calculator,
    sequence: u32,
) {
    let title: &[u8] = match surface.app {
        logos_atrium::AppId::Calculator => b"Calculator",
        logos_atrium::AppId::Files => b"Files",
        logos_atrium::AppId::Terminal => b"Terminal",
        logos_atrium::AppId::System => b"System",
    };
    match surface.app {
        logos_atrium::AppId::Calculator => {
            draw_surface_chrome(display, surface, sequence, title, true, true);
            draw_calculator_ui(display, surface, calculator, sequence)
        }
        logos_atrium::AppId::Files => {
            draw_surface_chrome(display, surface, sequence, title, true, true);
            let mut panel = GuiDrawBatch::new(surface.reference, sequence, surface.bounds);
            panel.flags = logos_abi::GUI_DRAW_FLAG_MORE;
            let _ = panel.push(GuiDrawCommand::fill_rect(
                GuiRect::new(
                    surface.bounds.x.saturating_add(20),
                    surface.bounds.y.saturating_add(52),
                    260,
                    48,
                ),
                0x263548,
            ));
            let _ = common::ipc_send_scene_batch(display, &panel, 6);
            let mut detail = GuiDrawBatch::new(
                surface.reference,
                sequence,
                GuiRect::new(
                    surface.bounds.x,
                    surface.bounds.y,
                    surface.bounds.width,
                    surface.bounds.height,
                ),
            );
            push_surface_text(&mut detail, surface.bounds, 32, 82, 0xffffff, calculator.display());
            push_surface_text(
                &mut detail,
                surface.bounds,
                24,
                132,
                0xb8c7da,
                b"0-9  +  -  *  /  Enter",
            );
            let _ = common::ipc_send_scene_batch(display, &detail, 7);
        }
        logos_atrium::AppId::Terminal => {
            draw_surface_chrome(display, surface, sequence, title, false, false);
        }
        logos_atrium::AppId::System => {
            draw_surface_chrome(display, surface, sequence, title, false, true);
        }
    }
}

fn next_request_id(next: &mut u32) -> u32 {
    let value = *next;
    *next = next.wrapping_add(1).max(1);
    value
}

struct SurfaceCommandQueue {
    requests: [Option<GuiSurfaceRequest>; MAX_PENDING_SURFACE_COMMANDS],
    head: usize,
    len: usize,
}

impl SurfaceCommandQueue {
    const fn new() -> Self {
        Self { requests: [None; MAX_PENDING_SURFACE_COMMANDS], head: 0, len: 0 }
    }

    fn push(&mut self, request: GuiSurfaceRequest) -> bool {
        for offset in 0..self.len {
            let index = (self.head + offset) % self.requests.len();
            let Some(queued) = self.requests[index] else { continue };
            if queued.surface == request.surface
                && (queued.operation == request.operation
                    || request.operation == GuiSurfaceOperation::Destroy)
            {
                self.requests[index] = Some(request);
                return true;
            }
        }
        if self.len == self.requests.len() {
            return false;
        }
        let index = (self.head + self.len) % self.requests.len();
        self.requests[index] = Some(request);
        self.len += 1;
        true
    }

    fn flush(&mut self, display: logos_abi::CapabilityHandle) {
        while self.len != 0 {
            let Some(request) = self.requests[self.head] else {
                self.len = 0;
                break;
            };
            match common::ipc_send_handle(display, &request) {
                IpcStatus::Ok => self.pop(),
                IpcStatus::Full => break,
                IpcStatus::Stale
                | IpcStatus::Disconnected
                | IpcStatus::Unauthorized
                | IpcStatus::Malformed
                | IpcStatus::Empty => self.pop(),
            }
        }
    }

    fn pop(&mut self) {
        self.requests[self.head] = None;
        self.head = (self.head + 1) % self.requests.len();
        self.len -= 1;
    }
}

fn send_surface_command(
    display: logos_abi::CapabilityHandle,
    queue: &mut SurfaceCommandQueue,
    operation: GuiSurfaceOperation,
    surface: SurfaceHandle,
    bounds: GuiRect,
    next: &mut u32,
) {
    let mut request = GuiSurfaceRequest::new(operation, next_request_id(next));
    request.surface = surface;
    request.bounds = bounds;
    if queue.push(request) {
        queue.flush(display);
    }
}

fn is_fps_toggle(input: &InputMessage) -> bool {
    input.kind == MessageKind::Key
        && input.state == KeyState::Pressed
        && input.code == KeyCode::function(12).raw()
        && input.modifiers & logos_abi::MOD_CTRL != 0
}

fn queue_fps_toggle(queue: &mut SurfaceCommandQueue, next: &mut u32) {
    let request = GuiSurfaceRequest::new(GuiSurfaceOperation::ToggleFps, next_request_id(next));
    let _ = queue.push(request);
}

fn send_lockscreen_section(lockscreen: logos_abi::CapabilityHandle, visible: bool, next: &mut u32) {
    let mut hook = GuiHook::new(GuiHookKind::Section, next_request_id(next));
    hook.deadline = u64::from(visible);
    let _ = common::ipc_send_handle(lockscreen, &hook);
}

fn queue_terminal_response(
    pending: &mut Option<AtriumSurfaceResponse>,
    request: AtriumSurfaceRequest,
    status: logos_abi::GuiStatus,
    surface: SurfaceHandle,
) {
    if pending.is_some() {
        return;
    }
    let mut response = AtriumSurfaceResponse::new(request, status);
    response.surface = surface;
    *pending = Some(response);
}

fn queue_terminal_revoke(
    pending: &mut Option<AtriumSurfaceResponse>,
    deferred: &mut Option<SurfaceHandle>,
    next: &mut u32,
    surface: SurfaceHandle,
) {
    if !surface.is_valid() {
        return;
    }
    if pending.is_none() {
        *pending = Some(AtriumSurfaceResponse::revoke(next_request_id(next), surface));
    } else {
        *deferred = Some(surface);
    }
}

fn queue_system_revoke(
    pending: &mut Option<AtriumSurfaceResponse>,
    deferred: &mut Option<SurfaceHandle>,
    response_capability: logos_abi::CapabilityHandle,
    pending_capability: &mut logos_abi::CapabilityHandle,
    next: &mut u32,
    surface: SurfaceHandle,
) {
    if !surface.is_valid() {
        return;
    }
    if pending.is_none() {
        *pending = Some(AtriumSurfaceResponse::revoke(next_request_id(next), surface));
        *pending_capability = response_capability;
    } else {
        *deferred = Some(surface);
    }
}

fn atrium_status(error: logos_atrium::AtriumError) -> logos_abi::GuiStatus {
    match error {
        logos_atrium::AtriumError::Capacity => logos_abi::GuiStatus::Capacity,
        logos_atrium::AtriumError::AlreadyRegistered | logos_atrium::AtriumError::NotFound => {
            logos_abi::GuiStatus::NotFound
        }
        logos_atrium::AtriumError::Locked => logos_abi::GuiStatus::Unauthorized,
        logos_atrium::AtriumError::InvalidSurface => logos_abi::GuiStatus::Malformed,
    }
}

fn hide_surfaces(
    display: logos_abi::CapabilityHandle,
    commands: &mut SurfaceCommandQueue,
    atrium: &mut logos_atrium::Atrium,
    next: &mut u32,
) {
    let mut handles = [SurfaceHandle::EMPTY; logos_atrium::MAX_ATRIUM_SURFACES];
    let mut count = 0;
    for surface in atrium.surfaces() {
        handles[count] = surface.reference;
        count += 1;
    }
    for surface in handles[..count].iter().copied() {
        send_surface_command(
            display,
            commands,
            GuiSurfaceOperation::Destroy,
            surface,
            GuiRect::EMPTY,
            next,
        );
    }
    if atrium.home_surface().is_valid() {
        send_surface_command(
            display,
            commands,
            GuiSurfaceOperation::Destroy,
            atrium.home_surface(),
            GuiRect::EMPTY,
            next,
        );
    }
    atrium.lock();
    atrium.clear_surfaces();
}

fn render(
    display: logos_abi::CapabilityHandle,
    atrium: &logos_atrium::Atrium,
    calculator: &logos_atrium::Calculator,
    sequence: &mut u32,
) {
    let Some(home) = atrium.home_surface().is_valid().then_some(atrium.home_surface()) else {
        return;
    };
    *sequence = sequence.wrapping_add(1).max(1);
    draw_home(display, home, atrium.launcher_index(), *sequence);
    if let Some(surface) = atrium.focused_surface() {
        draw_app(display, surface, calculator, *sequence);
    }
}

fn queue_home_surface(
    display_control: logos_abi::CapabilityHandle,
    pending_surface: &mut Option<(GuiSurfaceRequest, Option<logos_atrium::SurfaceRequest>)>,
    pending_surface_for_client: &mut bool,
    next: &mut u32,
) {
    if pending_surface.is_some() {
        return;
    }
    let mut request =
        GuiSurfaceRequest::new(GuiSurfaceOperation::CreateModal, next_request_id(next));
    request.bounds = GuiRect::new(0, 0, 640, 400);
    request.z_order = 1;
    if common::ipc_send_handle(display_control, &request) == IpcStatus::Ok {
        *pending_surface = Some((request, None));
        *pending_surface_for_client = false;
    }
}

fn queue_cursor_surface(
    display_control: logos_abi::CapabilityHandle,
    next: &mut u32,
) -> Option<GuiSurfaceRequest> {
    let mut request =
        GuiSurfaceRequest::new(GuiSurfaceOperation::CreateModal, next_request_id(next));
    request.bounds = CURSOR_BOUNDS;
    request.z_order = 3;
    let sent = common::ipc_send_handle(display_control, &request) == IpcStatus::Ok;
    sent.then_some(request)
}

fn cursor_op(
    surface: SurfaceHandle,
    x: i16,
    y: i16,
    pressed: bool,
    sequence: &mut u32,
) -> GuiSceneOp {
    let mut command =
        GuiDrawCommand::fill_rect(GuiRect::new(i32::from(x), i32::from(y), 3, 14), 0xffffff);
    command.auxiliary = pressed as u32;
    GuiSceneOp::upsert(surface, next_request_id(sequence), 1, command)
}

fn discover_program_capability(
    client: logos_abi::ServiceHandle,
    rights: logos_abi::IpcRights,
    contract_id: u16,
    message_bytes: usize,
) -> Option<logos_abi::CapabilityHandle> {
    common::discover_capabilities_contract(rights, contract_id, message_bytes)
        .ok()?
        .into_iter()
        .find_map(|(peer, capability)| (peer == client).then_some(capability))
}

fn app_id(app: AtriumApp) -> logos_atrium::AppId {
    match app {
        AtriumApp::Calculator => logos_atrium::AppId::Calculator,
        AtriumApp::Files => logos_atrium::AppId::Files,
        AtriumApp::Terminal => logos_atrium::AppId::Terminal,
        AtriumApp::System => logos_atrium::AppId::System,
    }
}

fn program_client_live(client: logos_abi::ServiceHandle) -> bool {
    common::discover_capabilities_contract(
        logos_abi::IpcRights::Receive,
        logos_abi::IPC_CONTRACT_ATRIUM_SURFACE_REQUEST,
        core::mem::size_of::<AtriumSurfaceRequest>(),
    )
    .is_ok_and(|clients| clients.into_iter().any(|(peer, _)| peer == client))
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    common::init_service_allocator();
    let input = common::capability_handle(INPUT_CAPABILITY).unwrap_or_else(|_| common::idle());
    let display =
        common::capability_handle(DISPLAY_DRAW_CAPABILITY).unwrap_or_else(|_| common::idle());
    let terminal_render =
        common::capability_handle(TERMINAL_RENDER_CAPABILITY).unwrap_or_else(|_| common::idle());
    let display_render =
        common::capability_handle(DISPLAY_RENDER_CAPABILITY).unwrap_or_else(|_| common::idle());
    let display_control =
        common::capability_handle(DISPLAY_CONTROL_CAPABILITY).unwrap_or_else(|_| common::idle());
    let display_response =
        common::capability_handle(DISPLAY_RESPONSE_CAPABILITY).unwrap_or_else(|_| common::idle());
    let terminal_surface_request = common::capability_handle(TERMINAL_SURFACE_REQUEST_CAPABILITY)
        .unwrap_or_else(|_| common::idle());
    let terminal_surface_response = common::capability_handle(TERMINAL_SURFACE_RESPONSE_CAPABILITY)
        .unwrap_or_else(|_| common::idle());
    let terminal = common::capability_handle(TERMINAL_SURFACE_INPUT_CAPABILITY)
        .unwrap_or_else(|_| common::idle());
    let system_surface_request = common::capability_handle(SYSTEM_SURFACE_REQUEST_CAPABILITY)
        .unwrap_or_else(|_| common::idle());
    let system_surface_response = common::capability_handle(SYSTEM_SURFACE_RESPONSE_CAPABILITY)
        .unwrap_or_else(|_| common::idle());
    let system_surface_input = common::capability_handle(SYSTEM_SURFACE_INPUT_CAPABILITY)
        .unwrap_or_else(|_| common::idle());
    let system_surface_draw = common::capability_handle(SYSTEM_SURFACE_DRAW_CAPABILITY)
        .unwrap_or_else(|_| common::idle());
    let shell = common::capability_handle(SHELL_CAPABILITY).unwrap_or_else(|_| common::idle());
    let shell_context =
        common::capability_handle(SHELL_CONTEXT_CAPABILITY).unwrap_or_else(|_| common::idle());
    let lockscreen_input =
        common::capability_handle(LOCKSCREEN_INPUT_CAPABILITY).unwrap_or_else(|_| common::idle());
    let lockscreen_control =
        common::capability_handle(LOCKSCREEN_CONTROL_CAPABILITY).unwrap_or_else(|_| common::idle());

    let atrium = unsafe { &mut *core::ptr::addr_of_mut!(ATRIUM) };
    let calculator = unsafe { &mut *core::ptr::addr_of_mut!(CALCULATOR) };
    let atrium_client = common::bootstrap_page().service;
    let mut next_request = 1u32;
    let mut sequence = 0u32;
    let mut pending_surface: Option<(GuiSurfaceRequest, Option<logos_atrium::SurfaceRequest>)> =
        None;
    let mut pending_surface_for_client = false;
    let mut pending_client_request: Option<AtriumSurfaceRequest> = None;
    let mut pending_client_response_capability = logos_abi::CapabilityHandle::EMPTY;
    let mut program_surface_capabilities: [Option<ProgramSurfaceCapabilities>;
        logos_atrium::MAX_ATRIUM_SURFACES] = [None; logos_atrium::MAX_ATRIUM_SURFACES];
    let mut last_terminal_request: Option<AtriumSurfaceRequest> = None;
    let mut last_system_request: Option<AtriumSurfaceRequest> = None;
    let mut terminal_client = logos_abi::ServiceHandle::EMPTY;
    let mut system_client = common::discover_capabilities_contract(
        logos_abi::IpcRights::Receive,
        logos_abi::IPC_CONTRACT_ATRIUM_SURFACE_REQUEST,
        core::mem::size_of::<AtriumSurfaceRequest>(),
    )
    .ok()
    .and_then(|records| {
        records
            .into_iter()
            .find(|(_, capability)| *capability == system_surface_request)
            .map(|(client, _)| client)
    })
    .unwrap_or(logos_abi::ServiceHandle::EMPTY);
    let mut pending_client_response: Option<AtriumSurfaceResponse> = None;
    let mut deferred_terminal_revoke: Option<SurfaceHandle> = None;
    let mut deferred_system_revoke: Option<SurfaceHandle> = None;
    let mut pending_render: Option<RenderMessage> = None;
    let mut pending_draw: Option<GuiSceneOp> = None;
    let mut cursor_surface = SurfaceHandle::EMPTY;
    let mut pending_cursor_surface = queue_cursor_surface(display_control, &mut next_request);
    let mut cursor_x = 320i16;
    let mut cursor_y = 200i16;
    let mut cursor_sequence = 1u32;
    let mut pending_cursor_draw: Option<GuiSceneOp> = None;
    let mut surface_commands = SurfaceCommandQueue::new();
    let mut authenticated = false;
    let mut heartbeat_ticks = 0u16;
    let mut event = InputMessage::key(KeyCode::Unknown, KeyState::Released, 0);
    let mut deferred_event = None;
    let mut response = GuiSurfaceResponse::new(
        GuiSurfaceRequest::new(GuiSurfaceOperation::CreateModal, 1),
        logos_abi::GuiStatus::Malformed,
    );
    atrium.lock();
    proof_line(b"LogOS vNext: Atrium locked route ready");
    send_lockscreen_section(lockscreen_control, true, &mut next_request);

    loop {
        common::heartbeat_tick(&mut heartbeat_ticks);
        surface_commands.flush(display_control);
        if !cursor_surface.is_valid() && pending_cursor_surface.is_none() {
            pending_cursor_surface = queue_cursor_surface(display_control, &mut next_request);
        }
        if let Some(batch) = pending_cursor_draw {
            match common::ipc_send_handle(display, &batch) {
                IpcStatus::Ok => pending_cursor_draw = None,
                IpcStatus::Full => {}
                _ => {
                    pending_cursor_draw = None;
                    cursor_surface = SurfaceHandle::EMPTY;
                }
            }
        }
        if pending_client_response.is_none() {
            if let Some(surface) = deferred_terminal_revoke.take() {
                pending_client_response = Some(AtriumSurfaceResponse::revoke(
                    next_request_id(&mut next_request),
                    surface,
                ));
            } else if let Some(surface) = deferred_system_revoke.take() {
                pending_client_response = Some(AtriumSurfaceResponse::revoke(
                    next_request_id(&mut next_request),
                    surface,
                ));
                pending_client_response_capability = system_surface_response;
            }
        }
        if let Some(response) = pending_client_response {
            let response_capability = if pending_client_response_capability.is_valid() {
                pending_client_response_capability
            } else {
                terminal_surface_response
            };
            match common::ipc_send_handle(response_capability, &response) {
                IpcStatus::Ok => {
                    pending_client_response = None;
                    pending_client_response_capability = logos_abi::CapabilityHandle::EMPTY;
                }
                IpcStatus::Full => {}
                IpcStatus::Stale
                | IpcStatus::Disconnected
                | IpcStatus::Unauthorized
                | IpcStatus::Malformed
                | IpcStatus::Empty => {
                    pending_client_response = None;
                    pending_client_response_capability = logos_abi::CapabilityHandle::EMPTY;
                }
            }
        }
        if let Some(batch) = pending_draw {
            let live = atrium.surface_by_reference(batch.surface).is_some_and(|surface| {
                (surface.app == logos_atrium::AppId::System && surface.client == system_client)
                    || program_surface_capabilities
                        .iter()
                        .flatten()
                        .any(|caps| caps.client == surface.client)
            });
            if !live {
                pending_draw = None;
            } else {
                match common::ipc_send_handle(display, &batch) {
                    IpcStatus::Ok => pending_draw = None,
                    IpcStatus::Full => {}
                    IpcStatus::Stale
                    | IpcStatus::Disconnected
                    | IpcStatus::Unauthorized
                    | IpcStatus::Malformed
                    | IpcStatus::Empty => pending_draw = None,
                }
            }
        }
        if let Some(message) = pending_render {
            let live = atrium.surface_by_reference(message.surface).is_some_and(|surface| {
                surface.app == logos_atrium::AppId::Terminal
                    || program_surface_capabilities
                        .iter()
                        .flatten()
                        .any(|caps| caps.client == surface.client)
            });
            if !live {
                pending_render = None;
            } else {
                match common::ipc_send_handle(display_render, &message) {
                    IpcStatus::Ok => pending_render = None,
                    IpcStatus::Full => {}
                    IpcStatus::Stale
                    | IpcStatus::Disconnected
                    | IpcStatus::Unauthorized
                    | IpcStatus::Malformed
                    | IpcStatus::Empty => pending_render = None,
                }
            }
        }
        let mut terminal_request = AtriumSurfaceRequest::new(AtriumApp::Terminal, atrium_client, 1);
        while common::ipc_receive_handle(terminal_surface_request, &mut terminal_request)
            == IpcStatus::Ok
        {
            if !terminal_request.is_valid() || terminal_request.app() != Some(AtriumApp::Terminal) {
                queue_terminal_response(
                    &mut pending_client_response,
                    terminal_request,
                    logos_abi::GuiStatus::Malformed,
                    SurfaceHandle::EMPTY,
                );
            } else {
                terminal_client = terminal_request.client();
                last_terminal_request = Some(terminal_request);
                if let Some(surface) = atrium
                    .surface_for_client(terminal_request.client(), logos_atrium::AppId::Terminal)
                {
                    queue_terminal_response(
                        &mut pending_client_response,
                        terminal_request,
                        logos_abi::GuiStatus::Ok,
                        surface.reference,
                    );
                } else if let Some(surface) = atrium.surface_for_app(logos_atrium::AppId::Terminal)
                {
                    let _ = atrium.close_reference(surface.reference);
                    send_surface_command(
                        display_control,
                        &mut surface_commands,
                        GuiSurfaceOperation::Destroy,
                        surface.reference,
                        GuiRect::EMPTY,
                        &mut next_request,
                    );
                    if pending_client_request.is_none() {
                        pending_client_request = Some(terminal_request);
                    } else {
                        queue_terminal_response(
                            &mut pending_client_response,
                            terminal_request,
                            logos_abi::GuiStatus::Backpressure,
                            SurfaceHandle::EMPTY,
                        );
                    }
                } else if atrium.phase() == logos_atrium::AtriumPhase::Home
                    && pending_client_request.is_none()
                {
                    pending_client_request = Some(terminal_request);
                } else {
                    queue_terminal_response(
                        &mut pending_client_response,
                        terminal_request,
                        logos_abi::GuiStatus::Backpressure,
                        SurfaceHandle::EMPTY,
                    );
                }
            }
        }
        let mut system_request = AtriumSurfaceRequest::new(AtriumApp::System, system_client, 1);
        while common::ipc_receive_handle(system_surface_request, &mut system_request)
            == IpcStatus::Ok
        {
            if !system_request.is_valid()
                || system_request.app() != Some(AtriumApp::System)
                || system_request.client() != system_client
            {
                let response_was_empty = pending_client_response.is_none();
                queue_terminal_response(
                    &mut pending_client_response,
                    system_request,
                    logos_abi::GuiStatus::Malformed,
                    SurfaceHandle::EMPTY,
                );
                if response_was_empty {
                    pending_client_response_capability = system_surface_response;
                }
            } else {
                system_client = system_request.client();
                last_system_request = Some(system_request);
                if let Some(surface) =
                    atrium.surface_for_client(system_client, logos_atrium::AppId::System)
                {
                    let response_was_empty = pending_client_response.is_none();
                    queue_terminal_response(
                        &mut pending_client_response,
                        system_request,
                        logos_abi::GuiStatus::Ok,
                        surface.reference,
                    );
                    if response_was_empty {
                        pending_client_response_capability = system_surface_response;
                    }
                }
            }
        }
        if pending_surface.is_none()
            && pending_client_request.is_none()
            && pending_client_response.is_none()
        {
            let requests = common::discover_capabilities_contract(
                logos_abi::IpcRights::Receive,
                logos_abi::IPC_CONTRACT_ATRIUM_SURFACE_REQUEST,
                core::mem::size_of::<AtriumSurfaceRequest>(),
            )
            .unwrap_or_default();
            for (client, request_capability) in requests {
                if client == atrium_client || client == terminal_client {
                    continue;
                }
                let mut request = AtriumSurfaceRequest::new(AtriumApp::Calculator, client, 1);
                if common::ipc_receive_handle(request_capability, &mut request) != IpcStatus::Ok {
                    continue;
                }
                if !request.is_valid() || request.client() != client {
                    queue_terminal_response(
                        &mut pending_client_response,
                        request,
                        logos_abi::GuiStatus::Malformed,
                        SurfaceHandle::EMPTY,
                    );
                    pending_client_response_capability = discover_program_capability(
                        client,
                        logos_abi::IpcRights::Send,
                        logos_abi::IPC_CONTRACT_ATRIUM_SURFACE_RESPONSE,
                        core::mem::size_of::<AtriumSurfaceResponse>(),
                    )
                    .unwrap_or(logos_abi::CapabilityHandle::EMPTY);
                    break;
                }
                let Some(response_capability) = discover_program_capability(
                    client,
                    logos_abi::IpcRights::Send,
                    logos_abi::IPC_CONTRACT_ATRIUM_SURFACE_RESPONSE,
                    core::mem::size_of::<AtriumSurfaceResponse>(),
                ) else {
                    continue;
                };
                let Some(input_capability) = discover_program_capability(
                    client,
                    logos_abi::IpcRights::Send,
                    logos_abi::IPC_CONTRACT_ATRIUM_SURFACE_INPUT,
                    core::mem::size_of::<AtriumSurfaceInput>(),
                ) else {
                    continue;
                };
                let Some(render_capability) = discover_program_capability(
                    client,
                    logos_abi::IpcRights::Receive,
                    logos_abi::IPC_CONTRACT_RENDER,
                    core::mem::size_of::<RenderMessage>(),
                ) else {
                    continue;
                };
                let Some(draw_capability) = discover_program_capability(
                    client,
                    logos_abi::IpcRights::Receive,
                    logos_abi::IPC_CONTRACT_ATRIUM_SURFACE_DRAW,
                    core::mem::size_of::<logos_abi::GuiSceneOp>(),
                ) else {
                    continue;
                };
                let app = request.app().unwrap_or(AtriumApp::Calculator);
                let Ok(surface_request) = atrium.request_surface(app_id(app), client) else {
                    let response =
                        AtriumSurfaceResponse::new(request, logos_abi::GuiStatus::Capacity);
                    pending_client_response = Some(response);
                    pending_client_response_capability = response_capability;
                    break;
                };
                let mut display_request = GuiSurfaceRequest::new(
                    GuiSurfaceOperation::CreateModal,
                    next_request_id(&mut next_request),
                );
                display_request.bounds = surface_request.bounds();
                display_request.z_order = 2;
                if common::ipc_send_handle(display_control, &display_request) == IpcStatus::Ok {
                    pending_surface = Some((display_request, Some(surface_request)));
                    pending_surface_for_client = true;
                    pending_client_request = Some(request);
                    pending_client_response_capability = response_capability;
                    let caps = ProgramSurfaceCapabilities {
                        client,
                        input: input_capability,
                        render: render_capability,
                        draw: draw_capability,
                    };
                    if let Some(slot) = program_surface_capabilities
                        .iter()
                        .position(|entry| entry.is_none_or(|entry| entry.client == client))
                    {
                        program_surface_capabilities[slot] = Some(caps);
                    }
                    break;
                }
            }
        }
        let mut context = GuiSessionContext::EMPTY;
        while common::ipc_receive_handle(shell_context, &mut context) == IpcStatus::Ok {
            if context.is_authenticated() {
                authenticated = true;
                proof_line(b"LogOS vNext: Atrium authenticated");
                send_lockscreen_section(lockscreen_control, false, &mut next_request);
                atrium.authenticate();
                if !atrium.home_surface().is_valid() && pending_surface.is_none() {
                    queue_home_surface(
                        display_control,
                        &mut pending_surface,
                        &mut pending_surface_for_client,
                        &mut next_request,
                    );
                }
            } else if authenticated {
                authenticated = false;
                pending_surface_for_client = false;
                let terminal_surface =
                    atrium.surface_for_app(logos_atrium::AppId::Terminal).map(|s| s.reference);
                let system_surface =
                    atrium.surface_for_app(logos_atrium::AppId::System).map(|s| s.reference);
                hide_surfaces(display_control, &mut surface_commands, atrium, &mut next_request);
                if let Some(surface) = terminal_surface {
                    queue_terminal_revoke(
                        &mut pending_client_response,
                        &mut deferred_terminal_revoke,
                        &mut next_request,
                        surface,
                    );
                }
                if let Some(surface) = system_surface {
                    queue_system_revoke(
                        &mut pending_client_response,
                        &mut deferred_system_revoke,
                        system_surface_response,
                        &mut pending_client_response_capability,
                        &mut next_request,
                        surface,
                    );
                }
                send_lockscreen_section(lockscreen_control, true, &mut next_request);
            }
        }

        let mut stale_program_surfaces = [SurfaceHandle::EMPTY; logos_atrium::MAX_ATRIUM_SURFACES];
        let mut stale_count = 0;
        for surface in atrium.surfaces() {
            if surface.client == atrium_client
                || surface.client == terminal_client
                || (surface.client == system_client && surface.app == logos_atrium::AppId::System)
            {
                continue;
            }
            if !program_client_live(surface.client) {
                stale_program_surfaces[stale_count] = surface.reference;
                stale_count += 1;
            }
        }
        for surface in stale_program_surfaces[..stale_count].iter().copied() {
            if let Ok(closed) = atrium.close_reference(surface) {
                send_surface_command(
                    display_control,
                    &mut surface_commands,
                    GuiSurfaceOperation::Destroy,
                    closed.reference,
                    GuiRect::EMPTY,
                    &mut next_request,
                );
                if let Some(caps) = program_surface_capabilities
                    .iter_mut()
                    .flatten()
                    .find(|caps| caps.client == closed.client)
                {
                    *caps = ProgramSurfaceCapabilities {
                        client: logos_abi::ServiceHandle::EMPTY,
                        input: logos_abi::CapabilityHandle::EMPTY,
                        render: logos_abi::CapabilityHandle::EMPTY,
                        draw: logos_abi::CapabilityHandle::EMPTY,
                    };
                }
            }
        }

        if pending_surface.is_none()
            && pending_client_request.is_some()
            && atrium.home_surface().is_valid()
        {
            if let Some(client_request) = pending_client_request {
                let app = client_request.app().map(app_id).unwrap_or(logos_atrium::AppId::Terminal);
                if let Ok(surface_request) = atrium.request_surface(app, client_request.client()) {
                    let mut request = GuiSurfaceRequest::new(
                        GuiSurfaceOperation::CreateModal,
                        next_request_id(&mut next_request),
                    );
                    request.bounds = surface_request.bounds();
                    request.z_order = 2;
                    if app == logos_atrium::AppId::Terminal {
                        request.flags = logos_abi::GUI_SURFACE_FLAG_TERMINAL;
                    }
                    if common::ipc_send_handle(display_control, &request) == IpcStatus::Ok {
                        pending_surface = Some((request, Some(surface_request)));
                        pending_surface_for_client = true;
                    }
                }
            }
        }

        while common::ipc_receive_handle(display_response, &mut response) == IpcStatus::Ok {
            if pending_cursor_surface.is_some_and(|request| response.is_valid_for(request)) {
                pending_cursor_surface = None;
                if response.status == logos_abi::GuiStatus::Ok && response.surface.is_valid() {
                    cursor_surface = response.surface;
                    pending_cursor_draw = Some(cursor_op(
                        cursor_surface,
                        cursor_x,
                        cursor_y,
                        false,
                        &mut cursor_sequence,
                    ));
                }
                continue;
            }
            let Some((request, app)) = pending_surface.take() else { continue };
            let for_client = pending_surface_for_client;
            if !response.is_valid_for(request) || response.request_id != request.request_id {
                pending_surface = Some((request, app));
                continue;
            }
            pending_surface_for_client = false;
            if response.status != logos_abi::GuiStatus::Ok || !response.surface.is_valid() {
                if for_client {
                    if let Some(client_request) = pending_client_request.take() {
                        queue_terminal_response(
                            &mut pending_client_response,
                            client_request,
                            response.status,
                            SurfaceHandle::EMPTY,
                        );
                    }
                }
                continue;
            }
            let home_surface = app.is_none();
            let admitted = if let Some(request) = app {
                match atrium.spawn_surface(request, response.surface) {
                    Ok(surface) if for_client => {
                        if let Some(client_request) = pending_client_request.take() {
                            queue_terminal_response(
                                &mut pending_client_response,
                                client_request,
                                logos_abi::GuiStatus::Ok,
                                surface.reference,
                            );
                        }
                        true
                    }
                    Ok(_) => true,
                    Err(error) => {
                        send_surface_command(
                            display_control,
                            &mut surface_commands,
                            GuiSurfaceOperation::Destroy,
                            response.surface,
                            GuiRect::EMPTY,
                            &mut next_request,
                        );
                        if for_client {
                            if let Some(client_request) = pending_client_request.take() {
                                queue_terminal_response(
                                    &mut pending_client_response,
                                    client_request,
                                    atrium_status(error),
                                    SurfaceHandle::EMPTY,
                                );
                            }
                        }
                        false
                    }
                }
            } else if atrium.set_home_surface(response.surface).is_ok() {
                true
            } else {
                send_surface_command(
                    display_control,
                    &mut surface_commands,
                    GuiSurfaceOperation::Destroy,
                    response.surface,
                    GuiRect::EMPTY,
                    &mut next_request,
                );
                false
            };
            if admitted {
                if !home_surface {
                    if let Some(surface) = atrium.focused_surface() {
                        send_surface_command(
                            display_control,
                            &mut surface_commands,
                            GuiSurfaceOperation::Focus,
                            surface.reference,
                            GuiRect::EMPTY,
                            &mut next_request,
                        );
                    }
                }
                if home_surface {
                    proof_line(b"LogOS vNext: Atrium home surface ready");
                }
                render(display, atrium, calculator, &mut sequence);
            }
            if authenticated
                && atrium.phase() == logos_atrium::AtriumPhase::Home
                && !atrium.home_surface().is_valid()
            {
                queue_home_surface(
                    display_control,
                    &mut pending_surface,
                    &mut pending_surface_for_client,
                    &mut next_request,
                );
            }
        }

        if pending_render.is_none() {
            let mut render = RenderMessage::empty(MessageKind::RenderCells);
            while common::ipc_receive_handle(terminal_render, &mut render) == IpcStatus::Ok {
                let terminal_surface_is_live = render.surface.is_valid()
                    && matches!(render.kind, MessageKind::RenderCells | MessageKind::FullRedraw)
                    && atrium
                        .surface_by_reference(render.surface)
                        .is_some_and(|surface| surface.app == logos_atrium::AppId::Terminal);
                if terminal_surface_is_live {
                    pending_render = Some(render);
                    break;
                }
            }
        }
        if pending_draw.is_none() {
            let mut op = GuiSceneOp::clear(SurfaceHandle::new(0, 1, 13).unwrap(), 1);
            while common::ipc_receive_handle(system_surface_draw, &mut op) == IpcStatus::Ok {
                let live = op.is_valid()
                    && atrium.surface_by_reference(op.surface).is_some_and(|surface| {
                        surface.app == logos_atrium::AppId::System
                            && surface.client == system_client
                    });
                if live {
                    pending_draw = Some(op);
                    break;
                }
                op = GuiSceneOp::clear(SurfaceHandle::new(0, 1, 13).unwrap(), 1);
            }
        }
        if pending_draw.is_none() {
            for caps in program_surface_capabilities.iter().flatten().copied() {
                let mut op = GuiSceneOp::clear(SurfaceHandle::new(0, 1, 13).unwrap(), 1);
                while common::ipc_receive_handle(caps.draw, &mut op) == IpcStatus::Ok {
                    let live = op.is_valid()
                        && atrium
                            .surface_by_reference(op.surface)
                            .is_some_and(|surface| surface.client == caps.client);
                    if live {
                        pending_draw = Some(op);
                        break;
                    }
                    op = GuiSceneOp::clear(SurfaceHandle::new(0, 1, 13).unwrap(), 1);
                }
                if pending_draw.is_some() {
                    break;
                }
            }
        }
        if pending_render.is_none() {
            for caps in program_surface_capabilities.iter().flatten().copied() {
                let mut render = RenderMessage::empty(MessageKind::RenderCells);
                while common::ipc_receive_handle(caps.render, &mut render) == IpcStatus::Ok {
                    let live =
                        matches!(render.kind, MessageKind::RenderCells | MessageKind::FullRedraw)
                            && atrium
                                .surface_by_reference(render.surface)
                                .is_some_and(|surface| surface.client == caps.client);
                    if live {
                        pending_render = Some(render);
                        break;
                    }
                }
                if pending_render.is_some() {
                    break;
                }
            }
        }

        let mut cursor_sent_in_input = false;
        loop {
            if let Some(next) = deferred_event.take() {
                event = next;
            } else if common::ipc_receive_handle(input, &mut event) != IpcStatus::Ok {
                break;
            }
            if event.pointer_event().is_some_and(|pointer| pointer.state == PointerState::Move) {
                let (latest, deferred) = logos_atrium::coalesce_pointer_move(event, &mut |next| {
                    common::ipc_receive_handle(input, next) == IpcStatus::Ok
                });
                event = latest;
                deferred_event = deferred;
            }
            if let Some(pointer) = event.pointer_event() {
                cursor_x = pointer.x.clamp(0, 639);
                cursor_y = pointer.y.clamp(0, 399);
                if cursor_surface.is_valid() {
                    let cursor = cursor_op(
                        cursor_surface,
                        cursor_x,
                        cursor_y,
                        pointer.buttons & 1 != 0,
                        &mut cursor_sequence,
                    );
                    if cursor_sent_in_input || pending_cursor_draw.is_some() {
                        pending_cursor_draw = Some(cursor);
                    } else {
                        match common::ipc_send_handle(display, &cursor) {
                            IpcStatus::Ok => cursor_sent_in_input = true,
                            IpcStatus::Full => pending_cursor_draw = Some(cursor),
                            _ => {
                                pending_cursor_draw = None;
                                cursor_surface = SurfaceHandle::EMPTY;
                            }
                        }
                    }
                }
            }
            if is_fps_toggle(&event) {
                queue_fps_toggle(&mut surface_commands, &mut next_request);
                surface_commands.flush(display_control);
                continue;
            }
            if !authenticated || atrium.phase() != logos_atrium::AtriumPhase::Home {
                if common::ipc_send_handle(lockscreen_input, &event) == IpcStatus::Ok {
                    proof_line(b"LogOS vNext: Atrium sent LockScreen input");
                } else {
                    proof_line(b"LogOS vNext: Atrium dropped LockScreen input");
                }
                continue;
            }
            let menu_selected = event
                .pointer_event()
                .and_then(|pointer| {
                    (pointer.state == PointerState::Down && pointer.buttons & 1 != 0)
                        .then(|| {
                            atrium.command_menu_item_at(i32::from(pointer.x), i32::from(pointer.y))
                        })
                        .flatten()
                })
                .is_some();
            if menu_selected {
                event = InputMessage::key(KeyCode::ENTER, KeyState::Pressed, 0);
            } else if let Some(pointer) = event.pointer_event() {
                if let Some(surface) = atrium.pointer_target(&event) {
                    if pointer.state == PointerState::Down {
                        send_surface_command(
                            display_control,
                            &mut surface_commands,
                            GuiSurfaceOperation::Focus,
                            surface.reference,
                            GuiRect::EMPTY,
                            &mut next_request,
                        );
                    }
                    let local_x = i32::from(pointer.x).saturating_sub(surface.bounds.x);
                    let local_y = i32::from(pointer.y).saturating_sub(surface.bounds.y);
                    let close_clicked = pointer.state == PointerState::Down
                        && logos_atrium::STATUS_BAR_CLOSE_BOUNDS.contains(local_x, local_y);
                    let local = InputMessage::pointer(
                        local_x.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16,
                        local_y.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16,
                        pointer.buttons,
                        pointer.state,
                    )
                    .unwrap_or(event);
                    if close_clicked {
                        event = InputMessage::key(KeyCode::ESCAPE, KeyState::Pressed, 0);
                    } else {
                        let routed = AtriumSurfaceInput::new(surface.reference, local);
                        if routed.is_valid() {
                            if surface.app == logos_atrium::AppId::Terminal {
                                let _ = common::ipc_send_handle(terminal, &routed);
                            } else if surface.app == logos_atrium::AppId::System {
                                let _ = common::ipc_send_handle(system_surface_input, &routed);
                            } else if surface.app == logos_atrium::AppId::Calculator
                                && calculator.input(&local)
                            {
                                render(display, atrium, calculator, &mut sequence);
                            } else if let Some(caps) = program_surface_capabilities
                                .iter()
                                .flatten()
                                .copied()
                                .find(|caps| caps.client == surface.client)
                            {
                                let _ = common::ipc_send_handle(caps.input, &routed);
                            }
                        }
                        continue;
                    }
                }
                if event.pointer_event().is_some() {
                    continue;
                }
            }
            let action = atrium.input(&event);
            match action {
                logos_atrium::AtriumAction::Launch(app) if pending_surface.is_none() => {
                    if let Some(surface) = atrium.surface_for_app(app) {
                        if atrium.focus(surface.id).is_ok() {
                            send_surface_command(
                                display_control,
                                &mut surface_commands,
                                GuiSurfaceOperation::Focus,
                                surface.reference,
                                GuiRect::EMPTY,
                                &mut next_request,
                            );
                            render(display, atrium, calculator, &mut sequence);
                        }
                        continue;
                    }
                    if app == logos_atrium::AppId::Terminal {
                        let Some(client_request) = last_terminal_request else { continue };
                        if pending_client_request.is_none() {
                            pending_client_request = Some(client_request);
                        }
                    } else if app == logos_atrium::AppId::System {
                        let Some(client_request) = last_system_request else { continue };
                        if pending_client_request.is_none() {
                            pending_client_request = Some(client_request);
                            pending_client_response_capability = system_surface_response;
                        }
                    }
                    let client = match app {
                        logos_atrium::AppId::Terminal => terminal_client,
                        logos_atrium::AppId::System => system_client,
                        _ => atrium_client,
                    };
                    if !client.is_valid() {
                        continue;
                    }
                    let Ok(surface_request) = atrium.request_surface(app, client) else { continue };
                    let mut request = GuiSurfaceRequest::new(
                        GuiSurfaceOperation::CreateModal,
                        next_request_id(&mut next_request),
                    );
                    request.bounds = surface_request.bounds();
                    request.z_order = 2;
                    if app == logos_atrium::AppId::Terminal {
                        request.flags = logos_abi::GUI_SURFACE_FLAG_TERMINAL;
                    }
                    if common::ipc_send_handle(display_control, &request) == IpcStatus::Ok {
                        pending_surface = Some((request, Some(surface_request)));
                        pending_surface_for_client = matches!(
                            app,
                            logos_atrium::AppId::Terminal | logos_atrium::AppId::System
                        );
                    }
                }
                logos_atrium::AtriumAction::Logout => {
                    let terminal_surface =
                        atrium.surface_for_app(logos_atrium::AppId::Terminal).map(|s| s.reference);
                    let system_surface =
                        atrium.surface_for_app(logos_atrium::AppId::System).map(|s| s.reference);
                    let _ = atrium.apply_action(action);
                    pending_surface_for_client = false;
                    hide_surfaces(
                        display_control,
                        &mut surface_commands,
                        atrium,
                        &mut next_request,
                    );
                    if let Some(surface) = terminal_surface {
                        queue_terminal_revoke(
                            &mut pending_client_response,
                            &mut deferred_terminal_revoke,
                            &mut next_request,
                            surface,
                        );
                    }
                    if let Some(surface) = system_surface {
                        queue_system_revoke(
                            &mut pending_client_response,
                            &mut deferred_system_revoke,
                            system_surface_response,
                            &mut pending_client_response_capability,
                            &mut next_request,
                            surface,
                        );
                    }
                    let command = AtriumControl::new(AtriumControlOperation::Logout, 1);
                    let _ = common::ipc_send_handle(shell, &command);
                }
                logos_atrium::AtriumAction::LauncherChanged => {
                    render(display, atrium, calculator, &mut sequence);
                }
                logos_atrium::AtriumAction::CloseFocused => {
                    let old = atrium.focused_surface();
                    if atrium.apply_action(action).is_ok() {
                        if let Some(surface) = old {
                            if surface.app == logos_atrium::AppId::Terminal {
                                queue_terminal_revoke(
                                    &mut pending_client_response,
                                    &mut deferred_terminal_revoke,
                                    &mut next_request,
                                    surface.reference,
                                );
                            } else if surface.app == logos_atrium::AppId::System {
                                queue_system_revoke(
                                    &mut pending_client_response,
                                    &mut deferred_system_revoke,
                                    system_surface_response,
                                    &mut pending_client_response_capability,
                                    &mut next_request,
                                    surface.reference,
                                );
                            }
                            send_surface_command(
                                display_control,
                                &mut surface_commands,
                                GuiSurfaceOperation::Destroy,
                                surface.reference,
                                GuiRect::EMPTY,
                                &mut next_request,
                            );
                        }
                        render(display, atrium, calculator, &mut sequence);
                    }
                }
                logos_atrium::AtriumAction::FocusNext
                | logos_atrium::AtriumAction::FocusPrevious
                | logos_atrium::AtriumAction::MoveFocused(_, _) => {
                    if atrium.apply_action(action).is_ok() {
                        if let Some(surface) = atrium.focused_surface() {
                            if matches!(action, logos_atrium::AtriumAction::MoveFocused(_, _)) {
                                send_surface_command(
                                    display_control,
                                    &mut surface_commands,
                                    GuiSurfaceOperation::Update,
                                    surface.reference,
                                    surface.bounds,
                                    &mut next_request,
                                );
                            } else {
                                send_surface_command(
                                    display_control,
                                    &mut surface_commands,
                                    GuiSurfaceOperation::Focus,
                                    surface.reference,
                                    GuiRect::EMPTY,
                                    &mut next_request,
                                );
                            }
                        }
                        render(display, atrium, calculator, &mut sequence);
                    }
                }
                _ => {}
            }
            if action.routes_to_surface() {
                if let Some(surface) = atrium.focused_surface() {
                    if surface.app == logos_atrium::AppId::Terminal {
                        let routed = AtriumSurfaceInput::new(surface.reference, event);
                        if routed.is_valid() {
                            let _ = common::ipc_send_handle(terminal, &routed);
                        }
                    } else if surface.app == logos_atrium::AppId::System {
                        let routed = AtriumSurfaceInput::new(surface.reference, event);
                        if routed.is_valid() {
                            let _ = common::ipc_send_handle(system_surface_input, &routed);
                        }
                    } else if surface.app == logos_atrium::AppId::Calculator
                        && calculator.input(&event)
                    {
                        render(display, atrium, calculator, &mut sequence);
                    } else if let Some(caps) = program_surface_capabilities
                        .iter()
                        .flatten()
                        .copied()
                        .find(|caps| caps.client == surface.client)
                    {
                        let routed = AtriumSurfaceInput::new(surface.reference, event);
                        if routed.is_valid() {
                            let _ = common::ipc_send_handle(caps.input, &routed);
                        }
                    }
                }
            }
        }
        if let Some(op) = pending_cursor_draw {
            match common::ipc_send_handle(display, &op) {
                IpcStatus::Ok => pending_cursor_draw = None,
                IpcStatus::Full => {}
                _ => {
                    pending_cursor_draw = None;
                    cursor_surface = SurfaceHandle::EMPTY;
                }
            }
        }
        let mut wait_capabilities = [logos_abi::CapabilityHandle::EMPTY; 24];
        let mut wait_count = 0;
        for capability in [
            input,
            display,
            display_control,
            display_response,
            shell_context,
            terminal_surface_request,
            terminal_surface_response,
            system_surface_request,
            system_surface_draw,
            terminal_render,
            display_render,
        ] {
            wait_capabilities[wait_count] = capability;
            wait_count += 1;
        }
        if let Ok(requests) = common::discover_capabilities_contract(
            logos_abi::IpcRights::Receive,
            logos_abi::IPC_CONTRACT_ATRIUM_SURFACE_REQUEST,
            core::mem::size_of::<AtriumSurfaceRequest>(),
        ) {
            for (client, capability) in requests {
                if client == atrium_client || client == terminal_client {
                    continue;
                }
                if wait_count < wait_capabilities.len() {
                    wait_capabilities[wait_count] = capability;
                    wait_count += 1;
                }
            }
        }
        for caps in program_surface_capabilities.iter().flatten().copied() {
            if wait_count < wait_capabilities.len() {
                wait_capabilities[wait_count] = caps.render;
                wait_count += 1;
            }
            if wait_count < wait_capabilities.len() {
                wait_capabilities[wait_count] = caps.draw;
                wait_count += 1;
            }
        }
        common::wait_on_capabilities(&wait_capabilities[..wait_count]);
    }
}

#[cfg(target_os = "none")]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo<'_>) -> ! {
    common::idle()
}

#[cfg(not(target_os = "none"))]
fn main() {}
