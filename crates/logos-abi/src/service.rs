use crate as logos_abi;
use core::mem::{align_of, size_of};

pub const MAGIC: [u8; 4] = *b"LGSV";
pub const ABI: u16 = 4;
pub const MAX_TEXT: usize = 256;
pub const READY: u32 = 1;
pub const READ_INPUT: u32 = 2;
pub const PRESENT_PIXEL: u32 = 3;
pub const PRESENT_TEXT: u32 = 4;
pub const CLEAR_DISPLAY: u32 = 5;
pub const COMPLETE: u32 = 6;
pub const SYSCALL: u32 = 7;
pub const SESSION_REPLY: u32 = 8;
pub const SESSION_EFFECT: u32 = 9;
pub const STORE_REQUEST: u32 = 10;
pub const STORE_REPLY: u32 = 11;
pub const BLOCK_REQUEST: u32 = 12;
pub const BLOCK_REPLY: u32 = 13;
pub const NETWORK_REQUEST: u32 = 14;
pub const NETWORK_REPLY: u32 = 15;
pub const NETWORK_WAIT: u32 = 16;
pub const NETWORK_EVENT: u32 = 17;
pub const NETWORK_DEVICE_REQUEST: u32 = 18;
pub const NETWORK_DEVICE_REPLY: u32 = 19;
pub const REMOTE_GATE: u32 = 20;
pub const PANIC: u32 = 21;
pub const ACKNOWLEDGED: u32 = 1;
pub const LIFECYCLE_STARTING: u32 = 0;
pub const LIFECYCLE_READY: u32 = 1;
pub const STORAGE_FORMATTED: u32 = 1;
pub const STORAGE_RECOVERED: u32 = 2;
pub const STORAGE_RECOVERED_INCOMPLETE: u32 = 3;
pub const STORAGE_CORRUPT: u32 = 4;
pub const STORAGE_IO_FAILED: u32 = 5;
pub const STORAGE_UNAVAILABLE: u32 = 6;

/// Core-owned control page shared by one native service.
///
/// ABI v4 keeps the control header compact and puts service-specific request
/// payloads behind typed endpoint pages. The header is stored in a dedicated
/// page mapping; endpoint mappings are granted explicitly by the service
/// specification.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct ControlPage {
    pub abi: u16,
    pub reserved: u16,
    pub operation: u32,
    pub status: u32,
    pub generation: u32,
    pub lifecycle: u32,
    pub input_page: u64,
    pub display_page: u64,
    pub session_client_page: u64,
    pub session_server_page: u64,
    pub effect_page: u64,
    pub slot0: u32,
    pub slot1: u32,
    pub slot2: u32,
    pub payload_length: u32,
    pub payload: [u8; MAX_TEXT],
    pub shared_page: u32,
    pub network_rx_page: u32,
    pub network_tx_page: u32,
}

/// Explicit state values shared by typed endpoint pages.
///
/// | page | service transition | Core transition | reset/replacement |
/// | --- | --- | --- | --- |
/// | Input | `Ready -> Waiting -> Ready` (`wait_at`, `take_at`) | `Waiting -> Reply` (`deliver_at`) | reset to `Ready`; generation mismatch rejects |
/// | Display | `Ready -> Request -> Ready` (`request_*`, `finish_at`) | `Request -> Complete` (`complete_at`) | reset to `Ready`; generation mismatch rejects |
///
/// Unknown scalar states and malformed payloads are rejected without a write.
/// Session endpoint transitions are role-specific:
///
/// | role | service transitions | Core transitions |
/// | --- | --- | --- |
/// | client | `Ready -> Request`, terminal result -> `Ready` | `Request -> Waiting -> Reply/Denied/Failed/Cancelled` |
/// | server | `Ready -> Waiting`, `Request -> Processing -> Reply/Failed/Cancelled` | `Waiting -> Request`, terminal result -> `Ready` |
/// | effect | `Ready -> Request`, terminal result -> `Ready` | `Request -> Waiting -> Reply/Denied/Failed/Cancelled` |
///
/// Every transition requires both generations and the active request ID. Unknown
/// states and malformed bounded values are rejected without a write. Reset and
/// replacement install `Ready` with new generations, invalidating pending work.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum EndpointState {
    Empty = 0,
    Ready = 1,
    Request = 2,
    Reply = 3,
    Waiting = 4,
    Complete = 5,
    Failed = 6,
}

impl EndpointState {
    pub const fn wire(self) -> u32 {
        self as u32
    }

    pub const fn from_wire(value: u32) -> Option<Self> {
        Some(match value {
            0 => Self::Empty,
            1 => Self::Ready,
            2 => Self::Request,
            3 => Self::Reply,
            4 => Self::Waiting,
            5 => Self::Complete,
            6 => Self::Failed,
            _ => return None,
        })
    }
}

/// Fixed-size Input endpoint page.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct InputPage {
    pub generation: u32,
    pub state: u32,
    pub event: u32,
    pub reserved: [u8; logos_abi::PAGE_SIZE - 12],
}

/// Fixed-size Display endpoint page.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct DisplayPage {
    pub generation: u32,
    pub state: u32,
    pub operation: u32,
    pub x: u32,
    pub y: u32,
    pub color: u32,
    pub text_length: u32,
    pub text: [u8; MAX_TEXT],
    pub reserved: [u8; logos_abi::PAGE_SIZE - 284],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum SessionPageState {
    Ready = 1,
    Waiting = 2,
    Request = 3,
    Processing = 4,
    Reply = 5,
    Failed = 6,
    Cancelled = 7,
    Denied = 8,
}

impl SessionPageState {
    pub const fn wire(self) -> u32 {
        self as u32
    }

    pub const fn from_wire(value: u32) -> Option<Self> {
        Some(match value {
            1 => Self::Ready,
            2 => Self::Waiting,
            3 => Self::Request,
            4 => Self::Processing,
            5 => Self::Reply,
            6 => Self::Failed,
            7 => Self::Cancelled,
            8 => Self::Denied,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum SessionStatus {
    Complete = 1,
    Denied = 2,
    Failed = 3,
    Cancelled = 4,
}

impl SessionStatus {
    pub const fn wire(self) -> u32 {
        self as u32
    }

    pub const fn from_wire(value: u32) -> Option<Self> {
        Some(match value {
            1 => Self::Complete,
            2 => Self::Denied,
            3 => Self::Failed,
            4 => Self::Cancelled,
            _ => return None,
        })
    }

    const fn state(self) -> SessionPageState {
        match self {
            Self::Complete => SessionPageState::Reply,
            Self::Denied => SessionPageState::Denied,
            Self::Failed => SessionPageState::Failed,
            Self::Cancelled => SessionPageState::Cancelled,
        }
    }
}

/// Terminal-owned Session request/reply endpoint. Core mediates this page.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct SessionClientPage {
    pub service_generation: u32,
    pub endpoint_generation: u32,
    pub state: u32,
    pub request_id: u32,
    pub operation: u32,
    pub request_length: u32,
    pub reply_status: u32,
    pub reply_length: u32,
    pub request: [u8; MAX_TEXT],
    pub reply: [u8; MAX_TEXT],
}

/// Sessions-owned server endpoint. It is never mapped into a client service.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct SessionServerPage {
    pub service_generation: u32,
    pub endpoint_generation: u32,
    pub state: u32,
    pub request_id: u32,
    pub caller_low: u32,
    pub caller_high: u32,
    pub operation: u32,
    pub request_length: u32,
    pub reply_status: u32,
    pub reply_length: u32,
    pub request: [u8; MAX_TEXT],
    pub reply: [u8; MAX_TEXT],
}

/// Sessions-owned privileged-effect endpoint. Core alone executes requests.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct EffectPage {
    pub service_generation: u32,
    pub endpoint_generation: u32,
    pub state: u32,
    pub request_id: u32,
    pub effect: u32,
    pub request_length: u32,
    pub result: u32,
    pub reply_length: u32,
    pub request: [u8; MAX_TEXT],
    pub reply: [u8; MAX_TEXT],
}

/// Fixed-size Store endpoint page.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct StoreEndpointPage {
    pub generation: u32,
    pub state: EndpointState,
    pub request: [u8; STORE_REQUEST_BYTES],
    pub reply: [u8; STORE_REPLY_BYTES],
    pub reserved: [u8; logos_abi::PAGE_SIZE - 127],
}

/// Fixed-size Block endpoint page.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct BlockEndpointPage {
    pub generation: u32,
    pub state: EndpointState,
    pub request: [u8; BLOCK_REQUEST_BYTES],
    pub reply: [u8; BLOCK_REPLY_BYTES],
    pub reserved: [u8; logos_abi::PAGE_SIZE - 61],
}

/// Fixed-size Network endpoint page.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct NetworkPage {
    pub generation: u32,
    pub state: EndpointState,
    pub request: [u8; NETWORK_REQUEST_BYTES],
    pub reply: [u8; NETWORK_REPLY_BYTES],
    pub event: [u8; NETWORK_EVENT_BYTES],
    pub reserved: [u8; logos_abi::PAGE_SIZE - 208],
}

/// Fixed-size Remote endpoint page.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct RemotePage {
    pub generation: u32,
    pub state: EndpointState,
    pub request: [u8; REMOTE_GATE_REQUEST_BYTES],
    pub reply: [u8; REMOTE_GATE_REPLY_BYTES],
    pub reserved: [u8; logos_abi::PAGE_SIZE - 52],
}

impl InputPage {
    pub const fn new(generation: u32) -> Self {
        Self {
            generation,
            state: EndpointState::Ready.wire(),
            event: 0,
            reserved: [0; logos_abi::PAGE_SIZE - 12],
        }
    }

    /// # Safety
    /// `address` must point to a live, aligned InputPage mapping.
    pub unsafe fn reset_at(address: u64, generation: u32) -> bool {
        if address == 0 || generation == 0 || !address.is_multiple_of(align_of::<Self>() as u64) {
            return false;
        }
        unsafe { (address as *mut Self).write_volatile(Self::new(generation)) };
        true
    }

