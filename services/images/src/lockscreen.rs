#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", no_main)]
#![cfg_attr(not(target_os = "none"), allow(dead_code, unused_imports, unused_variables))]

mod common;

mod login_ui {
    include!(concat!(env!("OUT_DIR"), "/login_ui.rs"));
}

mod register_ui {
    include!(concat!(env!("OUT_DIR"), "/register_ui.rs"));
}

use logos_abi::{
    GuiDrawCommand, GuiHook, GuiHookKind, GuiRect, GuiSceneOp, GuiSurfaceOperation,
    GuiSurfaceRequest, GuiSurfaceResponse, InputMessage, IpcStatus, KeyCode, KeyState,
    SurfaceHandle, UserOperation, UserRequest, UserResponse, UserStatus,
};
use logos_ui::{UiComponentTree, UiExpression, UiStyleConditions, UiText};

const INPUT_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_GUI_INPUT,
    b"atrium",
    core::mem::size_of::<InputMessage>(),
    logos_abi::IpcRights::Receive,
);
const DISPLAY_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_GUI_DRAW,
    b"display",
    core::mem::size_of::<logos_abi::GuiSceneOp>(),
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
const SECTION_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_GUI_HOOK,
    b"atrium",
    core::mem::size_of::<GuiHook>(),
    logos_abi::IpcRights::Receive,
);
const SHELL_REQUEST_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_GUI_USER_REQUEST,
    b"shell",
    core::mem::size_of::<UserRequest>(),
    logos_abi::IpcRights::Send,
);
const SHELL_RESPONSE_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_GUI_USER_RESPONSE,
    b"shell",
    core::mem::size_of::<UserResponse>(),
    logos_abi::IpcRights::Receive,
);
const CURSOR_BOUNDS: GuiRect = GuiRect::new(0, 0, 640, 400);
static mut LOCKSCREEN: logos_lockscreen::LockScreen = logos_lockscreen::LockScreen::new();
static mut LOGIN_UI_BUILD: logos_ui_compiler::UiBuild =
    logos_ui_compiler::UiBuild::from_document(logos_ui::UiDocument::EMPTY);
static mut REGISTER_UI_BUILD: logos_ui_compiler::UiBuild =
    logos_ui_compiler::UiBuild::from_document(logos_ui::UiDocument::EMPTY);
static mut LOGIN_UI_READY: bool = false;
static mut REGISTER_UI_READY: bool = false;
static mut UI_TREE: UiComponentTree = UiComponentTree::new();

fn initialize_ui_build(claim: bool) {
    unsafe {
        if claim {
            if !REGISTER_UI_READY {
                REGISTER_UI_BUILD = register_ui::build();
                REGISTER_UI_READY = true;
            }
        } else if !LOGIN_UI_READY {
            LOGIN_UI_BUILD = login_ui::build();
            LOGIN_UI_READY = true;
        }
    }
}

fn ui_build(claim: bool) -> &'static logos_ui_compiler::UiBuild {
    initialize_ui_build(claim);
    unsafe {
        if claim {
            &*core::ptr::addr_of!(REGISTER_UI_BUILD)
        } else {
            &*core::ptr::addr_of!(LOGIN_UI_BUILD)
        }
    }
}

#[cfg(feature = "qemu-proof")]
fn proof_line(message: &[u8]) {
    common::proof_line(message);
}

#[cfg(not(feature = "qemu-proof"))]
fn proof_line(_message: &[u8]) {}

