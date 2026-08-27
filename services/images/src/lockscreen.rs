#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", no_main)]
#![cfg_attr(not(target_os = "none"), allow(dead_code, unused_imports, unused_variables))]

mod common;

use logos_abi::{
    GuiDrawBatch, GuiDrawCommand, GuiHook, GuiHookKind, GuiRect, GuiSurfaceOperation,
    GuiSurfaceRequest, GuiSurfaceResponse, InputMessage, IpcStatus, KeyCode, KeyState,
    SurfaceHandle, UserOperation, UserRequest, UserResponse, UserStatus,
};

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

static mut LOCKSCREEN: logos_lockscreen::LockScreen = logos_lockscreen::LockScreen::new();

fn push_text(batch: &mut GuiDrawBatch, x: i32, y: i32, color: u32, text: &[u8]) {
    if let Some(command) = GuiDrawCommand::glyph_run(x, y, color, text) {
        let _ = batch.push(command);
    }
}

fn draw(
    display: logos_abi::CapabilityHandle,
    surface: SurfaceHandle,
    lock: &logos_lockscreen::LockScreen,
    sequence: u32,
    include_static: bool,
) {
    let panel = GuiRect::new(150, 48, 340, 304);
    if include_static {
        let mut background = GuiDrawBatch::new(surface, sequence, GuiRect::SURFACE);
        background.flags = logos_abi::GUI_DRAW_FLAG_MORE;
        let _ = background.push(GuiDrawCommand::fill_surface(0x101820));
        let _ = background.push(GuiDrawCommand::shadow(panel, 0x55000000, 16, 4, 0, 6));
        let _ = background.push(GuiDrawCommand::fill_rounded_rect(panel, 0x182535, 16));
        let _ = common::ipc_send_scene_batch(display, &background, 1);

        let mut labels = GuiDrawBatch::new(surface, sequence, panel);
        labels.flags = logos_abi::GUI_DRAW_FLAG_MORE;
        let _ = labels.push(GuiDrawCommand::stroke_rounded_rect(panel, 0x4b89dc, 16, 2));
        push_text(
            &mut labels,
            176,
            84,
            0xffffff,
            if lock.mode() == logos_lockscreen::LockScreenMode::Claim {
                b"Create admin"
            } else {
                b"Unlock LogOS"
            },
        );
        push_text(&mut labels, 176, 116, 0xb8c7da, b"Username");
        let _ = common::ipc_send_scene_batch(display, &labels, 4);
    }

    let (username, password) = lock.credentials();
    let confirmation = lock.confirmation();
    let mut masked = [0u8; logos_abi::MAX_GUI_TEXT_BYTES];
    let password_len = password.len().min(masked.len());
    masked[..password_len].fill(b'*');
    let mut masked_confirmation = [0u8; logos_abi::MAX_GUI_TEXT_BYTES];
    let confirmation_len = confirmation.len().min(masked_confirmation.len());
    masked_confirmation[..confirmation_len].fill(b'*');
    let mut fields = GuiDrawBatch::new(surface, sequence, GuiRect::new(170, 100, 300, 220));
    fields.flags = logos_abi::GUI_DRAW_FLAG_MORE;
    let _ = fields.push(GuiDrawCommand::fill_rounded_rect(
        GuiRect::new(170, 132, 300, 36),
        0x263548,
        8,
    ));
    push_text(&mut fields, 184, 156, 0xffffff, username);
    push_text(&mut fields, 176, 176, 0xb8c7da, b"Password");
    let _ = common::ipc_send_scene_batch(display, &fields, 7);

    let mut password_field = GuiDrawBatch::new(surface, sequence, GuiRect::new(170, 192, 300, 120));
    password_field.flags = logos_abi::GUI_DRAW_FLAG_MORE;
    let _ = password_field.push(GuiDrawCommand::fill_rounded_rect(
        GuiRect::new(170, 192, 300, 36),
        0x263548,
        8,
    ));
    push_text(&mut password_field, 184, 216, 0xffffff, &masked[..password_len]);
    if lock.mode() == logos_lockscreen::LockScreenMode::Claim {
        let _ = password_field.push(GuiDrawCommand::fill_rounded_rect(
            logos_lockscreen::CONFIRM_PASSWORD_BOUNDS,
            0x263548,
            8,
        ));
    } else {
        push_text(&mut password_field, 184, 276, 0x7890aa, b"Tab fields  Enter submit");
    }
    let _ = common::ipc_send_scene_batch(display, &password_field, 10);

    let mut action = GuiDrawBatch::new(surface, sequence, GuiRect::new(220, 240, 200, 104));
    let submit_bounds = if lock.mode() == logos_lockscreen::LockScreenMode::Claim {
        logos_lockscreen::CLAIM_SUBMIT_BOUNDS
    } else {
        logos_lockscreen::SUBMIT_BOUNDS
    };
    if lock.mode() == logos_lockscreen::LockScreenMode::Claim {
        push_text(
            &mut action,
            184,
            276,
            0xffffff,
            if confirmation_len == 0 {
                b"Confirm password"
            } else {
                &masked_confirmation[..confirmation_len]
            },
        );
    }
    let _ = action.push(GuiDrawCommand::fill_rounded_rect(submit_bounds, 0x356bd8, 8));
    push_text(
        &mut action,
        submit_bounds.x.saturating_add(24),
        submit_bounds.y.saturating_add(24),
        0xffffff,
        if lock.mode() == logos_lockscreen::LockScreenMode::Claim { b"Create" } else { b"Unlock" },
    );
    let _ = common::ipc_send_scene_batch(display, &action, 13);
}

