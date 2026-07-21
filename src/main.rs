#![no_main]
#![no_std]

mod debug;

use uefi::{
    ResultExt, boot,
    prelude::*,
    proto::console::gop::{BltOp, BltPixel, GraphicsOutput},
    proto::console::text::{Input, Key, ScanCode},
};

const BACKGROUND: BltPixel = BltPixel::new(12, 18, 30);
const ACCENT: BltPixel = BltPixel::new(61, 220, 151);

#[entry]
fn main() -> Status {
    debug::write_line(b"LogOS: kernel entered");

    if shell().is_ok() {
        debug::write_line(b"LogOS: shell exited");
    } else {
        debug::write_line(b"LogOS: shell unavailable");
    }

    Status::SUCCESS
}

fn shell() -> uefi::Result {
    let graphics_handle = boot::get_handle_for_protocol::<GraphicsOutput>()?;
    let mut gop = boot::open_protocol_exclusive::<GraphicsOutput>(graphics_handle)?;
    let (width, height) = gop.current_mode_info().resolution();
    let mut terminal = Terminal::new(&mut gop, width, height);
    terminal.reset()?;
    terminal.write(b"LOGOS SHELL\nTYPE HELP OR EXIT\n")?;
    debug::write_line(b"LogOS: framebuffer terminal online");

    let input_handle = boot::get_handle_for_protocol::<Input>()?;
    let mut input = boot::open_protocol_exclusive::<Input>(input_handle)?;
    input.reset(false)?;

    loop {
        terminal.write(b"> ")?;
        let mut command = [0; 16];
        let mut length = 0;

        loop {
            match read_key(&mut input)? {
                Key::Printable(key) if key == '\r' => break,
                Key::Printable(key) if key == '\x08' && length > 0 => {
                    length -= 1;
                    terminal.backspace()?;
                }
                Key::Printable(key) if key.is_ascii() && length < command.len() => {
                    let byte = u16::from(key) as u8;
                    if glyph(byte.to_ascii_uppercase()).is_some() {
                        command[length] = byte;
                        length += 1;
                        terminal.write_byte(byte)?;
                    }
                }
                Key::Special(scan_code) if scan_code == ScanCode::ESCAPE => return Ok(()),
                _ => {}
            }
        }

        terminal.newline();
        match &command[..length] {
            b"" => {}
            b"help" => terminal.write(b"COMMANDS HELP CLEAR VERSION EXIT\n")?,
            b"clear" => terminal.reset()?,
            b"version" => terminal.write(b"LOGOS 0 1 0\n")?,
            b"exit" => return Ok(()),
            _ => terminal.write(b"UNKNOWN COMMAND\n")?,
        }
    }
}

fn read_key(input: &mut Input) -> uefi::Result<Key> {
    let mut events = [input.wait_for_key_event().ok_or(Status::NOT_READY)?];
    boot::wait_for_event(&mut events).discard_errdata()?;
    input.read_key()?.ok_or(Status::NOT_READY.into())
}

struct Terminal<'a> {
    gop: &'a mut GraphicsOutput,
    cursor: (usize, usize),
    width: usize,
    height: usize,
}

impl<'a> Terminal<'a> {
    const ORIGIN: (usize, usize) = (32, 136);
    const SCALE: usize = 3;

    fn new(gop: &'a mut GraphicsOutput, width: usize, height: usize) -> Self {
        Self { gop, cursor: Self::ORIGIN, width, height }
    }

    fn reset(&mut self) -> uefi::Result {
        self.fill(BACKGROUND, (0, 0), (self.width, self.height))?;
        self.fill(ACCENT, (32, 32), (self.width.saturating_sub(64), 80))?;
        self.cursor = (56, 48);
        self.write_with_color(b"LOGOS", BACKGROUND)?;
        self.cursor = Self::ORIGIN;
        Ok(())
    }

    fn write(&mut self, text: &[u8]) -> uefi::Result {
        self.write_with_color(text, ACCENT)
    }

    fn write_with_color(&mut self, text: &[u8], color: BltPixel) -> uefi::Result {
        for &byte in text {
            if byte == b'\n' {
                self.newline();
            } else {
                self.draw_glyph(byte.to_ascii_uppercase(), color)?;
            }
        }
        Ok(())
    }

    fn write_byte(&mut self, byte: u8) -> uefi::Result {
        self.draw_glyph(byte.to_ascii_uppercase(), ACCENT)
    }

