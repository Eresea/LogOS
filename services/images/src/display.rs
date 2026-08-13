#![no_std]
#![no_main]

mod common;

use logos_abi::{
    DISPLAY_CONFIG_BASE, DISPLAY_FRAMEBUFFER_BASE, FramebufferConfig, FramebufferFormat,
    IPC_PAGE_BYTES, RenderIpc, SERVICE_IPC_BASE,
};

const TERMINAL_TO_DISPLAY: usize = SERVICE_IPC_BASE + IPC_PAGE_BYTES;

static mut DISPLAY: logos_display::Display = logos_display::Display::new(1);
static mut FONT: logos_display::GlyphCache = logos_display::GlyphCache::new();

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    let display = unsafe { &mut *core::ptr::addr_of_mut!(DISPLAY) };
    let font = unsafe { &mut *core::ptr::addr_of_mut!(FONT) };
    let ring = unsafe { &*(TERMINAL_TO_DISPLAY as *const RenderIpc) };
    let config = unsafe { &*(DISPLAY_CONFIG_BASE as *const FramebufferConfig) };
    let framebuffer = unsafe {
        core::slice::from_raw_parts_mut(DISPLAY_FRAMEBUFFER_BASE as *mut u8, config.bytes as usize)
    };
    loop {
        let identity = ring.endpoint().identity();
        if display.generation() != identity.generation {
            display.replace_generation(identity.generation);
        }
        let mut progressed = false;
        while let Ok(message) = ring.receive(identity) {
            progressed = true;
            if display.apply(identity.generation, &message).is_ok() {
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
                    font,
                );
            }
        }
        if !progressed {
            core::hint::spin_loop();
        }
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo<'_>) -> ! {
    common::idle()
}
