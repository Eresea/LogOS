#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", no_main)]
#![cfg_attr(not(target_os = "none"), allow(dead_code, unused_imports, unused_variables))]

mod common;

use core::{mem, ptr};

use logos_abi::{
    GUI_DRAW_FLAG_MORE, GuiDrawBatch, GuiDrawCommand, GuiRect, GuiSessionContext, InputMessage,
    IpcBytes, IpcStatus, MAX_GUI_BATCH_FRAGMENTS, MAX_GUI_TEXT_BYTES, MessageKind, SurfaceHandle,
    UserRequest, UserResponse,
};

const INPUT_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_GUI_INPUT,
    b"input",
    core::mem::size_of::<InputMessage>(),
    logos_abi::IpcRights::Receive,
);
const TERMINAL_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_GUI_INPUT,
    b"terminal",
    core::mem::size_of::<InputMessage>(),
    logos_abi::IpcRights::Send,
);
const DISPLAY_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_GUI_DRAW,
    b"display",
    core::mem::size_of::<GuiDrawBatch>(),
    logos_abi::IpcRights::Send,
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

static mut SHELL: logos_shell::Shell = logos_shell::Shell::new();
static mut LOCKSCREEN: logos_lockscreen::LockScreen = logos_lockscreen::LockScreen::new();

