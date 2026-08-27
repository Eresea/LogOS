#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", no_main)]
#![cfg_attr(not(target_os = "none"), allow(dead_code, unused_imports, unused_variables))]

mod common;

use logos_abi::{
    AtriumControl, AtriumControlOperation, GuiDrawBatch, GuiDrawCommand, GuiHook, GuiHookKind,
    GuiRect, GuiSessionContext, GuiSurfaceOperation, GuiSurfaceRequest, GuiSurfaceResponse,
    InputMessage, IpcStatus, KeyCode, KeyState, SurfaceHandle,
};

const INPUT_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_GUI_INPUT,
    b"atrium",
    core::mem::size_of::<InputMessage>(),
    logos_abi::IpcRights::Receive,
);
const DISPLAY_DRAW_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_GUI_DRAW,
    b"display",
    core::mem::size_of::<GuiDrawBatch>(),
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
const TERMINAL_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_GUI_INPUT,
    b"terminal",
    core::mem::size_of::<InputMessage>(),
    logos_abi::IpcRights::Send,
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

static mut ATRIUM: logos_atrium::Atrium = logos_atrium::Atrium::new();
static mut CALCULATOR: logos_atrium::Calculator = logos_atrium::Calculator::new();

fn push_text(batch: &mut GuiDrawBatch, x: i32, y: i32, color: u32, text: &[u8]) {
    if let Some(command) = GuiDrawCommand::glyph_run(x, y, color, text) {
        let _ = batch.push(command);
    }
}

fn draw_home(display: logos_abi::CapabilityHandle, surface: SurfaceHandle, sequence: u32) {
    let mut batch = GuiDrawBatch::new(surface, sequence, GuiRect::SURFACE);
    let _ = batch.push(GuiDrawCommand::fill_surface(0x101820));
    let _ = batch.push(GuiDrawCommand::fill_rect(GuiRect::new(0, 0, 180, 400), 0x182535));
    push_text(&mut batch, 24, 24, 0xffffff, b"LogOS Atrium");
    push_text(&mut batch, 24, 72, 0xd9e5f5, b"Calculator");
    push_text(&mut batch, 24, 104, 0xd9e5f5, b"Files");
    push_text(&mut batch, 24, 136, 0xd9e5f5, b"Terminal");
    let _ = common::ipc_send_handle(display, &batch);

    let mut detail = GuiDrawBatch::new(surface, sequence, GuiRect::new(180, 0, 460, 400));
    detail.flags = logos_abi::GUI_DRAW_FLAG_MORE;
    push_text(&mut detail, 220, 48, 0xffffff, b"Welcome to Atrium");
    push_text(&mut detail, 220, 88, 0xb8c7da, b"Ctrl+1 Calculator  Ctrl+2 Files");
    push_text(&mut detail, 220, 112, 0xb8c7da, b"Ctrl+3 Terminal");
    push_text(&mut detail, 220, 128, 0xb8c7da, b"Tab focuses; Ctrl+Arrow moves apps");
    let _ = common::ipc_send_handle(display, &detail);
}

fn draw_app(
    display: logos_abi::CapabilityHandle,
    window: logos_atrium::Window,
    calculator: &logos_atrium::Calculator,
    sequence: u32,
) {
    let mut batch = GuiDrawBatch::new(window.surface, sequence, GuiRect::SURFACE);
    let _ = batch.push(GuiDrawCommand::fill_surface(0x151c26));
    let title: &[u8] = match window.app {
        logos_atrium::AppId::Calculator => b"Calculator",
        logos_atrium::AppId::Files => b"Files",
        logos_atrium::AppId::Terminal => b"Terminal",
    };
    push_text(&mut batch, 20, 24, 0xffffff, title);
    match window.app {
        logos_atrium::AppId::Calculator => {
            let _ = batch.push(GuiDrawCommand::fill_rect(GuiRect::new(20, 52, 260, 48), 0x263548));
            push_text(&mut batch, 32, 82, 0xffffff, calculator.display());
            push_text(&mut batch, 24, 132, 0xb8c7da, b"0-9  +  -  *  /  Enter");
        }
        logos_atrium::AppId::Files => {
            push_text(&mut batch, 24, 76, 0xb8c7da, b"Files placeholder");
            push_text(&mut batch, 24, 108, 0x7890aa, b"Filesystem UI is planned");
            push_text(&mut batch, 24, 132, 0x7890aa, b"separately.");
        }
        logos_atrium::AppId::Terminal => {
            push_text(&mut batch, 24, 76, 0xb8c7da, b"Terminal surface managed");
            push_text(&mut batch, 24, 100, 0xb8c7da, b"by Atrium");
        }
    }
    let _ = common::ipc_send_handle(display, &batch);
}

