#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", no_main)]

#[cfg(target_os = "none")]
use logos_abi::{AtriumApp, GuiDrawBatch, GuiDrawCommand, GuiRect, SurfaceHandle};
#[cfg(target_os = "none")]
use logos_program::{ProgramClient, SurfaceEvent};

#[cfg(target_os = "none")]
#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    let mut client = match unsafe { ProgramClient::from_fixed_bootstrap() } {
        Ok(client) => client,
        Err(_) => idle(),
    };
    let _ = client.request_surface(AtriumApp::Calculator);
    loop {
        let _ = client.retry_surface_request();
        if let Ok(Some(SurfaceEvent::Created(surface))) = client.poll_surface() {
            let mut batch = GuiDrawBatch::new(surface, 1, GuiRect::new(0, 0, 320, 220));
            let _ = batch.push(GuiDrawCommand::fill_rect(GuiRect::new(0, 0, 320, 220), 0x20252b));
            let _ =
                batch.push(GuiDrawCommand::glyph_run(16, 24, 0xffffff, b"Atrium program").unwrap());
            let _ = client.send_draw(batch);
        }
        let mut input = logos_abi::AtriumSurfaceInput::new(
            SurfaceHandle::EMPTY,
            logos_abi::InputMessage::key(
                logos_abi::KeyCode::ESCAPE,
                logos_abi::KeyState::Pressed,
                0,
            ),
        );
        let _ = client.receive_input(&mut input);
        yield_now();
    }
}

#[cfg(target_os = "none")]
#[inline(always)]
fn yield_now() {
    unsafe {
        core::arch::asm!("mov eax, 1", "int 49", lateout("rax") _, options(preserves_flags));
    }
}

#[cfg(target_os = "none")]
fn idle() -> ! {
    loop {
        yield_now();
    }
}

#[cfg(target_os = "none")]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    idle()
}

#[cfg(not(target_os = "none"))]
fn main() {}
