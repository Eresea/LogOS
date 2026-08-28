#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", no_main)]
#![cfg_attr(not(target_os = "none"), allow(dead_code, unused_imports, unused_variables))]

mod common;

use logos_abi::{
    DISPLAY_CONFIG_BASE, DISPLAY_FRAMEBUFFER_BASE, FramebufferConfig, FramebufferFormat, GuiRect,
    GuiSceneOp, GuiStatus, GuiSurfaceOperation, GuiSurfaceRequest, GuiSurfaceResponse, IpcStatus,
    MessageKind, RENDER_FLAG_MORE, RenderMessage, SurfaceHandle,
};
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

fn render(
    display: &mut logos_display::Display,
    framebuffer: &mut [u8],
    config: &FramebufferConfig,
) {
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
}

static mut DISPLAY: logos_display::Display = logos_display::Display::new(1);

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    common::init_service_allocator();
    let display = unsafe { &mut *core::ptr::addr_of_mut!(DISPLAY) };
    let config = unsafe { &*(DISPLAY_CONFIG_BASE as *const FramebufferConfig) };
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
    #[cfg(feature = "qemu-proof")]
    let _ = common::ipc_probe(logos_abi::IPC_SYSCALL_SEND, 0, 0);
    let mut heartbeat_ticks = 0u16;
    let mut render_pending = false;
    let mut render_complete = false;
    let mut gui_render_pending = false;
    let mut gui_dirty = false;
    loop {
        if render_pending && render_complete || gui_dirty || gui_render_pending {
            // Rendering is intentionally resumed across loop iterations, but it
            // still keeps the Display task busy and must report health directly.
            common::heartbeat();
        } else {
            common::heartbeat_tick(&mut heartbeat_ticks);
        }
        let generation = common::bootstrap_page().service.generation() as u16;
        if display.generation() != generation {
            display.replace_generation(generation);
            let mut root_request = GuiSurfaceRequest::new(GuiSurfaceOperation::CreateRoot, 1);
            root_request.bounds = GuiRect::new(0, 0, config.width, config.height);
            let _ = display.gui_mut().create(11, root_request);
            render_pending = false;
            render_complete = false;
            gui_render_pending = false;
            gui_dirty = true;
        }
        let mut progressed = false;
        let mut surface_request = GuiSurfaceRequest::new(GuiSurfaceOperation::Update, 1);
        while common::ipc_receive_handle(surface_capability, &mut surface_request) == IpcStatus::Ok
        {
            progressed = true;
            let mut response = GuiSurfaceResponse::new(surface_request, GuiStatus::Malformed);
            let result = match surface_request.operation {
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
                GuiSurfaceOperation::Focus => display.gui_mut().focus(13, surface_request.surface),
                GuiSurfaceOperation::Destroy => {
                    display.gui_mut().destroy(13, surface_request.surface)
                }
            };
            response.status = match result {
                Ok(()) => GuiStatus::Ok,
                Err(error) => gui_status(error),
            };
            let _ = common::ipc_send_handle(surface_response_capability, &response);
            gui_dirty = true;
        }
        let mut lockscreen_surface_request = GuiSurfaceRequest::new(GuiSurfaceOperation::Update, 1);
        while common::ipc_receive_handle(
            lockscreen_surface_capability,
            &mut lockscreen_surface_request,
        ) == IpcStatus::Ok
        {
            progressed = true;
            let mut response =
                GuiSurfaceResponse::new(lockscreen_surface_request, GuiStatus::Malformed);
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
            };
            response.status = match result {
                Ok(()) => GuiStatus::Ok,
                Err(error) => gui_status(error),
            };
            let _ = common::ipc_send_handle(lockscreen_response_capability, &response);
            gui_dirty = true;
        }
        let mut message = RenderMessage::empty(MessageKind::RenderCells);
        while common::ipc_receive_handle(input_capability, &mut message) == IpcStatus::Ok {
            progressed = true;
            if display.apply(generation, &message).is_ok() {
                let more = message.flags & RENDER_FLAG_MORE != 0;
                render_pending = true;
                render_complete = !more;
            }
        }
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
        let mut gui_op = GuiSceneOp::clear(SurfaceHandle::new(0, 1, 11).unwrap(), 1);
        while common::ipc_receive_handle(gui_capability, &mut gui_op) == IpcStatus::Ok {
            progressed = true;
            let _ = display.gui_mut().apply_scene_op(11, gui_op);
            gui_dirty = true;
            gui_op = GuiSceneOp::clear(SurfaceHandle::new(0, 1, 11).unwrap(), 1);
        }
        let mut atrium_op = GuiSceneOp::clear(SurfaceHandle::new(0, 1, 13).unwrap(), 1);
        while common::ipc_receive_handle(atrium_gui_capability, &mut atrium_op) == IpcStatus::Ok {
            progressed = true;
            let _ = display.gui_mut().apply_scene_op(13, atrium_op);
            gui_dirty = true;
            atrium_op = GuiSceneOp::clear(SurfaceHandle::new(0, 1, 13).unwrap(), 1);
        }
        let mut lockscreen_op = GuiSceneOp::clear(SurfaceHandle::new(0, 1, 12).unwrap(), 1);
        while common::ipc_receive_handle(lockscreen_gui_capability, &mut lockscreen_op)
            == IpcStatus::Ok
        {
            progressed = true;
            let _ = display.gui_mut().apply_scene_op(12, lockscreen_op);
            gui_dirty = true;
            lockscreen_op = GuiSceneOp::clear(SurfaceHandle::new(0, 1, 12).unwrap(), 1);
        }
        if render_pending && render_complete && !gui_render_pending {
            render(display, framebuffer, config);
            if display.gui().terminal_bounds().is_some() {
                gui_dirty = true;
            }
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
            gui_render_pending = display.render_pending();
            gui_dirty = false;
            progressed = true;
        }
        if !progressed {
            common::wait_on_capabilities(&[
                input_capability,
                atrium_render_capability,
                gui_capability,
                atrium_gui_capability,
                surface_capability,
                lockscreen_gui_capability,
                lockscreen_surface_capability,
            ]);
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
