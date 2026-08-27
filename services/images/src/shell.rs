#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", no_main)]
#![cfg_attr(not(target_os = "none"), allow(dead_code, unused_imports, unused_variables))]

mod common;

mod generated_login_ui {
    include!(concat!(env!("OUT_DIR"), "/login_ui.rs"));
}

mod generated_register_ui {
    include!(concat!(env!("OUT_DIR"), "/register_ui.rs"));
}

use core::{mem, ptr};

use logos_abi::{
    AtriumControl, AtriumControlOperation, GUI_DRAW_FLAG_MORE, GuiDrawBatch, GuiDrawCommand,
    GuiRect, GuiSessionContext, InputMessage, IpcBytes, IpcStatus, MAX_GUI_BATCH_FRAGMENTS,
    MAX_GUI_TEXT_BYTES, MessageKind, SurfaceHandle, UserOperation, UserRequest, UserResponse,
    UserStatus,
};

const INPUT_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_GUI_INPUT,
    b"input",
    core::mem::size_of::<InputMessage>(),
    logos_abi::IpcRights::Receive,
);
const DISPLAY_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_GUI_DRAW,
    b"display",
    core::mem::size_of::<logos_abi::GuiSceneOp>(),
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
static mut LOCKSCREEN: logos_lockscreen::LockScreen = logos_lockscreen::LockScreen::new();
static mut LOGIN_PAGE: Option<logos_ui_compiler::UiBuild> = None;
static mut REGISTER_PAGE: Option<logos_ui_compiler::UiBuild> = None;
static mut LOGIN_LAYOUT: Option<logos_shell::LoginLayout> = None;
static mut REGISTER_LAYOUT: Option<logos_shell::LoginLayout> = None;

struct ShellPages<'a> {
    login: (&'a logos_ui_compiler::UiBuild, &'a logos_shell::LoginLayout),
    register: (&'a logos_ui_compiler::UiBuild, &'a logos_shell::LoginLayout),
}

fn send_current_surface(
    display: logos_abi::CapabilityHandle,
    pages: ShellPages<'_>,
    lockscreen: &logos_lockscreen::LockScreen,
    sequence: &mut u32,
    visible: bool,
) {
    if lockscreen.mode() == logos_lockscreen::LockScreenMode::Claim {
        send_shell_surface(
            display,
            pages.register.0,
            pages.register.1,
            lockscreen,
            sequence,
            visible,
        );
    } else {
        send_shell_surface(display, pages.login.0, pages.login.1, lockscreen, sequence, visible);
    }
}

