#![no_std]

//! Fixed wire shapes shared by the terminal services.
//!
//! These are deliberately boring `repr(C)` values.  Services exchange values
//! through bounded pages; they never share Rust references or kernel objects.

use core::{
    cell::UnsafeCell,
    mem::MaybeUninit,
    sync::atomic::{AtomicBool, AtomicU16, AtomicU64, Ordering},
};

pub const ABI_VERSION: u16 = 2;
pub const MAX_TEXT_BYTES: usize = 64;
pub const MAX_RENDER_CELLS: usize = 128;
pub const MAX_COLUMNS: usize = 160;
pub const MAX_ROWS: usize = 100;
pub const DEFAULT_COLUMNS: usize = 80;
pub const DEFAULT_ROWS: usize = 25;
pub const DISPLAY_CELL_WIDTH: usize = 8;
pub const DISPLAY_CELL_HEIGHT: usize = 16;
pub const MIN_FRAMEBUFFER_WIDTH: usize = DEFAULT_COLUMNS * DISPLAY_CELL_WIDTH;
pub const MIN_FRAMEBUFFER_HEIGHT: usize = DEFAULT_ROWS * DISPLAY_CELL_HEIGHT;
pub const MAX_SCROLLBACK_LINES: usize = 2048;
pub const MAX_HISTORY_ENTRIES: usize = 64;
pub const MAX_HISTORY_BYTES: usize = 256;
pub const MAX_SERVICE_IMAGE_BYTES: usize = 512 * 1024;
pub const MAX_MEMORY_DESCRIPTORS: usize = 256;
pub const MAX_MANAGED_FRAMES: usize = 65_536;
pub const MAX_FRAMEBUFFER_BYTES: usize = 16 * 1024 * 1024;
pub const DISPLAY_FRAMEBUFFER_BASE: usize = 0x0000_0100_1000_0000;
pub const DISPLAY_CONFIG_BASE: usize = 0x0000_0100_1200_0000;
pub const INPUT_KEYBOARD_RING_BASE: usize = 0x0000_0100_1100_0000;
pub const KEYBOARD_RING_CAPACITY: usize = 256;
pub const IPC_PAGE_BYTES: usize = 4096;
pub const MAX_IPC_BYTES: usize = 256;
pub const IPC_FLAG_MORE: u8 = 1 << 0;
pub const SERVICE_IPC_BASE: usize = 0x0000_0100_0200_0000;

pub const IPC_ENDPOINT_COUNT: usize = 6;
pub const IPC_READ_EVENT_BASE: usize = 0;
pub const IPC_WRITE_EVENT_BASE: usize = IPC_READ_EVENT_BASE + IPC_ENDPOINT_COUNT;
pub const KEYBOARD_READ_EVENT: usize = IPC_WRITE_EVENT_BASE + IPC_ENDPOINT_COUNT;
pub const EVENT_COUNT: usize = KEYBOARD_READ_EVENT + 1;

pub const fn ipc_read_event_mask(endpoint: usize) -> u64 {
    if endpoint < IPC_ENDPOINT_COUNT { 1u64 << (IPC_READ_EVENT_BASE + endpoint) } else { 0 }
}

pub const fn ipc_write_event_mask(endpoint: usize) -> u64 {
    if endpoint < IPC_ENDPOINT_COUNT { 1u64 << (IPC_WRITE_EVENT_BASE + endpoint) } else { 0 }
}

