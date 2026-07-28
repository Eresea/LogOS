#![no_std]

pub const ABI: u16 = 1;
pub const MAX_TEXT: usize = 64;

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
}
