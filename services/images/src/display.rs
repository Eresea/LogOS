#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", no_main)]
#![cfg_attr(not(target_os = "none"), allow(dead_code, unused_imports, unused_variables))]

mod common;

use logos_abi::{
    DISPLAY_CONFIG_BASE, DISPLAY_FRAMEBUFFER_BASE, DISPLAY_PRESENT_BASE, FramebufferConfig,
    FramebufferFormat, FramebufferPresentState, GuiDrawCommand, GuiDrawKind, GuiRect, GuiSceneOp,
    GuiStatus, GuiSurfaceOperation, GuiSurfaceRequest, GuiSurfaceResponse, IpcStatus, MessageKind,
    RENDER_FLAG_MORE, RenderMessage, SurfaceHandle,
};
const FPS_SURFACE_BOUNDS: GuiRect = GuiRect::new(8, 8, 72, 24);
const FPS_NODE_ID: u32 = u32::MAX - 1;
const FPS_Z_ORDER: i16 = i16::MAX;
const FPS_WINDOW_TICKS: u64 = logos_abi::SERVICE_HEARTBEAT_INTERVAL_TICKS;
const READY_SURFACE: usize = 1 << 0;
const READY_LOCKSCREEN_SURFACE: usize = 1 << 1;
const READY_INPUT: usize = 1 << 2;
const READY_ATRIUM_RENDER: usize = 1 << 3;
const READY_GUI: usize = 1 << 4;
const READY_ATRIUM_GUI: usize = 1 << 5;
const READY_LOCKSCREEN_GUI: usize = 1 << 6;
const READY_ALL: usize = (1 << 7) - 1;

struct FpsCounter {
    window_start: Option<u64>,
    frames: u16,
    value: u16,
}

impl FpsCounter {
    const fn new() -> Self {
        Self { window_start: None, frames: 0, value: 0 }
    }

    fn record(&mut self, ticks: u64) -> bool {
        self.window_start.get_or_insert(ticks);
        self.frames = self.frames.saturating_add(1);
        self.refresh(ticks)
    }

    fn refresh(&mut self, ticks: u64) -> bool {
        let Some(window_start) = self.window_start else {
            return false;
        };
        if ticks.wrapping_sub(window_start) < FPS_WINDOW_TICKS {
            return false;
        }
        self.value = self.frames;
        self.window_start = Some(ticks);
        self.frames = 0;
        true
    }
}
const INPUT_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_RENDER,
    b"terminal",
    core::mem::size_of::<RenderMessage>(),
    logos_abi::IpcRights::Receive,
);
const ATRIUM_RENDER_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_RENDER,
    b"atrium",
    core::mem::size_of::<RenderMessage>(),
    logos_abi::IpcRights::Receive,
);
const GUI_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_GUI_DRAW,
    b"shell",
    core::mem::size_of::<GuiSceneOp>(),
    logos_abi::IpcRights::Receive,
);
const ATRIUM_GUI_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_GUI_DRAW,
    b"atrium",
    core::mem::size_of::<GuiSceneOp>(),
    logos_abi::IpcRights::Receive,
);
const LOCKSCREEN_GUI_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_GUI_DRAW,
    b"lockscreen",
    core::mem::size_of::<GuiSceneOp>(),
    logos_abi::IpcRights::Receive,
);
const SURFACE_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_GUI_SURFACE,
    b"atrium",
    core::mem::size_of::<GuiSurfaceRequest>(),
    logos_abi::IpcRights::Receive,
);
const SURFACE_RESPONSE_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_GUI_SURFACE,
    b"atrium",
    core::mem::size_of::<logos_abi::GuiSurfaceResponse>(),
    logos_abi::IpcRights::Send,
);
const LOCKSCREEN_SURFACE_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_GUI_SURFACE,
    b"lockscreen",
    core::mem::size_of::<GuiSurfaceRequest>(),
    logos_abi::IpcRights::Receive,
);
const LOCKSCREEN_RESPONSE_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_GUI_SURFACE,
    b"lockscreen",
    core::mem::size_of::<logos_abi::GuiSurfaceResponse>(),
    logos_abi::IpcRights::Send,
);

