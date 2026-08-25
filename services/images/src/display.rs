#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", no_main)]
#![cfg_attr(not(target_os = "none"), allow(dead_code, unused_imports, unused_variables))]

mod common;

use logos_abi::{
    DISPLAY_CONFIG_BASE, DISPLAY_FRAMEBUFFER_BASE, FramebufferConfig, FramebufferFormat,
    GuiDrawBatch, GuiRect, GuiSurfaceOperation, GuiSurfaceRequest, IpcStatus, MessageKind,
    RENDER_FLAG_MORE, RenderMessage,
};
const INPUT_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_RENDER,
    b"terminal",
    core::mem::size_of::<RenderMessage>(),
    logos_abi::IpcRights::Receive,
);
const GUI_CAPABILITY: common::CapabilitySpec = common::capability_contract_named(
    logos_abi::IPC_CONTRACT_GUI_DRAW,
    b"shell",
    core::mem::size_of::<GuiDrawBatch>(),
    logos_abi::IpcRights::Receive,
);

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
    let gui_capability = match common::capability_handle(GUI_CAPABILITY) {
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
    loop {
        common::heartbeat_tick(&mut heartbeat_ticks);
        let generation = common::bootstrap_page().service.generation() as u16;
        if display.generation() != generation {
            display.replace_generation(generation);
            let mut root_request = GuiSurfaceRequest::new(GuiSurfaceOperation::CreateRoot, 1);
            root_request.bounds = GuiRect::new(0, 0, config.width, config.height);
            let _ = display.gui_mut().create(11, root_request);
            render_pending = false;
            render_complete = false;
            gui_render_pending = false;
        }
        let mut progressed = false;
        let mut message = RenderMessage::empty(MessageKind::RenderCells);
        while common::ipc_receive_handle(input_capability, &mut message) == IpcStatus::Ok {
            progressed = true;
            if display.apply(generation, &message).is_ok() {
                let more = message.flags & RENDER_FLAG_MORE != 0;
                render_pending = true;
                render_complete = !more;
            }
        }
        let mut gui_batch = GuiDrawBatch::new(
            logos_abi::SurfaceHandle::new(0, 1, 11).unwrap(),
            1,
            GuiRect::new(0, 0, config.width, config.height),
        );
        while common::ipc_receive_handle(gui_capability, &mut gui_batch) == IpcStatus::Ok {
            progressed = true;
            let _ = display.gui_mut().update(11, gui_batch);
            let _ = display.render_gui(
                framebuffer,
                config.width as usize,
                config.height as usize,
                config.stride as usize * 4,
                format,
            );
            gui_render_pending = display.render_pending();
        }
        if render_pending && render_complete && !gui_render_pending {
            render(display, framebuffer, config);
            render_pending = false;
        }
        if gui_render_pending {
            let _ = display.render_gui(
                framebuffer,
                config.width as usize,
                config.height as usize,
                config.stride as usize * 4,
                format,
            );
            gui_render_pending = display.render_pending();
            progressed = true;
        }
        if !progressed {
            common::wait_on_capabilities(&[input_capability, gui_capability]);
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