pub const fn keyboard_read_event_mask() -> u64 {
    1u64 << KEYBOARD_READ_EVENT
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum FramebufferFormat {
    Bgr8 = 1,
    Rgb8 = 2,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct FramebufferConfig {
    pub bytes: u64,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub format: FramebufferFormat,
}

impl FramebufferConfig {
    pub const fn new(
        bytes: u64,
        width: u32,
        height: u32,
        stride: u32,
        format: FramebufferFormat,
    ) -> Self {
        Self { bytes, width, height, stride, format }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ServiceId {
    Input = 1,
    Display = 2,
    Terminal = 3,
    Session = 4,
    Commands = 5,
}

impl ServiceId {
    pub const fn index(self) -> usize {
        match self {
            Self::Input => 0,
            Self::Display => 1,
            Self::Terminal => 2,
            Self::Session => 3,
            Self::Commands => 4,
        }
    }

    pub const fn from_index(index: usize) -> Option<Self> {
        match index {
            0 => Some(Self::Input),
            1 => Some(Self::Display),
            2 => Some(Self::Terminal),
            3 => Some(Self::Session),
            4 => Some(Self::Commands),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum MessageKind {
    Key = 1,
    Text = 2,
    Paste = 3,
    SessionInput = 4,
    SessionOutput = 5,
    RenderCells = 6,
    FullRedraw = 7,
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

/// Compact stream payload for one-page service endpoint rings.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct IpcBytes {
    pub kind: MessageKind,
    pub flags: u8,
    pub len: u16,
    pub bytes: [u8; MAX_IPC_BYTES],
}

impl IpcBytes {
    pub const fn empty(kind: MessageKind) -> Self {
        Self { kind, flags: 0, len: 0, bytes: [0; MAX_IPC_BYTES] }
    }

    pub fn from_bytes(kind: MessageKind, bytes: &[u8]) -> Option<Self> {
        if bytes.len() > MAX_IPC_BYTES {
            return None;
        }
        let mut message = Self::empty(kind);
        message.len = bytes.len() as u16;
        message.bytes[..bytes.len()].copy_from_slice(bytes);
        Some(message)
    }

    pub fn as_bytes(&self) -> Option<&[u8]> {
        (self.len as usize <= MAX_IPC_BYTES).then(|| &self.bytes[..self.len as usize])
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct EndpointHeader {
    pub abi_version: u16,
    pub generation: u16,
    pub service_epoch: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyboardRingError {
    Full,
}

/// Kernel-produced, Input-consumed PS/2 byte ring.
#[repr(C)]
pub struct KeyboardByteRing {
    head: AtomicU16,
    tail: AtomicU16,
    dropped: AtomicU64,
    bytes: [UnsafeCell<u8>; KEYBOARD_RING_CAPACITY],
}

unsafe impl Sync for KeyboardByteRing {}

impl KeyboardByteRing {
    pub const fn new() -> Self {
        Self {
            head: AtomicU16::new(0),
            tail: AtomicU16::new(0),
            dropped: AtomicU64::new(0),
            bytes: [const { UnsafeCell::new(0) }; KEYBOARD_RING_CAPACITY],
        }
    }

    pub fn push(&self, byte: u8) -> Result<Notify, KeyboardRingError> {
        let head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Acquire);
        if head.wrapping_sub(tail) >= KEYBOARD_RING_CAPACITY as u16 {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            return Err(KeyboardRingError::Full);
        }
        let was_empty = head == tail;
        let slot = usize::from(head) % KEYBOARD_RING_CAPACITY;
        unsafe { *self.bytes[slot].get() = byte };
        self.head.store(head.wrapping_add(1), Ordering::Release);
        Ok(if was_empty { Notify::Notified } else { Notify::AlreadyNotified })
    }

    pub fn pop(&self) -> Option<u8> {
        let tail = self.tail.load(Ordering::Relaxed);
        let head = self.head.load(Ordering::Acquire);
        if tail == head {
            return None;
        }
        let slot = usize::from(tail) % KEYBOARD_RING_CAPACITY;
        let byte = unsafe { *self.bytes[slot].get() };
        self.tail.store(tail.wrapping_add(1), Ordering::Release);
        Some(byte)
    }

    pub fn pending(&self) -> usize {
        self.head.load(Ordering::Acquire).wrapping_sub(self.tail.load(Ordering::Acquire)) as usize
    }

    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Acquire)
    }
}

impl Default for KeyboardByteRing {
    fn default() -> Self {
        Self::new()
    }
}

impl EndpointHeader {
    pub const fn new(generation: u16, service_epoch: u64) -> Self {
        Self { abi_version: ABI_VERSION, generation, service_epoch }
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SharedSendError {
    Full,
    Stale,
    Disconnected,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SharedReceiveError {
    Empty,
    Stale,
    Disconnected,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Notify {
    Notified,
    AlreadyNotified,
}

#[derive(Debug)]
pub struct Doorbell {
    notified: AtomicBool,
}

impl Doorbell {
    pub const fn new() -> Self {
        Self { notified: AtomicBool::new(false) }
    }

    pub fn ring(&self) -> bool {
        !self.notified.swap(true, Ordering::AcqRel)
    }

    pub fn take(&self) -> bool {
        self.notified.swap(false, Ordering::AcqRel)
    }
}

impl Default for Doorbell {
    fn default() -> Self {
        Self::new()
    }
}

/// Fixed SPSC ring that can be placed directly in a shared endpoint page.
#[repr(C)]
/// Fixed SPSC transport for trusted producer/consumer peers.
///
/// Endpoint identity protects the kernel and restart generations; it is not a
/// hostile-peer memory-isolation boundary, so service payload validation stays
/// at the service boundary.
pub struct SharedIpc<T: Copy, const N: usize> {
    endpoint: EndpointHeader,
    connected: AtomicBool,
    head: AtomicU16,
    tail: AtomicU16,
    doorbell: Doorbell,
    entries: [UnsafeCell<MaybeUninit<T>>; N],
}

unsafe impl<T: Copy + Send, const N: usize> Send for SharedIpc<T, N> {}
unsafe impl<T: Copy + Send, const N: usize> Sync for SharedIpc<T, N> {}

impl<T: Copy, const N: usize> SharedIpc<T, N> {
    pub const fn new(endpoint: EndpointHeader) -> Self {
        assert!(N > 0 && N <= u16::MAX as usize);
        Self {
            endpoint,
            connected: AtomicBool::new(true),
            head: AtomicU16::new(0),
            tail: AtomicU16::new(0),
            doorbell: Doorbell::new(),
            entries: [const { UnsafeCell::new(MaybeUninit::uninit()) }; N],
        }
    }

    pub const fn endpoint(&self) -> EndpointHeader {
        self.endpoint
    }

    pub fn disconnect(&self) {
        self.connected.store(false, Ordering::Release);
        self.doorbell.ring();
    }

    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Acquire)
    }

    pub fn send(&self, identity: MessageIdentity, entry: T) -> Result<Notify, SharedSendError> {
        if !identity.accepts(self.endpoint) {
            return Err(SharedSendError::Stale);
        }
        if !self.connected.load(Ordering::Acquire) {
            return Err(SharedSendError::Disconnected);
        }
        let head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Acquire);
        if head.wrapping_sub(tail) >= N as u16 {
            return Err(SharedSendError::Full);
        }
        let was_empty = head == tail;
        let slot = usize::from(head) % N;
        unsafe { (*self.entries[slot].get()).write(entry) };
        self.head.store(head.wrapping_add(1), Ordering::Release);
        Ok(if was_empty && self.doorbell.ring() {
            Notify::Notified
        } else {
            Notify::AlreadyNotified
        })
    }

    pub fn receive(&self, identity: MessageIdentity) -> Result<T, SharedReceiveError> {
        self.receive_with_notify(identity).map(|(entry, _)| entry)
    }

    pub fn receive_with_notify(
        &self,
        identity: MessageIdentity,
    ) -> Result<(T, Notify), SharedReceiveError> {
        if !identity.accepts(self.endpoint) {
            return Err(SharedReceiveError::Stale);
        }
        if !self.connected.load(Ordering::Acquire) {
            return Err(SharedReceiveError::Disconnected);
        }
        let tail = self.tail.load(Ordering::Relaxed);
        let head = self.head.load(Ordering::Acquire);
        if tail == head {
            return Err(SharedReceiveError::Empty);
        }
        let was_full = head.wrapping_sub(tail) >= N as u16;
        let slot = usize::from(tail) % N;
        let entry = unsafe { (*self.entries[slot].get()).assume_init_read() };
        self.tail.store(tail.wrapping_add(1), Ordering::Release);
        if tail.wrapping_add(1) == head {
            self.doorbell.take();
        }
        Ok((entry, if was_full { Notify::Notified } else { Notify::AlreadyNotified }))
    }

    pub fn pending(&self) -> usize {
        self.head.load(Ordering::Acquire).wrapping_sub(self.tail.load(Ordering::Acquire)) as usize
    }
}

pub type InputIpc = SharedIpc<InputMessage, 32>;
pub type RenderIpc = SharedIpc<RenderMessage, 1>;
pub type StreamIpc = SharedIpc<IpcBytes, 8>;

const _: () = assert!(core::mem::size_of::<FramebufferConfig>() <= IPC_PAGE_BYTES);

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
        assert!(IpcBytes::from_bytes(MessageKind::SessionOutput, &[0; MAX_IPC_BYTES]).is_some());
        assert!(
            IpcBytes::from_bytes(MessageKind::SessionOutput, &[0; MAX_IPC_BYTES + 1]).is_none()
        );
    }

    #[test]
    fn endpoint_rejects_stale_identity() {
        let header = EndpointHeader::new(4, 8);
        assert!(header.accepts(4, 8));
        assert!(!header.accepts(3, 8));
        assert!(!header.accepts(4, 9));
    }

    #[test]
    fn keyboard_ring_is_bounded_and_fifo() {
        let ring = KeyboardByteRing::new();
        for byte in 0..KEYBOARD_RING_CAPACITY as u16 {
            assert!(ring.push(byte as u8).is_ok());
        }
        assert_eq!(ring.push(0xff), Err(KeyboardRingError::Full));
        assert_eq!(ring.pending(), KEYBOARD_RING_CAPACITY);
        assert_eq!(ring.dropped(), 1);
        for byte in 0..KEYBOARD_RING_CAPACITY as u16 {
            assert_eq!(ring.pop(), Some(byte as u8));
        }
        assert_eq!(ring.pop(), None);
    }

    #[test]
    fn keyboard_ring_reports_only_the_empty_to_nonempty_edge() {
        let ring = KeyboardByteRing::new();
        assert_eq!(ring.push(1), Ok(Notify::Notified));
        assert_eq!(ring.push(2), Ok(Notify::AlreadyNotified));
    }

    #[test]
    fn shared_ring_reports_the_full_to_not_full_edge() {
        let ring = RenderIpc::new(EndpointHeader::new(1, 1));
        let identity = ring.endpoint().identity();
        let message = RenderMessage::empty(MessageKind::RenderCells);
        assert_eq!(ring.send(identity, message), Ok(Notify::Notified));
        assert_eq!(ring.send(identity, message), Err(SharedSendError::Full));
        assert_eq!(ring.receive_with_notify(identity), Ok((message, Notify::Notified)));
        assert_eq!(ring.receive(identity), Err(SharedReceiveError::Empty));
    }

    #[test]
    fn event_masks_are_fixed_and_disjoint() {
        let mut all = 0;
        for endpoint in 0..IPC_ENDPOINT_COUNT {
            let read = ipc_read_event_mask(endpoint);
            let write = ipc_write_event_mask(endpoint);
            assert_eq!(all & read, 0);
            all |= read;
            assert_eq!(all & write, 0);
            all |= write;
        }
        let keyboard = keyboard_read_event_mask();
        assert_eq!(all & keyboard, 0);
        all |= keyboard;
        assert_eq!(EVENT_COUNT, 13);
        assert_eq!(all.count_ones(), EVENT_COUNT as u32);
    }

    #[test]
    fn shared_rings_fit_their_endpoint_page() {
        assert!(core::mem::size_of::<SharedIpc<InputMessage, 32>>() <= IPC_PAGE_BYTES);
        assert!(core::mem::size_of::<SharedIpc<RenderMessage, 1>>() <= IPC_PAGE_BYTES);
        assert!(core::mem::size_of::<SharedIpc<IpcBytes, 8>>() <= IPC_PAGE_BYTES);
        assert_eq!(
            IpcBytes::from_bytes(MessageKind::Text, b"ok").unwrap().as_bytes(),
            Some(&b"ok"[..])
        );
    }
}
