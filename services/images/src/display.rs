#![no_std]
#![no_main]

mod common;

use logos_abi::{
    DISPLAY_CONFIG_BASE, DISPLAY_FRAMEBUFFER_BASE, FramebufferConfig, FramebufferFormat, IpcStatus,
    MessageKind, RenderMessage,
};
const INPUT_CAPABILITY: usize = 0;

static mut DISPLAY: logos_display::Display = logos_display::Display::new(1);

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    let display = unsafe { &mut *core::ptr::addr_of_mut!(DISPLAY) };
    let config = unsafe { &*(DISPLAY_CONFIG_BASE as *const FramebufferConfig) };
    let framebuffer = unsafe {
        core::slice::from_raw_parts_mut(DISPLAY_FRAMEBUFFER_BASE as *mut u8, config.bytes as usize)
    };
    let mut heartbeat_ticks = 0u16;
    loop {
        common::heartbeat_tick(&mut heartbeat_ticks, logos_abi::ServiceId::Display);
        let generation = common::capability(INPUT_CAPABILITY)
            .map(|capability| capability.generation)
            .unwrap_or(0);
        if display.generation() != generation {
            display.replace_generation(generation);
        }
        let mut progressed = false;
        let mut message = RenderMessage::empty(MessageKind::RenderCells);
        while common::ipc_receive(INPUT_CAPABILITY, &mut message) == IpcStatus::Ok {
            progressed = true;
            if display.apply(generation, &message).is_ok() {
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
        }
        if !progressed {
            common::wait(common::ipc_read_event(1), logos_abi::ServiceId::Display);
        }
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo<'_>) -> ! {
    common::idle()
}
