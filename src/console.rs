use crate::{
    capabilities::{Capability, CapabilityManager},
    glyph,
    ipc::{Channel, Message},
    keyboard,
    services::ServiceHandle,
};
use core::cell::Cell;

const BACKGROUND: [u8; 3] = [12, 18, 30];
const ACCENT: [u8; 3] = [61, 220, 151];
const ORIGIN: (usize, usize) = (32, 136);
const SCALE: usize = 3;

pub struct Shell<'a> {
    console: Console,
    command: [u8; 16],
    length: usize,
    endpoint: Endpoint<'a>,
}

#[derive(Clone, Copy)]
pub struct Endpoint<'a> {
    channel: &'a Channel,
    capabilities: &'a CapabilityManager,
    capability: Capability,
    destination: ServiceHandle,
    reply: &'a Cell<Option<Message>>,
}

impl<'a> Endpoint<'a> {
    pub const fn new(
        channel: &'a Channel,
        capabilities: &'a CapabilityManager,
        capability: Capability,
        destination: ServiceHandle,
        reply: &'a Cell<Option<Message>>,
    ) -> Self {
        Self { channel, capabilities, capability, destination, reply }
    }

    fn ping(self) -> bool {
        self.channel.send(self.capabilities, self.capability, self.destination, Message::Ping)
    }

    fn reply(self) -> Option<Message> {
        self.reply.take()
    }
}

struct Console {
    framebuffer: *mut u8,
    width: usize,
    height: usize,
    stride: usize,
    cursor: (usize, usize),
}

impl<'a> Shell<'a> {
    pub fn new(
        framebuffer: *mut u8,
        width: usize,
        height: usize,
        stride: usize,
        endpoint: Endpoint<'a>,
    ) -> Option<Self> {
        (!framebuffer.is_null() && width > 0 && height > ORIGIN.1 && stride >= width).then_some(
            Self {
                console: Console { framebuffer, width, height, stride, cursor: ORIGIN },
                command: [0; 16],
                length: 0,
                endpoint,
            },
        )
    }

    pub fn start(&mut self) -> bool {
        self.console.reset();
        self.console.write(b"LOGOS KERNEL CONSOLE\nTYPE HELP PING OR EXIT\n");
        self.prompt();
        true
    }

    pub fn run(mut self, mut schedule: impl FnMut()) -> ! {
        loop {
            schedule();
            if self.endpoint.reply() == Some(Message::Pong) {
                self.console.newline();
                self.console.write(b"PONG RECEIVED\n");
                self.prompt();
            }
            if let Some(key) = keyboard::poll() {
                self.key(key);
            } else {
                unsafe { core::arch::asm!("hlt") };
            }
        }
    }

    fn key(&mut self, key: u8) {
        match key {
            0x1b => self.exit(),
            b'\n' => self.submit(),
            0x08 if self.length > 0 => {
                self.length -= 1;
                self.console.backspace();
            }
            key if key.is_ascii()
                && self.length < self.command.len()
                && glyph(key.to_ascii_uppercase()).is_some() =>
            {
                self.command[self.length] = key;
                self.length += 1;
                self.console.write_byte(key);
            }
            _ => {}
        }
    }

    fn submit(&mut self) {
        self.console.newline();
        match &self.command[..self.length] {
            b"help" => self.console.write(b"COMMANDS HELP CLEAR VERSION PING EXIT\n"),
            b"clear" => self.console.reset(),
            b"ping" if self.endpoint.ping() => self.console.write(b"PING SENT\n"),
            b"version" => self.console.write(b"LOGOS 0 1 0\n"),
            b"exit" => self.exit(),
            _ => self.console.write(b"UNKNOWN COMMAND\n"),
        }
        self.length = 0;
        self.prompt();
    }

    fn prompt(&mut self) {
        self.console.write(b"> ");
    }

    fn exit(&self) -> ! {
        loop {
            unsafe { core::arch::asm!("cli", "hlt") };
        }
    }
}

impl Console {
    fn reset(&mut self) {
        self.fill(BACKGROUND, (0, ORIGIN.1), (self.width, self.height - ORIGIN.1));
        self.cursor = ORIGIN;
    }

    fn write(&mut self, text: &[u8]) {
        for &byte in text {
            if byte == b'\n' {
                self.newline();
            } else {
                self.write_byte(byte);
            }
        }
    }

    fn write_byte(&mut self, byte: u8) {
        if self.cursor.0 + 5 * SCALE > self.width.saturating_sub(32) {
            self.newline();
        }
        let Some(rows) = glyph(byte.to_ascii_uppercase()) else {
            return;
        };
        for (row, &bits) in rows.iter().enumerate() {
            for column in 0..5 {
                if bits & (1 << (4 - column)) != 0 {
                    self.fill(
                        ACCENT,
                        (self.cursor.0 + column * SCALE, self.cursor.1 + row * SCALE),
                        (SCALE, SCALE),
                    );
                }
            }
        }
        self.cursor.0 += 6 * SCALE;
    }

    fn newline(&mut self) {
        self.cursor = (ORIGIN.0, self.cursor.1 + 8 * SCALE);
        if self.cursor.1 + 7 * SCALE > self.height {
            self.cursor = ORIGIN;
        }
    }

    fn backspace(&mut self) {
        let step = 6 * SCALE;
        if self.cursor.0 >= ORIGIN.0 + step {
            self.cursor.0 -= step;
            self.fill(BACKGROUND, self.cursor, (5 * SCALE, 7 * SCALE));
        }
    }

    fn fill(&mut self, color: [u8; 3], origin: (usize, usize), dims: (usize, usize)) {
        for y in origin.1..origin.1.saturating_add(dims.1).min(self.height) {
            for x in origin.0..origin.0.saturating_add(dims.0).min(self.width) {
                let pixel = unsafe { self.framebuffer.add((y * self.stride + x) * 4) };
                unsafe {
                    pixel.write_volatile(color[0]);
                    pixel.add(1).write_volatile(color[1]);
                    pixel.add(2).write_volatile(color[2]);
                }
            }
        }
    }
}
