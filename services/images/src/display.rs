#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", no_main)]
#![cfg_attr(not(target_os = "none"), allow(dead_code, unused_imports, unused_variables))]

mod common;

use logos_abi::{
    DISPLAY_CONFIG_BASE, DISPLAY_FRAMEBUFFER_BASE, FramebufferConfig, FramebufferFormat, IpcStatus,
    MessageKind, RENDER_FLAG_MORE, RenderMessage,
};
const INPUT_CAPABILITY: common::CapabilitySpec = common::capability_contract(
    logos_abi::IPC_CONTRACT_RENDER,
    logos_abi::ServiceId::Terminal.index() as u32,
    core::mem::size_of::<RenderMessage>(),
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
    #[cfg(feature = "qemu-proof")]
    let _ = common::ipc_probe(logos_abi::IPC_SYSCALL_SEND, 0, 0);
    let mut heartbeat_ticks = 0u16;
    let mut render_pending = false;
    let mut render_complete = false;
    loop {
        common::heartbeat_tick(&mut heartbeat_ticks, logos_abi::ServiceId::Display);
        let generation = common::bootstrap_page().service.generation() as u16;
        if display.generation() != generation {
            display.replace_generation(generation);
            render_pending = false;
            render_complete = false;
        }
        let mut progressed = false;
        let mut message = RenderMessage::empty(MessageKind::RenderCells);
        while common::ipc_receive(INPUT_CAPABILITY, &mut message) == IpcStatus::Ok {
            progressed = true;
            if display.apply(generation, &message).is_ok() {
                let more = message.flags & RENDER_FLAG_MORE != 0;
                render_pending = true;
                render_complete = !more;
            }
        }
        if render_pending && render_complete {
            render(display, framebuffer, config);
            render_pending = false;
        }
        if !progressed {
            if !render_pending && display.toggle_cursor() {
                render(display, framebuffer, config);
            }
            common::wait(
                common::ipc_read_event(logos_abi::IpcEndpointId::TerminalToDisplay),
                logos_abi::ServiceId::Display,
            );
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
