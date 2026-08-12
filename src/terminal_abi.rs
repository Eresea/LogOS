//! Fixed wire shapes shared by the terminal services.
//!
//! These are deliberately boring `repr(C)` values.  Services exchange values
//! through bounded pages; they never share Rust references or kernel objects.

pub const ABI_VERSION: u16 = 1;
pub const MAX_MESSAGE_BYTES: usize = 4096;
pub const IPC_RING_SLOTS: usize = 8;
pub const MAX_TEXT_BYTES: usize = 64;
pub const MAX_RENDER_CELLS: usize = 128;
pub const MAX_COLUMNS: usize = 160;
pub const MAX_ROWS: usize = 100;
pub const DEFAULT_COLUMNS: usize = 80;
pub const DEFAULT_ROWS: usize = 25;
pub const MAX_SCROLLBACK_LINES: usize = 2048;
pub const MAX_HISTORY_ENTRIES: usize = 64;
pub const MAX_HISTORY_BYTES: usize = 256;
pub const MAX_CHILD_PROCESSES: usize = 8;
pub const MAX_PIPELINE_STAGES: usize = 4;
pub const MAX_VOLATILE_FILES: usize = 32;
pub const MAX_VOLATILE_FILE_BYTES: usize = 16 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum MessageKind {
    Empty = 0,
    Key = 1,
    Text = 2,
    Paste = 3,
    SessionInput = 4,
    SessionOutput = 5,
    RenderCells = 6,
    FullRedraw = 7,
    Resize = 8,
    Reset = 9,
    Heartbeat = 10,
    Fault = 11,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum KeyState {
    Pressed = 1,
    Released = 2,
    Repeat = 3,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KeyCode(pub(crate) u16);

#[allow(non_upper_case_globals)]
impl KeyCode {
    pub const Unknown: Self = Self(0);
    pub const Escape: Self = Self(1);
    pub const Enter: Self = Self(2);
    pub const Backspace: Self = Self(3);
    pub const Tab: Self = Self(4);
    pub const BackTab: Self = Self(5);
    pub const Insert: Self = Self(6);
    pub const Delete: Self = Self(7);
    pub const Home: Self = Self(8);
    pub const End: Self = Self(9);
    pub const PageUp: Self = Self(10);
    pub const PageDown: Self = Self(11);
    pub const Up: Self = Self(12);
    pub const Down: Self = Self(13);
    pub const Left: Self = Self(14);
    pub const Right: Self = Self(15);

    pub const fn function(number: u8) -> Self {
        Self(0x100 + number as u16)
    }

    pub const fn character(byte: u8) -> Self {
        Self(0x200 + byte as u16)
    }

    pub const fn raw(self) -> u16 {
        self.0
    }

    pub const fn from_raw(raw: u16) -> Self {
        Self(raw)
    }

    pub fn function_number(self) -> Option<u8> {
        if self.0 >= 0x100 && self.0 <= 0x1ff { Some((self.0 - 0x100) as u8) } else { None }
    }

    pub fn character_byte(self) -> Option<u8> {
        if self.0 >= 0x200 && self.0 <= 0x2ff { Some((self.0 - 0x200) as u8) } else { None }
    }
}

pub const MOD_SHIFT: u16 = 1 << 0;
pub const MOD_CTRL: u16 = 1 << 1;
pub const MOD_ALT: u16 = 1 << 2;
pub const MOD_META: u16 = 1 << 3;
pub const MOD_CAPS_LOCK: u16 = 1 << 4;
pub const MOD_NUM_LOCK: u16 = 1 << 5;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct InputMessage {
    pub kind: MessageKind,
    pub state: KeyState,
    pub code: u16,
    pub modifiers: u16,
    pub len: u16,
    pub text: [u8; MAX_TEXT_BYTES],
}

impl InputMessage {
    pub const fn key(code: KeyCode, state: KeyState, modifiers: u16) -> Self {
        Self {
            kind: MessageKind::Key,
            state,
            code: code.raw(),
            modifiers,
            len: 0,
            text: [0; MAX_TEXT_BYTES],
        }
    }

    pub fn text(bytes: &[u8]) -> Option<Self> {
        if bytes.is_empty() || bytes.len() > MAX_TEXT_BYTES {
            return None;
        }
        let mut text = [0; MAX_TEXT_BYTES];
        text[..bytes.len()].copy_from_slice(bytes);
        Some(Self {
            kind: MessageKind::Text,
            state: KeyState::Pressed,
            code: 0,
            modifiers: 0,
            len: bytes.len() as u16,
            text,
        })
    }

    pub fn text_bytes(&self) -> Option<&[u8]> {
        (self.kind == MessageKind::Text && self.len as usize <= MAX_TEXT_BYTES)
            .then(|| &self.text[..self.len as usize])
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct Cell {
    pub codepoint: u32,
    pub foreground: u32,
    pub background: u32,
    pub attributes: u16,
    pub width: u8,
    pub reserved: u8,
}

impl Cell {
    pub const EMPTY: Self = Self {
        codepoint: b' ' as u32,
        foreground: 0x00ff_ffff,
        background: 0,
        attributes: 0,
        width: 1,
        reserved: 0,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct RenderMessage {
    pub kind: MessageKind,
    pub columns: u16,
    pub rows: u16,
    pub cursor_column: u16,
    pub cursor_row: u16,
    pub count: u16,
    pub positions: [u16; MAX_RENDER_CELLS],
    pub cells: [Cell; MAX_RENDER_CELLS],
}

impl RenderMessage {
    pub const fn empty(kind: MessageKind) -> Self {
        Self {
            kind,
            columns: 0,
            rows: 0,
            cursor_column: 0,
            cursor_row: 0,
            count: 0,
            positions: [0; MAX_RENDER_CELLS],
            cells: [Cell::EMPTY; MAX_RENDER_CELLS],
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct StreamMessage {
    pub kind: MessageKind,
    pub flags: u8,
    pub len: u16,
    pub bytes: [u8; MAX_MESSAGE_BYTES - 4],
}

impl StreamMessage {
    pub const fn empty(kind: MessageKind) -> Self {
        Self { kind, flags: 0, len: 0, bytes: [0; MAX_MESSAGE_BYTES - 4] }
    }

    pub fn from_bytes(kind: MessageKind, bytes: &[u8]) -> Option<Self> {
        if bytes.len() > MAX_MESSAGE_BYTES - 4 {
            return None;
        }
        let mut message = Self::empty(kind);
        message.len = bytes.len() as u16;
        message.bytes[..bytes.len()].copy_from_slice(bytes);
        Some(message)
    }

    pub fn as_bytes(&self) -> Option<&[u8]> {
        (self.len as usize <= self.bytes.len()).then(|| &self.bytes[..self.len as usize])
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct EndpointHeader {
    pub abi_version: u16,
    pub generation: u16,
    pub service_epoch: u64,
    pub producer: u16,
    pub consumer: u16,
}

impl EndpointHeader {
    pub const fn new(generation: u16, service_epoch: u64) -> Self {
        Self { abi_version: ABI_VERSION, generation, service_epoch, producer: 0, consumer: 0 }
    }

    pub const fn accepts(self, generation: u16, service_epoch: u64) -> bool {
        self.abi_version == ABI_VERSION
            && self.generation == generation
            && self.service_epoch == service_epoch
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_codes_round_trip() {
        for code in [KeyCode::Unknown, KeyCode::Escape, KeyCode::Right, KeyCode::function(12)] {
            assert_eq!(KeyCode::from_raw(code.raw()), code);
        }
    }

    #[test]
    fn message_lengths_are_bounded() {
        assert!(InputMessage::text(&[b'a'; MAX_TEXT_BYTES]).is_some());
        assert!(InputMessage::text(&[b'a'; MAX_TEXT_BYTES + 1]).is_none());
        assert!(
            StreamMessage::from_bytes(MessageKind::SessionOutput, &[0; MAX_MESSAGE_BYTES - 4])
                .is_some()
        );
        assert!(
            StreamMessage::from_bytes(MessageKind::SessionOutput, &[0; MAX_MESSAGE_BYTES - 3])
                .is_none()
        );
    }

    #[test]
    fn endpoint_rejects_stale_identity() {
        let header = EndpointHeader::new(4, 8);
        assert!(header.accepts(4, 8));
        assert!(!header.accepts(3, 8));
        assert!(!header.accepts(4, 9));
    }
}