fn draw_ui(
    display: logos_abi::CapabilityHandle,
    surface: SurfaceHandle,
    lock: &logos_lockscreen::LockScreen,
    sequence: u32,
    _include_static: bool,
) -> IpcStatus {
    let build = ui_build(lock.mode() == logos_lockscreen::LockScreenMode::Claim);
    let tree = unsafe { &mut *core::ptr::addr_of_mut!(UI_TREE) };
    if tree.reset_from_document(&build.document).is_err() {
        return IpcStatus::Malformed;
    }
    let Some(layout) = logos_shell::LoginLayout::from_build(build, CURSOR_BOUNDS) else {
        return IpcStatus::Malformed;
    };

    for index in 0..build.document.node_count() {
        let Ok(handle) = tree.tree().handle_at(index) else { return IpcStatus::Malformed };
        let Some(layout_node) = layout.node(index as u16) else { return IpcStatus::Malformed };
        let bounds = if index == 0 {
            logos_ui::UiRect::new(
                CURSOR_BOUNDS.x,
                CURSOR_BOUNDS.y,
                CURSOR_BOUNDS.width,
                CURSOR_BOUNDS.height,
            )
        } else {
            logos_ui::UiRect::new(
                layout_node.bounds.x,
                layout_node.bounds.y,
                layout_node.bounds.width,
                layout_node.bounds.height,
            )
        };
        if tree.tree_mut().set_bounds(handle, bounds).is_err() {
            return IpcStatus::Malformed;
        }
        if tree
            .tree_mut()
            .set_focused(
                handle,
                field_for_node(build, index as u16).is_some_and(|field| field == lock.field()),
            )
            .is_err()
        {
            return IpcStatus::Malformed;
        }
    }

    let mut conditions = UiStyleConditions::EMPTY;
    if let Some(failure) = UiExpression::from_bytes(b"failure") {
        let _ = conditions.set(failure, lock.failure());
    }
    for index in 0..build.document.node_count() {
        if tree.apply_document_styles(&build.document, index as u16, &conditions).is_err() {
            return IpcStatus::Malformed;
        }
    }

    let set_named_text = |tree: &mut UiComponentTree, name: &[u8], text: &[u8]| {
        let Some(index) = build.document.node_index_by_name(name) else { return false };
        let Ok(handle) = tree.tree().handle_at(usize::from(index)) else { return false };
        let Some(text) = UiText::from_bytes(text) else { return false };
        tree.set_text(handle, text).is_ok()
    };
    if lock.failure()
        && !set_named_text(
            tree,
            b"title",
            if lock.mode() == logos_lockscreen::LockScreenMode::Claim {
                b"Retry setup"
            } else {
                b"Retry login"
            },
        )
    {
        return IpcStatus::Malformed;
    }

    let (username, password) = lock.credentials();
    if !set_named_value(tree, build, b"username", username, false)
        || !set_named_value(tree, build, b"password", password, true)
    {
        return IpcStatus::Malformed;
    }
    if lock.mode() == logos_lockscreen::LockScreenMode::Claim
        && !set_named_value(tree, build, b"confirmPassword", lock.confirmation(), true)
    {
        return IpcStatus::Malformed;
    }
    let Some(submit_index) = build.document.node_index_by_name(b"submit") else {
        return IpcStatus::Malformed;
    };
    let Ok(submit) = tree.tree().handle_at(usize::from(submit_index)) else {
        return IpcStatus::Malformed;
    };
    if tree.set_disabled(submit, !lock.form().can_submit()).is_err() {
        return IpcStatus::Malformed;
    }

    let scene = match logos_ui_graphics::emit(
        surface,
        sequence,
        tree,
        logos_ui_graphics::UiSceneTheme::DEFAULT,
    ) {
        Ok(scene) => scene,
        Err(_) => return IpcStatus::Malformed,
    };
    for operation in scene.as_slice() {
        let status = common::ipc_send_handle(display, operation);
        if status != IpcStatus::Ok {
            return status;
        }
    }
    IpcStatus::Ok
}

fn field_for_node(
    build: &logos_ui_compiler::UiBuild,
    index: u16,
) -> Option<logos_lockscreen::LockScreenField> {
    let node = build.document.node(usize::from(index))?;
    match node.key.as_bytes() {
        b"username" => Some(logos_lockscreen::LockScreenField::Username),
        b"password" => Some(logos_lockscreen::LockScreenField::Password),
        b"confirmPassword" => Some(logos_lockscreen::LockScreenField::ConfirmPassword),
        b"submit" => Some(logos_lockscreen::LockScreenField::Submit),
        _ => None,
    }
}

fn set_named_value(
    tree: &mut UiComponentTree,
    build: &logos_ui_compiler::UiBuild,
    name: &[u8],
    value: &[u8],
    masked: bool,
) -> bool {
    let Some(index) = build.document.node_index_by_name(name) else { return false };
    let Ok(handle) = tree.tree().handle_at(usize::from(index)) else { return false };
    let mut bytes = [0u8; logos_ui::MAX_UI_TEXT_BYTES];
    let length = value.len().min(bytes.len());
    if masked {
        bytes[..length].fill(b'*');
    } else {
        bytes[..length].copy_from_slice(&value[..length]);
    }
    let Some(text) = UiText::from_bytes(&bytes[..length]) else { return false };
    tree.set_value(handle, text).is_ok()
}

fn next_id(next: &mut u32) -> u32 {
    let value = *next;
    *next = next.wrapping_add(1).max(1);
    value
}

fn request_surface(_display: logos_abi::CapabilityHandle, next: &mut u32) -> GuiSurfaceRequest {
    let mut request = GuiSurfaceRequest::new(GuiSurfaceOperation::CreateModal, next_id(next));
    request.bounds = GuiRect::new(0, 0, 640, 400);
    request.z_order = 1;
    request
}

fn request_cursor_surface(
    display: logos_abi::CapabilityHandle,
    next: &mut u32,
) -> Option<GuiSurfaceRequest> {
    let mut request = GuiSurfaceRequest::new(GuiSurfaceOperation::CreateModal, next_id(next));
    request.bounds = CURSOR_BOUNDS;
    request.z_order = 2;
    (common::ipc_send_handle(display, &request) == IpcStatus::Ok).then_some(request)
}