    /// # Safety
    /// `address` must point to a live, aligned InputPage mapping.
    pub unsafe fn wait_at(address: u64, generation: u32) -> bool {
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if page.generation != generation
            || EndpointState::from_wire(page.state) != Some(EndpointState::Ready)
        {
            return false;
        }
        page.state = EndpointState::Waiting.wire();
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    /// # Safety
    /// `address` must point to a live, aligned InputPage mapping.
    pub unsafe fn waiting_at(address: u64, generation: u32) -> bool {
        if address == 0 {
            return false;
        }
        let page = address as *const Self;
        let page_generation = unsafe { core::ptr::addr_of!((*page).generation).read_volatile() };
        let state = unsafe { core::ptr::addr_of!((*page).state).read_volatile() };
        page_generation == generation
            && EndpointState::from_wire(state) == Some(EndpointState::Waiting)
    }

    /// # Safety
    /// `address` must point to a live, aligned InputPage mapping owned by Core.
    pub unsafe fn deliver_at(address: u64, generation: u32, event: u8) -> bool {
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if page.generation != generation
            || EndpointState::from_wire(page.state) != Some(EndpointState::Waiting)
        {
            return false;
        }
        page.event = u32::from(event);
        page.state = EndpointState::Reply.wire();
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    /// # Safety
    /// `address` must point to a live, aligned InputPage mapping owned by the service.
    pub unsafe fn take_at(address: u64, generation: u32) -> Option<u8> {
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if page.generation != generation
            || EndpointState::from_wire(page.state) != Some(EndpointState::Reply)
        {
            return None;
        }
        let event = u8::try_from(page.event).ok()?;
        page.event = 0;
        page.state = EndpointState::Ready.wire();
        unsafe { (address as *mut Self).write_volatile(page) };
        Some(event)
    }
}

impl DisplayPage {
    pub const fn new(generation: u32) -> Self {
        Self {
            generation,
            state: EndpointState::Ready.wire(),
            operation: 0,
            x: 0,
            y: 0,
            color: 0,
            text_length: 0,
            text: [0; MAX_TEXT],
            reserved: [0; logos_abi::PAGE_SIZE - 284],
        }
    }

    /// # Safety
    /// `address` must point to a live, aligned DisplayPage mapping.
    pub unsafe fn reset_at(address: u64, generation: u32) -> bool {
        if address == 0 || generation == 0 || !address.is_multiple_of(align_of::<Self>() as u64) {
            return false;
        }
        unsafe { (address as *mut Self).write_volatile(Self::new(generation)) };
        true
    }

    /// # Safety
    /// `address` must point to a live, aligned DisplayPage mapping owned by the service.
    pub unsafe fn request_pixel_at(
        address: u64,
        generation: u32,
        x: u32,
        y: u32,
        color: logos_abi::DisplayColor,
    ) -> bool {
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if page.generation != generation
            || EndpointState::from_wire(page.state) != Some(EndpointState::Ready)
        {
            return false;
        }
        page.operation = PRESENT_PIXEL;
        page.x = x;
        page.y = y;
        page.color = color.wire();
        page.text_length = 0;
        page.state = EndpointState::Request.wire();
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    /// # Safety
    /// `address` must point to a live, aligned DisplayPage mapping owned by the service.
    pub unsafe fn request_text_at(
        address: u64,
        generation: u32,
        x: u32,
        y: u32,
        color: logos_abi::DisplayColor,
        text: &[u8],
    ) -> bool {
        if text.len() > MAX_TEXT {
            return false;
        }
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if page.generation != generation
            || EndpointState::from_wire(page.state) != Some(EndpointState::Ready)
        {
            return false;
        }
        page.operation = PRESENT_TEXT;
        page.x = x;
        page.y = y;
        page.color = color.wire();
        page.text = [0; MAX_TEXT];
        page.text[..text.len()].copy_from_slice(text);
        page.text_length = text.len() as u32;
        page.state = EndpointState::Request.wire();
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    /// # Safety
    /// `address` must point to a live, aligned DisplayPage mapping owned by the service.
    pub unsafe fn request_clear_at(address: u64, generation: u32) -> bool {
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if page.generation != generation
            || EndpointState::from_wire(page.state) != Some(EndpointState::Ready)
        {
            return false;
        }
        page.operation = CLEAR_DISPLAY;
        page.text_length = 0;
        page.state = EndpointState::Request.wire();
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    /// # Safety
    /// `address` must point to a live, aligned DisplayPage mapping.
    pub unsafe fn pending_at(address: u64, generation: u32) -> bool {
        if address == 0 {
            return false;
        }
        let page = address as *const Self;
        let page_generation = unsafe { core::ptr::addr_of!((*page).generation).read_volatile() };
        let state = unsafe { core::ptr::addr_of!((*page).state).read_volatile() };
        page_generation == generation
            && EndpointState::from_wire(state) == Some(EndpointState::Request)
    }

    /// # Safety
    /// `address` must point to a live, aligned DisplayPage mapping owned by Core.
    pub unsafe fn request_at(address: u64, generation: u32) -> Option<DisplayRequest> {
        let page = unsafe { (address as *const Self).read_volatile() };
        if page.generation != generation
            || EndpointState::from_wire(page.state) != Some(EndpointState::Request)
            || !matches!(page.operation, PRESENT_PIXEL | PRESENT_TEXT | CLEAR_DISPLAY)
        {
            return None;
        }
        let color = logos_abi::DisplayColor::from_wire(page.color)?;
        let length = usize::try_from(page.text_length).ok()?;
        (length <= MAX_TEXT).then_some(DisplayRequest {
            operation: page.operation,
            x: page.x,
            y: page.y,
            color,
            text: page.text,
            length,
        })
    }

    /// # Safety
    /// `address` must point to a live, aligned DisplayPage mapping owned by Core.
    pub unsafe fn complete_at(address: u64, generation: u32) -> bool {
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if page.generation != generation
            || EndpointState::from_wire(page.state) != Some(EndpointState::Request)
        {
            return false;
        }
        page.state = EndpointState::Complete.wire();
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    /// # Safety
    /// `address` must point to a live, aligned DisplayPage mapping owned by the service.
    pub unsafe fn finish_at(address: u64, generation: u32) -> bool {
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if page.generation != generation
            || EndpointState::from_wire(page.state) != Some(EndpointState::Complete)
        {
            return false;
        }
        page.state = EndpointState::Ready.wire();
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }
}

#[derive(Clone, Copy)]
pub struct DisplayRequest {
    pub operation: u32,
    pub x: u32,
    pub y: u32,
    pub color: logos_abi::DisplayColor,
    pub text: [u8; MAX_TEXT],
    pub length: usize,
}

#[derive(Clone, Copy)]
pub struct SessionClientRequest {
    pub id: u32,
    pub request: logos_abi::SessionRequest,
}

#[derive(Clone, Copy)]
pub struct SessionClientReply {
    pub id: u32,
    pub status: SessionStatus,
    pub reply: logos_abi::SessionReply,
}

#[derive(Clone, Copy)]
pub struct SessionServerRequest {
    pub id: u32,
    pub caller: u64,
    pub request: logos_abi::SessionRequest,
}

#[derive(Clone, Copy)]
pub struct SessionServerReply {
    pub id: u32,
    pub status: SessionStatus,
    pub reply: logos_abi::SessionReply,
}

#[derive(Clone, Copy)]
pub struct EffectMessage {
    pub id: u32,
    pub request: logos_abi::EffectRequest,
}

#[derive(Clone, Copy)]
pub struct EffectResponse {
    pub id: u32,
    pub reply: logos_abi::EffectReply,
}

impl SessionClientPage {
    pub const fn new(service_generation: u32, endpoint_generation: u32) -> Self {
        Self {
            service_generation,
            endpoint_generation,
            state: SessionPageState::Ready.wire(),
            request_id: 0,
            operation: 0,
            request_length: 0,
            reply_status: 0,
            reply_length: 0,
            request: [0; MAX_TEXT],
            reply: [0; MAX_TEXT],
        }
    }

    /// # Safety
    /// `address` must point to a live, aligned client page mapping.
    pub unsafe fn reset_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> bool {
        if !valid_page_identity::<Self>(address, service_generation, endpoint_generation) {
            return false;
        }
        unsafe {
            (address as *mut Self)
                .write_volatile(Self::new(service_generation, endpoint_generation))
        };
        true
    }

    /// # Safety
    /// The mapped client service owns the page while creating the request.
    pub unsafe fn request_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        id: u32,
        request: logos_abi::SessionRequest,
    ) -> bool {
        if id == 0 || !request.valid() {
            return false;
        }
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if !client_identity(&page, service_generation, endpoint_generation)
            || SessionPageState::from_wire(page.state) != Some(SessionPageState::Ready)
        {
            return false;
        }
        page.request_id = id;
        page.operation = request.syscall as u32;
        page.request_length = request.length as u32;
        page.request = [0; MAX_TEXT];
        page.request[..request.length].copy_from_slice(&request.argument[..request.length]);
        page.reply_status = 0;
        page.reply_length = 0;
        page.reply = [0; MAX_TEXT];
        page.state = SessionPageState::Request.wire();
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    /// # Safety
    /// Core owns the transition from request to waiting.
    pub unsafe fn take_request_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> Option<SessionClientRequest> {
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if !client_identity(&page, service_generation, endpoint_generation)
            || SessionPageState::from_wire(page.state) != Some(SessionPageState::Request)
        {
            return None;
        }
        let request = decode_session_request(page.operation, page.request_length, page.request)?;
        if page.request_id == 0 {
            return None;
        }
        page.state = SessionPageState::Waiting.wire();
        unsafe { (address as *mut Self).write_volatile(page) };
        Some(SessionClientRequest { id: page.request_id, request })
    }

    /// # Safety
    /// Core may inspect the current request while coordinating a synchronous relay.
    pub unsafe fn current_request_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> Option<SessionClientRequest> {
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        let state = SessionPageState::from_wire(page.state)?;
        if !client_identity(&page, service_generation, endpoint_generation)
            || page.request_id == 0
            || !matches!(state, SessionPageState::Request | SessionPageState::Waiting)
        {
            return None;
        }
        let request = decode_session_request(page.operation, page.request_length, page.request)?;
        if state == SessionPageState::Request {
            page.state = SessionPageState::Waiting.wire();
            unsafe { (address as *mut Self).write_volatile(page) };
        }
        Some(SessionClientRequest { id: page.request_id, request })
    }

    /// # Safety
    /// Core reads only scalar state and generation fields.
    pub unsafe fn pending_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> bool {
        let page = unsafe { (address as *const Self).read_volatile() };
        client_identity(&page, service_generation, endpoint_generation)
            && SessionPageState::from_wire(page.state) == Some(SessionPageState::Request)
    }

    /// # Safety
    /// Core owns client completion.
    pub unsafe fn reply_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        id: u32,
        status: SessionStatus,
        reply: logos_abi::SessionReply,
    ) -> bool {
        if !reply.valid() || reply.length > MAX_TEXT {
            return false;
        }
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if !client_identity(&page, service_generation, endpoint_generation)
            || page.request_id != id
            || SessionPageState::from_wire(page.state) != Some(SessionPageState::Waiting)
        {
            return false;
        }
        page.reply_status = status.wire();
        page.reply_length = reply.length as u32;
        page.reply = [0; MAX_TEXT];
        page.reply[..reply.length].copy_from_slice(&reply.text[..reply.length]);
        page.state = status.state().wire();
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    /// # Safety
    /// The mapped client service owns reply consumption.
    pub unsafe fn finish_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        id: u32,
    ) -> Option<SessionClientReply> {
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        let state = SessionPageState::from_wire(page.state)?;
        if !client_identity(&page, service_generation, endpoint_generation)
            || page.request_id != id
            || !matches!(
                state,
                SessionPageState::Reply
                    | SessionPageState::Denied
                    | SessionPageState::Failed
                    | SessionPageState::Cancelled
            )
        {
            return None;
        }
        let status = SessionStatus::from_wire(page.reply_status)?;
        let reply = decode_session_reply(page.reply_length, page.reply)?;
        page = Self::new(service_generation, endpoint_generation);
        unsafe { (address as *mut Self).write_volatile(page) };
        Some(SessionClientReply { id, status, reply })
    }

    /// # Safety
    /// Core may inspect a completed reply before waking the client service.
    pub unsafe fn reply_at_current(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> Option<SessionClientReply> {
        let page = unsafe { (address as *const Self).read_volatile() };
        let state = SessionPageState::from_wire(page.state)?;
        if !client_identity(&page, service_generation, endpoint_generation)
            || page.request_id == 0
            || !matches!(
                state,
                SessionPageState::Reply
                    | SessionPageState::Denied
                    | SessionPageState::Failed
                    | SessionPageState::Cancelled
            )
        {
            return None;
        }
        Some(SessionClientReply {
            id: page.request_id,
            status: SessionStatus::from_wire(page.reply_status)?,
            reply: decode_session_reply(page.reply_length, page.reply)?,
        })
    }

    /// # Safety
    /// Core may cancel only the current request.
    pub unsafe fn cancel_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        id: u32,
    ) -> bool {
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if !client_identity(&page, service_generation, endpoint_generation)
            || page.request_id != id
            || !matches!(
                SessionPageState::from_wire(page.state),
                Some(SessionPageState::Request | SessionPageState::Waiting)
            )
        {
            return false;
        }
        page.reply_status = SessionStatus::Cancelled.wire();
        page.reply_length = 0;
        page.reply = [0; MAX_TEXT];
        page.state = SessionPageState::Cancelled.wire();
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }
}

impl SessionServerPage {
    pub const fn new(service_generation: u32, endpoint_generation: u32) -> Self {
        Self {
            service_generation,
            endpoint_generation,
            state: SessionPageState::Ready.wire(),
            request_id: 0,
            caller_low: 0,
            caller_high: 0,
            operation: 0,
            request_length: 0,
            reply_status: 0,
            reply_length: 0,
            request: [0; MAX_TEXT],
            reply: [0; MAX_TEXT],
        }
    }

    /// # Safety
    /// `address` must point to a live, aligned server page mapping.
    pub unsafe fn reset_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> bool {
        if !valid_page_identity::<Self>(address, service_generation, endpoint_generation) {
            return false;
        }
        unsafe {
            (address as *mut Self)
                .write_volatile(Self::new(service_generation, endpoint_generation))
        };
        true
    }

    /// # Safety
    /// The Sessions service owns the ready-to-waiting transition.
    pub unsafe fn wait_at(address: u64, service_generation: u32, endpoint_generation: u32) -> bool {
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if !server_identity(&page, service_generation, endpoint_generation)
            || SessionPageState::from_wire(page.state) != Some(SessionPageState::Ready)
        {
            return false;
        }
        page.state = SessionPageState::Waiting.wire();
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    /// # Safety
    /// Core reads only scalar state and generation fields.
    pub unsafe fn waiting_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> bool {
        let page = unsafe { (address as *const Self).read_volatile() };
        server_identity(&page, service_generation, endpoint_generation)
            && SessionPageState::from_wire(page.state) == Some(SessionPageState::Waiting)
    }

    /// # Safety
    /// Core owns delivery into a waiting server page.
    pub unsafe fn deliver_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        id: u32,
        caller: u64,
        request: logos_abi::SessionRequest,
    ) -> bool {
        if id == 0 || !request.valid() {
            return false;
        }
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if !server_identity(&page, service_generation, endpoint_generation)
            || SessionPageState::from_wire(page.state) != Some(SessionPageState::Waiting)
        {
            return false;
        }
        page.request_id = id;
        page.caller_low = caller as u32;
        page.caller_high = (caller >> 32) as u32;
        page.operation = request.syscall as u32;
        page.request_length = request.length as u32;
        page.request = [0; MAX_TEXT];
        page.request[..request.length].copy_from_slice(&request.argument[..request.length]);
        page.state = SessionPageState::Request.wire();
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    /// # Safety
    /// The Sessions service owns request consumption.
    pub unsafe fn take_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> Option<SessionServerRequest> {
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if !server_identity(&page, service_generation, endpoint_generation)
            || SessionPageState::from_wire(page.state) != Some(SessionPageState::Request)
            || page.request_id == 0
        {
            return None;
        }
        let request = decode_session_request(page.operation, page.request_length, page.request)?;
        page.state = SessionPageState::Processing.wire();
        unsafe { (address as *mut Self).write_volatile(page) };
        Some(SessionServerRequest {
            id: page.request_id,
            caller: u64::from(page.caller_low) | (u64::from(page.caller_high) << 32),
            request,
        })
    }

    /// # Safety
    /// The Sessions service replies only to its current processing request.
    pub unsafe fn reply_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        id: u32,
        status: SessionStatus,
        reply: logos_abi::SessionReply,
    ) -> bool {
        if !reply.valid() || reply.length > MAX_TEXT {
            return false;
        }
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if !server_identity(&page, service_generation, endpoint_generation)
            || page.request_id != id
            || SessionPageState::from_wire(page.state) != Some(SessionPageState::Processing)
        {
            return false;
        }
        page.reply_status = status.wire();
        page.reply_length = reply.length as u32;
        page.reply = [0; MAX_TEXT];
        page.reply[..reply.length].copy_from_slice(&reply.text[..reply.length]);
        page.state = status.state().wire();
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    /// # Safety
    /// Core owns reply consumption and deterministic reset.
    pub unsafe fn take_reply_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        id: u32,
    ) -> Option<SessionServerReply> {
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        let state = SessionPageState::from_wire(page.state)?;
        if !server_identity(&page, service_generation, endpoint_generation)
            || page.request_id != id
            || !matches!(
                state,
                SessionPageState::Reply
                    | SessionPageState::Denied
                    | SessionPageState::Failed
                    | SessionPageState::Cancelled
            )
        {
            return None;
        }
        let status = SessionStatus::from_wire(page.reply_status)?;
        let reply = decode_session_reply(page.reply_length, page.reply)?;
        page = Self::new(service_generation, endpoint_generation);
        unsafe { (address as *mut Self).write_volatile(page) };
        Some(SessionServerReply { id, status, reply })
    }

    /// # Safety
    /// Core reads only scalar state and generation fields.
    pub unsafe fn reply_pending_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> bool {
        let page = unsafe { (address as *const Self).read_volatile() };
        server_identity(&page, service_generation, endpoint_generation)
            && matches!(
                SessionPageState::from_wire(page.state),
                Some(
                    SessionPageState::Reply
                        | SessionPageState::Denied
                        | SessionPageState::Failed
                        | SessionPageState::Cancelled
                )
            )
    }
}

impl EffectPage {
    pub const fn new(service_generation: u32, endpoint_generation: u32) -> Self {
        Self {
            service_generation,
            endpoint_generation,
            state: SessionPageState::Ready.wire(),
            request_id: 0,
            effect: 0,
            request_length: 0,
            result: 0,
            reply_length: 0,
            request: [0; MAX_TEXT],
            reply: [0; MAX_TEXT],
        }
    }

    /// # Safety
    /// `address` must point to a live, aligned effect page mapping.
    pub unsafe fn reset_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> bool {
        if !valid_page_identity::<Self>(address, service_generation, endpoint_generation) {
            return false;
        }
        unsafe {
            (address as *mut Self)
                .write_volatile(Self::new(service_generation, endpoint_generation))
        };
        true
    }

    /// # Safety
    /// The Sessions service owns effect request creation.
    pub unsafe fn request_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        id: u32,
        request: logos_abi::EffectRequest,
    ) -> bool {
        if id == 0 || !request.valid() {
            return false;
        }
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if !effect_identity(&page, service_generation, endpoint_generation)
            || SessionPageState::from_wire(page.state) != Some(SessionPageState::Ready)
        {
            return false;
        }
        page.request_id = id;
        page.effect = request.effect as u32;
        page.request_length = request.length as u32;
        page.request = [0; MAX_TEXT];
        page.request[..request.length].copy_from_slice(&request.argument[..request.length]);
        page.result = 0;
        page.reply_length = 0;
        page.reply = [0; MAX_TEXT];
        page.state = SessionPageState::Request.wire();
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    /// # Safety
    /// Core owns effect request consumption.
    pub unsafe fn take_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> Option<EffectMessage> {
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if !effect_identity(&page, service_generation, endpoint_generation)
            || SessionPageState::from_wire(page.state) != Some(SessionPageState::Request)
            || page.request_id == 0
        {
            return None;
        }
        let request = decode_effect_request(page.effect, page.request_length, page.request)?;
        page.state = SessionPageState::Waiting.wire();
        unsafe { (address as *mut Self).write_volatile(page) };
        Some(EffectMessage { id: page.request_id, request })
    }

    /// # Safety
    /// Core reads only scalar state and generation fields.
    pub unsafe fn pending_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> bool {
        let page = unsafe { (address as *const Self).read_volatile() };
        effect_identity(&page, service_generation, endpoint_generation)
            && SessionPageState::from_wire(page.state) == Some(SessionPageState::Request)
    }

    /// # Safety
    /// Core owns effect completion after authorization and execution.
    pub unsafe fn reply_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        id: u32,
        reply: logos_abi::EffectReply,
    ) -> bool {
        if !reply.valid() || reply.length as usize > MAX_TEXT {
            return false;
        }
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if !effect_identity(&page, service_generation, endpoint_generation)
            || page.request_id != id
            || SessionPageState::from_wire(page.state) != Some(SessionPageState::Waiting)
        {
            return false;
        }
        page.result = reply.result as u32;
        page.reply_length = u32::from(reply.length);
        page.reply = [0; MAX_TEXT];
        page.reply[..usize::from(reply.length)]
            .copy_from_slice(&reply.text[..usize::from(reply.length)]);
        page.state = if reply.result == logos_abi::EffectResult::Denied {
            SessionPageState::Denied.wire()
        } else {
            SessionPageState::Reply.wire()
        };
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    /// # Safety
    /// The Sessions service owns result consumption.
    pub unsafe fn finish_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        id: u32,
    ) -> Option<EffectResponse> {
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        let state = SessionPageState::from_wire(page.state)?;
        if !effect_identity(&page, service_generation, endpoint_generation)
            || page.request_id != id
            || !matches!(
                state,
                SessionPageState::Reply
                    | SessionPageState::Denied
                    | SessionPageState::Failed
                    | SessionPageState::Cancelled
            )
        {
            return None;
        }
        let result = logos_abi::EffectResult::from_wire(page.result)?;
        let length = usize::try_from(page.reply_length).ok()?;
        if length > MAX_TEXT || page.reply[length..].iter().any(|byte| *byte != 0) {
            return None;
        }
        let reply = logos_abi::EffectReply::new(result, &page.reply[..length]);
        page = Self::new(service_generation, endpoint_generation);
        unsafe { (address as *mut Self).write_volatile(page) };
        Some(EffectResponse { id, reply })
    }

    /// # Safety
    /// Core may recover the current ID only while an effect waits for completion.
    pub unsafe fn waiting_id_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> Option<u32> {
        let page = unsafe { (address as *const Self).read_volatile() };
        (effect_identity(&page, service_generation, endpoint_generation)
            && page.request_id != 0
            && SessionPageState::from_wire(page.state) == Some(SessionPageState::Waiting))
        .then_some(page.request_id)
    }
}

fn valid_page_identity<T>(address: u64, service_generation: u32, endpoint_generation: u32) -> bool {
    address != 0
        && service_generation != 0
        && endpoint_generation != 0
        && address.is_multiple_of(align_of::<T>() as u64)
}

fn client_identity(
    page: &SessionClientPage,
    service_generation: u32,
    endpoint_generation: u32,
) -> bool {
    page.service_generation == service_generation && page.endpoint_generation == endpoint_generation
}

fn server_identity(
    page: &SessionServerPage,
    service_generation: u32,
    endpoint_generation: u32,
) -> bool {
    page.service_generation == service_generation && page.endpoint_generation == endpoint_generation
}

fn effect_identity(page: &EffectPage, service_generation: u32, endpoint_generation: u32) -> bool {
    page.service_generation == service_generation && page.endpoint_generation == endpoint_generation
}

fn decode_session_request(
    operation: u32,
    length: u32,
    argument: [u8; MAX_TEXT],
) -> Option<logos_abi::SessionRequest> {
    let length = usize::try_from(length).ok()?;
    if length > MAX_TEXT || argument[length..].iter().any(|byte| *byte != 0) {
        return None;
    }
    let request =
        logos_abi::SessionRequest::new(logos_abi::Syscall::from_wire(operation)?, argument, length);
    request.valid().then_some(request)
}

fn decode_session_reply(length: u32, text: [u8; MAX_TEXT]) -> Option<logos_abi::SessionReply> {
    let length = usize::try_from(length).ok()?;
    if length > MAX_TEXT || text[length..].iter().any(|byte| *byte != 0) {
        return None;
    }
    Some(logos_abi::SessionReply { text, length })
}

fn decode_effect_request(
    effect: u32,
    length: u32,
    argument: [u8; MAX_TEXT],
) -> Option<logos_abi::EffectRequest> {
    let length = usize::try_from(length).ok()?;
    if length > MAX_TEXT || argument[length..].iter().any(|byte| *byte != 0) {
        return None;
    }
    let request =
        logos_abi::EffectRequest::new(logos_abi::Effect::from_wire(effect)?, argument, length);
    request.valid().then_some(request)
}

const _: () = assert!(size_of::<ControlPage>() <= logos_abi::PAGE_SIZE);
const _: () = assert!(size_of::<InputPage>() == logos_abi::PAGE_SIZE);
const _: () = assert!(size_of::<DisplayPage>() == logos_abi::PAGE_SIZE);
const _: () = assert!(size_of::<SessionClientPage>() <= logos_abi::PAGE_SIZE);
const _: () = assert!(size_of::<SessionServerPage>() <= logos_abi::PAGE_SIZE);
const _: () = assert!(size_of::<EffectPage>() <= logos_abi::PAGE_SIZE);
const _: () = assert!(size_of::<StoreEndpointPage>() == logos_abi::PAGE_SIZE);
const _: () = assert!(size_of::<BlockEndpointPage>() == logos_abi::PAGE_SIZE);
const _: () = assert!(size_of::<NetworkPage>() == logos_abi::PAGE_SIZE);
const _: () = assert!(size_of::<RemotePage>() == logos_abi::PAGE_SIZE);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum RemoteGateOperation {
    Handshake = 1,
    Open,
    Invoke,
    Seal,
    Subscribe,
    Credit,
    Acknowledge,
    Reset,
}

impl RemoteGateOperation {
    fn from_wire(value: u32) -> Option<Self> {
        Some(match value {
            1 => Self::Handshake,
            2 => Self::Open,
            3 => Self::Invoke,
            4 => Self::Seal,
            5 => Self::Subscribe,
            6 => Self::Credit,
            7 => Self::Acknowledge,
            8 => Self::Reset,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum RemoteGateStatus {
    Complete = 1,
    Busy,
    Denied,
    Invalid,
    Unavailable,
    Indeterminate,
}

impl RemoteGateStatus {
    fn from_wire(value: u32) -> Option<Self> {
        Some(match value {
            1 => Self::Complete,
            2 => Self::Busy,
            3 => Self::Denied,
            4 => Self::Invalid,
            5 => Self::Unavailable,
            6 => Self::Indeterminate,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct RemoteGateRequest {
    pub id: u32,
    pub operation: RemoteGateOperation,
    pub page: logos_abi::PageHandle,
    pub length: u16,
    pub deadline: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct RemoteGateReply {
    pub id: u32,
    pub status: RemoteGateStatus,
    pub length: u16,
    pub cursor: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BlockPage {
    pub handle: logos_abi::PageHandle,
    pub address: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NetworkPages {
    pub rx_handle: logos_abi::PageHandle,
    pub rx_address: u64,
    pub tx_handle: logos_abi::PageHandle,
    pub tx_address: u64,
}

const BLOCK_REQUEST_BYTES: usize = 32;
const STORE_REQUEST_BYTES: usize = 102;
const BLOCK_REPLY_BYTES: usize = 21;
const STORE_REPLY_BYTES: usize = 17;
const NETWORK_REQUEST_BYTES: usize = 34;
const NETWORK_REPLY_BYTES: usize = 148;
const NETWORK_DEVICE_REQUEST_BYTES: usize = 18;
const NETWORK_DEVICE_REPLY_BYTES: usize = 34;
const NETWORK_EVENT_BYTES: usize = 18;
const REMOTE_GATE_REQUEST_BYTES: usize = 24;
const REMOTE_GATE_REPLY_BYTES: usize = 20;

fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(offset..offset + 4)?.try_into().ok()?))
}

fn read_u64(bytes: &[u8], offset: usize) -> Option<u64> {
    Some(u64::from_le_bytes(bytes.get(offset..offset + 8)?.try_into().ok()?))
}

fn write_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}
fn write_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}
fn write_u64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

fn read_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(offset..offset + 2)?.try_into().ok()?))
}

fn decode_remote_gate_request(bytes: &[u8]) -> Option<RemoteGateRequest> {
    (bytes.len() >= REMOTE_GATE_REQUEST_BYTES && bytes[14..16] == [0; 2]).then_some(())?;
    Some(RemoteGateRequest {
        id: read_u32(bytes, 0)?,
        operation: RemoteGateOperation::from_wire(read_u32(bytes, 4)?)?,
        page: logos_abi::PageHandle(read_u32(bytes, 8)?),
        length: read_u16(bytes, 12)?,
        deadline: read_u64(bytes, 16)?,
    })
}

fn encode_remote_gate_request(bytes: &mut [u8; MAX_TEXT], request: RemoteGateRequest) {
    *bytes = [0; MAX_TEXT];
    write_u32(bytes, 0, request.id);
    write_u32(bytes, 4, request.operation as u32);
    write_u32(bytes, 8, request.page.0);
    write_u16(bytes, 12, request.length);
    write_u64(bytes, 16, request.deadline);
}

fn decode_remote_gate_reply(bytes: &[u8]) -> Option<RemoteGateReply> {
    (bytes.len() >= REMOTE_GATE_REPLY_BYTES && bytes[10..12] == [0; 2]).then_some(())?;
    Some(RemoteGateReply {
        id: read_u32(bytes, 0)?,
        status: RemoteGateStatus::from_wire(read_u32(bytes, 4)?)?,
        length: read_u16(bytes, 8)?,
        cursor: read_u64(bytes, 12)?,
    })
}

fn encode_remote_gate_reply(bytes: &mut [u8; MAX_TEXT], reply: RemoteGateReply) {
    *bytes = [0; MAX_TEXT];
    write_u32(bytes, 0, reply.id);
    write_u32(bytes, 4, reply.status as u32);
    write_u16(bytes, 8, reply.length);
    write_u64(bytes, 12, reply.cursor);
}

fn decode_block_request(bytes: &[u8]) -> Option<logos_abi::BlockRequest> {
    Some(logos_abi::BlockRequest {
        id: read_u32(bytes, 0)?,
        operation: logos_abi::BlockOperation::from_wire(*bytes.get(4)?)?,
        lba: read_u64(bytes, 8)?,
        blocks: read_u32(bytes, 16)?,
        page: logos_abi::PageHandle(read_u32(bytes, 20)?),
        deadline: read_u64(bytes, 24)?,
    })
}

fn encode_block_request(bytes: &mut [u8; MAX_TEXT], request: logos_abi::BlockRequest) {
    *bytes = [0; MAX_TEXT];
    write_u32(bytes, 0, request.id);
    bytes[4] = request.operation as u8;
    write_u64(bytes, 8, request.lba);
    write_u32(bytes, 16, request.blocks);
    write_u32(bytes, 20, request.page.0);
    write_u64(bytes, 24, request.deadline);
}

fn decode_store_request(bytes: &[u8]) -> Option<logos_abi::StoreRequest> {
    let mut name = [0; logos_abi::MAX_OBJECT_NAME];
    name.copy_from_slice(bytes.get(14..14 + logos_abi::MAX_OBJECT_NAME)?);
    Some(logos_abi::StoreRequest {
        id: read_u32(bytes, 0)?,
        operation: logos_abi::StoreOperation::from_wire(*bytes.get(4)?)?,
        namespace: logos_abi::NamespaceId(read_u32(bytes, 8)?),
        name,
        name_length: *bytes.get(12)?,
        version: logos_abi::VersionSelector::from_wire(*bytes.get(13)?)?,
        offset: read_u64(bytes, 78)?,
        length: read_u32(bytes, 86)?,
        page: logos_abi::PageHandle(read_u32(bytes, 90)?),
        deadline: read_u64(bytes, 94)?,
    })
}

fn encode_store_request(bytes: &mut [u8; MAX_TEXT], request: logos_abi::StoreRequest) {
    *bytes = [0; MAX_TEXT];
    write_u32(bytes, 0, request.id);
    bytes[4] = request.operation as u8;
    write_u32(bytes, 8, request.namespace.0);
    bytes[12] = request.name_length;
    bytes[13] = request.version as u8;
    bytes[14..14 + logos_abi::MAX_OBJECT_NAME].copy_from_slice(&request.name);
    write_u64(bytes, 78, request.offset);
    write_u32(bytes, 86, request.length);
    write_u32(bytes, 90, request.page.0);
    write_u64(bytes, 94, request.deadline);
}

fn decode_block_reply(bytes: &[u8]) -> Option<logos_abi::BlockReply> {
    Some(logos_abi::BlockReply {
        id: read_u32(bytes, 0)?,
        status: logos_abi::PersistenceStatus::from_wire(*bytes.get(4)?)?,
        info: logos_abi::BlockInfo {
            logical_block_size: read_u32(bytes, 5)?,
            blocks: read_u64(bytes, 9)?,
            max_transfer_blocks: read_u32(bytes, 17)?,
        },
    })
}

fn encode_block_reply(bytes: &mut [u8; MAX_TEXT], reply: logos_abi::BlockReply) {
    *bytes = [0; MAX_TEXT];
    write_u32(bytes, 0, reply.id);
    bytes[4] = reply.status as u8;
    write_u32(bytes, 5, reply.info.logical_block_size);
    write_u64(bytes, 9, reply.info.blocks);
    write_u32(bytes, 17, reply.info.max_transfer_blocks);
}

fn decode_store_reply(bytes: &[u8]) -> Option<logos_abi::StoreReply> {
    Some(logos_abi::StoreReply {
        id: read_u32(bytes, 0)?,
        status: logos_abi::PersistenceStatus::from_wire(*bytes.get(4)?)?,
        version: read_u64(bytes, 5)?,
        length: read_u32(bytes, 13)?,
    })
}

fn encode_store_reply(bytes: &mut [u8; MAX_TEXT], reply: logos_abi::StoreReply) {
    *bytes = [0; MAX_TEXT];
    write_u32(bytes, 0, reply.id);
    bytes[4] = reply.status as u8;
    write_u64(bytes, 5, reply.version);
    write_u32(bytes, 13, reply.length);
}

fn decode_network_request(bytes: &[u8]) -> Option<logos_abi::NetworkRequest> {
    if *bytes.get(5)? != 0 {
        return None;
    }
    Some(logos_abi::NetworkRequest {
        id: read_u32(bytes, 0)?,
        operation: logos_abi::NetworkOperation::from_wire(*bytes.get(4)?)?,
        endpoint: logos_abi::NetworkEndpoint(read_u32(bytes, 6)?),
        peer: logos_abi::NetworkScope(read_u64(bytes, 10)?),
        page: logos_abi::PageHandle(read_u32(bytes, 18)?),
        length: read_u16(bytes, 22)?,
        generation: read_u16(bytes, 24)?,
        deadline: read_u64(bytes, 26)?,
    })
}

fn encode_network_request(bytes: &mut [u8; MAX_TEXT], request: logos_abi::NetworkRequest) {
    *bytes = [0; MAX_TEXT];
    write_u32(bytes, 0, request.id);
    bytes[4] = request.operation as u8;
    write_u32(bytes, 6, request.endpoint.0);
    write_u64(bytes, 10, request.peer.0);
    write_u32(bytes, 18, request.page.0);
    write_u16(bytes, 22, request.length);
    write_u16(bytes, 24, request.generation);
    write_u64(bytes, 26, request.deadline);
}

fn decode_network_reply(bytes: &[u8]) -> Option<logos_abi::NetworkReply> {
    if *bytes.get(5)? != 0 {
        return None;
    }
    Some(logos_abi::NetworkReply {
        id: read_u32(bytes, 0)?,
        status: logos_abi::NetworkStatus::from_wire(*bytes.get(4)?)?,
        endpoint: logos_abi::NetworkEndpoint(read_u32(bytes, 6)?),
        generation: read_u16(bytes, 10)?,
        source_address: read_u32(bytes, 12)?,
        source_port: read_u16(bytes, 16)?,
        length: read_u16(bytes, 18)?,
        info: decode_network_info(bytes, 20)?,
        counters: decode_network_counters(bytes, 44)?,
    })
}

fn encode_network_reply(bytes: &mut [u8; MAX_TEXT], reply: logos_abi::NetworkReply) {
    *bytes = [0; MAX_TEXT];
    write_u32(bytes, 0, reply.id);
    bytes[4] = reply.status as u8;
    write_u32(bytes, 6, reply.endpoint.0);
    write_u16(bytes, 10, reply.generation);
    write_u32(bytes, 12, reply.source_address);
    write_u16(bytes, 16, reply.source_port);
    write_u16(bytes, 18, reply.length);
    encode_network_info(bytes, 20, reply.info);
    encode_network_counters(bytes, 44, reply.counters);
}

fn decode_network_device_request(bytes: &[u8]) -> Option<logos_abi::NetworkDeviceRequest> {
    if *bytes.get(5)? != 0 {
        return None;
    }
    Some(logos_abi::NetworkDeviceRequest {
        id: read_u32(bytes, 0)?,
        operation: logos_abi::NetworkDeviceOperation::from_wire(*bytes.get(4)?)?,
        length: read_u16(bytes, 6)?,
        generation: read_u16(bytes, 8)?,
        deadline: read_u64(bytes, 10)?,
    })
}

fn encode_network_device_request(
    bytes: &mut [u8; MAX_TEXT],
    request: logos_abi::NetworkDeviceRequest,
) {
    *bytes = [0; MAX_TEXT];
    write_u32(bytes, 0, request.id);
    bytes[4] = request.operation as u8;
    write_u16(bytes, 6, request.length);
    write_u16(bytes, 8, request.generation);
    write_u64(bytes, 10, request.deadline);
}

fn decode_network_device_reply(bytes: &[u8]) -> Option<logos_abi::NetworkDeviceReply> {
    if *bytes.get(5)? != 0 {
        return None;
    }
    Some(logos_abi::NetworkDeviceReply {
        id: read_u32(bytes, 0)?,
        status: logos_abi::NetworkStatus::from_wire(*bytes.get(4)?)?,
        generation: read_u16(bytes, 6)?,
        info: decode_network_info(bytes, 8)?,
    })
}

fn encode_network_device_reply(bytes: &mut [u8; MAX_TEXT], reply: logos_abi::NetworkDeviceReply) {
    *bytes = [0; MAX_TEXT];
    write_u32(bytes, 0, reply.id);
    bytes[4] = reply.status as u8;
    write_u16(bytes, 6, reply.generation);
    encode_network_info(bytes, 8, reply.info);
}

fn decode_network_info(bytes: &[u8], offset: usize) -> Option<logos_abi::NetworkInfo> {
    let mut mac = [0; 6];
    mac.copy_from_slice(bytes.get(offset..offset + 6)?);
    Some(logos_abi::NetworkInfo {
        mac,
        mtu: read_u16(bytes, offset + 6)?,
        generation: read_u16(bytes, offset + 8)?,
        link_up: *bytes.get(offset + 10)?,
        configuration: *bytes.get(offset + 11)?,
        ipv4: read_u32(bytes, offset + 12)?,
        subnet_mask: read_u32(bytes, offset + 16)?,
        router: read_u32(bytes, offset + 20)?,
    })
}

fn encode_network_info(bytes: &mut [u8; MAX_TEXT], offset: usize, info: logos_abi::NetworkInfo) {
    bytes[offset..offset + 6].copy_from_slice(&info.mac);
    write_u16(bytes, offset + 6, info.mtu);
    write_u16(bytes, offset + 8, info.generation);
    bytes[offset + 10] = info.link_up;
    bytes[offset + 11] = info.configuration;
    write_u32(bytes, offset + 12, info.ipv4);
    write_u32(bytes, offset + 16, info.subnet_mask);
    write_u32(bytes, offset + 20, info.router);
}

fn decode_network_counters(bytes: &[u8], offset: usize) -> Option<logos_abi::NetworkCounters> {
    Some(logos_abi::NetworkCounters {
        rx_frames: read_u64(bytes, offset)?,
        tx_frames: read_u64(bytes, offset + 8)?,
        rx_bytes: read_u64(bytes, offset + 16)?,
        tx_bytes: read_u64(bytes, offset + 24)?,
        malformed: read_u64(bytes, offset + 32)?,
        unsupported: read_u64(bytes, offset + 40)?,
        rx_dropped: read_u64(bytes, offset + 48)?,
        udp_no_endpoint: read_u64(bytes, offset + 56)?,
        udp_queue_dropped: read_u64(bytes, offset + 64)?,
        timeouts: read_u64(bytes, offset + 72)?,
        cancellations: read_u64(bytes, offset + 80)?,
        resets: read_u64(bytes, offset + 88)?,
        denied: read_u64(bytes, offset + 96)?,
    })
}

fn encode_network_counters(
    bytes: &mut [u8; MAX_TEXT],
    offset: usize,
    counters: logos_abi::NetworkCounters,
) {
    for (index, value) in [
        counters.rx_frames,
        counters.tx_frames,
        counters.rx_bytes,
        counters.tx_bytes,
        counters.malformed,
        counters.unsupported,
        counters.rx_dropped,
        counters.udp_no_endpoint,
        counters.udp_queue_dropped,
        counters.timeouts,
        counters.cancellations,
        counters.resets,
        counters.denied,
    ]
    .into_iter()
    .enumerate()
    {
        write_u64(bytes, offset + index * 8, value);
    }
}

fn decode_network_event(bytes: &[u8]) -> Option<logos_abi::NetworkEvent> {
    if *bytes.get(5)? != 0 {
        return None;
    }
    Some(logos_abi::NetworkEvent {
        id: read_u32(bytes, 0)?,
        kind: logos_abi::NetworkEventKind::from_wire(*bytes.get(4)?)?,
        generation: read_u16(bytes, 6)?,
        length: read_u16(bytes, 8)?,
        now: read_u64(bytes, 10)?,
    })
}

fn encode_network_event(bytes: &mut [u8; MAX_TEXT], event: logos_abi::NetworkEvent) {
    *bytes = [0; MAX_TEXT];
    write_u32(bytes, 0, event.id);
    bytes[4] = event.kind as u8;
    write_u16(bytes, 6, event.generation);
    write_u16(bytes, 8, event.length);
    write_u64(bytes, 10, event.now);
}

impl ControlPage {
    pub const fn new() -> Self {
        Self::with_generation(1)
    }

    pub const fn with_generation(generation: u32) -> Self {
        Self {
            abi: ABI,
            reserved: 0,
            operation: 0,
            status: 0,
            generation,
            lifecycle: LIFECYCLE_STARTING,
            input_page: 0,
            display_page: 0,
            session_client_page: 0,
            session_server_page: 0,
            effect_page: 0,
            slot0: 0,
            slot1: 0,
            slot2: 0,
            payload_length: 0,
            payload: [0; MAX_TEXT],
            shared_page: 0,
            network_rx_page: 0,
            network_tx_page: 0,
        }
    }

    /// # Safety
    /// `address` must point to a live, aligned `ControlPage` mapping.
    pub unsafe fn panicked_at(address: u64) -> bool {
        let Some(context) = (unsafe { (address as *const Self).as_ref() }) else { return false };
        context.abi == ABI && context.reserved == 0 && context.operation == PANIC
    }

    /// # Safety
    /// `address` must point to a live, aligned `ControlPage` mapping.
    pub unsafe fn reset_at(address: u64) -> bool {
        if address == 0 || !address.is_multiple_of(align_of::<Self>() as u64) {
            return false;
        }
        let current = unsafe { (address as *const Self).read_volatile() };
        if current.abi != ABI || current.reserved != 0 || current.generation == 0 {
            return false;
        }
        unsafe { (address as *mut Self).write_volatile(Self::with_generation(current.generation)) };
        let mut reset = unsafe { (address as *mut Self).read_volatile() };
        reset.input_page = current.input_page;
        reset.display_page = current.display_page;
        reset.session_client_page = current.session_client_page;
        reset.session_server_page = current.session_server_page;
        reset.effect_page = current.effect_page;
        reset.shared_page = current.shared_page;
        reset.network_rx_page = current.network_rx_page;
        reset.network_tx_page = current.network_tx_page;
        unsafe { (address as *mut Self).write_volatile(reset) };
        true
    }

    /// # Safety
    /// `address` must point to a live, aligned `ControlPage` mapping owned by Core.
    pub unsafe fn configure_endpoint_pages_at(
        address: u64,
        generation: u32,
        input_page: Option<u64>,
        display_page: Option<u64>,
        session_client_page: Option<u64>,
        session_server_page: Option<u64>,
        effect_page: Option<u64>,
    ) -> bool {
        if generation == 0 {
            return false;
        }
        let mut context = unsafe { (address as *mut Self).read_volatile() };
        if context.abi != ABI || context.reserved != 0 || context.operation != 0 {
            return false;
        }
        context.generation = generation;
        context.lifecycle = LIFECYCLE_STARTING;
        context.input_page = input_page.unwrap_or(0);
        context.display_page = display_page.unwrap_or(0);
        context.session_client_page = session_client_page.unwrap_or(0);
        context.session_server_page = session_server_page.unwrap_or(0);
        context.effect_page = effect_page.unwrap_or(0);
        unsafe { (address as *mut Self).write_volatile(context) };
        true
    }

    /// # Safety
    /// `address` must point to a live, aligned `ControlPage` mapping owned by Core.
    pub unsafe fn set_generation_at(address: u64, generation: u32) -> bool {
        if generation == 0 {
            return false;
        }
        let mut context = unsafe { (address as *mut Self).read_volatile() };
        if context.abi != ABI || context.reserved != 0 {
            return false;
        }
        context.generation = generation;
        context.lifecycle = LIFECYCLE_STARTING;
        context.operation = 0;
        context.status = 0;
        // slot0..2 may hold a configured block page; keep endpoint configuration across reset.
        context.payload_length = 0;
        context.payload = [0; MAX_TEXT];
        unsafe { (address as *mut Self).write_volatile(context) };
        true
    }

    /// # Safety
    /// `address` must point to a live, aligned `ControlPage` mapping.
    pub unsafe fn generation_at(address: u64) -> Option<u32> {
        let context = unsafe { (address as *const Self).read_volatile() };
        (context.abi == ABI && context.reserved == 0 && context.generation != 0)
            .then_some(context.generation)
    }

    /// # Safety
    /// `address` must point to a live, aligned `ControlPage` mapping.
    pub unsafe fn input_page_at(address: u64) -> Option<u64> {
        let context = unsafe { (address as *const Self).read_volatile() };
        (context.abi == ABI && context.reserved == 0 && context.input_page != 0)
            .then_some(context.input_page)
    }

    /// # Safety
    /// `address` must point to a live, aligned `ControlPage` mapping.
    pub unsafe fn display_page_at(address: u64) -> Option<u64> {
        let context = unsafe { (address as *const Self).read_volatile() };
        (context.abi == ABI && context.reserved == 0 && context.display_page != 0)
            .then_some(context.display_page)
    }

    /// # Safety
    /// `address` must point to a live, aligned `ControlPage` mapping.
    pub unsafe fn session_client_page_at(address: u64) -> Option<u64> {
        let context = unsafe { (address as *const Self).read_volatile() };
        (context.abi == ABI && context.reserved == 0 && context.session_client_page != 0)
            .then_some(context.session_client_page)
    }

    /// # Safety
    /// `address` must point to a live, aligned `ControlPage` mapping.
    pub unsafe fn session_server_page_at(address: u64) -> Option<u64> {
        let context = unsafe { (address as *const Self).read_volatile() };
        (context.abi == ABI && context.reserved == 0 && context.session_server_page != 0)
            .then_some(context.session_server_page)
    }

    /// # Safety
    /// `address` must point to a live, aligned `ControlPage` mapping.
    pub unsafe fn effect_page_at(address: u64) -> Option<u64> {
        let context = unsafe { (address as *const Self).read_volatile() };
        (context.abi == ABI && context.reserved == 0 && context.effect_page != 0)
            .then_some(context.effect_page)
    }

    /// # Safety
    ///
    /// `address` must point to a live, aligned `ControlPage` mapping.
    pub unsafe fn ready_at(address: u64) -> bool {
        let context = unsafe { (address as *const Self).read_volatile() };
        context.abi == ABI && context.reserved == 0 && context.operation == READY
    }

    /// # Safety
    ///
    /// `address` must point to a live, aligned `ControlPage` mapping.
    pub unsafe fn acknowledge_at(address: u64) -> bool {
        let context = unsafe { (address as *mut Self).read_volatile() };
        if context.abi != ABI
            || context.reserved != 0
            || context.operation != READY
            || context.status != 0
        {
            return false;
        }
        let mut acknowledged = context;
        acknowledged.status = ACKNOWLEDGED;
        acknowledged.lifecycle = LIFECYCLE_READY;
        unsafe { (address as *mut Self).write_volatile(acknowledged) };
        true
    }

    /// # Safety
    ///
    /// `address` must point to a live, aligned `ControlPage` mapping.
    pub unsafe fn complete_at(address: u64) -> bool {
        let context = unsafe { (address as *const Self).read_volatile() };
        context.abi == ABI
            && context.reserved == 0
            && context.operation == COMPLETE
            && context.status == ACKNOWLEDGED
    }

    /// # Safety
    ///
    /// `address` must point to a live, aligned `ControlPage` mapping.
    pub unsafe fn input_waiting_at(address: u64) -> bool {
        let context = unsafe { (address as *const Self).read_volatile() };
        if context.abi != ABI
            || context.reserved != 0
            || context.operation != READ_INPUT
            || context.status != ACKNOWLEDGED
        {
            return false;
        }
        context.input_page == 0
            || unsafe { InputPage::waiting_at(context.input_page, context.generation) }
    }

    /// # Safety
    /// `address` must point to a live, aligned `ControlPage` mapping owned by Core.
    pub unsafe fn request_remote_gate_at(address: u64, request: RemoteGateRequest) -> bool {
        if request.length as usize > MAX_TEXT {
            return false;
        }
        let mut context = unsafe { (address as *mut Self).read_volatile() };
        if context.abi != ABI
            || context.reserved != 0
            || context.status != ACKNOWLEDGED
            || !matches!(context.operation, READY | READ_INPUT | REMOTE_GATE)
        {
            return false;
        }
        encode_remote_gate_request(&mut context.payload, request);
        context.payload_length = REMOTE_GATE_REQUEST_BYTES as u32;
        context.operation = REMOTE_GATE;
        unsafe { (address as *mut Self).write_volatile(context) };
        true
    }

    /// # Safety
    /// `address` must point to a live, aligned `ControlPage` mapping owned by Core.
    pub unsafe fn deliver_remote_gate_at(address: u64, request: RemoteGateRequest) -> bool {
        if !unsafe { Self::request_remote_gate_at(address, request) } {
            return false;
        }
        true
    }

    /// # Safety
    /// `address` must point to a live, aligned `ControlPage` mapping.
    pub unsafe fn remote_gate_at(address: u64) -> Option<RemoteGateRequest> {
        let context = unsafe { (address as *const Self).read_volatile() };
        if context.abi != ABI
            || context.reserved != 0
            || context.operation != REMOTE_GATE
            || context.status != ACKNOWLEDGED
            || context.payload_length != REMOTE_GATE_REQUEST_BYTES as u32
        {
            return None;
        }
        let request = decode_remote_gate_request(&context.payload)?;
        (request.length as usize <= MAX_TEXT).then_some(request)
    }

    /// # Safety
    /// `address` must point to a live, aligned ControlPage mapping owned by the service.
    pub unsafe fn reply_remote_gate_at(address: u64, reply: RemoteGateReply) -> bool {
        let mut context = unsafe { (address as *mut Self).read_volatile() };
        let valid = unsafe { Self::remote_gate_at(address) }
            .is_some_and(|request| request.id == reply.id && reply.length as usize <= MAX_TEXT);
        if !valid {
            return false;
        }
        encode_remote_gate_reply(&mut context.payload, reply);
        context.payload_length = REMOTE_GATE_REPLY_BYTES as u32;
        context.operation = REMOTE_GATE;
        unsafe { (address as *mut Self).write_volatile(context) };
        true
    }

    /// # Safety
    /// `address` must point to a live, aligned `ControlPage` mapping.
    pub unsafe fn remote_gate_reply_at(address: u64, expected_id: u32) -> Option<RemoteGateReply> {
        let context = unsafe { (address as *const Self).read_volatile() };
        if context.abi != ABI
            || context.reserved != 0
            || context.operation != REMOTE_GATE
            || context.status != ACKNOWLEDGED
            || context.payload_length != REMOTE_GATE_REPLY_BYTES as u32
        {
            return None;
        }
        let reply = decode_remote_gate_reply(&context.payload)?;
        (reply.id == expected_id).then_some(reply)
    }

    /// # Safety
    /// `address` must point to a live, aligned `ControlPage` mapping.
    pub unsafe fn storage_status_at(address: u64) -> Option<u32> {
        let context = unsafe { (address as *const Self).read_volatile() };
        (context.abi == ABI
            && context.reserved == 0
            && (STORAGE_FORMATTED..=STORAGE_IO_FAILED).contains(&context.slot0))
        .then_some(context.slot0)
    }

    /// # Safety
    /// `address` must point to a live, aligned `ControlPage` mapping.
    pub unsafe fn session_client_pending_at(address: u64) -> bool {
        let context = unsafe { (address as *const Self).read_volatile() };
        context.abi == ABI
            && context.reserved == 0
            && context.operation == SYSCALL
            && context.status == ACKNOWLEDGED
            && context.session_client_page != 0
            && unsafe {
                SessionClientPage::pending_at(
                    context.session_client_page,
                    context.generation,
                    context.generation,
                )
            }
    }

    /// # Safety
    /// `address` must point to a live, aligned `ControlPage` mapping.
    pub unsafe fn session_server_waiting_at(address: u64) -> bool {
        let context = unsafe { (address as *const Self).read_volatile() };
        context.abi == ABI
            && context.reserved == 0
            && context.operation == READ_INPUT
            && context.status == ACKNOWLEDGED
            && context.session_server_page != 0
            && unsafe {
                SessionServerPage::waiting_at(
                    context.session_server_page,
                    context.generation,
                    context.generation,
                )
            }
    }

    /// # Safety
    /// `address` must point to a live, aligned `ControlPage` mapping.
    pub unsafe fn session_server_reply_pending_at(address: u64) -> bool {
        let context = unsafe { (address as *const Self).read_volatile() };
        context.abi == ABI
            && context.reserved == 0
            && context.operation == SESSION_REPLY
            && context.status == ACKNOWLEDGED
            && context.session_server_page != 0
            && unsafe {
                SessionServerPage::reply_pending_at(
                    context.session_server_page,
                    context.generation,
                    context.generation,
                )
            }
    }

    /// # Safety
    /// `address` must point to a live, aligned `ControlPage` mapping.
    pub unsafe fn effect_pending_at(address: u64) -> bool {
        let context = unsafe { (address as *const Self).read_volatile() };
        context.abi == ABI
            && context.reserved == 0
            && context.operation == SESSION_EFFECT
            && context.status == ACKNOWLEDGED
            && context.effect_page != 0
            && unsafe {
                EffectPage::pending_at(context.effect_page, context.generation, context.generation)
            }
    }

    /// # Safety
    /// `address` must point to a live, aligned `ControlPage` mapping.
    pub unsafe fn store_at(address: u64) -> Option<logos_abi::StoreRequest> {
        let context = unsafe { (address as *const Self).read_volatile() };
        if context.abi != ABI
            || context.reserved != 0
            || context.operation != STORE_REQUEST
            || context.status != ACKNOWLEDGED
            || context.payload_length != STORE_REQUEST_BYTES as u32
        {
            return None;
        }
        let request = decode_store_request(&context.payload)?;
        request.valid().then_some(request)
    }

    /// # Safety
    /// `address` must point to a live, aligned `ControlPage` mapping owned by the caller.
    pub unsafe fn request_store_at(address: u64, request: logos_abi::StoreRequest) -> bool {
        if !request.valid() {
            return false;
        }
        let mut context = unsafe { (address as *mut Self).read_volatile() };
        if context.abi != ABI
            || context.reserved != 0
            || context.status != ACKNOWLEDGED
            || !matches!(context.operation, READY | READ_INPUT | STORE_REPLY)
        {
            return false;
        }
        encode_store_request(&mut context.payload, request);
        context.payload_length = STORE_REQUEST_BYTES as u32;
        context.operation = STORE_REQUEST;
        unsafe { (address as *mut Self).write_volatile(context) };
        true
    }

    /// # Safety
    /// `address` must point to a live, aligned `ControlPage` mapping.
    pub unsafe fn deliver_store_at(address: u64, request: logos_abi::StoreRequest) -> bool {
        if !request.valid() {
            return false;
        }
        let mut context = unsafe { (address as *mut Self).read_volatile() };
        if context.abi != ABI
            || context.reserved != 0
            || context.operation != READ_INPUT
            || context.status != ACKNOWLEDGED
        {
            return false;
        }
        context.payload = [0; MAX_TEXT];
        encode_store_request(&mut context.payload, request);
        context.payload_length = STORE_REQUEST_BYTES as u32;
        context.operation = STORE_REQUEST;
        unsafe { (address as *mut Self).write_volatile(context) };
        true
    }

    /// # Safety
    /// `address` must point to a live, aligned `ControlPage` mapping.
    pub unsafe fn reply_store_at(address: u64, reply: logos_abi::StoreReply) -> bool {
        let mut context = unsafe { (address as *mut Self).read_volatile() };
        let valid = match context.operation {
            STORE_REQUEST => decode_store_request(&context.payload)
                .is_some_and(|request| reply.valid_for(request)),
            BLOCK_REPLY => {
                reply.length as usize <= logos_abi::PAGE_SIZE && context.slot2 == reply.id
            }
            _ => false,
        };
        if !valid {
            return false;
        }
        context.operation = STORE_REPLY;
        context.slot2 = 0;
        encode_store_reply(&mut context.payload, reply);
        context.payload_length = STORE_REPLY_BYTES as u32;
        unsafe { (address as *mut Self).write_volatile(context) };
        true
    }

    /// # Safety
    /// `address` must point to a live, aligned `ControlPage` mapping.
    pub unsafe fn store_reply_at(address: u64, expected_id: u32) -> Option<logos_abi::StoreReply> {
        let reply = unsafe { Self::store_reply_pending_at(address) }?;
        (reply.id == expected_id).then_some(reply)
    }

    /// # Safety
    /// `address` must point to a live, aligned `ControlPage` mapping.
    pub unsafe fn store_reply_pending_at(address: u64) -> Option<logos_abi::StoreReply> {
        let context = unsafe { (address as *const Self).read_volatile() };
        if context.abi != ABI
            || context.reserved != 0
            || context.operation != STORE_REPLY
            || context.status != ACKNOWLEDGED
            || context.payload_length != STORE_REPLY_BYTES as u32
        {
            return None;
        }
        decode_store_reply(&context.payload)
            .filter(|reply| reply.length as usize <= logos_abi::PAGE_SIZE)
    }

    /// # Safety
    /// `address` must point to a live, aligned `ControlPage` mapping.
    pub unsafe fn network_at(address: u64) -> Option<logos_abi::NetworkRequest> {
        let context = unsafe { (address as *const Self).read_volatile() };
        if context.abi != ABI
            || context.reserved != 0
            || context.operation != NETWORK_REQUEST
            || context.status != ACKNOWLEDGED
            || context.payload_length != NETWORK_REQUEST_BYTES as u32
        {
            return None;
        }
        let request = decode_network_request(&context.payload)?;
        request.valid_shape().then_some(request)
    }

    /// # Safety
    /// `address` must point to a live, aligned `ControlPage` mapping owned by the caller.
    pub unsafe fn request_network_at(address: u64, request: logos_abi::NetworkRequest) -> bool {
        if !request.valid_shape() {
            return false;
        }
        let mut context = unsafe { (address as *mut Self).read_volatile() };
        if context.abi != ABI
            || context.reserved != 0
            || context.status != ACKNOWLEDGED
            || !matches!(
                context.operation,
                READY
                    | READ_INPUT
                    | NETWORK_REPLY
                    | NETWORK_EVENT
                    | NETWORK_DEVICE_REPLY
                    | NETWORK_WAIT
                    | NETWORK_REQUEST
            )
        {
            return false;
        }
        encode_network_request(&mut context.payload, request);
        context.payload_length = NETWORK_REQUEST_BYTES as u32;
        context.operation = NETWORK_REQUEST;
        unsafe { (address as *mut Self).write_volatile(context) };
        true
    }

    /// # Safety
    /// `address` must point to a live, aligned `ControlPage` mapping owned by Core.
    pub unsafe fn deliver_network_at(address: u64, request: logos_abi::NetworkRequest) -> bool {
        if !request.valid_shape() {
            return false;
        }
        let mut context = unsafe { (address as *mut Self).read_volatile() };
        if context.abi != ABI
            || context.reserved != 0
            || !matches!(context.operation, READ_INPUT | NETWORK_WAIT)
            || context.status != ACKNOWLEDGED
        {
            return false;
        }
        encode_network_request(&mut context.payload, request);
        context.payload_length = NETWORK_REQUEST_BYTES as u32;
        context.operation = NETWORK_REQUEST;
        unsafe { (address as *mut Self).write_volatile(context) };
        true
    }

    /// # Safety
    /// `address` must point to a live Network service context owned by Core.
    pub unsafe fn deliver_network_for_owner_at(
        address: u64,
        request: logos_abi::NetworkRequest,
        owner: u64,
    ) -> bool {
        if !unsafe { Self::deliver_network_at(address, request) } {
            return false;
        }
        let mut context = unsafe { (address as *mut Self).read_volatile() };
        context.slot0 = owner as u32;
        context.slot1 = (owner >> 32) as u32;
        unsafe { (address as *mut Self).write_volatile(context) };
        true
    }

    /// # Safety
    /// `address` must point to a live Network service context.
    pub unsafe fn network_owner_at(address: u64) -> Option<u64> {
        let context = unsafe { (address as *const Self).read_volatile() };
        (context.abi == ABI && context.reserved == 0 && context.operation == NETWORK_REQUEST)
            .then_some(u64::from(context.slot0) | (u64::from(context.slot1) << 32))
    }

    /// # Safety
    /// `address` must point to a live, aligned `ControlPage` mapping owned by Core.
    pub unsafe fn reply_network_at(address: u64, reply: logos_abi::NetworkReply) -> bool {
        let mut context = unsafe { (address as *mut Self).read_volatile() };
        let valid = context.operation == NETWORK_REQUEST
            && decode_network_request(&context.payload)
                .is_some_and(|request| reply.valid_for(request));
        if !valid {
            return false;
        }
        encode_network_reply(&mut context.payload, reply);
        context.payload_length = NETWORK_REPLY_BYTES as u32;
        context.operation = NETWORK_REPLY;
        unsafe { (address as *mut Self).write_volatile(context) };
        true
    }

    /// # Safety
    /// `address` must point to a live, aligned `ControlPage` mapping owned by the service.
    pub unsafe fn reply_network_after_device_at(
        address: u64,
        request: logos_abi::NetworkRequest,
        reply: logos_abi::NetworkReply,
    ) -> bool {
        let mut context = unsafe { (address as *mut Self).read_volatile() };
        if context.operation != NETWORK_DEVICE_REPLY || !reply.valid_for(request) {
            return false;
        }
        encode_network_reply(&mut context.payload, reply);
        context.payload_length = NETWORK_REPLY_BYTES as u32;
        context.operation = NETWORK_REPLY;
        unsafe { (address as *mut Self).write_volatile(context) };
        true
    }

    /// # Safety
    /// `address` must point to a live, aligned `ControlPage` mapping owned by the service.
    pub unsafe fn reply_network_after_event_at(
        address: u64,
        request: logos_abi::NetworkRequest,
        reply: logos_abi::NetworkReply,
    ) -> bool {
        let mut context = unsafe { (address as *mut Self).read_volatile() };
        if context.operation != NETWORK_EVENT || !reply.valid_for(request) {
            return false;
        }
        encode_network_reply(&mut context.payload, reply);
        context.payload_length = NETWORK_REPLY_BYTES as u32;
        context.operation = NETWORK_REPLY;
        unsafe { (address as *mut Self).write_volatile(context) };
        true
    }

    /// # Safety
    /// `address` must point to a live, aligned `ControlPage` mapping.
    pub unsafe fn network_reply_at(
        address: u64,
        expected_id: u32,
    ) -> Option<logos_abi::NetworkReply> {
        let context = unsafe { (address as *const Self).read_volatile() };
        if context.abi != ABI
            || context.reserved != 0
            || context.operation != NETWORK_REPLY
            || context.status != ACKNOWLEDGED
            || context.payload_length != NETWORK_REPLY_BYTES as u32
        {
            return None;
        }
        decode_network_reply(&context.payload).filter(|reply| reply.id == expected_id)
    }

    /// # Safety
    /// `address` must point to a live, aligned `ControlPage` mapping.
    pub unsafe fn network_reply_pending_at(address: u64) -> bool {
        let context = unsafe { (address as *const Self).read_volatile() };
        context.abi == ABI
            && context.reserved == 0
            && context.operation == NETWORK_REPLY
            && context.status == ACKNOWLEDGED
            && context.payload_length == NETWORK_REPLY_BYTES as u32
    }

    /// # Safety
    /// `address` must point to a live, aligned `ControlPage` mapping owned by the caller.
    pub unsafe fn network_device_at(address: u64) -> Option<logos_abi::NetworkDeviceRequest> {
        let context = unsafe { (address as *const Self).read_volatile() };
        if context.abi != ABI
            || context.reserved != 0
            || context.operation != NETWORK_DEVICE_REQUEST
            || context.status != ACKNOWLEDGED
            || context.payload_length != NETWORK_DEVICE_REQUEST_BYTES as u32
        {
            return None;
        }
        let request = decode_network_device_request(&context.payload)?;
        request.valid_shape().then_some(request)
    }

    /// # Safety
    /// `address` must point to a live, aligned `ControlPage` mapping.
    pub unsafe fn network_device_pending_at(address: u64) -> bool {
        let context = unsafe { (address as *const Self).read_volatile() };
        context.abi == ABI
            && context.reserved == 0
            && context.operation == NETWORK_DEVICE_REQUEST
            && context.status == ACKNOWLEDGED
            && context.payload_length == NETWORK_DEVICE_REQUEST_BYTES as u32
    }

    /// # Safety
    /// `address` must point to a live, aligned `ControlPage` mapping owned by the caller.
    pub unsafe fn request_network_device_at(
        address: u64,
        request: logos_abi::NetworkDeviceRequest,
    ) -> bool {
        if !request.valid_shape() {
            return false;
        }
        let mut context = unsafe { (address as *mut Self).read_volatile() };
        if context.abi != ABI
            || context.reserved != 0
            || context.status != ACKNOWLEDGED
            || !matches!(
                context.operation,
                READY
                    | READ_INPUT
                    | NETWORK_REQUEST
                    | NETWORK_REPLY
                    | NETWORK_EVENT
                    | NETWORK_DEVICE_REPLY
            )
        {
            return false;
        }
        encode_network_device_request(&mut context.payload, request);
        context.payload_length = NETWORK_DEVICE_REQUEST_BYTES as u32;
        context.operation = NETWORK_DEVICE_REQUEST;
        unsafe { (address as *mut Self).write_volatile(context) };
        true
    }

    /// # Safety
    /// `address` must point to a live, aligned `ControlPage` mapping owned by Core.
    pub unsafe fn reply_network_device_at(
        address: u64,
        reply: logos_abi::NetworkDeviceReply,
    ) -> bool {
        let mut context = unsafe { (address as *mut Self).read_volatile() };
        let valid = context.operation == NETWORK_DEVICE_REQUEST
            && decode_network_device_request(&context.payload)
                .is_some_and(|request| reply.valid_for(request));
        if !valid {
            return false;
        }
        encode_network_device_reply(&mut context.payload, reply);
        context.payload_length = NETWORK_DEVICE_REPLY_BYTES as u32;
        context.operation = NETWORK_DEVICE_REPLY;
        unsafe { (address as *mut Self).write_volatile(context) };
        true
    }

    /// # Safety
    /// `address` must point to a live, aligned `ControlPage` mapping.
    pub unsafe fn network_device_reply_at(
        address: u64,
        expected_id: u32,
    ) -> Option<logos_abi::NetworkDeviceReply> {
        let context = unsafe { (address as *const Self).read_volatile() };
        if context.abi != ABI
            || context.reserved != 0
            || context.operation != NETWORK_DEVICE_REPLY
            || context.status != ACKNOWLEDGED
            || context.payload_length != NETWORK_DEVICE_REPLY_BYTES as u32
        {
            return None;
        }
        decode_network_device_reply(&context.payload).filter(|reply| reply.id == expected_id)
    }

    /// # Safety
    /// `address` must point to a live, aligned `ControlPage` mapping owned by Core.
    pub unsafe fn deliver_network_device_reply_at(
        address: u64,
        reply: logos_abi::NetworkDeviceReply,
    ) -> bool {
        let mut context = unsafe { (address as *mut Self).read_volatile() };
        if context.abi != ABI
            || context.reserved != 0
            || context.operation != NETWORK_WAIT
            || context.status != ACKNOWLEDGED
        {
            return false;
        }
        encode_network_device_reply(&mut context.payload, reply);
        context.payload_length = NETWORK_DEVICE_REPLY_BYTES as u32;
        context.operation = NETWORK_DEVICE_REPLY;
        unsafe { (address as *mut Self).write_volatile(context) };
        true
    }

    /// # Safety
    /// `address` must point to a live, aligned `ControlPage` mapping owned by the caller.
    pub unsafe fn network_wait_at(address: u64, deadline: u64) -> bool {
        if deadline == 0 {
            return false;
        }
        let mut context = unsafe { (address as *mut Self).read_volatile() };
        if context.abi != ABI
            || context.reserved != 0
            || context.status != ACKNOWLEDGED
            || !matches!(
                context.operation,
                READY
                    | READ_INPUT
                    | NETWORK_REQUEST
                    | NETWORK_REPLY
                    | NETWORK_EVENT
                    | NETWORK_DEVICE_REQUEST
                    | NETWORK_DEVICE_REPLY
            )
        {
            return false;
        }
        context.slot0 = deadline as u32;
        context.slot1 = (deadline >> 32) as u32;
        context.payload = [0; MAX_TEXT];
        context.payload_length = 0;
        context.operation = NETWORK_WAIT;
        unsafe { (address as *mut Self).write_volatile(context) };
        true
    }

    /// # Safety
    /// `address` must point to a live, aligned `ControlPage` mapping.
    pub unsafe fn network_waiting_at(address: u64) -> bool {
        let context = unsafe { (address as *const Self).read_volatile() };
        context.abi == ABI
            && context.reserved == 0
            && context.operation == NETWORK_WAIT
            && context.status == ACKNOWLEDGED
            && (u64::from(context.slot0) | (u64::from(context.slot1) << 32)) != 0
    }

    /// # Safety
    /// `address` must point to a live, aligned `ControlPage` mapping.
    pub unsafe fn network_deadline_at(address: u64) -> Option<u64> {
        let context = unsafe { (address as *const Self).read_volatile() };
        (context.abi == ABI
            && context.reserved == 0
            && context.operation == NETWORK_WAIT
            && context.status == ACKNOWLEDGED)
            .then_some(u64::from(context.slot0) | (u64::from(context.slot1) << 32))
            .filter(|deadline| *deadline != 0)
    }

    /// # Safety
    /// `address` must point to a live, aligned `ControlPage` mapping owned by Core.
    pub unsafe fn deliver_network_event_at(address: u64, event: logos_abi::NetworkEvent) -> bool {
        if !event.valid() {
            return false;
        }
        let mut context = unsafe { (address as *mut Self).read_volatile() };
        if context.abi != ABI
            || context.reserved != 0
            || context.operation != NETWORK_WAIT
            || context.status != ACKNOWLEDGED
        {
            return false;
        }
        encode_network_event(&mut context.payload, event);
        context.payload_length = NETWORK_EVENT_BYTES as u32;
        context.operation = NETWORK_EVENT;
        unsafe { (address as *mut Self).write_volatile(context) };
        true
    }

    /// # Safety
    /// `address` must point to a live, aligned `ControlPage` mapping.
    pub unsafe fn network_event_at(address: u64) -> Option<logos_abi::NetworkEvent> {
        let context = unsafe { (address as *const Self).read_volatile() };
        if context.abi != ABI
            || context.reserved != 0
            || context.operation != NETWORK_EVENT
            || context.status != ACKNOWLEDGED
            || context.payload_length != NETWORK_EVENT_BYTES as u32
        {
            return None;
        }
        let event = decode_network_event(&context.payload)?;
        event.valid().then_some(event)
    }

    /// # Safety
    /// `address` must point to a live, aligned `ControlPage` mapping.
    pub unsafe fn block_at(address: u64) -> Option<logos_abi::BlockRequest> {
        let context = unsafe { (address as *const Self).read_volatile() };
        if context.abi != ABI
            || context.reserved != 0
            || context.operation != BLOCK_REQUEST
            || context.status != ACKNOWLEDGED
            || context.payload_length != BLOCK_REQUEST_BYTES as u32
        {
            return None;
        }
        let request = decode_block_request(&context.payload)?;
        request.valid_shape().then_some(request)
    }

    /// # Safety
    /// `address` must point to a live, aligned `ControlPage` mapping before service startup.
    pub unsafe fn configure_block_page_at(address: u64, page: BlockPage) -> bool {
        if page.handle.0 == 0
            || page.address == 0
            || !page.address.is_multiple_of(logos_abi::PAGE_SIZE as u64)
        {
            return false;
        }
        let mut context = unsafe { (address as *mut Self).read_volatile() };
        if context.abi != ABI
            || context.reserved != 0
            || context.operation != 0
            || context.status != 0
        {
            return false;
        }
        context.slot0 = page.handle.0;
        context.slot1 = page.address as u32;
        context.slot2 = (page.address >> 32) as u32;
        unsafe { (address as *mut Self).write_volatile(context) };
        true
    }

    /// # Safety
    /// `address` must point to a live, aligned `ControlPage` mapping before service startup.
    pub unsafe fn configure_shared_page_at(address: u64, page: logos_abi::PageHandle) -> bool {
        if page.0 == 0 {
            return false;
        }
        let mut context = unsafe { (address as *mut Self).read_volatile() };
        if context.abi != ABI
            || context.reserved != 0
            || context.operation != 0
            || context.status != 0
        {
            return false;
        }
        context.shared_page = page.0;
        unsafe { (address as *mut Self).write_volatile(context) };
        true
    }

    /// # Safety
    /// `address` must point to a live, aligned `ControlPage` mapping owned by Core.
    pub unsafe fn remap_shared_page_at(address: u64, page: logos_abi::PageHandle) -> bool {
        if page.0 == 0 {
            return false;
        }
        let mut context = unsafe { (address as *mut Self).read_volatile() };
        if context.abi != ABI || context.reserved != 0 || context.shared_page == 0 {
            return false;
        }
        context.shared_page = page.0;
        unsafe { (address as *mut Self).write_volatile(context) };
        true
    }

    /// # Safety
    /// `address` must point to a live, aligned `ControlPage` mapping before service startup.
    pub unsafe fn configure_network_pages_at(
        address: u64,
        rx: logos_abi::PageHandle,
        tx: logos_abi::PageHandle,
    ) -> bool {
        if rx.0 == 0 || tx.0 == 0 || rx == tx {
            return false;
        }
        let mut context = unsafe { (address as *mut Self).read_volatile() };
        if context.abi != ABI
            || context.reserved != 0
            || context.operation != 0
            || context.status != 0
        {
            return false;
        }
        context.network_rx_page = rx.0;
        context.network_tx_page = tx.0;
        unsafe { (address as *mut Self).write_volatile(context) };
        true
    }

    /// # Safety
    /// `address` must point to a live, aligned `ControlPage` mapping.
    pub unsafe fn shared_page_at(address: u64) -> Option<logos_abi::PageHandle> {
        let context = unsafe { (address as *const Self).read_volatile() };
        (context.abi == ABI && context.reserved == 0 && context.shared_page != 0)
            .then_some(logos_abi::PageHandle(context.shared_page))
    }

    /// # Safety
    /// `address` must point to a live, aligned `ControlPage` mapping.
    pub unsafe fn network_pages_at(address: u64) -> Option<NetworkPages> {
        let context = unsafe { (address as *const Self).read_volatile() };
        if context.abi != ABI
            || context.reserved != 0
            || context.network_rx_page == 0
            || context.network_tx_page == 0
        {
            return None;
        }
        let rx_address = address.checked_sub(logos_abi::PAGE_SIZE as u64 * 19)?;
        Some(NetworkPages {
            rx_handle: logos_abi::PageHandle(context.network_rx_page),
            rx_address,
            tx_handle: logos_abi::PageHandle(context.network_tx_page),
            tx_address: rx_address.checked_sub(logos_abi::PAGE_SIZE as u64)?,
        })
    }

    /// # Safety
    /// `address` must point to a live, aligned `ControlPage` mapping.
    pub unsafe fn block_page_at(address: u64) -> Option<BlockPage> {
        let context = unsafe { (address as *const Self).read_volatile() };
        let page = BlockPage {
            handle: logos_abi::PageHandle(context.slot0),
            address: u64::from(context.slot1) | (u64::from(context.slot2) << 32),
        };
        (context.abi == ABI
            && context.reserved == 0
            && page.handle.0 != 0
            && page.address != 0
            && page.address.is_multiple_of(logos_abi::PAGE_SIZE as u64))
        .then_some(page)
    }

    /// # Safety
    /// `address` must point to a live, aligned `ControlPage` mapping owned by the caller.
    pub unsafe fn request_block_at(address: u64, request: logos_abi::BlockRequest) -> bool {
        if !request.valid_shape() {
            return false;
        }
        let mut context = unsafe { (address as *mut Self).read_volatile() };
        if context.abi != ABI
            || context.reserved != 0
            || context.status != ACKNOWLEDGED
            || !matches!(context.operation, READY | READ_INPUT | STORE_REQUEST | BLOCK_REPLY)
        {
            return false;
        }
        let parent_store_id = if context.operation == STORE_REQUEST {
            let Some(parent) = decode_store_request(&context.payload) else {
                return false;
            };
            Some(parent.id)
        } else {
            None
        };
        if let Some(id) = parent_store_id {
            // `color` is free while a Block request is active and preserves the
            // Store request ID across the nested Block round trip.
            context.slot2 = id;
        } else if context.operation != BLOCK_REPLY {
            context.slot2 = 0;
        }
        encode_block_request(&mut context.payload, request);
        context.payload_length = BLOCK_REQUEST_BYTES as u32;
        context.operation = BLOCK_REQUEST;
        unsafe { (address as *mut Self).write_volatile(context) };
        true
    }

    /// # Safety
    /// `address` must point to a live, aligned `ControlPage` mapping.
    pub unsafe fn reply_block_at(address: u64, reply: logos_abi::BlockReply) -> bool {
        let Some(request) = (unsafe { Self::block_at(address) }) else {
            return false;
        };
        if !reply.valid_for(request) {
            return false;
        }
        let mut context = unsafe { (address as *mut Self).read_volatile() };
        context.operation = BLOCK_REPLY;
        encode_block_reply(&mut context.payload, reply);
        context.payload_length = BLOCK_REPLY_BYTES as u32;
        unsafe { (address as *mut Self).write_volatile(context) };
        true
    }

    /// # Safety
    /// `address` must point to a live, aligned `ControlPage` mapping.
    pub unsafe fn block_reply_at(address: u64, expected_id: u32) -> Option<logos_abi::BlockReply> {
        let context = unsafe { (address as *const Self).read_volatile() };
        if context.abi != ABI
            || context.reserved != 0
            || context.operation != BLOCK_REPLY
            || context.status != ACKNOWLEDGED
            || context.payload_length != BLOCK_REPLY_BYTES as u32
        {
            return None;
        }
        let reply = decode_block_reply(&context.payload)?;
        (reply.id == expected_id).then_some(reply)
    }

    /// # Safety
    ///
    /// `address` must point to a live, aligned `ControlPage` mapping.
    pub unsafe fn display_waiting_at(address: u64) -> bool {
        let context = unsafe { (address as *const Self).read_volatile() };
        if context.abi != ABI
            || context.reserved != 0
            || context.status != ACKNOWLEDGED
            || context.display_page == 0
        {
            return false;
        }
        unsafe { DisplayPage::pending_at(context.display_page, context.generation) }
    }
}

impl Default for ControlPage {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy)]
#[repr(C)]
pub struct Header {
    pub magic: [u8; 4],
    pub abi: u16,
    pub reserved: u16,
    pub name: [u8; 16],
    pub protocol: ProtocolVersion,
    pub entry: extern "C" fn(*mut ControlPage) -> !,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct ProtocolVersion {
    pub major: u16,
    pub minor: u16,
}

impl ProtocolVersion {
    pub const V1: Self = Self { major: 1, minor: 0 };

    pub const fn supports(self, required: Self) -> bool {
        self.major == required.major && self.minor >= required.minor
    }
}

impl Header {
    pub const fn new(
        name: [u8; 16],
        protocol: ProtocolVersion,
        entry: extern "C" fn(*mut ControlPage) -> !,
    ) -> Self {
        Self { magic: MAGIC, abi: ABI, reserved: 0, name, protocol, entry }
    }

    pub fn entry_address(&self) -> usize {
        self.entry as usize
    }

    pub fn valid_for(&self, name: &[u8], protocol: ProtocolVersion) -> bool {
        self.magic == MAGIC
            && self.abi == ABI
            && self.reserved == 0
            && self.protocol.supports(protocol)
            && self.name_starts_with(name)
    }

    fn name_starts_with(&self, name: &[u8]) -> bool {
        if name.len() > self.name.len() {
            return false;
        }
        let mut index = 0;
        while index < name.len() {
            if self.name[index] != name[index] {
                return false;
            }
            index += 1;
        }
        index == self.name.len() || self.name[index] == 0
    }
}

pub fn self_check() -> bool {
    let mut control = ControlPage::new();
    control.operation = READY;
    let reset = unsafe { ControlPage::reset_at((&mut control as *mut ControlPage) as u64) }
        && control.abi == ABI
        && control.operation == 0;
    Header::new(*b"terminal\0\0\0\0\0\0\0\0", ProtocolVersion::V1, self_check_entry)
        .valid_for(b"terminal", ProtocolVersion::V1)
        && !Header::new(*b"terminal\0\0\0\0\0\0\0\0", ProtocolVersion::V1, self_check_entry)
            .valid_for(b"other", ProtocolVersion::V1)
        && reset
}

extern "C" fn self_check_entry(_: *mut ControlPage) -> ! {
    loop {
        core::hint::spin_loop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abi_self_check_covers_header_and_control_reset() {
        assert!(self_check());
    }

    #[test]
    fn persistence_replies_round_trip_and_match_ids() {
        let mut context = ControlPage::new();
        context.operation = READ_INPUT;
        context.status = ACKNOWLEDGED;
        let store = logos_abi::StoreRequest {
            id: 7,
            operation: logos_abi::StoreOperation::Commit,
            namespace: logos_abi::NamespaceId(0),
            name: [0; logos_abi::MAX_OBJECT_NAME],
            name_length: 0,
            version: logos_abi::VersionSelector::None,
            offset: 0,
            length: 0,
            page: logos_abi::PageHandle(0),
            deadline: 0,
        };
        let address = (&mut context as *mut ControlPage) as u64;
        assert!(unsafe { ControlPage::request_store_at(address, store) });
        assert!(unsafe {
            ControlPage::store_at(address).is_some_and(|request| {
                request.id == store.id && request.operation == store.operation
            })
        });
        let block = logos_abi::BlockRequest {
            id: 9,
            operation: logos_abi::BlockOperation::Flush,
            lba: 0,
            blocks: 0,
            page: logos_abi::PageHandle(0),
            deadline: 0,
        };
        assert!(unsafe { ControlPage::request_block_at(address, block) });
        assert_eq!(unsafe { ControlPage::block_at(address) }, Some(block));
        let block_reply = logos_abi::BlockReply {
            id: 9,
            status: logos_abi::PersistenceStatus::Complete,
            info: logos_abi::BlockInfo::default(),
        };
        assert!(unsafe { ControlPage::reply_block_at(address, block_reply) });
        assert!(unsafe { ControlPage::block_reply_at(address, 9) }.is_some());
        assert!(unsafe {
            !ControlPage::reply_store_at(
                address,
                logos_abi::StoreReply {
                    id: 8,
                    status: logos_abi::PersistenceStatus::Complete,
                    version: 3,
                    length: 0,
                },
            )
        });
        let store_reply = logos_abi::StoreReply {
            id: 7,
            status: logos_abi::PersistenceStatus::Complete,
            version: 3,
            length: 0,
        };
        assert!(unsafe { ControlPage::reply_store_at(address, store_reply) });
        assert!(unsafe { ControlPage::store_reply_at(address, 8) }.is_none());
        assert_eq!(unsafe { ControlPage::store_reply_at(address, 7) }, Some(store_reply));

        context.operation = 0;
        context.status = 0;
        unsafe { (address as *mut ControlPage).write_volatile(context) };
        assert!(unsafe {
            ControlPage::configure_shared_page_at(address, logos_abi::PageHandle(0x10001))
        });
        assert_eq!(
            unsafe { ControlPage::shared_page_at(address) },
            Some(logos_abi::PageHandle(0x10001))
        );
        context.operation = READY;
        context.status = ACKNOWLEDGED;
        unsafe { (address as *mut ControlPage).write_volatile(context) };
        assert!(unsafe { ControlPage::request_store_at(address, store) });
        assert!(
            unsafe { ControlPage::store_at(address) }.is_some_and(
                |request| request.id == store.id && request.operation == store.operation
            )
        );

        context.operation = READ_INPUT;
        context.status = ACKNOWLEDGED;
        unsafe { (address as *mut ControlPage).write_volatile(context) };
        let block = logos_abi::BlockRequest {
            id: 9,
            operation: logos_abi::BlockOperation::Flush,
            lba: 0,
            blocks: 0,
            page: logos_abi::PageHandle(0),
            deadline: 0,
        };
        assert!(unsafe { ControlPage::request_block_at(address, block) });
        assert_eq!(unsafe { ControlPage::block_at(address) }, Some(block));
        let block_reply = logos_abi::BlockReply {
            id: 9,
            status: logos_abi::PersistenceStatus::Complete,
            info: logos_abi::BlockInfo::default(),
        };
        assert!(unsafe { ControlPage::reply_block_at(address, block_reply) });
        assert!(unsafe { ControlPage::block_reply_at(address, 10) }.is_none());
        assert_eq!(unsafe { ControlPage::block_reply_at(address, 9) }, Some(block_reply));
    }

    #[test]
    fn remote_gate_request_and_reply_round_trip() {
        let mut context = ControlPage::new();
        context.operation = READ_INPUT;
        context.status = ACKNOWLEDGED;
        let address = (&mut context as *mut ControlPage) as u64;
        let request = RemoteGateRequest {
            id: 3,
            operation: RemoteGateOperation::Invoke,
            page: logos_abi::PageHandle(8),
            length: 42,
            deadline: 99,
        };
        assert!(unsafe { ControlPage::request_remote_gate_at(address, request) });
        assert_eq!(unsafe { ControlPage::remote_gate_at(address) }, Some(request));
        let reply = RemoteGateReply {
            id: request.id,
            status: RemoteGateStatus::Complete,
            length: 7,
            cursor: 11,
        };
        assert!(unsafe { ControlPage::reply_remote_gate_at(address, reply) });
        assert_eq!(unsafe { ControlPage::remote_gate_reply_at(address, request.id) }, Some(reply));
        assert!(unsafe { ControlPage::remote_gate_reply_at(address, request.id + 1) }.is_none());
    }

    #[test]
    fn block_page_is_configured_and_reply_ids_are_checked() {
        let mut context = ControlPage::new();
        let address = (&mut context as *mut ControlPage) as u64;
        let page = BlockPage { handle: logos_abi::PageHandle(7), address: 0x2000 };
        assert!(unsafe { ControlPage::configure_block_page_at(address, page) });
        assert_eq!(unsafe { ControlPage::block_page_at(address) }, Some(page));
        assert!(unsafe { ControlPage::set_generation_at(address, 2) });
        assert_eq!(unsafe { ControlPage::block_page_at(address) }, Some(page));
        let mut ready = unsafe { (address as *const ControlPage).read_volatile() };
        ready.operation = READY;
        ready.status = ACKNOWLEDGED;
        unsafe { (address as *mut ControlPage).write_volatile(ready) };
        let request = logos_abi::BlockRequest {
            id: 3,
            operation: logos_abi::BlockOperation::Info,
            lba: 0,
            blocks: 0,
            page: logos_abi::PageHandle(0),
            deadline: 1,
        };
        assert!(unsafe { ControlPage::request_block_at(address, request) });
        assert!(!unsafe {
            ControlPage::reply_block_at(
                address,
                logos_abi::BlockReply {
                    id: 4,
                    status: logos_abi::PersistenceStatus::Complete,
                    info: logos_abi::BlockInfo {
                        logical_block_size: 512,
                        blocks: 1,
                        max_transfer_blocks: 1,
                    },
                },
            )
        });
    }

    #[test]
    fn network_request_reply_and_deadline_event_are_bounded() {
        let mut context = ControlPage::new();
        context.operation = READ_INPUT;
        context.status = ACKNOWLEDGED;
        let request = logos_abi::NetworkRequest {
            id: 11,
            operation: logos_abi::NetworkOperation::Bind,
            endpoint: logos_abi::NetworkEndpoint(0),
            peer: logos_abi::NetworkScope::new(logos_abi::NetworkProtocol::Udp, 0, 4000),
            page: logos_abi::PageHandle(0),
            length: 0,
            generation: 0,
            deadline: 100,
        };
        let address = (&mut context as *mut ControlPage) as u64;
        assert!(unsafe { ControlPage::request_network_at(address, request) });
        assert_eq!(unsafe { ControlPage::network_at(address) }, Some(request));
        let reply = logos_abi::NetworkReply {
            id: request.id,
            status: logos_abi::NetworkStatus::Complete,
            endpoint: logos_abi::NetworkEndpoint::new(1, 1).unwrap(),
            generation: 1,
            source_address: 0,
            source_port: 0,
            length: 0,
            info: logos_abi::NetworkInfo { generation: 1, ..Default::default() },
            counters: logos_abi::NetworkCounters::default(),
        };
        assert!(unsafe { ControlPage::reply_network_at(address, reply) });
        assert_eq!(unsafe { ControlPage::network_reply_at(address, request.id) }, Some(reply));
        assert!(unsafe { ControlPage::network_wait_at(address, 101) });
        assert!(unsafe { ControlPage::network_waiting_at(address) });
        let event = logos_abi::NetworkEvent {
            id: 12,
            kind: logos_abi::NetworkEventKind::Timer,
            generation: 1,
            length: 0,
            now: 101,
        };
        assert!(unsafe { ControlPage::deliver_network_event_at(address, event) });
        assert_eq!(unsafe { ControlPage::network_event_at(address) }, Some(event));
        assert!(unsafe { ControlPage::reply_network_after_event_at(address, request, reply) });
        assert_eq!(unsafe { ControlPage::network_reply_at(address, request.id) }, Some(reply));
        assert!(unsafe { !ControlPage::network_wait_at(address, 0) });
        assert!(unsafe { ControlPage::network_reply_at(address, request.id + 1) }.is_none());

        let mut pages_context = ControlPage::new();
        let pages_address = (&mut pages_context as *mut ControlPage) as u64;
        assert!(unsafe {
            ControlPage::configure_network_pages_at(
                pages_address,
                logos_abi::PageHandle(1),
                logos_abi::PageHandle(2),
            )
        });
        let pages = unsafe { ControlPage::network_pages_at(pages_address) }.unwrap();
        assert_eq!(pages.tx_address, pages.rx_address - 4096);
    }

    #[test]
    fn network_device_gate_rejects_mismatch_and_delivers_async_completion() {
        let mut context = ControlPage::new();
        context.operation = READ_INPUT;
        context.status = ACKNOWLEDGED;
        let address = (&mut context as *mut ControlPage) as u64;
        let request = logos_abi::NetworkDeviceRequest {
            id: 9,
            operation: logos_abi::NetworkDeviceOperation::Info,
            length: 0,
            generation: 0,
            deadline: 1,
        };
        assert!(unsafe { ControlPage::request_network_device_at(address, request) });
        let reply = logos_abi::NetworkDeviceReply {
            id: request.id,
            status: logos_abi::NetworkStatus::Complete,
            generation: 1,
            info: logos_abi::NetworkInfo { generation: 1, ..Default::default() },
        };
        assert!(!unsafe {
            ControlPage::reply_network_device_at(
                address,
                logos_abi::NetworkDeviceReply { id: 8, ..reply },
            )
        });
        assert!(unsafe { ControlPage::reply_network_device_at(address, reply) });
        assert!(unsafe { ControlPage::network_wait_at(address, 2) });
        assert!(unsafe { ControlPage::deliver_network_device_reply_at(address, reply) });
        assert_eq!(
            unsafe { ControlPage::network_device_reply_at(address, request.id) },
            Some(reply)
        );
        assert!(unsafe { ControlPage::network_device_reply_at(address, request.id + 1) }.is_none());
    }

    #[test]
    fn input_page_transitions_and_rejects_stale_generation() {
        let mut page = InputPage::new(7);
        let address = (&mut page as *mut InputPage) as u64;
        assert!(unsafe { InputPage::wait_at(address, 7) });
        assert!(unsafe { InputPage::waiting_at(address, 7) });
        assert!(!unsafe { InputPage::deliver_at(address, 8, b'x') });
        assert!(unsafe { InputPage::deliver_at(address, 7, b'x') });
        assert_eq!(unsafe { InputPage::take_at(address, 7) }, Some(b'x'));
        assert!(!unsafe { InputPage::take_at(address, 7) }.is_some());
        assert!(!unsafe { InputPage::deliver_at(address, 7, b'y') });
        assert!(unsafe { InputPage::reset_at(address, 8) });
        assert!(!unsafe { InputPage::wait_at(address, 7) });
        assert!(unsafe { InputPage::wait_at(address, 8) });
    }

    #[test]
    fn display_page_completes_and_rejects_stale_generation() {
        let mut page = DisplayPage::new(3);
        let address = (&mut page as *mut DisplayPage) as u64;
        assert!(unsafe {
            DisplayPage::request_text_at(address, 3, 8, 16, logos_abi::DisplayColor::GREEN, b"ok")
        });
        let request = unsafe { DisplayPage::request_at(address, 3) }.unwrap();
        assert_eq!(request.operation, PRESENT_TEXT);
        assert_eq!(request.length, 2);
        assert!(!unsafe { DisplayPage::complete_at(address, 4) });
        assert!(unsafe { DisplayPage::complete_at(address, 3) });
        assert!(unsafe { DisplayPage::finish_at(address, 3) });
        assert!(!unsafe { DisplayPage::finish_at(address, 3) });
        assert!(unsafe { DisplayPage::request_clear_at(address, 3) });
        assert!(unsafe { DisplayPage::reset_at(address, 4) });
        assert!(!unsafe { DisplayPage::pending_at(address, 3) });
    }

    #[test]
    fn session_client_page_matches_ids_and_rejects_stale_generations() {
        let mut page = SessionClientPage::new(2, 5);
        let address = (&mut page as *mut SessionClientPage) as u64;
        let request = logos_abi::SessionRequest::new(logos_abi::Syscall::Tasks, [0; MAX_TEXT], 0);
        assert!(unsafe { SessionClientPage::request_at(address, 2, 5, 11, request) });
        assert!(unsafe { SessionClientPage::take_request_at(address, 1, 5) }.is_none());
        assert_eq!(
            unsafe { SessionClientPage::take_request_at(address, 2, 5) }.map(|message| message.id),
            Some(11)
        );
        let reply = logos_abi::SessionReply::from_bytes(b"ok").unwrap();
        assert!(!unsafe {
            SessionClientPage::reply_at(address, 2, 5, 12, SessionStatus::Complete, reply)
        });
        assert!(unsafe {
            SessionClientPage::reply_at(address, 2, 5, 11, SessionStatus::Complete, reply)
        });
        assert!(unsafe { SessionClientPage::finish_at(address, 2, 4, 11) }.is_none());
        let completed = unsafe { SessionClientPage::finish_at(address, 2, 5, 11) }.unwrap();
        assert_eq!(completed.status, SessionStatus::Complete);
        assert_eq!(&completed.reply.text[..completed.reply.length], b"ok");
        assert!(unsafe { SessionClientPage::reset_at(address, 3, 6) });
        assert!(!unsafe { SessionClientPage::request_at(address, 2, 5, 12, request) });
    }

    #[test]
    fn session_server_page_preserves_caller_and_rejects_malformed_state() {
        let mut page = SessionServerPage::new(4, 9);
        let address = (&mut page as *mut SessionServerPage) as u64;
        let request = logos_abi::SessionRequest::new(
            logos_abi::Syscall::Inspect,
            {
                let mut bytes = [0; MAX_TEXT];
                bytes[..4].copy_from_slice(b"name");
                bytes
            },
            4,
        );
        assert!(unsafe { SessionServerPage::wait_at(address, 4, 9) });
        assert!(unsafe { SessionServerPage::waiting_at(address, 4, 9) });
        assert!(unsafe {
            SessionServerPage::deliver_at(address, 4, 9, 17, 0x2000_0000_0000_0007, request)
        });
        let delivered = unsafe { SessionServerPage::take_at(address, 4, 9) }.unwrap();
        assert_eq!(delivered.id, 17);
        assert_eq!(delivered.caller, 0x2000_0000_0000_0007);
        let reply = logos_abi::SessionReply::from_bytes(b"name").unwrap();
        assert!(unsafe {
            SessionServerPage::reply_at(address, 4, 9, 17, SessionStatus::Complete, reply)
        });
        assert!(unsafe { SessionServerPage::take_reply_at(address, 4, 8, 17) }.is_none());
        assert_eq!(
            unsafe { SessionServerPage::take_reply_at(address, 4, 9, 17) }.map(|reply| reply.id),
            Some(17)
        );
        page.state = u32::MAX;
        unsafe { (address as *mut SessionServerPage).write_volatile(page) };
        assert!(unsafe { SessionServerPage::take_at(address, 4, 9) }.is_none());
    }

    #[test]
    fn effect_page_round_trip_denies_and_rejects_stale_results() {
        let mut page = EffectPage::new(6, 3);
        let address = (&mut page as *mut EffectPage) as u64;
        let request = logos_abi::EffectRequest::new(logos_abi::Effect::ReadTasks, [0; MAX_TEXT], 0);
        assert!(unsafe { EffectPage::request_at(address, 6, 3, 21, request) });
        assert_eq!(
            unsafe { EffectPage::take_at(address, 6, 3) }.map(|message| message.id),
            Some(21)
        );
        let denied = logos_abi::EffectReply::new(logos_abi::EffectResult::Denied, &[]);
        assert!(!unsafe { EffectPage::reply_at(address, 6, 3, 20, denied) });
        assert!(unsafe { EffectPage::reply_at(address, 6, 3, 21, denied) });
        assert!(unsafe { EffectPage::finish_at(address, 7, 3, 21) }.is_none());
        assert_eq!(
            unsafe { EffectPage::finish_at(address, 6, 3, 21) }
                .map(|response| response.reply.result),
            Some(logos_abi::EffectResult::Denied)
        );
        assert!(unsafe { EffectPage::reset_at(address, 7, 4) });
        assert!(!unsafe { EffectPage::request_at(address, 6, 3, 22, request) });
    }
}
