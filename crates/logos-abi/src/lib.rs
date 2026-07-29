#![no_std]

pub const ABI: u16 = 1;
pub const MAX_TEXT: usize = 64;
pub const MAX_SESSION_TEXT: usize = 256;

/// `foundation.session` v1 command request.  The transport stays bounded by
/// `logos_core::native_service::Context`; this is the shared wire contract.
#[derive(Clone, Copy, Eq, PartialEq)]
#[repr(u32)]
pub enum Syscall {
    Recovery = 1,
    Reboot,
    PowerOff,
    Ping,
    Tasks,
    Services,
    Drivers,
    Trace,
    Inspect,
    Restart,
    Cancel,
    SetInputLayout,
}

impl Syscall {
    pub const fn from_wire(value: u32) -> Option<Self> {
        match value {
            1 => Some(Self::Recovery),
            2 => Some(Self::Reboot),
            3 => Some(Self::PowerOff),
            4 => Some(Self::Ping),
            5 => Some(Self::Tasks),
            6 => Some(Self::Services),
            7 => Some(Self::Drivers),
            8 => Some(Self::Trace),
            9 => Some(Self::Inspect),
            10 => Some(Self::Restart),
            11 => Some(Self::Cancel),
            12 => Some(Self::SetInputLayout),
            _ => None,
        }
    }

    pub const fn takes_argument(self) -> bool {
        matches!(self, Self::Inspect | Self::Restart | Self::Cancel | Self::SetInputLayout)
    }
}

/// Bounded `foundation.session` v1 request passed between native services.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct SessionRequest {
    pub syscall: Syscall,
    pub argument: [u8; MAX_SESSION_TEXT],
    pub length: usize,
}

impl SessionRequest {
    pub const fn new(syscall: Syscall, argument: [u8; MAX_SESSION_TEXT], length: usize) -> Self {
        Self { syscall, argument, length }
    }

    pub const fn valid(self) -> bool {
        self.length <= self.argument.len() && self.syscall.takes_argument() == (self.length != 0)
    }
}

/// Bounded `foundation.session` v1 reply returned to a terminal client.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct SessionReply {
    pub text: [u8; MAX_SESSION_TEXT],
    pub length: usize,
}

impl SessionReply {
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() > MAX_SESSION_TEXT {
            return None;
        }
        let mut text = [0; MAX_SESSION_TEXT];
        text[..bytes.len()].copy_from_slice(bytes);
        Some(Self { text, length: bytes.len() })
    }

    pub const fn valid(self) -> bool {
        self.length <= self.text.len()
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
#[repr(transparent)]
pub struct InputEvent(u8);

impl InputEvent {
    pub const BACKSPACE: Self = Self(0x08);
    pub const ENTER: Self = Self(b'\n');
    pub const ESCAPE: Self = Self(0x1b);

    pub const fn from_byte(byte: u8) -> Option<Self> {
        if byte == Self::BACKSPACE.0
            || byte == Self::ENTER.0
            || byte == Self::ESCAPE.0
            || (byte >= 0x20 && byte <= 0x7e)
        {
            Some(Self(byte))
        } else {
            None
        }
    }

    pub const fn byte(self) -> u8 {
        self.0
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
#[repr(u8)]
pub enum InputLayout {
    Qwerty = 1,
    Azerty = 2,
}

#[derive(Clone, Copy, Eq, PartialEq)]
#[repr(transparent)]
pub struct DisplayColor(u32);

impl DisplayColor {
    pub const BLACK: Self = Self(0);
    pub const GREEN: Self = Self(0x0000_ff00);

    pub const fn from_wire(value: u32) -> Option<Self> {
        if value & 0xff00_0000 == 0 { Some(Self(value)) } else { None }
    }

    pub const fn wire(self) -> u32 {
        self.0
    }

    pub const fn rgb(self) -> [u8; 3] {
        [self.0 as u8, (self.0 >> 8) as u8, (self.0 >> 16) as u8]
    }
}

impl InputLayout {
    pub const fn wire(self) -> u8 {
        match self {
            Self::Qwerty => b'q',
            Self::Azerty => b'a',
        }
    }

    pub const fn from_wire(value: u8) -> Option<Self> {
        match value {
            b'q' => Some(Self::Qwerty),
            b'a' => Some(Self::Azerty),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
#[repr(u8)]
pub enum Service {
    Input,
    Display,
    Session,
    Terminal,
}

#[derive(Clone, Copy, Eq, PartialEq)]
#[repr(u8)]
pub enum Capability {
    Input,
    Display,
    Session,
}

#[derive(Clone, Copy, Eq, PartialEq)]
#[repr(u8)]
pub enum Lifecycle {
    Starting,
    Ready,
    Failed,
    Stopped,
}

#[derive(Clone, Copy, Eq, PartialEq)]
#[repr(u8)]
pub enum Operation {
    Ready,
    ReadInput,
    PresentText,
    ClearDisplay,
    SubmitCommand,
}

#[derive(Clone, Copy, Eq, PartialEq)]
#[repr(C)]
pub struct Message {
    pub abi: u16,
    pub service: Service,
    pub capability: Capability,
    pub operation: Operation,
    pub length: u8,
    pub text: [u8; MAX_TEXT],
}

impl Message {
    pub const fn empty(service: Service, capability: Capability, operation: Operation) -> Self {
        Self { abi: ABI, service, capability, operation, length: 0, text: [0; MAX_TEXT] }
    }

    pub fn with_text(mut self, text: &[u8]) -> Option<Self> {
        if text.len() > self.text.len() || core::str::from_utf8(text).is_err() {
            return None;
        }
        self.text[..text.len()].copy_from_slice(text);
        self.length = text.len() as u8;
        Some(self)
    }

    pub fn valid(&self) -> bool {
        self.abi == ABI && usize::from(self.length) <= self.text.len()
    }
}

pub fn self_check() -> bool {
    Message::empty(Service::Terminal, Capability::Display, Operation::PresentText)
        .with_text(b"LogOS")
        .is_some_and(|message| {
            message.valid() && &message.text[..message.length as usize] == b"LogOS"
        })
        && InputEvent::from_byte(b'a').is_some_and(|event| event.byte() == b'a')
        && InputEvent::from_byte(0).is_none()
        && DisplayColor::from_wire(DisplayColor::GREEN.wire()) == Some(DisplayColor::GREEN)
        && DisplayColor::from_wire(0xff00_0000).is_none()
        && Syscall::from_wire(Syscall::Restart as u32)
            .is_some_and(|call| call == Syscall::Restart && call.takes_argument())
        && Syscall::from_wire(0).is_none()
        && SessionRequest::new(Syscall::Inspect, [0; MAX_SESSION_TEXT], 1).valid()
        && !SessionRequest::new(Syscall::Reboot, [0; MAX_SESSION_TEXT], 1).valid()
        && SessionReply::from_bytes(b"ok").is_some_and(|reply| reply.valid() && reply.length == 2)
}
