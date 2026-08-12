#![no_std]

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

pub const MAX_SERVICE_IMAGE_BYTES: usize = 512 * 1024;
pub const MAX_MEMORY_DESCRIPTORS: usize = 256;
pub const MAX_MANAGED_FRAMES: usize = 65_536;
pub const MAX_SERVICE_ENDPOINTS: usize = 32;
pub const MAX_SERVICE_DATA_BYTES: usize = 1024 * 1024;
pub const MAX_FRAMEBUFFER_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_GLYPH_CACHE: usize = 1024;
pub const MAX_CAPABILITIES: usize = 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ServiceId {
    Input = 1,
    Display = 2,
    Terminal = 3,
    Session = 4,
    Commands = 5,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum CapabilityKind {
    IpcEndpoint = 1,
    KeyboardBytes = 2,
    Framebuffer = 3,
    ProcessControl = 4,
    ServiceControl = 5,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum SyscallKind {
    Yield = 1,
    Wait = 2,
    Exit = 3,
    IpcCreate = 4,
    IpcMap = 5,
    IpcSignal = 6,
    ProcessStart = 7,
    ProcessReap = 8,
    CapabilityMap = 9,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u16)]
pub enum SyscallStatus {
    Ok = 0,
    InvalidArgument = 1,
    InvalidCapability = 2,
    NotFound = 3,
    Full = 4,
    Stale = 5,
    Disconnected = 6,
    Exhausted = 7,
    Denied = 8,
    Faulted = 9,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct Capability {
    pub kind: CapabilityKind,
    pub service: ServiceId,
    pub generation: u16,
    pub rights: u16,
    pub slot: u16,
}

impl Capability {
    pub const fn new(
        kind: CapabilityKind,
        service: ServiceId,
        generation: u16,
        rights: u16,
        slot: u16,
    ) -> Self {
        Self { kind, service, generation, rights, slot }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct SyscallRequest {
    pub kind: SyscallKind,
    pub capability: Capability,
    pub args: [u64; 4],
}

impl SyscallRequest {
    pub const fn new(kind: SyscallKind, capability: Capability) -> Self {
        Self { kind, capability, args: [0; 4] }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct SyscallResponse {
    pub status: SyscallStatus,
    pub value: u64,
}

impl SyscallResponse {
    pub const fn new(status: SyscallStatus, value: u64) -> Self {
        Self { status, value }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct ServiceDescriptor {
    pub service: ServiceId,
    pub generation: u16,
    pub epoch: u64,
    pub image_bytes: u32,
    pub data_bytes: u32,
    pub stack_pages: u16,
    pub capability_count: u16,
}

impl ServiceDescriptor {
    pub const fn new(service: ServiceId, generation: u16, epoch: u64) -> Self {
        Self {
            service,
            generation,
            epoch,
            image_bytes: 0,
            data_bytes: 0,
            stack_pages: 8,
            capability_count: 0,
        }
    }
}

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
pub struct KeyCode(pub u16);

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

    pub const UNKNOWN: Self = Self::Unknown;
    pub const ESCAPE: Self = Self::Escape;
    pub const ENTER: Self = Self::Enter;
    pub const BACKSPACE: Self = Self::Backspace;
    pub const TAB: Self = Self::Tab;
    pub const UP: Self = Self::Up;
    pub const DOWN: Self = Self::Down;
    pub const LEFT: Self = Self::Left;
    pub const RIGHT: Self = Self::Right;
    pub const HOME: Self = Self::Home;
    pub const END: Self = Self::End;
    pub const PAGE_UP: Self = Self::PageUp;
    pub const PAGE_DOWN: Self = Self::PageDown;
    pub const DELETE: Self = Self::Delete;
    pub const CTRL: Self = Self(0x300);
    pub const ALT: Self = Self(0x301);
    pub const CAPS_LOCK: Self = Self(0x302);
    pub const SHIFT_LEFT: Self = Self(0x303);
    pub const SHIFT_RIGHT: Self = Self(0x304);

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
pub struct MessageIdentity {
    pub generation: u16,
    pub service_epoch: u64,
}

impl MessageIdentity {
    pub const fn new(generation: u16, service_epoch: u64) -> Self {
        Self { generation, service_epoch }
    }

    pub const fn accepts(self, endpoint: EndpointHeader) -> bool {
        endpoint.accepts(self.generation, self.service_epoch)
    }
}

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
        Self::text_kind(MessageKind::Text, bytes)
    }

    pub fn paste(bytes: &[u8]) -> Option<Self> {
        Self::text_kind(MessageKind::Paste, bytes)
    }

    fn text_kind(kind: MessageKind, bytes: &[u8]) -> Option<Self> {
        if bytes.is_empty() || bytes.len() > MAX_TEXT_BYTES {
            return None;
        }
        let mut text = [0; MAX_TEXT_BYTES];
        text[..bytes.len()].copy_from_slice(bytes);
        Some(Self {
            kind,
            state: KeyState::Pressed,
            code: 0,
            modifiers: 0,
            len: bytes.len() as u16,
            text,
        })
    }

    pub fn text_bytes(&self) -> Option<&[u8]> {
        (matches!(self.kind, MessageKind::Text | MessageKind::Paste)
            && self.len as usize <= MAX_TEXT_BYTES)
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

    pub const fn identity(self) -> MessageIdentity {
        MessageIdentity::new(self.generation, self.service_epoch)
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
        assert_eq!(InputMessage::paste(b"abc").unwrap().text_bytes(), Some(&b"abc"[..]));
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
