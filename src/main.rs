#![no_main]
#![no_std]

mod debug;

use uefi::{
    boot,
    prelude::*,
    proto::console::gop::{BltOp, BltPixel, GraphicsOutput},
};

#[entry]
fn main() -> Status {
    debug::write_line(b"LogOS: kernel entered");

    if draw_banner().is_ok() {
        debug::write_line(b"LogOS: framebuffer online");
    } else {
        debug::write_line(b"LogOS: framebuffer unavailable");
    }

    Status::SUCCESS
}

fn draw_banner() -> uefi::Result {
    let handle = boot::get_handle_for_protocol::<GraphicsOutput>()?;
    let mut gop = boot::open_protocol_exclusive::<GraphicsOutput>(handle)?;
    let (width, height) = gop.current_mode_info().resolution();

    gop.blt(BltOp::VideoFill {
        color: BltPixel::new(12, 18, 30),
        dest: (0, 0),
        dims: (width, height),
    })?;
    gop.blt(BltOp::VideoFill {
        color: BltPixel::new(61, 220, 151),
        dest: (32, 32),
        dims: (width.saturating_sub(64), 80),
    })
}