    fn newline(&mut self) {
        self.cursor = (Self::ORIGIN.0, self.cursor.1 + 8 * Self::SCALE);
        if self.cursor.1 + 7 * Self::SCALE > self.height {
            self.cursor = Self::ORIGIN;
        }
    }

    fn backspace(&mut self) -> uefi::Result {
        let step = 6 * Self::SCALE;
        if self.cursor.0 >= Self::ORIGIN.0 + step {
            self.cursor.0 -= step;
            self.fill(BACKGROUND, self.cursor, (5 * Self::SCALE, 7 * Self::SCALE))?;
        }
        Ok(())
    }

    fn draw_glyph(&mut self, byte: u8, color: BltPixel) -> uefi::Result {
        if self.cursor.0 + 5 * Self::SCALE > self.width.saturating_sub(32) {
            self.newline();
        }
        let glyph = glyph(byte).ok_or(Status::UNSUPPORTED)?;
        for (row, &bits) in glyph.iter().enumerate() {
            for column in 0..5 {
                if bits & (1 << (4 - column)) != 0 {
                    self.fill(
                        color,
                        (self.cursor.0 + column * Self::SCALE, self.cursor.1 + row * Self::SCALE),
                        (Self::SCALE, Self::SCALE),
                    )?;
                }
            }
        }
        self.cursor.0 += 6 * Self::SCALE;
        Ok(())
    }

    fn fill(
        &mut self,
        color: BltPixel,
        dest: (usize, usize),
        dims: (usize, usize),
    ) -> uefi::Result {
        self.gop.blt(BltOp::VideoFill { color, dest, dims })
    }
}

fn glyph(byte: u8) -> Option<&'static [u8; 7]> {
    const A: [u8; 7] = [0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0];
    const B: [u8; 7] = [0b11110, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110, 0];
    const C: [u8; 7] = [0b01111, 0b10000, 0b10000, 0b10000, 0b10000, 0b01111, 0];
    const D: [u8; 7] = [0b11110, 0b10001, 0b10001, 0b10001, 0b10001, 0b11110, 0];
    const E: [u8; 7] = [0b11111, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111, 0];
    const G: [u8; 7] = [0b01110, 0b10000, 0b10111, 0b10001, 0b10001, 0b01110, 0];
    const H: [u8; 7] = [0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001, 0];
    const I: [u8; 7] = [0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b11111, 0];
    const K: [u8; 7] = [0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001];
    const L: [u8; 7] = [0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111, 0];
    const M: [u8; 7] = [0b10001, 0b11011, 0b10101, 0b10001, 0b10001, 0b10001, 0];
    const N: [u8; 7] = [0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001, 0];
    const O: [u8; 7] = [0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110, 0];
    const P: [u8; 7] = [0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0];
    const R: [u8; 7] = [0b11110, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001, 0];
    const S: [u8; 7] = [0b01111, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110, 0];
    const T: [u8; 7] = [0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0];
    const U: [u8; 7] = [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110, 0];
    const V: [u8; 7] = [0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100, 0];
    const W: [u8; 7] = [0b10001, 0b10001, 0b10001, 0b10101, 0b11011, 0b10001, 0];
    const X: [u8; 7] = [0b10001, 0b01010, 0b00100, 0b00100, 0b01010, 0b10001, 0];
    const Y: [u8; 7] = [0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0b00100, 0];
    const ZERO: [u8; 7] = [0b01110, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110, 0];
    const ONE: [u8; 7] = [0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b01110, 0];
    const SPACE: [u8; 7] = [0; 7];
    const PROMPT: [u8; 7] = [0b10000, 0b01000, 0b00100, 0b00010, 0b00100, 0b01000, 0b10000];

    match byte {
        b'A' => Some(&A),
        b'B' => Some(&B),
        b'C' => Some(&C),
        b'D' => Some(&D),
        b'E' => Some(&E),
        b'G' => Some(&G),
        b'H' => Some(&H),
        b'I' => Some(&I),
        b'K' => Some(&K),
        b'L' => Some(&L),
        b'M' => Some(&M),
        b'N' => Some(&N),
        b'O' => Some(&O),
        b'P' => Some(&P),
        b'R' => Some(&R),
        b'S' => Some(&S),
        b'T' => Some(&T),
        b'U' => Some(&U),
        b'V' => Some(&V),
        b'W' => Some(&W),
        b'X' => Some(&X),
        b'Y' => Some(&Y),
        b'0' => Some(&ZERO),
        b'1' => Some(&ONE),
        b' ' => Some(&SPACE),
        b'>' => Some(&PROMPT),
        _ => None,
    }
}
