#![no_main]
#![no_std]

mod debug;

use uefi::{
    ResultExt, boot,
    prelude::*,
    proto::console::gop::{BltOp, BltPixel, GraphicsOutput},
    proto::console::text::{Input, Key},
    system,
};

#[entry]
fn main() -> Status {
    debug::write_line(b"LogOS: kernel entered");

    if draw_banner().is_ok() {
        debug::write_line(b"LogOS: framebuffer online");
    } else {
        debug::write_line(b"LogOS: framebuffer unavailable");
    }

    debug::write_line(b"LogOS: shell ready");
    if shell().is_err() {
        debug::write_line(b"LogOS: keyboard unavailable");
    }

    Status::SUCCESS
}

fn shell() -> uefi::Result {
    let handle = boot::get_handle_for_protocol::<Input>()?;
    let mut input = boot::open_protocol_exclusive::<Input>(handle)?;
    input.reset(false)?;
    system::with_stdout(|stdout| stdout.clear())?;
    uefi::println!("LogOS shell");
    uefi::println!("Type help. Press Escape to exit.");

    loop {
        uefi::print!("> ");
        let mut command = [0; 16];
        let mut length = 0;

        loop {
            match read_key(&mut input)? {
                Key::Printable(key) if key == '\r' => break,
                Key::Printable(key) if key == '\x08' && length > 0 => {
                    length -= 1;
                    uefi::print!("\x08 \x08");
                }
                Key::Printable(key) if key.is_ascii() && length < command.len() => {
                    let byte = u16::from(key) as u8;
                    command[length] = byte;
                    length += 1;
                    uefi::print!("{}", byte as char);
                }
                Key::Special(scan_code)
                    if scan_code == uefi::proto::console::text::ScanCode::ESCAPE =>
                {
                    uefi::println!();
                    return Ok(());
                }
                _ => {}
            }
        }

        uefi::println!();
        match &command[..length] {
            b"" => {}
            b"help" => uefi::println!("Commands: help, clear, version, exit"),
            b"clear" => system::with_stdout(|stdout| stdout.clear())?,
            b"version" => uefi::println!("LogOS 0.1.0"),
            b"exit" => return Ok(()),
            _ => uefi::println!("Unknown command"),
        }
    }
}

fn read_key(input: &mut Input) -> uefi::Result<Key> {
    let mut events = [input.wait_for_key_event().ok_or(Status::NOT_READY)?];
    boot::wait_for_event(&mut events).discard_errdata()?;
    input.read_key()?.ok_or(Status::NOT_READY.into())
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
    })?;
    draw_word(&mut gop, b"LOGOS", (56, 48), 6)
}

fn draw_word(
    gop: &mut GraphicsOutput,
    word: &[u8],
    origin: (usize, usize),
    scale: usize,
) -> uefi::Result {
    for (letter_index, &letter) in word.iter().enumerate() {
        let glyph = glyph(letter).ok_or(Status::UNSUPPORTED)?;
        for (row, &bits) in glyph.iter().enumerate() {
            for column in 0..5 {
                if bits & (1 << (4 - column)) != 0 {
                    gop.blt(BltOp::VideoFill {
                        color: BltPixel::new(12, 18, 30),
                        dest: (
                            origin.0 + (letter_index * 6 + column) * scale,
                            origin.1 + row * scale,
                        ),
                        dims: (scale, scale),
                    })?;
                }
            }
        }
    }
    Ok(())
}

fn glyph(letter: u8) -> Option<&'static [u8; 7]> {
    const G: [u8; 7] = [0b01110, 0b10000, 0b10111, 0b10001, 0b10001, 0b01110, 0];
    const L: [u8; 7] = [0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111, 0];
    const O: [u8; 7] = [0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110, 0];
    const S: [u8; 7] = [0b01111, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110, 0];

    match letter {
        b'G' => Some(&G),
        b'L' => Some(&L),
        b'O' => Some(&O),
        b'S' => Some(&S),
        _ => None,
    }
}