fn next_request_id(next: &mut u32) -> u32 {
    let value = *next;
    *next = next.wrapping_add(1).max(1);
    value
}

fn send_surface_command(
    display: logos_abi::CapabilityHandle,
    operation: GuiSurfaceOperation,
    surface: SurfaceHandle,
    bounds: GuiRect,
    next: &mut u32,
) {
    let mut request = GuiSurfaceRequest::new(operation, next_request_id(next));
    request.surface = surface;
    request.bounds = bounds;
    let _ = common::ipc_send_handle(display, &request);
}

fn send_lockscreen_section(lockscreen: logos_abi::CapabilityHandle, visible: bool, next: &mut u32) {
    let mut hook = GuiHook::new(GuiHookKind::Section, next_request_id(next));
    hook.deadline = u64::from(visible);
    let _ = common::ipc_send_handle(lockscreen, &hook);
}

fn hide_surfaces(
    display: logos_abi::CapabilityHandle,
    atrium: &mut logos_atrium::Atrium,
    next: &mut u32,
) {
    let mut handles = [SurfaceHandle::EMPTY; logos_atrium::MAX_ATRIUM_WINDOWS];
    let mut count = 0;
    for window in atrium.windows() {
        handles[count] = window.surface;
        count += 1;
    }
    for surface in handles[..count].iter().copied() {
        send_surface_command(display, GuiSurfaceOperation::Destroy, surface, GuiRect::EMPTY, next);
    }
    if atrium.home_surface().is_valid() {
        send_surface_command(
            display,
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
    draw_home(display, home, *sequence);
    if let Some(window) = atrium.focused_window() {
        draw_app(display, window, calculator, *sequence);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    common::init_service_allocator();
    let input = common::capability_handle(INPUT_CAPABILITY).unwrap_or_else(|_| common::idle());
    let display =
        common::capability_handle(DISPLAY_DRAW_CAPABILITY).unwrap_or_else(|_| common::idle());
    let display_control =
        common::capability_handle(DISPLAY_CONTROL_CAPABILITY).unwrap_or_else(|_| common::idle());
    let display_response =
        common::capability_handle(DISPLAY_RESPONSE_CAPABILITY).unwrap_or_else(|_| common::idle());
    let terminal =
        common::capability_handle(TERMINAL_CAPABILITY).unwrap_or_else(|_| common::idle());
    let shell = common::capability_handle(SHELL_CAPABILITY).unwrap_or_else(|_| common::idle());
    let shell_context =
        common::capability_handle(SHELL_CONTEXT_CAPABILITY).unwrap_or_else(|_| common::idle());
    let lockscreen_input =
        common::capability_handle(LOCKSCREEN_INPUT_CAPABILITY).unwrap_or_else(|_| common::idle());
    let lockscreen_control =
        common::capability_handle(LOCKSCREEN_CONTROL_CAPABILITY).unwrap_or_else(|_| common::idle());

    let atrium = unsafe { &mut *core::ptr::addr_of_mut!(ATRIUM) };
    let calculator = unsafe { &mut *core::ptr::addr_of_mut!(CALCULATOR) };
    let mut next_request = 1u32;
    let mut sequence = 0u32;
    let mut pending_surface: Option<(GuiSurfaceRequest, Option<logos_atrium::AppId>)> = None;
    let mut authenticated = false;
    let mut heartbeat_ticks = 0u16;
    let mut event = InputMessage::key(KeyCode::Unknown, KeyState::Released, 0);
    let mut response = GuiSurfaceResponse::new(
        GuiSurfaceRequest::new(GuiSurfaceOperation::CreateModal, 1),
        logos_abi::GuiStatus::Malformed,
    );
    send_lockscreen_section(lockscreen_control, true, &mut next_request);

    loop {
        common::heartbeat_tick(&mut heartbeat_ticks);
        let mut context = GuiSessionContext::EMPTY;
        while common::ipc_receive_handle(shell_context, &mut context) == IpcStatus::Ok {
            if context.is_authenticated() {
                authenticated = true;
                send_lockscreen_section(lockscreen_control, false, &mut next_request);
                atrium.authenticate();
                if !atrium.home_surface().is_valid() && pending_surface.is_none() {
                    let mut request = GuiSurfaceRequest::new(
                        GuiSurfaceOperation::CreateModal,
                        next_request_id(&mut next_request),
                    );
                    request.bounds = GuiRect::new(0, 0, 640, 400);
                    request.z_order = 1;
                    let _ = common::ipc_send_handle(display_control, &request);
                    pending_surface = Some((request, None));
                }
            } else if authenticated {
                authenticated = false;
                pending_surface = None;
                hide_surfaces(display_control, atrium, &mut next_request);
                send_lockscreen_section(lockscreen_control, true, &mut next_request);
            }
        }

        while common::ipc_receive_handle(display_response, &mut response) == IpcStatus::Ok {
            let Some((request, app)) = pending_surface else { continue };
            if !response.is_valid_for(request) || response.request_id != request.request_id {
                continue;
            }
            pending_surface = None;
            if response.status != logos_abi::GuiStatus::Ok || !response.surface.is_valid() {
                continue;
            }
            if let Some(app) = app {
                if atrium.launch(app, response.surface, request.bounds).is_err() {
                    send_surface_command(
                        display_control,
                        GuiSurfaceOperation::Destroy,
                        response.surface,
                        GuiRect::EMPTY,
                        &mut next_request,
                    );
                }
            } else {
                let _ = atrium.set_home_surface(response.surface);
            }
            render(display, atrium, calculator, &mut sequence);
        }

        while common::ipc_receive_handle(input, &mut event) == IpcStatus::Ok {
            if !authenticated || atrium.phase() != logos_atrium::AtriumPhase::Home {
                let _ = common::ipc_send_handle(lockscreen_input, &event);
                continue;
            }
            let action = atrium.input(&event);
            match action {
                logos_atrium::AtriumAction::Launch(app) if pending_surface.is_none() => {
                    let bounds = match app {
                        logos_atrium::AppId::Calculator => GuiRect::new(220, 72, 320, 220),
                        logos_atrium::AppId::Files => GuiRect::new(248, 88, 340, 190),
                        logos_atrium::AppId::Terminal => GuiRect::new(200, 48, 420, 300),
                    };
                    let mut request = GuiSurfaceRequest::new(
                        GuiSurfaceOperation::CreateModal,
                        next_request_id(&mut next_request),
                    );
                    request.bounds = bounds;
                    request.z_order = 2;
                    let _ = common::ipc_send_handle(display_control, &request);
                    pending_surface = Some((request, Some(app)));
                }
                logos_atrium::AtriumAction::Logout => {
                    let _ = atrium.apply_action(action);
                    hide_surfaces(display_control, atrium, &mut next_request);
                    let command = AtriumControl::new(AtriumControlOperation::Logout, 1);
                    let _ = common::ipc_send_handle(shell, &command);
                }
                logos_atrium::AtriumAction::CloseFocused => {
                    let old = atrium.focused_window();
                    if atrium.apply_action(action).is_ok() {
                        if let Some(window) = old {
                            send_surface_command(
                                display_control,
                                GuiSurfaceOperation::Destroy,
                                window.surface,
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
                        if let Some(window) = atrium.focused_window() {
                            if matches!(action, logos_atrium::AtriumAction::MoveFocused(_, _)) {
                                send_surface_command(
                                    display_control,
                                    GuiSurfaceOperation::Update,
                                    window.surface,
                                    window.bounds,
                                    &mut next_request,
                                );
                            } else {
                                send_surface_command(
                                    display_control,
                                    GuiSurfaceOperation::Focus,
                                    window.surface,
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
            if let Some(window) = atrium.focused_window() {
                if window.app == logos_atrium::AppId::Terminal {
                    let _ = common::ipc_send_handle(terminal, &event);
                } else if window.app == logos_atrium::AppId::Calculator && calculator.input(&event)
                {
                    render(display, atrium, calculator, &mut sequence);
                }
            }
        }
        common::wait_on_capabilities(&[input, display_response, shell_context]);
    }
}

#[cfg(target_os = "none")]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo<'_>) -> ! {
    common::idle()
}

#[cfg(not(target_os = "none"))]
fn main() {}
