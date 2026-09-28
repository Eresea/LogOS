#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", no_main)]
#![cfg_attr(not(target_os = "none"), allow(dead_code, unused_imports, unused_variables))]

mod common;

use logos_abi::{
    AtriumApp, AtriumSurfaceInput, AtriumSurfaceRequest, AtriumSurfaceResponse, GuiRect,
    GuiTextGridRow, InputMessage, IpcBytes, IpcStatus, KeyCode, KeyState, MessageKind,
    PointerState, SurfaceHandle, TerminalTabHit, pack_terminal_tab_state, terminal_tab_bar_hit,
};

const INPUT_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_ATRIUM_SURFACE_INPUT,
    b"atrium",
    core::mem::size_of::<AtriumSurfaceInput>(),
    logos_abi::IpcRights::Receive,
);
const ATRIUM_RENDER_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_RENDER,
    b"atrium",
    core::mem::size_of::<GuiTextGridRow>(),
    logos_abi::IpcRights::Send,
);
const ATRIUM_SURFACE_REQUEST_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_ATRIUM_SURFACE_REQUEST,
    b"atrium",
    core::mem::size_of::<AtriumSurfaceRequest>(),
    logos_abi::IpcRights::Send,
);
const ATRIUM_SURFACE_RESPONSE_CAPABILITY: common::CapabilitySpec =
    common::capability_contract_named(
        logos_abi::IPC_CONTRACT_ATRIUM_SURFACE_RESPONSE,
        b"atrium",
        core::mem::size_of::<AtriumSurfaceResponse>(),
        logos_abi::IpcRights::Receive,
    );
const SESSION_INPUT_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_BYTES,
    b"session",
    core::mem::size_of::<IpcBytes>(),
    logos_abi::IpcRights::Send,
);
const SESSION_OUTPUT_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_BYTES,
    b"session",
    core::mem::size_of::<IpcBytes>(),
    logos_abi::IpcRights::Receive,
);