fn send_register_surface(
    display: logos_abi::CapabilityHandle,
    page: &logos_ui_compiler::UiBuild,
    layout: &logos_shell::LoginLayout,
    lockscreen: &logos_lockscreen::LockScreen,
    sequence: &mut u32,
    surface: SurfaceHandle,
) {
    let state = logos_shell::LoginUiState::new(true, lockscreen.failure());
    let Some(panel) = layout.node(0).map(|node| node.bounds) else { return };
    let Some(title_index) = logos_shell::named_node_index(page, b"title") else { return };
    let Some(notice_index) = logos_shell::named_node_index(page, b"notice") else { return };
    let Some(username_index) = logos_shell::named_node_index(page, b"username") else { return };
    let Some(password_index) = logos_shell::named_node_index(page, b"password") else { return };
    let Some(confirm_index) = logos_shell::named_node_index(page, b"confirmPassword") else {
        return;
    };
    let Some(submit_index) = logos_shell::named_node_index(page, b"submit") else { return };
    let Some(title) = layout.node(title_index).map(|node| node.bounds) else { return };
    let Some(notice) = layout.node(notice_index).map(|node| node.bounds) else { return };
    let Some(username) = layout.bounds_for(logos_shell::LoginHitTarget::Username) else {
        return;
    };
    let Some(password) = layout.bounds_for(logos_shell::LoginHitTarget::Password) else { return };
    let Some(confirm) = layout.bounds_for(logos_shell::LoginHitTarget::ConfirmPassword) else {
        return;
    };
    let Some(submit) = layout.bounds_for(logos_shell::LoginHitTarget::Submit) else { return };

    let mut batches =
        [GuiDrawBatch::new(surface, *sequence, GuiRect::SURFACE); MAX_GUI_BATCH_FRAGMENTS];
    for batch in &mut batches[..MAX_GUI_BATCH_FRAGMENTS - 1] {
        batch.flags = GUI_DRAW_FLAG_MORE;
    }
    let _ = batches[0].push(GuiDrawCommand::fill_surface(0x101820));
    let _ = batches[0].push(GuiDrawCommand::shadow(panel, 0x55000000, 16, 4, 0, 6));
    let _ = batches[0].push(GuiDrawCommand::fill_rounded_rect(panel, 0x182535, 16));

    let mut text = [0; MAX_GUI_TEXT_BYTES];
    let length = logos_shell::login_page_node_text(page, title_index, state, &mut text);
    if let Some(text) = GuiDrawCommand::glyph_run_styled(
        title.x,
        title.y,
        0xffffff,
        logos_shell::login_text_flags(page, title_index),
        &text[..length],
    ) {
        let _ = batches[1].push(text);
    }

    let _ = batches[1].push(GuiDrawCommand::stroke_rounded_rect(panel, 0x4b89dc, 16, 2));

    let length = logos_shell::login_page_node_text(page, notice_index, state, &mut text);
    if let Some(text) = GuiDrawCommand::glyph_run_styled(
        notice.x,
        notice.y,
        0xb8c7da,
        logos_shell::login_text_flags(page, notice_index),
        &text[..length],
    ) {
        let _ = batches[1].push(text);
    }

    let (username_value, password_value) = lockscreen.credentials();
    let confirm_value = lockscreen.confirmation();
    let username_color = if lockscreen.field() == logos_lockscreen::LockScreenField::Username {
        0x315e91
    } else {
        0x263548
    };
    let password_color = if lockscreen.field() == logos_lockscreen::LockScreenField::Password {
        0x315e91
    } else {
        0x263548
    };
    let confirm_color = if lockscreen.field() == logos_lockscreen::LockScreenField::ConfirmPassword
    {
        0x315e91
    } else {
        0x263548
    };
    let username_invalid = lockscreen.form().controls.username.touched()
        && !lockscreen.form().controls.username.valid();
    let password_invalid = lockscreen.form().controls.password.touched()
        && !lockscreen.form().controls.password.valid();
    let confirm_invalid = lockscreen.form().controls.confirm_password.touched()
        && !lockscreen.form().controls.confirm_password.valid();
    let _ = batches[2].push(GuiDrawCommand::fill_rounded_rect(
        username,
        if username_invalid { 0xb84a4a } else { username_color },
        8,
    ));
    let length = if username_value.is_empty() {
        logos_shell::login_page_node_text(page, username_index, state, &mut text)
    } else {
        logos_shell::login_input_text(username_value, false, username.width, &mut text)
    };
    if let Some(text) = GuiDrawCommand::glyph_run_styled(
        username.x.saturating_add(12),
        username.y.saturating_add(12),
        0xd9e5f5,
        logos_shell::login_text_flags(page, username_index),
        &text[..length],
    ) {
        let _ = batches[2].push(text);
    }
    let _ = batches[2].push(GuiDrawCommand::fill_rounded_rect(
        password,
        if password_invalid { 0xb84a4a } else { password_color },
        8,
    ));
    let length = if password_value.is_empty() {
        logos_shell::login_page_node_text(page, password_index, state, &mut text)
    } else {
        logos_shell::login_input_text(password_value, true, password.width, &mut text)
    };
    if let Some(text) = GuiDrawCommand::glyph_run_styled(
        password.x.saturating_add(12),
        password.y.saturating_add(12),
        0xd9e5f5,
        logos_shell::login_text_flags(page, password_index),
        &text[..length],
    ) {
        let _ = batches[3].push(text);
    }
    let _ = batches[3].push(GuiDrawCommand::fill_rounded_rect(
        confirm,
        if confirm_invalid { 0xb84a4a } else { confirm_color },
        8,
    ));
    let length = if confirm_value.is_empty() {
        logos_shell::login_page_node_text(page, confirm_index, state, &mut text)
    } else {
        logos_shell::login_input_text(confirm_value, true, confirm.width, &mut text)
    };
    if let Some(text) = GuiDrawCommand::glyph_run_styled(
        confirm.x.saturating_add(12),
        confirm.y.saturating_add(12),
        0xd9e5f5,
        logos_shell::login_text_flags(page, confirm_index),
        &text[..length],
    ) {
        let _ = batches[3].push(text);
    }
    let submit_disabled = !lockscreen.form().can_submit();
    let submit_color = if submit_disabled { 0x1b356b } else { 0x356bd8 };
    let _ = batches[4].push(GuiDrawCommand::fill_rounded_rect(submit, submit_color, 8));
    let length = logos_shell::login_page_node_text(page, submit_index, state, &mut text);
    if let Some(text) = GuiDrawCommand::glyph_run_styled(
        submit.x.saturating_add(12),
        submit.y.saturating_add(12),
        0xffffff,
        logos_shell::login_text_flags(page, submit_index),
        &text[..length],
    ) {
        let _ = batches[4].push(text);
    }
    for (index, batch) in batches.iter().enumerate() {
        let _ = common::ipc_send_scene_batch(display, batch, 1 + index as u32 * 3);
    }
}

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
        let _ = common::ipc_send_scene_batch(display, &batch, 1);
        return;
    }
    if logos_shell::named_node_index(login_page, b"confirmPassword").is_some() {
        send_register_surface(display, login_page, layout, lockscreen, sequence, surface);
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
    let _ = batches[0].push(GuiDrawCommand::shadow(panel, 0x55000000, 16, 4, 0, 6));
    let _ = batches[0].push(GuiDrawCommand::fill_rounded_rect(panel, 0x182535, 16));

    let mut text = [0; MAX_GUI_TEXT_BYTES];
    let length = logos_shell::login_page_node_text(login_page, 2, state, &mut text);
    if let Some(text) = GuiDrawCommand::glyph_run_styled(
        title.x,
        title.y,
        0xffffff,
        logos_shell::login_text_flags(login_page, 2),
        &text[..length],
    ) {
        let _ = batches[1].push(text);
    }
    let _ = batches[1].push(GuiDrawCommand::stroke_rounded_rect(panel, 0x4b89dc, 16, 2));
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
    let username_invalid = lockscreen.form().controls.username.touched()
        && !lockscreen.form().controls.username.valid();
    let password_invalid = lockscreen.form().controls.password.touched()
        && !lockscreen.form().controls.password.valid();
    let _ = batches[1].push(GuiDrawCommand::fill_rounded_rect(username, username_color, 8));
    let _ = batches[2].push(GuiDrawCommand::stroke_rounded_rect(
        username,
        if username_invalid { 0xb84a4a } else { 0x6d8fc4 },
        8,
        1,
    ));

    let (username_value, password_value) = lockscreen.credentials();
    let length = if username_value.is_empty() {
        logos_shell::login_page_node_text(login_page, 3, state, &mut text)
    } else {
        logos_shell::login_input_text(username_value, false, username.width, &mut text)
    };
    if let Some(text) = GuiDrawCommand::glyph_run_styled(
        username.x.saturating_add(12),
        username.y.saturating_add(12),
        0xd9e5f5,
        logos_shell::login_text_flags(login_page, 3),
        &text[..length],
    ) {
        let _ = batches[2].push(text);
    }
    let _ = batches[2].push(GuiDrawCommand::fill_rounded_rect(password, password_color, 8));
    let _ = batches[3].push(GuiDrawCommand::stroke_rounded_rect(
        password,
        if password_invalid { 0xb84a4a } else { 0x6d8fc4 },
        8,
        1,
    ));

    let length = if password_value.is_empty() {
        logos_shell::login_page_node_text(login_page, 4, state, &mut text)
    } else {
        logos_shell::login_input_text(password_value, true, password.width, &mut text)
    };
    if let Some(text) = GuiDrawCommand::glyph_run_styled(
        password.x.saturating_add(12),
        password.y.saturating_add(12),
        0xd9e5f5,
        logos_shell::login_text_flags(login_page, 4),
        &text[..length],
    ) {
        let _ = batches[3].push(text);
    }
    let submit_disabled = !lockscreen.form().can_submit();
    let submit_color = if submit_disabled
        || logos_shell::login_style_active(
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
    let _ = batches[3].push(GuiDrawCommand::fill_rounded_rect(submit, submit_color, 8));
    let length = logos_shell::login_page_node_text(login_page, 5, state, &mut text);
    if let Some(text) = GuiDrawCommand::glyph_run_styled(
        submit.x.saturating_add(12),
        submit.y.saturating_add(12),
        0xffffff,
        logos_shell::login_text_flags(login_page, 5),
        &text[..length],
    ) {
        let _ = batches[4].push(text);
    }
    for (index, batch) in batches.iter().enumerate() {
        let _ = common::ipc_send_scene_batch(display, batch, 1 + index as u32 * 3);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    common::init_service_allocator();
    let input = match common::capability_handle(INPUT_CAPABILITY) {
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
    unsafe {
        ptr::write(core::ptr::addr_of_mut!(LOGIN_PAGE), Some(generated_login_ui::build()));
        ptr::write(core::ptr::addr_of_mut!(REGISTER_PAGE), Some(generated_register_ui::build()));
    }
    let login_page = unsafe { (*core::ptr::addr_of!(LOGIN_PAGE)).as_ref() }.unwrap();
    let register_page = unsafe { (*core::ptr::addr_of!(REGISTER_PAGE)).as_ref() }.unwrap();
    if !login_page.is_valid() || !register_page.is_valid() {
        common::idle();
    }
    let Some(login_layout) =
        logos_shell::LoginLayout::from_build(login_page, GuiRect::new(0, 0, 640, 400))
    else {
        common::idle();
    };
    let Some(register_layout) =
        logos_shell::LoginLayout::from_build(register_page, GuiRect::new(0, 0, 640, 400))
    else {
        common::idle();
    };
    unsafe {
        ptr::write(core::ptr::addr_of_mut!(LOGIN_LAYOUT), Some(login_layout));
        ptr::write(core::ptr::addr_of_mut!(REGISTER_LAYOUT), Some(register_layout));
    }
    let login_layout = unsafe { (*core::ptr::addr_of!(LOGIN_LAYOUT)).as_ref() }.unwrap();
    let register_layout = unsafe { (*core::ptr::addr_of!(REGISTER_LAYOUT)).as_ref() }.unwrap();
    let lockscreen = unsafe { &*core::ptr::addr_of!(LOCKSCREEN) };
    let mut surface_sequence = 0u32;
    send_current_surface(
        display,
        ShellPages {
            login: (login_page, login_layout),
            register: (register_page, register_layout),
        },
        lockscreen,
        &mut surface_sequence,
        true,
    );
    let context = GuiSessionContext::EMPTY;
    let _ = common::ipc_send_handle(flow, &context);
    let _ = common::ipc_send_handle(atrium_context, &context);
    let mut pending_user: Option<IpcBytes> = None;
    let mut pending_lock_request: Option<UserRequest> = None;
    let mut response = IpcBytes::empty(MessageKind::UserResponse);
    let mut lock_request = UserRequest::new(UserOperation::Login, 1);
    let mut atrium_command = AtriumControl::new(AtriumControlOperation::Reset, 1);
    let mut message =
        InputMessage::key(logos_abi::KeyCode::Unknown, logos_abi::KeyState::Released, 0);
    let mut heartbeat_ticks = 0u16;
    loop {
        common::heartbeat_tick(&mut heartbeat_ticks);
        while common::ipc_receive_handle(atrium_control, &mut atrium_command) == IpcStatus::Ok {
            if atrium_command.operation == AtriumControlOperation::Logout && pending_user.is_none()
            {
                if let Ok(mut request) = shell.logout() {
                    let _ = common::ipc_send_handle(atrium_context, &GuiSessionContext::EMPTY);
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
                || pending_lock_request.is_some()
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
                let applied = shell.apply_user_response(result);
                if let Some(request) = pending_lock_request.take() {
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
                    let _ = common::ipc_send_handle(lockscreen_response, &lock_response);
                }
                if applied.is_ok() {
                    let context = shell.context();
                    let _ = common::ipc_send_handle(flow, &context);
                    let _ = common::ipc_send_handle(atrium_context, &context);
                }
            }
        }
        common::wait_on_capabilities(&[
            input,
            user_send,
            user_receive,
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
