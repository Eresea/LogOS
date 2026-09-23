#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", no_main)]
#![cfg_attr(not(target_os = "none"), allow(dead_code, unused_imports, unused_variables))]

mod common;

use core::mem;

use logos_abi::{
    AtriumApp, AtriumSurfaceInput, AtriumSurfaceRequest, AtriumSurfaceResponse, GuiRect,
    GuiSceneOp, IpcStatus, ManagerOperation, ManagerRequest, ManagerResponse, ManagerState,
    SurfaceHandle,
};
use logos_ui::{UiComponentTree, UiNodeHandle, UiNodeKind, UiRect, UiStyle, UiStyleList, UiText};
use logos_ui_graphics::{UiScenePublisher, UiSceneSink, UiSceneTheme};

const SYSTEM_THEME: UiSceneTheme = UiSceneTheme {
    surface: 0x101820,
    panel: 0x182535,
    input: 0x263548,
    border: 0x334155,
    accent: 0x9f3b3b,
    focus: 0x4b82f2,
    text: 0xffffff,
    muted: 0x7890aa,
};

static mut UI_TREE: UiComponentTree = UiComponentTree::new();
static mut UI_SCENE_PUBLISHER: UiScenePublisher = UiScenePublisher::new();

const SYSTEM_CONTENT_PADDING: i32 = 16;
const SYSTEM_GLYPH_INSET: i32 = 12;
const SYSTEM_ROW_COUNT: usize = 4;