static mut TERMINAL: logos_terminal::TerminalService = logos_terminal::TerminalService::new();
static mut PENDING_RENDER: Option<GuiTextGridRow> = None;
static mut PENDING_SESSION_INPUT: Option<IpcBytes> = None;

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    common::init_service_allocator();
    let terminal = unsafe { &mut *core::ptr::addr_of_mut!(TERMINAL) };
    let pending_render = unsafe { &mut *core::ptr::addr_of_mut!(PENDING_RENDER) };
    let pending_session_input = unsafe { &mut *core::ptr::addr_of_mut!(PENDING_SESSION_INPUT) };
    let input_capability = match common::capability_handle(INPUT_CAPABILITY) {
        Ok(capability) => capability,
        Err(_) => common::idle(),
    };
    let atrium_render_capability = match common::capability_handle(ATRIUM_RENDER_CAPABILITY) {
        Ok(capability) => capability,
        Err(_) => common::idle(),
    };
    let atrium_surface_request_capability =
        match common::capability_handle(ATRIUM_SURFACE_REQUEST_CAPABILITY) {
            Ok(capability) => capability,
            Err(_) => common::idle(),
        };
    let atrium_surface_response_capability =
        match common::capability_handle(ATRIUM_SURFACE_RESPONSE_CAPABILITY) {
            Ok(capability) => capability,
            Err(_) => common::idle(),
        };
    let session_input_capability = match common::capability_handle(SESSION_INPUT_CAPABILITY) {
        Ok(capability) => capability,
        Err(_) => common::idle(),
    };
    let session_output_capability = match common::capability_handle(SESSION_OUTPUT_CAPABILITY) {
        Ok(capability) => capability,
        Err(_) => common::idle(),
    };
    let mut heartbeat_ticks = 0u16;
    let client = common::bootstrap_page().service;
    let base_surface_request = AtriumSurfaceRequest::new(AtriumApp::Terminal, client, 1);
    // The very first request is sent at most once, exactly like before #76:
    // Atrium only grants a surface once the user actually activates
    // Terminal, and retrying an unanswered/backpressured request every loop
    // tick would race that gate and auto-attach a surface as soon as Home
    // is reached, before the user ever asked for one.
    let mut surface_request_sent = false;
    let mut terminal_surface = SurfaceHandle::EMPTY;
    let mut terminal_bounds = GuiRect::EMPTY;
    // Re-sent whenever the tab bar changes (open/close/switch) once a
    // surface already exists: Atrium's "surface already exists for this
    // client" path already reconfirms the same surface on a repeat
    // request, so this doubles as the tab-state channel without a new
    // message kind (#76).
    let mut last_sent_tab_state: Option<u16> = None;
    loop {
        if pending_render.is_some() {
            common::heartbeat();
        } else {
            common::heartbeat_tick(&mut heartbeat_ticks);
        }
        let tab_state =
            pack_terminal_tab_state(terminal.open_bitmap(), terminal.active_slot() as u8);
        let surface_request = base_surface_request.with_tab_state(tab_state);
        let want_send = (!surface_request_sent && !terminal_surface.is_valid())
            || (terminal_surface.is_valid() && last_sent_tab_state != Some(tab_state));
        if want_send {
            match common::ipc_send_handle(atrium_surface_request_capability, &surface_request) {
                IpcStatus::Ok => {
                    surface_request_sent = true;
                    last_sent_tab_state = Some(tab_state);
                }
                IpcStatus::Full => {}
                IpcStatus::Stale
                | IpcStatus::Disconnected
                | IpcStatus::Unauthorized
                | IpcStatus::Malformed
                | IpcStatus::Empty => {}
            }
        }
        let mut surface_response =
            AtriumSurfaceResponse::new(surface_request, logos_abi::GuiStatus::Malformed);
        while common::ipc_receive_handle(atrium_surface_response_capability, &mut surface_response)
            == IpcStatus::Ok
        {
            if surface_response.is_update() && surface_response.surface == terminal_surface {
                terminal_bounds = surface_response.bounds;
                terminal.resize_to_surface(surface_response.bounds);
            } else if surface_response.is_valid_for(surface_request)
                && surface_response.status == logos_abi::GuiStatus::Ok
                && surface_response.surface.is_valid()
            {
                if surface_response.surface != terminal_surface {
                    terminal.reset();
                    terminal_bounds = surface_response.bounds;
                    terminal.resize_to_surface(surface_response.bounds);
                } else if surface_response.bounds != terminal_bounds {
                    // A same-surface reconfirmation just echoes the tab
                    // state we sent; only re-layout if bounds actually
                    // moved (tiling), so switching tabs never forces a
                    // spurious resize/redraw of the active session.
                    terminal_bounds = surface_response.bounds;
                    terminal.resize_to_surface(surface_response.bounds);
                }
                terminal_surface = surface_response.surface;
            } else if surface_response.is_revoke() && surface_response.surface == terminal_surface {
                terminal_surface = SurfaceHandle::EMPTY;
                *pending_render = None;
            }
        }
        if let Some(message) = *pending_session_input {
            match common::ipc_send_handle(session_input_capability, &message) {
                IpcStatus::Ok => {
                    *pending_session_input = None;
                }
                IpcStatus::Full => {}
                IpcStatus::Stale
                | IpcStatus::Disconnected
                | IpcStatus::Unauthorized
                | IpcStatus::Malformed
                | IpcStatus::Empty => *pending_session_input = None,
            }
        }
        if pending_session_input.is_none() {
            let mut event = AtriumSurfaceInput::new(
                SurfaceHandle::EMPTY,
                InputMessage::key(KeyCode::Unknown, KeyState::Released, 0),
            );
            while common::ipc_receive_handle(input_capability, &mut event) == IpcStatus::Ok {
                // Appearance is a global preference (ADR-0089): apply it even
                // if it overtakes this Terminal's own surface response.
                if event.is_valid() && event.input.appearance_flags().is_some() {
                    let _ = terminal.input(&event.input);
                    continue;
                }
                if !event.is_valid() || event.surface != terminal_surface {
                    continue;
                }
                if let Some(close) = handle_tab_bar_click(terminal, terminal_bounds, &event.input) {
                    if let Some(message) = close {
                        // Best-effort: a dropped close on backpressure just
                        // leaves Session's queued/in-flight command running
                        // a little longer, same as any other slow reply.
                        let _ = common::ipc_send_handle(session_input_capability, &message);
                    }
                    continue;
                }
                if let Some(message) = terminal.input(&event.input) {
                    match common::ipc_send_handle(session_input_capability, &message) {
                        IpcStatus::Ok => {}
                        IpcStatus::Full => {
                            *pending_session_input = Some(message);
                            break;
                        }
                        IpcStatus::Stale
                        | IpcStatus::Disconnected
                        | IpcStatus::Unauthorized
                        | IpcStatus::Malformed
                        | IpcStatus::Empty => {}
                    }
                }
            }
            if pending_session_input.is_none() {}
        }
        let mut message = IpcBytes::empty(MessageKind::SessionOutput);
        while common::ipc_receive_handle(session_output_capability, &mut message) == IpcStatus::Ok {
            terminal.session_output(&message);
        }
        // Waits time out every `WAIT_TIMEOUT_TICKS`, which paces the blink.
        terminal.blink(common::current_ticks());
        if pending_render.is_none() {
            *pending_render = terminal.next_grid_row();
        }
        if let Some(mut row) = *pending_render {
            if !terminal_surface.is_valid() {
                common::wait_on_capabilities(&[
                    input_capability,
                    session_input_capability,
                    session_output_capability,
                    atrium_render_capability,
                    atrium_surface_request_capability,
                    atrium_surface_response_capability,
                ]);
                continue;
            }
            // `node_id` is left at 0: only Atrium knows this surface's own
            // text-grid node id, and fills it in before relaying the row
            // to Display (#74).
            row.surface = terminal_surface;
            match common::ipc_send_handle(atrium_render_capability, &row) {
                IpcStatus::Ok => {
                    *pending_render = terminal.next_grid_row();
                }
                IpcStatus::Full => *pending_render = Some(row),
                IpcStatus::Stale
                | IpcStatus::Disconnected
                | IpcStatus::Unauthorized
                | IpcStatus::Malformed
                | IpcStatus::Empty => {
                    *pending_render = None;
                }
            }
        }
        if pending_render.is_some() {
            continue;
        }
        common::wait_on_capabilities(&[
            input_capability,
            session_input_capability,
            session_output_capability,
            atrium_render_capability,
            atrium_surface_request_capability,
            atrium_surface_response_capability,
        ]);
    }
}