fn gui_status(error: logos_display::GuiRegistryError) -> logos_abi::GuiStatus {
    match error {
        logos_display::GuiRegistryError::Stale => logos_abi::GuiStatus::Stale,
        logos_display::GuiRegistryError::Capacity => logos_abi::GuiStatus::Capacity,
        logos_display::GuiRegistryError::Unauthorized => logos_abi::GuiStatus::Unauthorized,
        logos_display::GuiRegistryError::Backpressure => logos_abi::GuiStatus::Backpressure,
        logos_display::GuiRegistryError::NotFound => logos_abi::GuiStatus::NotFound,
        logos_display::GuiRegistryError::InvalidRequest
        | logos_display::GuiRegistryError::Malformed => logos_abi::GuiStatus::Malformed,
    }
}

fn is_cursor_surface_request(
    request: GuiSurfaceRequest,
    owner: u32,
    width: u32,
    height: u32,
) -> bool {
    let expected_z = match owner {
        12 => 2,
        13 => 3,
        _ => return false,
    };
    request.operation == GuiSurfaceOperation::CreateModal
        && request.bounds == GuiRect::new(0, 0, width, height)
        && request.z_order == expected_z
}

fn fps_command(surface: SurfaceHandle, frame: u32, fps: u16) -> GuiSceneOp {
    let value = fps;
    let mut text = *b"FPS:000";
    text[4] = b'0' + (value / 100) as u8;
    text[5] = b'0' + ((value / 10) % 10) as u8;
    text[6] = b'0' + (value % 10) as u8;
    let mut command = GuiDrawCommand::empty(GuiDrawKind::GlyphRun);
    command.x = FPS_SURFACE_BOUNDS.x + 4;
    command.y = FPS_SURFACE_BOUNDS.y + 4;
    command.color = 0xf0f6fc;
    command.text_len = text.len() as u8;
    command.text[..text.len()].copy_from_slice(&text);
    GuiSceneOp::upsert(surface, frame, FPS_NODE_ID, command)
}

fn update_fps_surface(
    display: &mut logos_display::Display,
    surface: SurfaceHandle,
    frame: &mut u32,
    fps: u16,
) {
    if !surface.is_valid() {
        return;
    }
    *frame = frame.wrapping_add(1).max(1);
    let _ = display.gui_mut().apply_scene_op(11, fps_command(surface, *frame, fps));
}

fn create_fps_surface(display: &mut logos_display::Display) -> SurfaceHandle {
    let mut request = GuiSurfaceRequest::new(GuiSurfaceOperation::CreateModal, 2);
    request.bounds = FPS_SURFACE_BOUNDS;
    request.z_order = FPS_Z_ORDER;
    display
        .gui_mut()
        .create(11, request)
        .map(|response| response.surface)
        .unwrap_or(SurfaceHandle::EMPTY)
}

#[allow(clippy::too_many_arguments)]
fn render(
    display: &mut logos_display::Display,
    framebuffer: &mut [u8],
    config: &FramebufferConfig,
    present_state: &FramebufferPresentState,
    fps: &mut FpsCounter,
    fps_surface: SurfaceHandle,
    fps_scene_frame: &mut u32,
    fps_enabled: bool,
) -> bool {
    let format = match config.format {
        FramebufferFormat::Bgr8 => logos_display::PixelFormat::Bgr8,
        FramebufferFormat::Rgb8 => logos_display::PixelFormat::Rgb8,
    };
    let _ = display.render(
        framebuffer,
        config.width as usize,
        config.height as usize,
        config.stride as usize * 4,
        format,
    );
    common::heartbeat();
    publish_render(display, present_state, fps, fps_surface, fps_scene_frame, true, fps_enabled)
}