#[derive(Clone, Copy)]
struct SystemLayout {
    surface: UiRect,
    status_bar: UiRect,
    title: UiRect,
    close: UiRect,
    status: UiRect,
    rows: [(UiRect, UiRect); SYSTEM_ROW_COUNT],
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
    mem::size_of::<GuiSceneOp>(),
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

fn append_u16(buffer: &mut [u8], len: &mut usize, mut value: u16) {
    let start = *len;
    loop {
        buffer[*len] = b'0' + (value % 10) as u8;
        *len += 1;
        value /= 10;
        if value == 0 {
            break;
        }
    }
    buffer[start..*len].reverse();
}

fn report_surface(surface: SurfaceHandle) {
    let mut message = [0u8; 64];
    let mut len = 0;
    let prefix = b"LogOS vNext: System surface=";
    message[..prefix.len()].copy_from_slice(prefix);
    len += prefix.len();
    append_u16(&mut message, &mut len, surface.slot);
    message[len] = b'/';
    len += 1;
    append_u16(&mut message, &mut len, surface.generation);
    proof_line(&message[..len]);
}

struct AtriumSceneSink(logos_abi::CapabilityHandle);

impl UiSceneSink for AtriumSceneSink {
    fn send(&mut self, operation: &GuiSceneOp) -> IpcStatus {
        common::ipc_send_handle(self.0, operation)
    }
}

fn insert_node(
    tree: &mut UiComponentTree,
    kind: UiNodeKind,
    bounds: UiRect,
    text: &[u8],
    style: Option<UiStyle>,
) -> bool {
    let parent = if tree.tree().is_empty() {
        UiNodeHandle::EMPTY
    } else {
        let Ok(root) = tree.tree().handle_at(0) else { return false };
        root
    };
    let Ok(handle) = tree.insert(kind, parent, tree.tree().len() as u16) else {
        return false;
    };
    if tree.tree_mut().set_bounds(handle, bounds).is_err() {
        return false;
    }
    if !text.is_empty() {
        let Some(text) = UiText::from_bytes(text) else { return false };
        if tree.set_text(handle, text).is_err() {
            return false;
        }
    }
    if let Some(style) = style {
        let mut styles = UiStyleList::EMPTY;
        if !styles.push(style) || tree.set_styles(handle, styles).is_err() {
            return false;
        }
    }
    true
}

fn system_layout(surface: GuiRect) -> SystemLayout {
    let text_left = surface.x.saturating_add(SYSTEM_CONTENT_PADDING - SYSTEM_GLYPH_INSET);
    let content_width = surface.width.saturating_sub((SYSTEM_CONTENT_PADDING * 2) as u32);
    let name_width = content_width / 2;
    let state_width = content_width.saturating_sub(name_width);
    let close_local = logos_atrium::surface_close_bounds(surface);
    let mut rows = [(UiRect::new(0, 0, 0, 0), UiRect::new(0, 0, 0, 0)); SYSTEM_ROW_COUNT];
    for (index, row) in rows.iter_mut().enumerate() {
        let y = surface.y.saturating_add(76 + index as i32 * 16);
        *row = (
            UiRect::new(text_left, y, name_width, 16),
            UiRect::new(text_left.saturating_add(name_width as i32), y, state_width, 16),
        );
    }

    SystemLayout {
        surface: UiRect::new(surface.x, surface.y, surface.width, surface.height),
        status_bar: UiRect::new(surface.x, surface.y, surface.width, surface.height.min(32)),
        title: UiRect::new(text_left, surface.y, name_width, 32),
        close: UiRect::new(
            surface.x.saturating_add(close_local.x),
            surface.y.saturating_add(close_local.y),
            close_local.width,
            close_local.height,
        ),
        status: UiRect::new(text_left, surface.y.saturating_add(48), content_width, 16),
        rows,
    }
}

fn build_status(tree: &mut UiComponentTree, surface: GuiRect) -> bool {
    *tree = UiComponentTree::new();
    let layout = system_layout(surface);
    if !insert_node(tree, UiNodeKind::Root, layout.surface, b"", None)
        || !insert_node(tree, UiNodeKind::Panel, layout.status_bar, b"", None)
        || !insert_node(tree, UiNodeKind::Label, layout.title, b"System", None)
        || !insert_node(
            tree,
            UiNodeKind::Button,
            layout.close,
            b"X",
            Some(UiStyle::BackgroundAccent),
        )
        || !insert_node(
            tree,
            UiNodeKind::Label,
            layout.status,
            b"Service manager status",
            Some(UiStyle::TextMuted),
        )
    {
        return false;
    }

    for (name, state) in layout.rows {
        if !insert_node(tree, UiNodeKind::Label, name, b"", None)
            || !insert_node(tree, UiNodeKind::Label, state, b"", None)
        {
            return false;
        }
    }
    true
}

fn set_label_text(tree: &mut UiComponentTree, index: usize, bytes: &[u8]) -> bool {
    let Ok(handle) = tree.tree().handle_at(index) else { return false };
    let Some(text) = UiText::from_bytes(bytes) else { return false };
    tree.set_text(handle, text).is_ok()
}

fn refresh_status(tree: &mut UiComponentTree, sequence: u32) -> bool {
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
        if !set_label_text(tree, 5 + row as usize * 2, &record.name[..name_len])
            || !set_label_text(tree, 6 + row as usize * 2, state_name(record.state))
        {
            return false;
        }
        row += 1;
        if response.cursor == u64::MAX || response.cursor <= cursor || row >= 4 {
            break;
        }
        cursor = response.cursor;
    }
    for empty_row in row..4 {
        if !set_label_text(tree, 5 + empty_row as usize * 2, b"")
            || !set_label_text(tree, 6 + empty_row as usize * 2, b"")
        {
            return false;
        }
    }
    true
}

fn publish_status(
    draw: logos_abi::CapabilityHandle,
    surface: SurfaceHandle,
    sequence: u32,
    tree: &UiComponentTree,
) -> IpcStatus {
    let mut sink = AtriumSceneSink(draw);
    match unsafe {
        (*core::ptr::addr_of_mut!(UI_SCENE_PUBLISHER)).publish(
            surface,
            sequence,
            tree,
            SYSTEM_THEME,
            None,
            &mut sink,
        )
    } {
        Ok((status, _)) => status,
        Err(_) => IpcStatus::Malformed,
    }
}