/// Left-button-down on the tab strip opens/closes/switches a tab instead of
/// reaching the shell (#76). Pointer coordinates arrive already local to
/// this surface (Atrium translates them before forwarding), so hit-testing
/// against a zero-origin rect the size of the surface matches exactly what
/// Atrium drew at `surface.x/y + ...` for the same bounds.
/// `None` means the click wasn't on the tab bar at all; `Some(_)` means it
/// was handled, optionally carrying a `SessionClose` message (T3c, #98)
/// for a closed tab's session to be forwarded to Session.
fn handle_tab_bar_click(
    terminal: &mut logos_terminal::TerminalService,
    bounds: GuiRect,
    input: &InputMessage,
) -> Option<Option<IpcBytes>> {
    let pointer = input.pointer_event()?;
    if pointer.state != PointerState::Down || pointer.buttons & 1 == 0 {
        return None;
    }
    let local_bounds = GuiRect::new(0, 0, bounds.width, bounds.height);
    match terminal_tab_bar_hit(local_bounds, i32::from(pointer.x), i32::from(pointer.y))? {
        TerminalTabHit::Tab(slot) => {
            if let Some(handle) = terminal.tab_at(slot) {
                terminal.switch_tab(handle);
            }
            Some(None)
        }
        TerminalTabHit::Close(slot) => {
            let close = terminal.tab_at(slot).and_then(|handle| terminal.close_tab(handle));
            Some(close)
        }
        TerminalTabHit::AddTab => {
            terminal.open_tab();
            Some(None)
        }
    }
}

#[cfg(target_os = "none")]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo<'_>) -> ! {
    common::idle()
}

#[cfg(not(target_os = "none"))]
fn main() {}