fn send_shell_surface(
    display: logos_abi::CapabilityHandle,
    login_page: &logos_ui_compiler::UiBuild,
    layout: &logos_shell::LoginLayout,
    lockscreen: &logos_lockscreen::LockScreen,
    sequence: &mut u32,
    visible: bool,
) {
    *sequence = sequence.wrapping_add(1).max(1);
    let surface = SurfaceHandle::new(0, 1, 11).unwrap();
    if !visible {
        let batch = GuiDrawBatch::new(surface, *sequence, GuiRect::SURFACE);
        let _ = common::ipc_send_handle(display, &batch);
        return;
    }
    let state = logos_shell::LoginUiState::new(
        lockscreen.mode() == logos_lockscreen::LockScreenMode::Claim,
        lockscreen.failure(),
    );
    let Some(panel) = layout.node(0).map(|node| node.bounds) else { return };
    let Some(title) = layout.node(2).map(|node| node.bounds) else { return };
    let Some(username) = layout.bounds_for(logos_shell::LoginHitTarget::Username) else {
        return;
    };
    let Some(password) = layout.bounds_for(logos_shell::LoginHitTarget::Password) else { return };
    let Some(submit) = layout.bounds_for(logos_shell::LoginHitTarget::Submit) else { return };

    let mut batches =
        [GuiDrawBatch::new(surface, *sequence, GuiRect::SURFACE); MAX_GUI_BATCH_FRAGMENTS];
    for batch in &mut batches[..MAX_GUI_BATCH_FRAGMENTS - 1] {
        batch.flags = GUI_DRAW_FLAG_MORE;
    }
    let _ = batches[0].push(GuiDrawCommand::fill_surface(0x101820));
    let _ = batches[0].push(GuiDrawCommand::fill_rect(panel, 0x182535));
    let _ = batches[0].push(GuiDrawCommand::stroke_rect(panel, 0x4b89dc, 2));

    let mut text = [0; MAX_GUI_TEXT_BYTES];
    let length = logos_shell::login_page_node_text(login_page, 2, state, &mut text);
    if let Some(text) = GuiDrawCommand::glyph_run(title.x, title.y, 0xffffff, &text[..length]) {
        let _ = batches[1].push(text);
    }
    let active = 0x315e91;
    let idle = 0x263548;
    let username_color = if logos_shell::login_style_active(
        login_page,
        3,
        state,
        lockscreen.field() == logos_lockscreen::LockScreenField::Username,
        logos_ui_compiler::UiStyle::BackgroundAccent,
    ) {
        active
    } else {
        idle
    };
    let password_color = if logos_shell::login_style_active(
        login_page,
        4,
        state,
        lockscreen.field() == logos_lockscreen::LockScreenField::Password,
        logos_ui_compiler::UiStyle::BackgroundAccent,
    ) {
        active
    } else {
        idle
    };
    let _ = batches[1].push(GuiDrawCommand::fill_rect(username, username_color));
    let _ = batches[1].push(GuiDrawCommand::stroke_rect(username, 0x6d8fc4, 1));

    let length = logos_shell::login_page_node_text(login_page, 3, state, &mut text);
    if let Some(text) = GuiDrawCommand::glyph_run(
        username.x.saturating_add(12),
        username.y.saturating_add(12),
        0xd9e5f5,
        &text[..length],
    ) {
        let _ = batches[2].push(text);
    }
    let _ = batches[2].push(GuiDrawCommand::fill_rect(password, password_color));
    let _ = batches[2].push(GuiDrawCommand::stroke_rect(password, 0x6d8fc4, 1));

    let length = logos_shell::login_page_node_text(login_page, 4, state, &mut text);
    if let Some(text) = GuiDrawCommand::glyph_run(
        password.x.saturating_add(12),
        password.y.saturating_add(12),
        0xd9e5f5,
        &text[..length],
    ) {
        let _ = batches[3].push(text);
    }
    let submit_color = if logos_shell::login_style_active(
        login_page,
        5,
        state,
        false,
        logos_ui_compiler::UiStyle::Opacity50,
    ) {
        0x1b356b
    } else {
        0x356bd8
    };
    let _ = batches[3].push(GuiDrawCommand::fill_rect(submit, submit_color));
    let length = logos_shell::login_page_node_text(login_page, 5, state, &mut text);
    if let Some(text) = GuiDrawCommand::glyph_run(
        submit.x.saturating_add(12),
        submit.y.saturating_add(12),
        0xffffff,
        &text[..length],
    ) {
        let _ = batches[3].push(text);
    }
    for batch in &batches {
        let _ = common::ipc_send_handle(display, batch);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    common::init_service_allocator();
    let input = match common::capability_handle(INPUT_CAPABILITY) {
        Ok(value) => value,
        Err(_) => common::idle(),
    };
    let terminal = match common::capability_handle(TERMINAL_CAPABILITY) {
        Ok(value) => value,
        Err(_) => common::idle(),
    };
    let display = match common::capability_handle(DISPLAY_CAPABILITY) {
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
    let shell = unsafe { &mut *core::ptr::addr_of_mut!(SHELL) };
    let login_page = logos_shell::compile_login_page();
    if !login_page.is_valid() {
        common::idle();
    }
    let Some(login_layout) =
        logos_shell::LoginLayout::from_build(&login_page, GuiRect::new(0, 0, 640, 400))
    else {
        common::idle();
    };
    let lockscreen = unsafe { &*core::ptr::addr_of!(LOCKSCREEN) };
    let mut surface_sequence = 0u32;
    send_shell_surface(
        display,
        &login_page,
        &login_layout,
        lockscreen,
        &mut surface_sequence,
        true,
    );
    let context = GuiSessionContext::EMPTY;
    let _ = common::ipc_send_handle(flow, &context);
    let mut pending_user: Option<IpcBytes> = None;
    let mut response = IpcBytes::empty(MessageKind::UserResponse);
    let mut message =
        InputMessage::key(logos_abi::KeyCode::Unknown, logos_abi::KeyState::Released, 0);
    let mut heartbeat_ticks = 0u16;
    loop {
        common::heartbeat_tick(&mut heartbeat_ticks);
        while common::ipc_receive_handle(input, &mut message) == IpcStatus::Ok {
            if shell.focus() == logos_shell::ShellFocus::Terminal {
                let _ = common::ipc_send_handle(terminal, &message);
                continue;
            }
            let lockscreen = unsafe { &mut *core::ptr::addr_of_mut!(LOCKSCREEN) };
            let action = lockscreen.input(message);
            if let logos_lockscreen::LockScreenAction::Submit(operation) = action {
                let (name, password) = lockscreen.credentials();
                if let Ok(mut request) = shell.begin_user_request(operation, name, password) {
                    let bytes = unsafe {
                        core::slice::from_raw_parts(
                            (&request as *const UserRequest).cast::<u8>(),
                            mem::size_of::<UserRequest>(),
                        )
                    };
                    pending_user = IpcBytes::from_bytes(MessageKind::UserRequest, bytes);
                    logos_shell::Shell::acknowledge_sent(&mut request);
                    lockscreen.clear_password();
                }
            }
            if action != logos_lockscreen::LockScreenAction::Ignored {
                send_shell_surface(
                    display,
                    &login_page,
                    &login_layout,
                    lockscreen,
                    &mut surface_sequence,
                    true,
                );
            }
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
                if shell.apply_user_response(result).is_ok() {
                    let context = shell.context();
                    let _ = common::ipc_send_handle(flow, &context);
                    let lockscreen = unsafe { &mut *core::ptr::addr_of_mut!(LOCKSCREEN) };
                    lockscreen.apply_status(result.status);
                    send_shell_surface(
                        display,
                        &login_page,
                        &login_layout,
                        lockscreen,
                        &mut surface_sequence,
                        shell.focus() == logos_shell::ShellFocus::LockScreen,
                    );
                }
            }
        }
        common::wait_on_capabilities(&[input, user_send, user_receive]);
    }
}

#[cfg(target_os = "none")]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo<'_>) -> ! {
    common::idle()
}

#[cfg(not(target_os = "none"))]
fn main() {}