fn publish_render(
    display: &mut logos_display::Display,
    present_state: &FramebufferPresentState,
    fps: &mut FpsCounter,
    fps_surface: SurfaceHandle,
    fps_scene_frame: &mut u32,
    complete: bool,
    fps_enabled: bool,
) -> bool {
    let presented = display.presented();
    let fps_changed = fps_enabled && presented && complete && fps.record(common::current_ticks());
    if fps_changed {
        update_fps_surface(display, fps_surface, fps_scene_frame, fps.value);
    }
    publish_present(display, present_state);
    fps_changed
}

fn publish_present(display: &mut logos_display::Display, present_state: &FramebufferPresentState) {
    let (full, rects, count) = display.take_presented_damage();
    if full || count != 0 {
        present_state.publish(full, &rects[..count]);
    }
}

fn publish_cursor(
    display: &logos_display::Display,
    present_state: &FramebufferPresentState,
    published: &mut Option<(bool, i16, i16)>,
) {
    let (visible, x, y) =
        display.cursor_position().map(|(x, y)| (true, x, y)).unwrap_or((false, 0, 0));
    if *published == Some((visible, x, y)) {
        return;
    }
    present_state.publish_cursor(visible, x, y);
    *published = Some((visible, x, y));
    #[cfg(feature = "qemu-proof")]
    common::proof_line(b"LogOS vNext: Display cursor published");
}

