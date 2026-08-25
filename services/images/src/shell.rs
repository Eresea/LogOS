#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", no_main)]
#![cfg_attr(not(target_os = "none"), allow(dead_code, unused_imports, unused_variables))]

mod common;

use core::{mem, ptr};

use logos_abi::{
    GuiDrawBatch, GuiDrawCommand, GuiRect, GuiSessionContext, InputMessage, IpcBytes, IpcStatus,
    MessageKind, SurfaceHandle, UserRequest, UserResponse,
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
    lockscreen: &logos_lockscreen::LockScreen,
    sequence: &mut u32,
    visible: bool,
) {
    *sequence = sequence.wrapping_add(1).max(1);
    let mut batch = GuiDrawBatch::new(
        SurfaceHandle::new(0, 1, 11).unwrap(),
        *sequence,
        GuiRect::new(0, 0, 640, 400),
    );
    if visible {
        let _ = batch.push(GuiDrawCommand::fill_surface(0x101820));
        let _ =
            batch.push(GuiDrawCommand::stroke_rect(GuiRect::new(24, 24, 592, 352), 0x4b89dc, 2));
        let title = if lockscreen.failure() {
            b"Login failed - try again".as_slice()
        } else if lockscreen.mode() == logos_lockscreen::LockScreenMode::Claim {
            b"Claim your LogOS account".as_slice()
        } else {
            b"LogOS login".as_slice()
        };
        if let Some(title) = GuiDrawCommand::glyph_run(48, 48, 0xffffff, title) {
            let _ = batch.push(title);
        }
    }
    let _ = common::ipc_send_handle(display, &batch);
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
    let lockscreen = unsafe { &*core::ptr::addr_of!(LOCKSCREEN) };
    let mut surface_sequence = 0u32;
    send_shell_surface(display, lockscreen, &mut surface_sequence, true);
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
                send_shell_surface(display, lockscreen, &mut surface_sequence, true);
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