fn cursor_op(surface: SurfaceHandle, x: i16, y: i16, sequence: u32) -> GuiSceneOp {
    GuiSceneOp::upsert(
        surface,
        sequence,
        1,
        GuiDrawCommand::fill_rect(GuiRect::new(i32::from(x), i32::from(y), 3, 14), 0xffffff),
    )
}

fn destroy_surface(display: logos_abi::CapabilityHandle, surface: SurfaceHandle, next: &mut u32) {
    let mut request = GuiSurfaceRequest::new(GuiSurfaceOperation::Destroy, next_id(next));
    request.surface = surface;
    let _ = common::ipc_send_handle(display, &request);
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    common::init_service_allocator();
    initialize_ui_build(false);
    let input = common::capability_handle(INPUT_CAPABILITY).unwrap_or_else(|_| common::idle());
    let display = common::capability_handle(DISPLAY_CAPABILITY).unwrap_or_else(|_| common::idle());
    let display_control =
        common::capability_handle(DISPLAY_CONTROL_CAPABILITY).unwrap_or_else(|_| common::idle());
    let display_response =
        common::capability_handle(DISPLAY_RESPONSE_CAPABILITY).unwrap_or_else(|_| common::idle());
    let section = common::capability_handle(SECTION_CAPABILITY).unwrap_or_else(|_| common::idle());
    let shell_request =
        common::capability_handle(SHELL_REQUEST_CAPABILITY).unwrap_or_else(|_| common::idle());
    let shell_response =
        common::capability_handle(SHELL_RESPONSE_CAPABILITY).unwrap_or_else(|_| common::idle());
    let lock = unsafe { &mut *core::ptr::addr_of_mut!(LOCKSCREEN) };
    let mut next_request = 1u32;
    let mut sequence = 0u32;
    let mut surface = SurfaceHandle::EMPTY;
    let mut visible = true;
    let mut static_cached = false;
    let mut pending_surface = Some(request_surface(display_control, &mut next_request));
    let mut pending_surface_sent = false;
    let mut cursor_surface = SurfaceHandle::EMPTY;
    let mut pending_cursor_surface = None;
    let mut cursor = (320i16, 200i16);
    let mut cursor_sequence = 1u32;
    let mut pending_cursor_draw: Option<GuiSceneOp> = None;
    let mut pending_draw: Option<bool> = None;
    let mut pending_draw_sequence = 0u32;
    let mut pending_auth: Option<UserRequest> = None;
    let mut response =
        UserResponse::new(UserRequest::new(UserOperation::Login, 1), UserStatus::Stale);
    let mut surface_response = GuiSurfaceResponse::new(
        GuiSurfaceRequest::new(GuiSurfaceOperation::CreateModal, 1),
        logos_abi::GuiStatus::Malformed,
    );
    let mut hook = GuiHook::new(GuiHookKind::Section, 1);
    let mut event = InputMessage::key(KeyCode::Unknown, KeyState::Released, 0);
    let mut heartbeat_ticks = 0u16;

    loop {
        common::heartbeat_tick(&mut heartbeat_ticks);
        if let Some(cursor) = pending_cursor_draw {
            match common::ipc_send_handle(display, &cursor) {
                IpcStatus::Ok => pending_cursor_draw = None,
                IpcStatus::Full => {}
                _ => {
                    pending_cursor_draw = None;
                    cursor_surface = SurfaceHandle::EMPTY;
                }
            }
        }
        if let Some(include_static) = pending_draw {
            if pending_draw_sequence == 0 {
                sequence = sequence.wrapping_add(1).max(1);
                pending_draw_sequence = sequence;
            }
            match draw_ui(display, surface, lock, pending_draw_sequence, include_static) {
                IpcStatus::Ok => {
                    pending_draw = None;
                    pending_draw_sequence = 0;
                    static_cached = true;
                }
                IpcStatus::Full => {}
                _ => {
                    pending_draw = None;
                    pending_draw_sequence = 0;
                    static_cached = false;
                }
            }
        }
        if let Some(request) = pending_surface {
            if !pending_surface_sent {
                match common::ipc_send_handle(display_control, &request) {
                    IpcStatus::Ok => pending_surface_sent = true,
                    IpcStatus::Full => {}
                    _ => pending_surface = None,
                }
            }
        }
        if visible
            && surface.is_valid()
            && static_cached
            && !cursor_surface.is_valid()
            && pending_cursor_surface.is_none()
        {
            pending_cursor_surface = request_cursor_surface(display_control, &mut next_request);
        }
        while common::ipc_receive_handle(section, &mut hook) == IpcStatus::Ok {
            let should_show = hook.deadline != 0;
            if should_show && !visible {
                visible = true;
                if !surface.is_valid() && pending_surface.is_none() {
                    pending_surface = Some(request_surface(display_control, &mut next_request));
                    pending_surface_sent = false;
                }
            } else if !should_show && visible {
                visible = false;
                pending_surface = None;
                pending_auth = None;
                static_cached = false;
                pending_cursor_surface = None;
                pending_cursor_draw = None;
                if cursor_surface.is_valid() {
                    destroy_surface(display_control, cursor_surface, &mut next_request);
                    cursor_surface = SurfaceHandle::EMPTY;
                }
                if surface.is_valid() {
                    destroy_surface(display_control, surface, &mut next_request);
                    surface = SurfaceHandle::EMPTY;
                }
            }
        }
        while common::ipc_receive_handle(display_response, &mut surface_response) == IpcStatus::Ok {
            if pending_cursor_surface.is_some_and(|request| surface_response.is_valid_for(request))
            {
                pending_cursor_surface = None;
                if surface_response.status == logos_abi::GuiStatus::Ok
                    && surface_response.surface.is_valid()
                {
                    cursor_surface = surface_response.surface;
                    pending_cursor_draw =
                        Some(cursor_op(cursor_surface, cursor.0, cursor.1, cursor_sequence));
                }
                continue;
            }
            let Some(request) = pending_surface.filter(|_| pending_surface_sent) else { continue };
            if !surface_response.is_valid_for(request) {
                continue;
            }
            pending_surface = None;
            pending_surface_sent = false;
            if surface_response.status == logos_abi::GuiStatus::Ok {
                surface = surface_response.surface;
                proof_line(b"LogOS vNext: LockScreen surface ready");
                pending_draw = Some(true);
            }
        }
        if let Some(request) = pending_auth {
            match common::ipc_send_handle(shell_request, &request) {
                IpcStatus::Ok => pending_auth = None,
                IpcStatus::Full => {}
                _ => pending_auth = None,
            }
        }
        while common::ipc_receive_handle(shell_response, &mut response) == IpcStatus::Ok {
            let request = UserRequest::new(response.operation, response.request_id);
            if response.is_valid_for(request) {
                let old_mode = lock.mode();
                lock.apply_status(response.status);
                if response.status == UserStatus::Unclaimed {
                    proof_line(b"LogOS vNext: LockScreen claim mode ready");
                } else if response.status == UserStatus::Ok
                    && response.operation == UserOperation::Claim
                {
                    proof_line(b"LogOS vNext: LockScreen admin claim PASS");
                } else if response.status == UserStatus::Ok
                    && response.operation == UserOperation::Login
                {
                    proof_line(b"LogOS vNext: LockScreen login PASS");
                }
                if surface.is_valid() {
                    pending_draw = Some(!static_cached || old_mode != lock.mode());
                }
            }
        }
        if visible && pending_auth.is_none() {
            let mut cursor_sent_in_input = false;
            while common::ipc_receive_handle(input, &mut event) == IpcStatus::Ok {
                if let Some(pointer) = event.pointer_event() {
                    cursor = (pointer.x.clamp(0, 639), pointer.y.clamp(0, 399));
                    cursor_sequence = cursor_sequence.wrapping_add(1).max(1);
                    if cursor_surface.is_valid() {
                        let cursor = cursor_op(cursor_surface, cursor.0, cursor.1, cursor_sequence);
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
                let action = if event.pointer_event().is_some() {
                    lock.pointer_input(event)
                } else {
                    lock.input(event)
                };
                proof_line(b"LogOS vNext: LockScreen received input");
                if let logos_lockscreen::LockScreenAction::Submit(operation) = action {
                    let (name, password) = lock.credentials();
                    let mut request = UserRequest::new(operation, next_id(&mut next_request));
                    if request.set_name(name) && request.set_password(password) {
                        pending_auth = Some(request);
                        proof_line(b"LogOS vNext: LockScreen auth submitted");
                        lock.clear_password();
                    } else {
                        lock.cancel_submission();
                    }
                }
                if action != logos_lockscreen::LockScreenAction::Ignored && surface.is_valid() {
                    pending_draw = Some(!static_cached);
                }
            }
        }
        if let Some(cursor) = pending_cursor_draw {
            match common::ipc_send_handle(display, &cursor) {
                IpcStatus::Ok => pending_cursor_draw = None,
                IpcStatus::Full => {}
                _ => {
                    pending_cursor_draw = None;
                    cursor_surface = SurfaceHandle::EMPTY;
                }
            }
        }
        common::wait_on_capabilities(&[
            input,
            display,
            display_control,
            display_response,
            section,
            shell_request,
            shell_response,
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
