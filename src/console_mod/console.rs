use crate::{
    glyph,
    ipc::{Channel, Message},
    keyboard,
    services::ServiceHandle,
    session::Principal,
    trace,
};
use logos_core::capabilities::{Capability, CapabilityManager};

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

pub struct Startup {
    console: Console,
}

#[derive(Clone, Copy)]
pub struct Endpoint<'a> {
    channel: &'a Channel,
    responses: &'a Channel,
    capabilities: &'a CapabilityManager,
    capability: Capability,
    destination: ServiceHandle,
}

impl<'a> Endpoint<'a> {
    pub const fn new(
        channel: &'a Channel,
        responses: &'a Channel,
        capabilities: &'a CapabilityManager,
        capability: Capability,
        destination: ServiceHandle,
    ) -> Self {
        Self { channel, responses, capabilities, capability, destination }
    }

    fn ping(self) -> bool {
        self.channel
            .send(
                self.capabilities,
                self.capability,
                Principal::LOCAL,
                self.destination,
                Message::Ping,
            )
            .is_some()
    }

    fn inflate(self) -> bool {
        self.channel
            .send(
                self.capabilities,
                self.capability,
                Principal::LOCAL,
                self.destination,
                Message::Inflate,
            )
            .is_some()
    }

    fn recover(self) -> bool {
        self.channel
            .send(
                self.capabilities,
                self.capability,
                Principal::LOCAL,
                self.destination,
                Message::Recover,
            )
            .is_some()
    }

    fn reply(self) -> Option<Message> {
        self.responses.receive().map(|reply| reply.message)
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
    pub fn from_startup(startup: Startup, endpoint: Endpoint<'a>) -> Self {
        Self { console: startup.console, command: [0; 16], length: 0, endpoint }
    }

    pub fn start(&mut self) -> bool {
        self.console.write(b"LOGOS KERNEL CONSOLE\nTYPE HELP TRACE PING INFLATE RECOVER OR EXIT\n");
        self.prompt();
        true
    }

    pub fn run(mut self, mut schedule: impl FnMut()) -> ! {
        loop {
            schedule();
            match self.endpoint.reply() {
                Some(Message::Pong) => {
                    self.console.newline();
                    self.console.write(b"PONG RECEIVED\n");
                    self.prompt();
                }
                Some(Message::Complete) => {
                    self.console.newline();
                    self.console.write(b"PAGE ADDED\n");
                    self.prompt();
                }
                Some(Message::Failed) => {
                    self.console.newline();
                    self.console.write(b"DEVICE FAILED\n");
                    self.prompt();
                }
                _ => {}
            }
            if let Some(key) = keyboard::recovery_poll() {
                self.key(key);
                schedule();
            } else {
                unsafe { core::arch::asm!("hlt") };
            }
        }
    }
}

impl Startup {
    pub fn new(framebuffer: *mut u8, width: usize, height: usize, stride: usize) -> Option<Self> {
        (!framebuffer.is_null() && width > 0 && height > ORIGIN.1 && stride >= width).then_some(
            Self { console: Console { framebuffer, width, height, stride, cursor: ORIGIN } },
        )
    }

    pub fn start(&mut self) -> bool {
        self.console.reset();
        self.console.write(b"LOGOS STARTUP HEALTH\n");
        true
    }
}

impl<'a> Shell<'a> {
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
            b"help" => {
                self.console.write(b"COMMANDS HELP TRACE CLEAR VERSION PING INFLATE RECOVER EXIT\n")
            }
            b"clear" => self.console.reset(),
            b"trace" => {
                for event in trace::snapshot().events() {
                    self.console.write(trace::message(*event));
                }
            }
            b"ping" if self.endpoint.ping() => self.console.write(b"PING SENT\n"),
            b"inflate" if self.endpoint.inflate() => self.console.write(b"INFLATE SENT\n"),
            b"recover" if self.endpoint.recover() => self.console.write(b"RECOVER SENT\n"),
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
        if crate::acpi::power_off() {
            loop {
                unsafe { core::arch::asm!("hlt") };
            }
        }
        if crate::acpi::reset() {
            loop {
                unsafe { core::arch::asm!("hlt") };
            }
        }
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