#[cfg(target_os = "none")]
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
    let mut scene_reported = false;
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
        if unsafe { (*core::ptr::addr_of!(UI_SCENE_PUBLISHER)).is_pending() } {
            let tree = unsafe { &*core::ptr::addr_of!(UI_TREE) };
            let status = publish_status(draw_cap, surface, sequence, tree);
            if status == IpcStatus::Full {
                common::heartbeat();
                continue;
            }
            if status == IpcStatus::Ok && !scene_reported {
                proof_line(b"LogOS vNext: System scene built");
                scene_reported = true;
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
            if response.is_update() {
                continue;
            }
            request_pending = false;
            if response.status == logos_abi::GuiStatus::Ok
                && response.surface.is_valid()
                && !response.bounds.is_empty()
            {
                surface = response.surface;
                sequence = sequence.wrapping_add(1).max(1);
                unsafe { (*core::ptr::addr_of_mut!(UI_SCENE_PUBLISHER)).reset() };
                let tree = unsafe { &mut *core::ptr::addr_of_mut!(UI_TREE) };
                scene_reported = false;
                if build_status(tree, response.bounds) && refresh_status(tree, sequence) {
                    let status = publish_status(draw_cap, surface, sequence, tree);
                    if status == IpcStatus::Ok && !scene_reported {
                        proof_line(b"LogOS vNext: System scene built");
                        scene_reported = true;
                    }
                }
                report_surface(surface);
            } else if response.is_revoke() || response.status == logos_abi::GuiStatus::NotFound {
                surface = SurfaceHandle::EMPTY;
                scene_reported = false;
                unsafe { (*core::ptr::addr_of_mut!(UI_SCENE_PUBLISHER)).reset() };
            }
        }
        while common::ipc_receive_handle(input_cap, &mut input) == IpcStatus::Ok {
            if input.surface == surface && input.is_valid() && surface.is_valid() {
                sequence = sequence.wrapping_add(1).max(1);
                let tree = unsafe { &mut *core::ptr::addr_of_mut!(UI_TREE) };
                if refresh_status(tree, sequence) {
                    let _ = publish_status(draw_cap, surface, sequence, tree);
                }
            }
        }

        if unsafe { (*core::ptr::addr_of!(UI_SCENE_PUBLISHER)).is_pending() } {
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

#[cfg(test)]
mod tests {
    use super::*;
    use logos_abi::{GuiDrawKind, GuiNodeOperation};

    #[test]
    fn layout_keeps_all_text_inside_atrium_surface_padding() {
        let bounds = logos_atrium::DESKTOP_SURFACE_BOUNDS;
        let layout = system_layout(bounds);
        assert_eq!(layout.surface.x, bounds.x);
        assert_eq!(layout.status_bar.x, bounds.x);
        let shared_close = logos_atrium::surface_close_bounds(bounds);
        assert_eq!(layout.close.x, bounds.x + shared_close.x);
        assert_eq!(layout.close.y, bounds.y + shared_close.y);
        assert_eq!(layout.close.width, shared_close.width);
        assert_eq!(layout.close.height, shared_close.height);

        let mut tree = UiComponentTree::new();
        assert!(build_status(&mut tree, bounds));
        for (row, name) in
            [b"Input".as_slice(), b"Display", b"Terminal", b"Session"].into_iter().enumerate()
        {
            assert!(set_label_text(&mut tree, 5 + row * 2, name));
            assert!(set_label_text(&mut tree, 6 + row * 2, b"running"));
        }

        let surface = SurfaceHandle::new(1, 1, 13).unwrap();
        let scene = logos_ui_graphics::emit(surface, 1, &tree, SYSTEM_THEME).unwrap();
        let minimum_text_x = bounds.x + SYSTEM_CONTENT_PADDING;
        let mut text_count = 0;
        for operation in scene.as_slice() {
            if operation.operation == GuiNodeOperation::Upsert
                && operation.command.kind == GuiDrawKind::GlyphRun
            {
                assert!(operation.command.x >= minimum_text_x);
                text_count += 1;
            }
        }
        assert_eq!(text_count, 3 + SYSTEM_ROW_COUNT * 2);
    }
}