static mut DISPLAY: logos_display::Display = logos_display::Display::new(1);

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    common::init_service_allocator();
    let display = unsafe { &mut *core::ptr::addr_of_mut!(DISPLAY) };
    let config = unsafe { &*(DISPLAY_CONFIG_BASE as *const FramebufferConfig) };
    let present_state = unsafe { &*(DISPLAY_PRESENT_BASE as *const FramebufferPresentState) };
    let framebuffer = unsafe {
        core::slice::from_raw_parts_mut(DISPLAY_FRAMEBUFFER_BASE as *mut u8, config.bytes as usize)
    };
    let format = match config.format {
        FramebufferFormat::Bgr8 => logos_display::PixelFormat::Bgr8,
        FramebufferFormat::Rgb8 => logos_display::PixelFormat::Rgb8,
    };
    let input_capability = match common::capability_handle(INPUT_CAPABILITY) {
        Ok(capability) => capability,
        Err(_) => common::idle(),
    };
    let atrium_render_capability = match common::capability_handle(ATRIUM_RENDER_CAPABILITY) {
        Ok(capability) => capability,
        Err(_) => common::idle(),
    };
    let gui_capability = match common::capability_handle(GUI_CAPABILITY) {
        Ok(capability) => capability,
        Err(_) => common::idle(),
    };
    let atrium_gui_capability = match common::capability_handle(ATRIUM_GUI_CAPABILITY) {
        Ok(capability) => capability,
        Err(_) => common::idle(),
    };
    let lockscreen_gui_capability = match common::capability_handle(LOCKSCREEN_GUI_CAPABILITY) {
        Ok(capability) => capability,
        Err(_) => common::idle(),
    };
    let surface_capability = match common::capability_handle(SURFACE_CAPABILITY) {
        Ok(capability) => capability,
        Err(_) => common::idle(),
    };
    let surface_response_capability = match common::capability_handle(SURFACE_RESPONSE_CAPABILITY) {
        Ok(capability) => capability,
        Err(_) => common::idle(),
    };
    let lockscreen_surface_capability =
        match common::capability_handle(LOCKSCREEN_SURFACE_CAPABILITY) {
            Ok(capability) => capability,
            Err(_) => common::idle(),
        };
    let lockscreen_response_capability =
        match common::capability_handle(LOCKSCREEN_RESPONSE_CAPABILITY) {
            Ok(capability) => capability,
            Err(_) => common::idle(),
        };
    let mut root_request = GuiSurfaceRequest::new(GuiSurfaceOperation::CreateRoot, 1);
    root_request.bounds = GuiRect::new(0, 0, config.width, config.height);
    let _ = display.gui_mut().create(11, root_request);
    let mut fps_surface = create_fps_surface(display);
    let mut fps_scene_frame = 0;
    update_fps_surface(display, fps_surface, &mut fps_scene_frame, 0);
    #[cfg(feature = "qemu-proof")]
    let _ = common::ipc_probe(logos_abi::IPC_SYSCALL_SEND, 0, 0);
    let mut heartbeat_ticks = 0u16;
    let mut render_pending = false;
    let mut render_complete = false;
    let mut gui_render_pending = false;
    let mut gui_dirty = true;
    let mut fps_enabled = true;
    let mut fps = FpsCounter::new();
    let mut published_cursor = None;
    let mut ready_mask = READY_ALL;
    loop {
        if render_pending && render_complete || gui_dirty || gui_render_pending {
            // Rendering is intentionally resumed across loop iterations, but it
            // still keeps the Display task busy and must report health directly.
            common::heartbeat();
        } else {
            common::heartbeat_tick(&mut heartbeat_ticks);
        }
        display.set_hardware_cursor(present_state.hardware_cursor());
        let generation = common::bootstrap_page().service.generation() as u16;
        if display.generation() != generation {
            display.replace_generation(generation);
            let mut root_request = GuiSurfaceRequest::new(GuiSurfaceOperation::CreateRoot, 1);
            root_request.bounds = GuiRect::new(0, 0, config.width, config.height);
            let _ = display.gui_mut().create(11, root_request);
            fps_surface = create_fps_surface(display);
            render_pending = false;
            render_complete = false;
            gui_render_pending = false;
            gui_dirty = true;
            fps = FpsCounter::new();
            fps_scene_frame = 0;
            if fps_enabled {
                update_fps_surface(display, fps_surface, &mut fps_scene_frame, 0);
            }
        }
        if fps_enabled && fps.refresh(common::current_ticks()) {
            update_fps_surface(display, fps_surface, &mut fps_scene_frame, fps.value);
            gui_dirty = true;
        }
        let mut progressed = false;
        if ready_mask & READY_SURFACE != 0 {
            let mut surface_request = GuiSurfaceRequest::new(GuiSurfaceOperation::Update, 1);
            while common::ipc_receive_handle(surface_capability, &mut surface_request)
                == IpcStatus::Ok
            {
                progressed = true;
                let mut response = GuiSurfaceResponse::new(surface_request, GuiStatus::Malformed);
                let cursor_request =
                    is_cursor_surface_request(surface_request, 13, config.width, config.height);
                let fps_toggle = surface_request.operation == GuiSurfaceOperation::ToggleFps;
                let cursor_destroy = display.is_cursor_surface(surface_request.surface);
                let result = match surface_request.operation {
                    GuiSurfaceOperation::ToggleFps => Ok(()),
                    GuiSurfaceOperation::CreateRoot | GuiSurfaceOperation::CreateModal => {
                        display.gui_mut().create(13, surface_request).map(|created| {
                            response.surface = created.surface;
                        })
                    }
                    GuiSurfaceOperation::Update => display.gui_mut().set_bounds(
                        13,
                        surface_request.surface,
                        surface_request.bounds,
                    ),
                    GuiSurfaceOperation::Focus => {
                        display.gui_mut().focus(13, surface_request.surface)
                    }
                    GuiSurfaceOperation::Destroy => {
                        display.gui_mut().destroy(13, surface_request.surface)
                    }
                };
                response.status = match result {
                    Ok(()) => GuiStatus::Ok,
                    Err(error) => gui_status(error),
                };
                if result.is_ok() && fps_toggle {
                    fps_enabled = !fps_enabled;
                    fps_scene_frame = fps_scene_frame.wrapping_add(1).max(1);
                    let op = if fps_enabled {
                        fps_command(fps_surface, fps_scene_frame, fps.value)
                    } else {
                        GuiSceneOp::remove(fps_surface, fps_scene_frame, FPS_NODE_ID)
                    };
                    let _ = display.gui_mut().apply_scene_op(11, op);
                }
                if result.is_ok() && cursor_request && response.surface.is_valid() {
                    display.register_cursor_surface(13, response.surface);
                } else if result.is_ok() && cursor_destroy {
                    display.unregister_cursor_surface(surface_request.surface);
                }
                let _ = common::ipc_send_handle(surface_response_capability, &response);
                gui_dirty = true;
            }
            ready_mask &= !READY_SURFACE;
        }
        if ready_mask & READY_LOCKSCREEN_SURFACE != 0 {
            let mut lockscreen_surface_request =
                GuiSurfaceRequest::new(GuiSurfaceOperation::Update, 1);
            while common::ipc_receive_handle(
                lockscreen_surface_capability,
                &mut lockscreen_surface_request,
            ) == IpcStatus::Ok
            {
                progressed = true;
                let mut response =
                    GuiSurfaceResponse::new(lockscreen_surface_request, GuiStatus::Malformed);
                let cursor_request = is_cursor_surface_request(
                    lockscreen_surface_request,
                    12,
                    config.width,
                    config.height,
                );
                let cursor_destroy = display.is_cursor_surface(lockscreen_surface_request.surface);
                let result = match lockscreen_surface_request.operation {
                    GuiSurfaceOperation::CreateRoot | GuiSurfaceOperation::CreateModal => {
                        display.gui_mut().create(12, lockscreen_surface_request).map(|created| {
                            response.surface = created.surface;
                        })
                    }
                    GuiSurfaceOperation::Update => display.gui_mut().set_bounds(
                        12,
                        lockscreen_surface_request.surface,
                        lockscreen_surface_request.bounds,
                    ),
                    GuiSurfaceOperation::Focus => {
                        display.gui_mut().focus(12, lockscreen_surface_request.surface)
                    }
                    GuiSurfaceOperation::Destroy => {
                        display.gui_mut().destroy(12, lockscreen_surface_request.surface)
                    }
                    GuiSurfaceOperation::ToggleFps => {
                        Err(logos_display::GuiRegistryError::InvalidRequest)
                    }
                };
                response.status = match result {
                    Ok(()) => GuiStatus::Ok,
                    Err(error) => gui_status(error),
                };
                if result.is_ok() && cursor_request && response.surface.is_valid() {
                    display.register_cursor_surface(12, response.surface);
                } else if result.is_ok() && cursor_destroy {
                    display.unregister_cursor_surface(lockscreen_surface_request.surface);
                }
                let _ = common::ipc_send_handle(lockscreen_response_capability, &response);
                gui_dirty = true;
            }
            ready_mask &= !READY_LOCKSCREEN_SURFACE;
        }
        if ready_mask & READY_INPUT != 0 {
            let mut message = RenderMessage::empty(MessageKind::RenderCells);
            while common::ipc_receive_handle(input_capability, &mut message) == IpcStatus::Ok {
                progressed = true;
                if display.apply(generation, &message).is_ok() {
                    let more = message.flags & RENDER_FLAG_MORE != 0;
                    render_pending = true;
                    render_complete = !more;
                }
            }
            ready_mask &= !READY_INPUT;
        }
        if ready_mask & READY_GUI != 0 {
            let mut gui_op = GuiSceneOp::clear(SurfaceHandle::new(0, 1, 11).unwrap(), 1);
            while common::ipc_receive_handle(gui_capability, &mut gui_op) == IpcStatus::Ok {
                progressed = true;
                let _ = display.gui_mut().apply_scene_op(11, gui_op);
                gui_dirty = true;
                gui_op = GuiSceneOp::clear(SurfaceHandle::new(0, 1, 11).unwrap(), 1);
            }
            ready_mask &= !READY_GUI;
        }
        let mut cursor_presented = false;
        let mut cursor_activity = false;
        // Poll cursor IPC independently so a busy render producer cannot delay motion.
        {
            let mut atrium_op = GuiSceneOp::clear(SurfaceHandle::new(0, 1, 13).unwrap(), 1);
            let mut cursor_dirty = false;
            while common::ipc_receive_handle(atrium_gui_capability, &mut atrium_op) == IpcStatus::Ok
            {
                progressed = true;
                if display.apply_cursor_scene_op(atrium_op) {
                    cursor_dirty = true;
                } else {
                    let _ = display.gui_mut().apply_scene_op(13, atrium_op);
                    gui_dirty = true;
                }
                atrium_op = GuiSceneOp::clear(SurfaceHandle::new(0, 1, 13).unwrap(), 1);
            }
            if cursor_dirty {
                cursor_activity = true;
                let painted = display.repaint_cursor(
                    framebuffer,
                    config.width as usize,
                    config.height as usize,
                    config.stride as usize * 4,
                    format,
                );
                cursor_presented = painted;
                if !painted && !display.hardware_cursor_enabled() {
                    gui_dirty = true;
                }
            }
        }
        {
            let mut lockscreen_op = GuiSceneOp::clear(SurfaceHandle::new(0, 1, 12).unwrap(), 1);
            let mut cursor_dirty = false;
            while common::ipc_receive_handle(lockscreen_gui_capability, &mut lockscreen_op)
                == IpcStatus::Ok
            {
                progressed = true;
                if display.apply_cursor_scene_op(lockscreen_op) {
                    cursor_dirty = true;
                } else {
                    let _ = display.gui_mut().apply_scene_op(12, lockscreen_op);
                    gui_dirty = true;
                }
                lockscreen_op = GuiSceneOp::clear(SurfaceHandle::new(0, 1, 12).unwrap(), 1);
            }
            if cursor_dirty {
                cursor_activity = true;
                let painted = display.repaint_cursor(
                    framebuffer,
                    config.width as usize,
                    config.height as usize,
                    config.stride as usize * 4,
                    format,
                );
                cursor_presented |= painted;
                if !painted && !display.hardware_cursor_enabled() {
                    gui_dirty = true;
                }
            }
        }
        if cursor_presented {
            publish_present(display, present_state);
        }
        if cursor_activity {
            publish_cursor(display, present_state, &mut published_cursor);
        }
        ready_mask &= !(READY_ATRIUM_GUI | READY_LOCKSCREEN_GUI);
        if ready_mask & READY_ATRIUM_RENDER != 0 {
            let mut atrium_render = RenderMessage::empty(MessageKind::RenderCells);
            while common::ipc_receive_handle(atrium_render_capability, &mut atrium_render)
                == IpcStatus::Ok
            {
                progressed = true;
                if display.apply(generation, &atrium_render).is_ok() {
                    let more = atrium_render.flags & RENDER_FLAG_MORE != 0;
                    render_pending = true;
                    render_complete = !more;
                }
            }
            ready_mask &= !READY_ATRIUM_RENDER;
        }
        if render_pending && render_complete && !gui_render_pending {
            let fps_changed = render(
                display,
                framebuffer,
                config,
                present_state,
                &mut fps,
                fps_surface,
                &mut fps_scene_frame,
                fps_enabled,
            );
            if display.gui().terminal_bounds().is_some() {
                gui_dirty = true;
            }
            gui_dirty |= fps_changed;
            render_pending = false;
        }
        if gui_dirty || gui_render_pending {
            let _ = display.render_gui(
                framebuffer,
                config.width as usize,
                config.height as usize,
                config.stride as usize * 4,
                format,
            );
            let complete = !display.render_pending();
            let fps_changed = publish_render(
                display,
                present_state,
                &mut fps,
                fps_surface,
                &mut fps_scene_frame,
                complete,
                fps_enabled,
            );
            gui_render_pending = display.render_pending();
            gui_dirty = fps_changed;
            progressed = true;
            // Keep cursor motion flowing between bounded GUI slices without
            // repeatedly draining render producers while the display is busy.
            ready_mask = READY_ATRIUM_GUI | READY_LOCKSCREEN_GUI;
        }
        if progressed {
            publish_cursor(display, present_state, &mut published_cursor);
        }
        if !progressed {
            let capabilities = [
                surface_capability,
                lockscreen_surface_capability,
                input_capability,
                atrium_render_capability,
                gui_capability,
                atrium_gui_capability,
                lockscreen_gui_capability,
            ];
            ready_mask = common::wait_on_capabilities_ready(&capabilities)
                .and_then(|ready| {
                    capabilities
                        .iter()
                        .position(|capability| *capability == ready)
                        .map(|index| 1usize << index)
                })
                .unwrap_or(READY_ALL);
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