fn next_id(next: &mut u32) -> u32 {
    let value = *next;
    *next = next.wrapping_add(1).max(1);
    value
}

fn request_surface(display: logos_abi::CapabilityHandle, next: &mut u32) -> GuiSurfaceRequest {
    let mut request = GuiSurfaceRequest::new(GuiSurfaceOperation::CreateModal, next_id(next));
    request.bounds = GuiRect::new(0, 0, 640, 400);
    request.z_order = 1;
    let _ = common::ipc_send_handle(display, &request);
    request
}

fn destroy_surface(display: logos_abi::CapabilityHandle, surface: SurfaceHandle, next: &mut u32) {
    let mut request = GuiSurfaceRequest::new(GuiSurfaceOperation::Destroy, next_id(next));
    request.surface = surface;
    let _ = common::ipc_send_handle(display, &request);
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    common::init_service_allocator();
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
        while common::ipc_receive_handle(section, &mut hook) == IpcStatus::Ok {
            let should_show = hook.deadline != 0;
            if should_show && !visible {
                visible = true;
                if !surface.is_valid() && pending_surface.is_none() {
                    pending_surface = Some(request_surface(display_control, &mut next_request));
                }
            } else if !should_show && visible {
                visible = false;
                pending_surface = None;
                pending_auth = None;
                static_cached = false;
                if surface.is_valid() {
                    destroy_surface(display_control, surface, &mut next_request);
                    surface = SurfaceHandle::EMPTY;
                }
            }
        }
        while common::ipc_receive_handle(display_response, &mut surface_response) == IpcStatus::Ok {
            let Some(request) = pending_surface else { continue };
            if !surface_response.is_valid_for(request) {
                continue;
            }
            pending_surface = None;
            if surface_response.status == logos_abi::GuiStatus::Ok {
                surface = surface_response.surface;
                sequence = sequence.wrapping_add(1).max(1);
                draw(display, surface, lock, sequence, true);
                static_cached = true;
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
                lock.apply_status(response.status);
                if surface.is_valid() {
                    sequence = sequence.wrapping_add(1).max(1);
                    draw(display, surface, lock, sequence, false);
                }
            }
        }
        if visible && pending_auth.is_none() {
            while common::ipc_receive_handle(input, &mut event) == IpcStatus::Ok {
                let action = if event.pointer_event().is_some() {
                    lock.pointer_input(event)
                } else {
                    lock.input(event)
                };
                if let logos_lockscreen::LockScreenAction::Submit(operation) = action {
                    let (name, password) = lock.credentials();
                    let mut request = UserRequest::new(operation, next_id(&mut next_request));
                    if request.set_name(name) && request.set_password(password) {
                        pending_auth = Some(request);
                        lock.clear_password();
                    } else {
                        lock.cancel_submission();
                    }
                }
                if action != logos_lockscreen::LockScreenAction::Ignored && surface.is_valid() {
                    sequence = sequence.wrapping_add(1).max(1);
                    draw(display, surface, lock, sequence, !static_cached);
                    static_cached = true;
                }
            }
        }
        common::wait_on_capabilities(&[input, display_response, section, shell_response]);
    }
}

#[cfg(target_os = "none")]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo<'_>) -> ! {
    common::idle()
}

#[cfg(not(target_os = "none"))]
fn main() {}
