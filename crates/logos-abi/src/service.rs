use crate as logos_abi;
use core::mem::{align_of, size_of};

pub mod block;
pub mod display;
pub mod input;
pub mod network;
pub mod remote;
pub mod session;
pub mod storage;
pub use remote::{
    RemoteGateOperation, RemoteGateStatus, RemotePage, RemotePageReply, RemotePageRequest,
    RemotePageState,
};

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
    pub store_client_page: u64,
    pub store_server_page: u64,
    pub block_client_page: u64,
    pub remote_page: u64,
    pub network_client_page: u64,
    pub network_server_page: u64,
    pub slot0: u32,
    pub slot1: u32,
    pub network_device_page: u64,
    pub network_event_page: u64,
    pub network_stream_page: u64,
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
///
/// Persistence roles use independent pages. Store clients submit `Ready ->
/// Request -> Waiting`, Core mediates to the Store server's `Ready -> Waiting
/// -> Request -> Processing` path, and terminal replies reset both pages to
/// `Ready`. Block clients use `Ready -> Request -> Submitted` and Core writes
/// a terminal result before the client resets the page. All terminal writes
/// require the matching generations and request ID; invalid scalar states,
/// malformed bounded values, and stale identities leave the page unchanged.
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum PersistencePageState {
    Ready = 1,
    Waiting = 2,
    Request = 3,
    Processing = 4,
    Submitted = 5,
    Reply = 6,
    Denied = 7,
    Failed = 8,
    Cancelled = 9,
    TimedOut = 10,
}

impl PersistencePageState {
    pub const fn wire(self) -> u32 {
        self as u32
    }

    pub const fn from_wire(value: u32) -> Option<Self> {
        Some(match value {
            1 => Self::Ready,
            2 => Self::Waiting,
            3 => Self::Request,
            4 => Self::Processing,
            5 => Self::Submitted,
            6 => Self::Reply,
            7 => Self::Denied,
            8 => Self::Failed,
            9 => Self::Cancelled,
            10 => Self::TimedOut,
            _ => return None,
        })
    }
}

/// Core-mediated Store client page. Only the owning client service maps it.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct StoreClientPage {
    pub service_generation: u32,
    pub endpoint_generation: u32,
    pub state: u32,
    pub request_id: u32,
    pub operation: u32,
    pub namespace: u32,
    pub name_length: u32,
    pub name: [u8; logos_abi::MAX_OBJECT_NAME],
    pub version: u32,
    pub offset: u64,
    pub length: u32,
    pub page: u32,
    pub deadline: u64,
    pub transfer_page: u32,
    pub reply_status: u32,
    pub reply_version: u64,
    pub reply_length: u32,
}

/// Core-mediated Store server page. Only Storage maps it.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct StoreServerPage {
    pub service_generation: u32,
    pub endpoint_generation: u32,
    pub state: u32,
    pub request_id: u32,
    pub caller_low: u32,
    pub caller_high: u32,
    pub operation: u32,
    pub namespace: u32,
    pub name_length: u32,
    pub name: [u8; logos_abi::MAX_OBJECT_NAME],
    pub version: u32,
    pub offset: u64,
    pub length: u32,
    pub page: u32,
    pub deadline: u64,
    pub transfer_page: u32,
    pub reply_status: u32,
    pub reply_version: u64,
    pub reply_length: u32,
    pub service_status: u32,
}

/// Core-mediated Block client page. Only Storage maps it.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct BlockClientPage {
    pub service_generation: u32,
    pub endpoint_generation: u32,
    pub state: u32,
    pub request_id: u32,
    pub operation: u32,
    pub lba: u64,
    pub blocks: u32,
    pub page: u32,
    pub deadline: u64,
    pub transfer_page: u32,
    pub reply_status: u32,
    pub logical_block_size: u32,
    pub block_count: u64,
    pub max_transfer_blocks: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum NetworkDevicePageState {
    Ready = 1,
    Request = 2,
    Submitted = 3,
    Reply = 4,
}

impl NetworkDevicePageState {
    pub const fn from_wire(value: u32) -> Option<Self> {
        Some(match value {
            1 => Self::Ready,
            2 => Self::Request,
            3 => Self::Submitted,
            4 => Self::Reply,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum NetworkEventPageState {
    Ready = 1,
    Waiting = 2,
    Event = 3,
    Consumed = 4,
}

impl NetworkEventPageState {
    pub const fn from_wire(value: u32) -> Option<Self> {
        Some(match value {
            1 => Self::Ready,
            2 => Self::Waiting,
            3 => Self::Event,
            4 => Self::Consumed,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NetworkDeviceMessage {
    pub request: logos_abi::NetworkDeviceRequest,
    pub rx_page: logos_abi::PageHandle,
    pub tx_page: logos_abi::PageHandle,
}

/// Core-owned Network device endpoint. Only the Network service maps it.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct NetworkDevicePage {
    pub service_generation: u32,
    pub endpoint_generation: u32,
    pub device_generation: u32,
    pub state: u32,
    pub request_id: u32,
    pub operation: u32,
    pub rx_page: u32,
    pub tx_page: u32,
    pub length: u32,
    pub deadline: u64,
    pub reply_status: u32,
    pub reset_generation: u32,
    pub info: logos_abi::NetworkInfo,
    pub metadata: [u8; 32],
    pub reserved: [u8; logos_abi::PAGE_SIZE - 112],
}

/// Core/Foundation-produced Network event endpoint. It holds one event only.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct NetworkEventPage {
    pub service_generation: u32,
    pub endpoint_generation: u32,
    pub device_generation: u32,
    pub state: u32,
    pub sequence: u32,
    pub kind: u32,
    pub transfer_page: u32,
    pub length: u32,
    pub deadline: u64,
    pub now: u64,
    pub generation: u16,
    pub reserved0: u16,
    pub metadata: [u8; 32],
    pub configured_rx_page: u32,
    pub reserved: [u8; logos_abi::PAGE_SIZE - 88],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum NetworkPageState {
    Ready = 1,
    Request = 2,
    Processing = 3,
    Reply = 4,
    Denied = 5,
    Failed = 6,
    Cancelled = 7,
    TimedOut = 8,
}

impl NetworkPageState {
    pub const fn from_wire(value: u32) -> Option<Self> {
        Some(match value {
            1 => Self::Ready,
            2 => Self::Request,
            3 => Self::Processing,
            4 => Self::Reply,
            5 => Self::Denied,
            6 => Self::Failed,
            7 => Self::Cancelled,
            8 => Self::TimedOut,
            _ => return None,
        })
    }
}

/// Auxiliary, generation-bound stream readiness/completion page.
///
/// The page is owned by Core and shared with the Network service. Records are
/// coalesced per endpoint; the page is not a second request/reply transport.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct StreamPage {
    pub service_generation: u32,
    pub endpoint_generation: u32,
    pub state: u32,
    pub overflow: u32,
    pub sequence: u64,
    pub records: [logos_abi::NetworkStreamRecord; logos_abi::NETWORK_MAX_STREAM_RECORDS],
    pub reserved: [u8; logos_abi::PAGE_SIZE - 408],
}

#[allow(clippy::missing_safety_doc)]
impl StreamPage {
    pub const fn new(service_generation: u32, endpoint_generation: u32) -> Self {
        Self {
            service_generation,
            endpoint_generation,
            state: NetworkPageState::Ready as u32,
            overflow: 0,
            sequence: 0,
            records: [logos_abi::NetworkStreamRecord::EMPTY; logos_abi::NETWORK_MAX_STREAM_RECORDS],
            reserved: [0; logos_abi::PAGE_SIZE - 408],
        }
    }

    pub unsafe fn reset_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> bool {
        if address == 0 || !address.is_multiple_of(core::mem::align_of::<Self>() as u64) {
            return false;
        }
        let current = unsafe { (address as *const Self).read_volatile() };
        if current.service_generation != service_generation
            || current.endpoint_generation != endpoint_generation
            || service_generation == 0
            || endpoint_generation == 0
        {
            return false;
        }
        unsafe {
            (address as *mut Self)
                .write_volatile(Self::new(service_generation, endpoint_generation))
        };
        true
    }

    pub unsafe fn publish_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        mut record: logos_abi::NetworkStreamRecord,
    ) -> bool {
        if !record.endpoint.valid() || record.generation == 0 {
            return false;
        }
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if page.service_generation != service_generation
            || page.endpoint_generation != endpoint_generation
            || service_generation == 0
            || endpoint_generation == 0
        {
            return false;
        }
        page.sequence = page.sequence.wrapping_add(1).max(1);
        record.sequence = page.sequence;
        if let Some(existing) = page.records.iter_mut().find(|item| {
            item.owner == record.owner
                && item.endpoint == record.endpoint
                && item.generation == record.generation
        }) {
            *existing = record;
        } else if let Some(empty) = page.records.iter_mut().find(|item| item.endpoint.0 == 0) {
            *empty = record;
        } else {
            page.overflow = 1;
            unsafe { (address as *mut Self).write_volatile(page) };
            return false;
        }
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    pub unsafe fn take_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        endpoint: logos_abi::NetworkEndpoint,
    ) -> Option<logos_abi::NetworkStreamRecord> {
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if page.service_generation != service_generation
            || page.endpoint_generation != endpoint_generation
            || !endpoint.valid()
        {
            return None;
        }
        let record = page.records.iter_mut().find(|item| item.endpoint == endpoint)?;
        let value = *record;
        *record = logos_abi::NetworkStreamRecord::EMPTY;
        unsafe { (address as *mut Self).write_volatile(page) };
        Some(value)
    }

    pub unsafe fn take_next_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> Option<logos_abi::NetworkStreamRecord> {
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if page.service_generation != service_generation
            || page.endpoint_generation != endpoint_generation
        {
            return None;
        }
        let record = page.records.iter_mut().find(|item| item.endpoint.valid())?;
        let value = *record;
        *record = logos_abi::NetworkStreamRecord::EMPTY;
        unsafe { (address as *mut Self).write_volatile(page) };
        Some(value)
    }

    pub unsafe fn overflow_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> bool {
        let page = unsafe { (address as *const Self).read_volatile() };
        page.service_generation == service_generation
            && page.endpoint_generation == endpoint_generation
            && page.overflow != 0
    }

    pub unsafe fn clear_overflow_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> bool {
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if page.service_generation != service_generation
            || page.endpoint_generation != endpoint_generation
        {
            return false;
        }
        page.overflow = 0;
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }
}

/// Client-owned Network request/reply page.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct NetworkClientPage {
    pub service_generation: u32,
    pub endpoint_generation: u32,
    pub state: u32,
    pub request_id: u32,
    pub operation: u32,
    pub endpoint: u32,
    pub peer: u64,
    pub page: u32,
    pub length: u16,
    pub generation: u16,
    pub deadline: u64,
    pub transfer_page: u32,
    pub reply_status: u32,
    pub reply_endpoint: u32,
    pub reply_generation: u16,
    pub reply_source_port: u16,
    pub reply_source_address: u32,
    pub reply_length: u16,
    pub reserved0: u16,
    pub reply_stream_readiness: u16,
    pub reply_stream_reserved: u16,
    pub reply_stream_accepted_bytes: u64,
    pub reply_stream_acknowledged_bytes: u64,
    pub reply_info: logos_abi::NetworkInfo,
    pub reply_counters: logos_abi::NetworkCounters,
}

/// Network-owned server request/reply page.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct NetworkServerPage {
    pub service_generation: u32,
    pub endpoint_generation: u32,
    pub state: u32,
    pub request_id: u32,
    pub caller_low: u32,
    pub caller_high: u32,
    pub operation: u32,
    pub endpoint: u32,
    pub peer: u64,
    pub page: u32,
    pub length: u16,
    pub generation: u16,
    pub deadline: u64,
    pub transfer_page: u32,
    pub reply_status: u32,
    pub reply_endpoint: u32,
    pub reply_generation: u16,
    pub reply_source_port: u16,
    pub reply_source_address: u32,
    pub reply_length: u16,
    pub reserved0: u16,
    pub reply_stream_readiness: u16,
    pub reply_stream_reserved: u16,
    pub reply_stream_accepted_bytes: u64,
    pub reply_stream_acknowledged_bytes: u64,
    pub reply_info: logos_abi::NetworkInfo,
    pub reply_counters: logos_abi::NetworkCounters,
}

#[derive(Clone, Copy)]
pub struct NetworkServerRequest {
    pub id: u32,
    pub caller: u64,
    pub request: logos_abi::NetworkRequest,
}

#[allow(clippy::too_many_arguments)]
fn network_request_from_fields(
    id: u32,
    operation: u32,
    endpoint: u32,
    peer: u64,
    page: u32,
    length: u16,
    generation: u16,
    deadline: u64,
) -> Option<logos_abi::NetworkRequest> {
    let request = logos_abi::NetworkRequest {
        id,
        operation: logos_abi::NetworkOperation::from_wire(u8::try_from(operation).ok()?)?,
        endpoint: logos_abi::NetworkEndpoint(endpoint),
        peer: logos_abi::NetworkScope(peer),
        page: logos_abi::PageHandle(page),
        length,
        generation,
        deadline,
    };
    request.valid_shape().then_some(request)
}

#[allow(clippy::too_many_arguments)]
fn network_reply_from_page(
    id: u32,
    status: u32,
    endpoint: u32,
    generation: u16,
    source_address: u32,
    source_port: u16,
    length: u16,
    stream_readiness: u16,
    stream_reserved: u16,
    stream_accepted_bytes: u64,
    stream_acknowledged_bytes: u64,
    info: logos_abi::NetworkInfo,
    counters: logos_abi::NetworkCounters,
) -> Option<logos_abi::NetworkReply> {
    Some(logos_abi::NetworkReply {
        id,
        status: logos_abi::NetworkStatus::from_wire(u8::try_from(status).ok()?)?,
        endpoint: logos_abi::NetworkEndpoint(endpoint),
        generation,
        source_address,
        source_port,
        length,
        stream_readiness,
        stream_reserved,
        stream_accepted_bytes,
        stream_acknowledged_bytes,
        info,
        counters,
    })
}

fn network_reply_state(status: logos_abi::NetworkStatus) -> NetworkPageState {
    match status {
        logos_abi::NetworkStatus::Complete => NetworkPageState::Reply,
        logos_abi::NetworkStatus::Denied => NetworkPageState::Denied,
        logos_abi::NetworkStatus::Cancelled => NetworkPageState::Cancelled,
        logos_abi::NetworkStatus::TimedOut => NetworkPageState::TimedOut,
        _ => NetworkPageState::Failed,
    }
}

#[allow(clippy::too_many_arguments)]
fn set_network_reply(
    state: &mut u32,
    reply_status: &mut u32,
    reply_endpoint: &mut u32,
    reply_generation: &mut u16,
    reply_source_address: &mut u32,
    reply_source_port: &mut u16,
    reply_length: &mut u16,
    reply_stream_readiness: &mut u16,
    reply_stream_reserved: &mut u16,
    reply_stream_accepted_bytes: &mut u64,
    reply_stream_acknowledged_bytes: &mut u64,
    reply_info: &mut logos_abi::NetworkInfo,
    reply_counters: &mut logos_abi::NetworkCounters,
    request: logos_abi::NetworkRequest,
    reply: logos_abi::NetworkReply,
) -> bool {
    if !reply.valid_for(request) {
        return false;
    }
    *reply_status = reply.status as u32;
    *reply_endpoint = reply.endpoint.0;
    *reply_generation = reply.generation;
    *reply_source_address = reply.source_address;
    *reply_source_port = reply.source_port;
    *reply_length = reply.length;
    *reply_stream_readiness = reply.stream_readiness;
    *reply_stream_reserved = reply.stream_reserved;
    *reply_stream_accepted_bytes = reply.stream_accepted_bytes;
    *reply_stream_acknowledged_bytes = reply.stream_acknowledged_bytes;
    *reply_info = reply.info;
    *reply_counters = reply.counters;
    *state = network_reply_state(reply.status) as u32;
    true
}

#[allow(clippy::missing_safety_doc)]
impl NetworkClientPage {
    pub const fn new(service_generation: u32, endpoint_generation: u32) -> Self {
        Self {
            service_generation,
            endpoint_generation,
            state: NetworkPageState::Ready as u32,
            request_id: 0,
            operation: 0,
            endpoint: 0,
            peer: 0,
            page: 0,
            length: 0,
            generation: 0,
            deadline: 0,
            transfer_page: 0,
            reply_status: 0,
            reply_endpoint: 0,
            reply_generation: 0,
            reply_source_port: 0,
            reply_source_address: 0,
            reply_length: 0,
            reserved0: 0,
            reply_stream_readiness: 0,
            reply_stream_reserved: 0,
            reply_stream_accepted_bytes: 0,
            reply_stream_acknowledged_bytes: 0,
            reply_info: logos_abi::NetworkInfo {
                mac: [0; 6],
                mtu: 0,
                generation: 0,
                link_up: 0,
                configuration: 0,
                ipv4: 0,
                subnet_mask: 0,
                router: 0,
            },
            reply_counters: logos_abi::NetworkCounters {
                rx_frames: 0,
                tx_frames: 0,
                rx_bytes: 0,
                tx_bytes: 0,
                malformed: 0,
                unsupported: 0,
                rx_dropped: 0,
                udp_no_endpoint: 0,
                udp_queue_dropped: 0,
                timeouts: 0,
                cancellations: 0,
                resets: 0,
                denied: 0,
            },
        }
    }

    pub unsafe fn reset_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> bool {
        if !valid_page_identity::<Self>(address, service_generation, endpoint_generation) {
            return false;
        }
        let old = unsafe { (address as *const Self).read_volatile() };
        let mut page = Self::new(service_generation, endpoint_generation);
        page.transfer_page = old.transfer_page;
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    pub unsafe fn configure_transfer_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        handle: logos_abi::PageHandle,
    ) -> bool {
        if handle.0 == 0 {
            return false;
        }
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if !client_identity(&page, service_generation, endpoint_generation)
            || NetworkPageState::from_wire(page.state) != Some(NetworkPageState::Ready)
        {
            return false;
        }
        page.transfer_page = handle.0;
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    pub unsafe fn transfer_page_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> Option<logos_abi::PageHandle> {
        let page = unsafe { (address as *const Self).read_volatile() };
        (client_identity(&page, service_generation, endpoint_generation) && page.transfer_page != 0)
            .then_some(logos_abi::PageHandle(page.transfer_page))
    }

    pub unsafe fn request_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        request: logos_abi::NetworkRequest,
    ) -> bool {
        if !request.valid_shape() {
            return false;
        }
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if !client_identity(&page, service_generation, endpoint_generation)
            || NetworkPageState::from_wire(page.state) != Some(NetworkPageState::Ready)
        {
            return false;
        }
        if matches!(
            request.operation,
            logos_abi::NetworkOperation::SendTo
                | logos_abi::NetworkOperation::ReceiveFrom
                | logos_abi::NetworkOperation::Read
                | logos_abi::NetworkOperation::Write
        ) && page.transfer_page != request.page.0
        {
            return false;
        }
        page.request_id = request.id;
        page.operation = request.operation as u32;
        page.endpoint = request.endpoint.0;
        page.peer = request.peer.0;
        page.page = request.page.0;
        page.length = request.length;
        page.generation = request.generation;
        page.deadline = request.deadline;
        page.reply_status = 0;
        page.state = NetworkPageState::Request as u32;
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    pub unsafe fn pending_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> bool {
        let page = unsafe { (address as *const Self).read_volatile() };
        client_identity(&page, service_generation, endpoint_generation)
            && NetworkPageState::from_wire(page.state) == Some(NetworkPageState::Request)
    }

    pub unsafe fn request_at_page(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> Option<logos_abi::NetworkRequest> {
        let page = unsafe { (address as *const Self).read_volatile() };
        if !client_identity(&page, service_generation, endpoint_generation)
            || NetworkPageState::from_wire(page.state) != Some(NetworkPageState::Request)
        {
            return None;
        }
        network_request_from_fields(
            page.request_id,
            page.operation,
            page.endpoint,
            page.peer,
            page.page,
            page.length,
            page.generation,
            page.deadline,
        )
    }

    pub unsafe fn mark_processing_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> bool {
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if !client_identity(&page, service_generation, endpoint_generation)
            || NetworkPageState::from_wire(page.state) != Some(NetworkPageState::Request)
        {
            return false;
        }
        page.state = NetworkPageState::Processing as u32;
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    pub unsafe fn reply_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        reply: logos_abi::NetworkReply,
    ) -> bool {
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        let Some(request) =
            unsafe { Self::request_at_page(address, service_generation, endpoint_generation) }
                .or_else(|| {
                    network_request_from_fields(
                        page.request_id,
                        page.operation,
                        page.endpoint,
                        page.peer,
                        page.page,
                        page.length,
                        page.generation,
                        page.deadline,
                    )
                })
        else {
            return false;
        };
        if !client_identity(&page, service_generation, endpoint_generation)
            || !matches!(
                NetworkPageState::from_wire(page.state),
                Some(NetworkPageState::Processing)
            )
        {
            return false;
        }
        set_network_reply(
            &mut page.state,
            &mut page.reply_status,
            &mut page.reply_endpoint,
            &mut page.reply_generation,
            &mut page.reply_source_address,
            &mut page.reply_source_port,
            &mut page.reply_length,
            &mut page.reply_stream_readiness,
            &mut page.reply_stream_reserved,
            &mut page.reply_stream_accepted_bytes,
            &mut page.reply_stream_acknowledged_bytes,
            &mut page.reply_info,
            &mut page.reply_counters,
            request,
            reply,
        ) && unsafe {
            (address as *mut Self).write_volatile(page);
            true
        }
    }

    pub unsafe fn reply_request_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        reply: logos_abi::NetworkReply,
    ) -> bool {
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        let Some(request) =
            (unsafe { Self::request_at_page(address, service_generation, endpoint_generation) })
        else {
            return false;
        };
        if !client_identity(&page, service_generation, endpoint_generation)
            || NetworkPageState::from_wire(page.state) != Some(NetworkPageState::Request)
        {
            return false;
        }
        set_network_reply(
            &mut page.state,
            &mut page.reply_status,
            &mut page.reply_endpoint,
            &mut page.reply_generation,
            &mut page.reply_source_address,
            &mut page.reply_source_port,
            &mut page.reply_length,
            &mut page.reply_stream_readiness,
            &mut page.reply_stream_reserved,
            &mut page.reply_stream_accepted_bytes,
            &mut page.reply_stream_acknowledged_bytes,
            &mut page.reply_info,
            &mut page.reply_counters,
            request,
            reply,
        ) && unsafe {
            (address as *mut Self).write_volatile(page);
            true
        }
    }

    pub unsafe fn finish_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        expected_id: u32,
    ) -> Option<logos_abi::NetworkReply> {
        let page = unsafe { (address as *const Self).read_volatile() };
        if !client_identity(&page, service_generation, endpoint_generation)
            || page.request_id != expected_id
            || !matches!(
                NetworkPageState::from_wire(page.state),
                Some(
                    NetworkPageState::Reply
                        | NetworkPageState::Denied
                        | NetworkPageState::Failed
                        | NetworkPageState::Cancelled
                        | NetworkPageState::TimedOut
                )
            )
        {
            return None;
        }
        let reply = network_reply_from_page(
            page.request_id,
            page.reply_status,
            page.reply_endpoint,
            page.reply_generation,
            page.reply_source_address,
            page.reply_source_port,
            page.reply_length,
            page.reply_stream_readiness,
            page.reply_stream_reserved,
            page.reply_stream_accepted_bytes,
            page.reply_stream_acknowledged_bytes,
            page.reply_info,
            page.reply_counters,
        )?;
        unsafe { Self::reset_at(address, service_generation, endpoint_generation) };
        Some(reply)
    }
}

#[allow(clippy::missing_safety_doc)]
impl NetworkServerPage {
    pub const fn new(service_generation: u32, endpoint_generation: u32) -> Self {
        Self {
            service_generation,
            endpoint_generation,
            state: NetworkPageState::Ready as u32,
            request_id: 0,
            caller_low: 0,
            caller_high: 0,
            operation: 0,
            endpoint: 0,
            peer: 0,
            page: 0,
            length: 0,
            generation: 0,
            deadline: 0,
            transfer_page: 0,
            reply_status: 0,
            reply_endpoint: 0,
            reply_generation: 0,
            reply_source_port: 0,
            reply_source_address: 0,
            reply_length: 0,
            reserved0: 0,
            reply_stream_readiness: 0,
            reply_stream_reserved: 0,
            reply_stream_accepted_bytes: 0,
            reply_stream_acknowledged_bytes: 0,
            reply_info: logos_abi::NetworkInfo {
                mac: [0; 6],
                mtu: 0,
                generation: 0,
                link_up: 0,
                configuration: 0,
                ipv4: 0,
                subnet_mask: 0,
                router: 0,
            },
            reply_counters: logos_abi::NetworkCounters {
                rx_frames: 0,
                tx_frames: 0,
                rx_bytes: 0,
                tx_bytes: 0,
                malformed: 0,
                unsupported: 0,
                rx_dropped: 0,
                udp_no_endpoint: 0,
                udp_queue_dropped: 0,
                timeouts: 0,
                cancellations: 0,
                resets: 0,
                denied: 0,
            },
        }
    }

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

    pub unsafe fn deliver_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        caller: u64,
        request: logos_abi::NetworkRequest,
    ) -> bool {
        if !request.valid_shape() {
            return false;
        }
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if !server_identity(&page, service_generation, endpoint_generation)
            || NetworkPageState::from_wire(page.state) != Some(NetworkPageState::Ready)
        {
            return false;
        }
        page.request_id = request.id;
        page.caller_low = caller as u32;
        page.caller_high = (caller >> 32) as u32;
        page.operation = request.operation as u32;
        page.endpoint = request.endpoint.0;
        page.peer = request.peer.0;
        page.page = request.page.0;
        page.length = request.length;
        page.generation = request.generation;
        page.deadline = request.deadline;
        page.state = NetworkPageState::Request as u32;
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    pub unsafe fn take_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> Option<NetworkServerRequest> {
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if !server_identity(&page, service_generation, endpoint_generation)
            || NetworkPageState::from_wire(page.state) != Some(NetworkPageState::Request)
        {
            return None;
        }
        let request = network_request_from_fields(
            page.request_id,
            page.operation,
            page.endpoint,
            page.peer,
            page.page,
            page.length,
            page.generation,
            page.deadline,
        )?;
        page.state = NetworkPageState::Processing as u32;
        unsafe { (address as *mut Self).write_volatile(page) };
        Some(NetworkServerRequest {
            id: request.id,
            caller: u64::from(page.caller_low) | (u64::from(page.caller_high) << 32),
            request,
        })
    }

    pub unsafe fn pending_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> bool {
        let page = unsafe { (address as *const Self).read_volatile() };
        server_identity(&page, service_generation, endpoint_generation)
            && NetworkPageState::from_wire(page.state) == Some(NetworkPageState::Request)
    }

    pub unsafe fn reply_pending_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> bool {
        let page = unsafe { (address as *const Self).read_volatile() };
        server_identity(&page, service_generation, endpoint_generation)
            && matches!(
                NetworkPageState::from_wire(page.state),
                Some(
                    NetworkPageState::Reply
                        | NetworkPageState::Denied
                        | NetworkPageState::Failed
                        | NetworkPageState::Cancelled
                        | NetworkPageState::TimedOut
                )
            )
    }

    pub unsafe fn reply_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        reply: logos_abi::NetworkReply,
    ) -> bool {
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        let Some(request) = network_request_from_fields(
            page.request_id,
            page.operation,
            page.endpoint,
            page.peer,
            page.page,
            page.length,
            page.generation,
            page.deadline,
        ) else {
            return false;
        };
        if !server_identity(&page, service_generation, endpoint_generation)
            || NetworkPageState::from_wire(page.state) != Some(NetworkPageState::Processing)
        {
            return false;
        }
        set_network_reply(
            &mut page.state,
            &mut page.reply_status,
            &mut page.reply_endpoint,
            &mut page.reply_generation,
            &mut page.reply_source_address,
            &mut page.reply_source_port,
            &mut page.reply_length,
            &mut page.reply_stream_readiness,
            &mut page.reply_stream_reserved,
            &mut page.reply_stream_accepted_bytes,
            &mut page.reply_stream_acknowledged_bytes,
            &mut page.reply_info,
            &mut page.reply_counters,
            request,
            reply,
        ) && unsafe {
            (address as *mut Self).write_volatile(page);
            true
        }
    }

    pub unsafe fn finish_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        expected_id: u32,
    ) -> Option<logos_abi::NetworkReply> {
        let page = unsafe { (address as *const Self).read_volatile() };
        if !server_identity(&page, service_generation, endpoint_generation)
            || page.request_id != expected_id
            || !matches!(
                NetworkPageState::from_wire(page.state),
                Some(
                    NetworkPageState::Reply
                        | NetworkPageState::Denied
                        | NetworkPageState::Failed
                        | NetworkPageState::Cancelled
                        | NetworkPageState::TimedOut
                )
            )
        {
            return None;
        }
        let reply = network_reply_from_page(
            page.request_id,
            page.reply_status,
            page.reply_endpoint,
            page.reply_generation,
            page.reply_source_address,
            page.reply_source_port,
            page.reply_length,
            page.reply_stream_readiness,
            page.reply_stream_reserved,
            page.reply_stream_accepted_bytes,
            page.reply_stream_acknowledged_bytes,
            page.reply_info,
            page.reply_counters,
        )?;
        unsafe { Self::reset_at(address, service_generation, endpoint_generation) };
        Some(reply)
    }
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
pub struct StoreServerRequest {
    pub id: u32,
    pub caller: u64,
    pub request: logos_abi::StoreRequest,
}

fn persistence_state(status: logos_abi::PersistenceStatus) -> PersistencePageState {
    match status {
        logos_abi::PersistenceStatus::Complete | logos_abi::PersistenceStatus::Recovered => {
            PersistencePageState::Reply
        }
        logos_abi::PersistenceStatus::Denied => PersistencePageState::Denied,
        logos_abi::PersistenceStatus::Cancelled => PersistencePageState::Cancelled,
        logos_abi::PersistenceStatus::TimedOut => PersistencePageState::TimedOut,
        _ => PersistencePageState::Failed,
    }
}

#[allow(clippy::too_many_arguments)]
fn store_request_from_fields(
    id: u32,
    operation: u32,
    namespace: u32,
    name_length: u32,
    name: [u8; logos_abi::MAX_OBJECT_NAME],
    version: u32,
    offset: u64,
    length: u32,
    page: u32,
    deadline: u64,
) -> Option<logos_abi::StoreRequest> {
    let operation = logos_abi::StoreOperation::from_wire(u8::try_from(operation).ok()?)?;
    let version = logos_abi::VersionSelector::from_wire(u8::try_from(version).ok()?)?;
    let request = logos_abi::StoreRequest {
        id,
        operation,
        namespace: logos_abi::NamespaceId(namespace),
        name,
        name_length: u8::try_from(name_length).ok()?,
        version,
        offset,
        length,
        page: logos_abi::PageHandle(page),
        deadline,
    };
    request.valid().then_some(request)
}

fn store_reply_from_fields(
    id: u32,
    status: u32,
    version: u64,
    length: u32,
) -> Option<logos_abi::StoreReply> {
    Some(logos_abi::StoreReply {
        id,
        status: logos_abi::PersistenceStatus::from_wire(u8::try_from(status).ok()?)?,
        version,
        length,
    })
}

#[allow(clippy::missing_safety_doc)]
impl StoreClientPage {
    pub const fn new(service_generation: u32, endpoint_generation: u32) -> Self {
        Self {
            service_generation,
            endpoint_generation,
            state: PersistencePageState::Ready.wire(),
            request_id: 0,
            operation: 0,
            namespace: 0,
            name_length: 0,
            name: [0; logos_abi::MAX_OBJECT_NAME],
            version: 0,
            offset: 0,
            length: 0,
            page: 0,
            deadline: 0,
            transfer_page: 0,
            reply_status: 0,
            reply_version: 0,
            reply_length: 0,
        }
    }

    pub unsafe fn reset_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> bool {
        if !valid_page_identity::<Self>(address, service_generation, endpoint_generation) {
            return false;
        }
        let old = unsafe { (address as *const Self).read_volatile() };
        let mut page = Self::new(service_generation, endpoint_generation);
        page.transfer_page = old.transfer_page;
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    pub unsafe fn configure_transfer_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        handle: logos_abi::PageHandle,
    ) -> bool {
        if handle.0 == 0 {
            return false;
        }
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if !client_identity(&page, service_generation, endpoint_generation)
            || PersistencePageState::from_wire(page.state) != Some(PersistencePageState::Ready)
        {
            return false;
        }
        page.transfer_page = handle.0;
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    pub unsafe fn transfer_page_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> Option<logos_abi::PageHandle> {
        let page = unsafe { (address as *const Self).read_volatile() };
        (client_identity(&page, service_generation, endpoint_generation) && page.transfer_page != 0)
            .then_some(logos_abi::PageHandle(page.transfer_page))
    }

    pub unsafe fn request_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        request: logos_abi::StoreRequest,
    ) -> bool {
        if request.id == 0 || !request.valid() {
            return false;
        }
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if !client_identity(&page, service_generation, endpoint_generation)
            || PersistencePageState::from_wire(page.state) != Some(PersistencePageState::Ready)
        {
            return false;
        }
        page.request_id = request.id;
        page.operation = request.operation as u32;
        page.namespace = request.namespace.0;
        page.name_length = u32::from(request.name_length);
        page.name = request.name;
        page.version = request.version as u32;
        page.offset = request.offset;
        page.length = request.length;
        page.page = request.page.0;
        page.deadline = request.deadline;
        page.reply_status = 0;
        page.reply_version = 0;
        page.reply_length = 0;
        page.state = PersistencePageState::Request.wire();
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    pub unsafe fn current_request_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> Option<logos_abi::StoreRequest> {
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        let state = PersistencePageState::from_wire(page.state)?;
        if !client_identity(&page, service_generation, endpoint_generation)
            || !matches!(state, PersistencePageState::Request | PersistencePageState::Waiting)
        {
            return None;
        }
        let request = store_request_from_fields(
            page.request_id,
            page.operation,
            page.namespace,
            page.name_length,
            page.name,
            page.version,
            page.offset,
            page.length,
            page.page,
            page.deadline,
        )?;
        if state == PersistencePageState::Request {
            page.state = PersistencePageState::Waiting.wire();
            unsafe { (address as *mut Self).write_volatile(page) };
        }
        Some(request)
    }

    pub unsafe fn pending_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> bool {
        let page = unsafe { (address as *const Self).read_volatile() };
        client_identity(&page, service_generation, endpoint_generation)
            && PersistencePageState::from_wire(page.state) == Some(PersistencePageState::Request)
    }

    pub unsafe fn reply_pending_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> bool {
        let page = unsafe { (address as *const Self).read_volatile() };
        client_identity(&page, service_generation, endpoint_generation)
            && matches!(
                PersistencePageState::from_wire(page.state),
                Some(
                    PersistencePageState::Reply
                        | PersistencePageState::Denied
                        | PersistencePageState::Failed
                        | PersistencePageState::Cancelled
                        | PersistencePageState::TimedOut
                )
            )
    }

    pub unsafe fn reply_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        reply: logos_abi::StoreReply,
    ) -> bool {
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        let Some(request) =
            (unsafe { Self::current_request_at(address, service_generation, endpoint_generation) })
        else {
            return false;
        };
        if !client_identity(&page, service_generation, endpoint_generation)
            || !reply.valid_for(request)
        {
            return false;
        }
        page.reply_status = reply.status as u32;
        page.reply_version = reply.version;
        page.reply_length = reply.length;
        page.state = persistence_state(reply.status).wire();
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    pub unsafe fn finish_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        id: u32,
    ) -> Option<logos_abi::StoreReply> {
        let page = unsafe { (address as *const Self).read_volatile() };
        let state = PersistencePageState::from_wire(page.state)?;
        if !client_identity(&page, service_generation, endpoint_generation)
            || page.request_id != id
            || !matches!(
                state,
                PersistencePageState::Reply
                    | PersistencePageState::Denied
                    | PersistencePageState::Failed
                    | PersistencePageState::Cancelled
                    | PersistencePageState::TimedOut
            )
        {
            return None;
        }
        let reply = store_reply_from_fields(
            page.request_id,
            page.reply_status,
            page.reply_version,
            page.reply_length,
        )?;
        unsafe { Self::reset_at(address, service_generation, endpoint_generation) };
        Some(reply)
    }

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
                PersistencePageState::from_wire(page.state),
                Some(PersistencePageState::Request | PersistencePageState::Waiting)
            )
        {
            return false;
        }
        page.reply_status = logos_abi::PersistenceStatus::Cancelled as u32;
        page.reply_version = 0;
        page.reply_length = 0;
        page.state = PersistencePageState::Cancelled.wire();
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }
}

#[allow(clippy::missing_safety_doc)]
impl StoreServerPage {
    pub const fn new(service_generation: u32, endpoint_generation: u32) -> Self {
        Self {
            service_generation,
            endpoint_generation,
            state: PersistencePageState::Ready.wire(),
            request_id: 0,
            caller_low: 0,
            caller_high: 0,
            operation: 0,
            namespace: 0,
            name_length: 0,
            name: [0; logos_abi::MAX_OBJECT_NAME],
            version: 0,
            offset: 0,
            length: 0,
            page: 0,
            deadline: 0,
            transfer_page: 0,
            reply_status: 0,
            reply_version: 0,
            reply_length: 0,
            service_status: STORAGE_UNAVAILABLE,
        }
    }

    pub unsafe fn reset_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> bool {
        if !valid_page_identity::<Self>(address, service_generation, endpoint_generation) {
            return false;
        }
        let old = unsafe { (address as *const Self).read_volatile() };
        let mut page = Self::new(service_generation, endpoint_generation);
        page.transfer_page = old.transfer_page;
        page.service_status = old.service_status;
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    pub unsafe fn configure_transfer_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        handle: logos_abi::PageHandle,
    ) -> bool {
        if handle.0 == 0 {
            return false;
        }
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if !server_identity(&page, service_generation, endpoint_generation)
            || !matches!(
                PersistencePageState::from_wire(page.state),
                Some(PersistencePageState::Ready | PersistencePageState::Waiting)
            )
        {
            return false;
        }
        page.transfer_page = handle.0;
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    pub unsafe fn transfer_page_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> Option<logos_abi::PageHandle> {
        let page = unsafe { (address as *const Self).read_volatile() };
        (server_identity(&page, service_generation, endpoint_generation) && page.transfer_page != 0)
            .then_some(logos_abi::PageHandle(page.transfer_page))
    }

    pub unsafe fn wait_at(address: u64, service_generation: u32, endpoint_generation: u32) -> bool {
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if !server_identity(&page, service_generation, endpoint_generation)
            || PersistencePageState::from_wire(page.state) != Some(PersistencePageState::Ready)
        {
            return false;
        }
        page.state = PersistencePageState::Waiting.wire();
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    pub unsafe fn waiting_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> bool {
        let page = unsafe { (address as *const Self).read_volatile() };
        server_identity(&page, service_generation, endpoint_generation)
            && PersistencePageState::from_wire(page.state) == Some(PersistencePageState::Waiting)
    }

    pub unsafe fn deliver_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        caller: u64,
        request: logos_abi::StoreRequest,
    ) -> bool {
        if request.id == 0 || !request.valid() {
            return false;
        }
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if !server_identity(&page, service_generation, endpoint_generation)
            || PersistencePageState::from_wire(page.state) != Some(PersistencePageState::Waiting)
        {
            return false;
        }
        page.request_id = request.id;
        page.caller_low = caller as u32;
        page.caller_high = (caller >> 32) as u32;
        page.operation = request.operation as u32;
        page.namespace = request.namespace.0;
        page.name_length = u32::from(request.name_length);
        page.name = request.name;
        page.version = request.version as u32;
        page.offset = request.offset;
        page.length = request.length;
        page.page = request.page.0;
        page.deadline = request.deadline;
        page.reply_status = 0;
        page.reply_version = 0;
        page.reply_length = 0;
        page.state = PersistencePageState::Request.wire();
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    pub unsafe fn take_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> Option<StoreServerRequest> {
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if !server_identity(&page, service_generation, endpoint_generation)
            || PersistencePageState::from_wire(page.state) != Some(PersistencePageState::Request)
        {
            return None;
        }
        let request = store_request_from_fields(
            page.request_id,
            page.operation,
            page.namespace,
            page.name_length,
            page.name,
            page.version,
            page.offset,
            page.length,
            page.page,
            page.deadline,
        )?;
        page.state = PersistencePageState::Processing.wire();
        unsafe { (address as *mut Self).write_volatile(page) };
        Some(StoreServerRequest {
            id: page.request_id,
            caller: u64::from(page.caller_low) | (u64::from(page.caller_high) << 32),
            request,
        })
    }

    pub unsafe fn reply_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        reply: logos_abi::StoreReply,
    ) -> bool {
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        let Some(request) = store_request_from_fields(
            page.request_id,
            page.operation,
            page.namespace,
            page.name_length,
            page.name,
            page.version,
            page.offset,
            page.length,
            page.page,
            page.deadline,
        ) else {
            return false;
        };
        if !server_identity(&page, service_generation, endpoint_generation)
            || PersistencePageState::from_wire(page.state) != Some(PersistencePageState::Processing)
            || !reply.valid_for(request)
        {
            return false;
        }
        page.reply_status = reply.status as u32;
        page.reply_version = reply.version;
        page.reply_length = reply.length;
        page.state = persistence_state(reply.status).wire();
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    pub unsafe fn take_reply_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        expected_id: u32,
    ) -> Option<logos_abi::StoreReply> {
        let page = unsafe { (address as *const Self).read_volatile() };
        if !server_identity(&page, service_generation, endpoint_generation)
            || page.request_id != expected_id
            || !matches!(
                PersistencePageState::from_wire(page.state),
                Some(
                    PersistencePageState::Reply
                        | PersistencePageState::Denied
                        | PersistencePageState::Failed
                        | PersistencePageState::Cancelled
                        | PersistencePageState::TimedOut
                )
            )
        {
            return None;
        }
        let reply = store_reply_from_fields(
            page.request_id,
            page.reply_status,
            page.reply_version,
            page.reply_length,
        )?;
        unsafe { Self::reset_at(address, service_generation, endpoint_generation) };
        Some(reply)
    }

    pub unsafe fn reply_pending_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> bool {
        let page = unsafe { (address as *const Self).read_volatile() };
        server_identity(&page, service_generation, endpoint_generation)
            && matches!(
                PersistencePageState::from_wire(page.state),
                Some(
                    PersistencePageState::Reply
                        | PersistencePageState::Denied
                        | PersistencePageState::Failed
                        | PersistencePageState::Cancelled
                        | PersistencePageState::TimedOut
                )
            )
    }

    pub unsafe fn set_status_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        status: u32,
    ) -> bool {
        if !(STORAGE_FORMATTED..=STORAGE_UNAVAILABLE).contains(&status) {
            return false;
        }
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if !server_identity(&page, service_generation, endpoint_generation) {
            return false;
        }
        page.service_status = status;
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    pub unsafe fn status_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> Option<u32> {
        let page = unsafe { (address as *const Self).read_volatile() };
        (server_identity(&page, service_generation, endpoint_generation)
            && (STORAGE_FORMATTED..=STORAGE_UNAVAILABLE).contains(&page.service_status))
        .then_some(page.service_status)
    }
}

#[allow(clippy::missing_safety_doc)]
impl BlockClientPage {
    pub const fn new(service_generation: u32, endpoint_generation: u32) -> Self {
        Self {
            service_generation,
            endpoint_generation,
            state: PersistencePageState::Ready.wire(),
            request_id: 0,
            operation: 0,
            lba: 0,
            blocks: 0,
            page: 0,
            deadline: 0,
            transfer_page: 0,
            reply_status: 0,
            logical_block_size: 0,
            block_count: 0,
            max_transfer_blocks: 0,
        }
    }

    pub unsafe fn reset_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> bool {
        if !valid_page_identity::<Self>(address, service_generation, endpoint_generation) {
            return false;
        }
        let old = unsafe { (address as *const Self).read_volatile() };
        let mut page = Self::new(service_generation, endpoint_generation);
        page.transfer_page = old.transfer_page;
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    pub unsafe fn configure_transfer_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        handle: logos_abi::PageHandle,
    ) -> bool {
        if handle.0 == 0 {
            return false;
        }
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if !client_identity(&page, service_generation, endpoint_generation)
            || PersistencePageState::from_wire(page.state) != Some(PersistencePageState::Ready)
        {
            return false;
        }
        page.transfer_page = handle.0;
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    pub unsafe fn transfer_page_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> Option<logos_abi::PageHandle> {
        let page = unsafe { (address as *const Self).read_volatile() };
        (client_identity(&page, service_generation, endpoint_generation) && page.transfer_page != 0)
            .then_some(logos_abi::PageHandle(page.transfer_page))
    }

    pub unsafe fn request_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        request: logos_abi::BlockRequest,
    ) -> bool {
        if request.id == 0 || !request.valid_shape() {
            return false;
        }
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if !client_identity(&page, service_generation, endpoint_generation)
            || PersistencePageState::from_wire(page.state) != Some(PersistencePageState::Ready)
        {
            return false;
        }
        page.request_id = request.id;
        page.operation = request.operation as u32;
        page.lba = request.lba;
        page.blocks = request.blocks;
        page.page = request.page.0;
        page.deadline = request.deadline;
        page.reply_status = 0;
        page.logical_block_size = 0;
        page.block_count = 0;
        page.max_transfer_blocks = 0;
        page.state = PersistencePageState::Request.wire();
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    pub unsafe fn take_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> Option<logos_abi::BlockRequest> {
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if !client_identity(&page, service_generation, endpoint_generation)
            || PersistencePageState::from_wire(page.state) != Some(PersistencePageState::Request)
        {
            return None;
        }
        let request = block_request_from_fields(
            page.request_id,
            page.operation,
            page.lba,
            page.blocks,
            page.page,
            page.deadline,
        )?;
        page.state = PersistencePageState::Submitted.wire();
        unsafe { (address as *mut Self).write_volatile(page) };
        Some(request)
    }

    pub unsafe fn request_at_current(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> Option<logos_abi::BlockRequest> {
        let page = unsafe { (address as *const Self).read_volatile() };
        if !client_identity(&page, service_generation, endpoint_generation)
            || PersistencePageState::from_wire(page.state) != Some(PersistencePageState::Submitted)
        {
            return None;
        }
        block_request_from_fields(
            page.request_id,
            page.operation,
            page.lba,
            page.blocks,
            page.page,
            page.deadline,
        )
    }

    pub unsafe fn pending_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> bool {
        let page = unsafe { (address as *const Self).read_volatile() };
        client_identity(&page, service_generation, endpoint_generation)
            && PersistencePageState::from_wire(page.state) == Some(PersistencePageState::Request)
    }

    pub unsafe fn reply_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        reply: logos_abi::BlockReply,
    ) -> bool {
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        let Some(request) =
            (unsafe { Self::request_at_current(address, service_generation, endpoint_generation) })
        else {
            return false;
        };
        if !reply.valid_for(request) {
            return false;
        }
        page.reply_status = reply.status as u32;
        page.logical_block_size = reply.info.logical_block_size;
        page.block_count = reply.info.blocks;
        page.max_transfer_blocks = reply.info.max_transfer_blocks;
        page.state = persistence_state(reply.status).wire();
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    pub unsafe fn finish_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        id: u32,
    ) -> Option<logos_abi::BlockReply> {
        let page = unsafe { (address as *const Self).read_volatile() };
        if !client_identity(&page, service_generation, endpoint_generation)
            || page.request_id != id
            || !matches!(
                PersistencePageState::from_wire(page.state),
                Some(
                    PersistencePageState::Reply
                        | PersistencePageState::Denied
                        | PersistencePageState::Failed
                        | PersistencePageState::Cancelled
                        | PersistencePageState::TimedOut
                )
            )
        {
            return None;
        }
        let reply = logos_abi::BlockReply {
            id: page.request_id,
            status: logos_abi::PersistenceStatus::from_wire(u8::try_from(page.reply_status).ok()?)?,
            info: logos_abi::BlockInfo {
                logical_block_size: page.logical_block_size,
                blocks: page.block_count,
                max_transfer_blocks: page.max_transfer_blocks,
            },
        };
        unsafe { Self::reset_at(address, service_generation, endpoint_generation) };
        Some(reply)
    }
}

fn block_request_from_fields(
    id: u32,
    operation: u32,
    lba: u64,
    blocks: u32,
    page: u32,
    deadline: u64,
) -> Option<logos_abi::BlockRequest> {
    let request = logos_abi::BlockRequest {
        id,
        operation: logos_abi::BlockOperation::from_wire(u8::try_from(operation).ok()?)?,
        lba,
        blocks,
        page: logos_abi::PageHandle(page),
        deadline,
    };
    request.valid_shape().then_some(request)
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

fn valid_network_page_address<T>(address: u64) -> bool {
    address != 0 && address.is_multiple_of(align_of::<T>() as u64)
}

fn valid_network_device_identity(
    page: &NetworkDevicePage,
    service_generation: u32,
    endpoint_generation: u32,
    device_generation: u32,
) -> bool {
    page.service_generation == service_generation
        && page.endpoint_generation == endpoint_generation
        && page.device_generation == device_generation
        && service_generation != 0
        && endpoint_generation != 0
        && device_generation != 0
}

fn valid_network_event_identity(
    page: &NetworkEventPage,
    service_generation: u32,
    endpoint_generation: u32,
    device_generation: u32,
) -> bool {
    page.service_generation == service_generation
        && page.endpoint_generation == endpoint_generation
        && page.device_generation == device_generation
        && service_generation != 0
        && endpoint_generation != 0
        && device_generation != 0
}

impl NetworkDevicePage {
    pub const fn new(
        service_generation: u32,
        endpoint_generation: u32,
        device_generation: u32,
    ) -> Self {
        Self {
            service_generation,
            endpoint_generation,
            device_generation,
            state: NetworkDevicePageState::Ready as u32,
            request_id: 0,
            operation: 0,
            rx_page: 0,
            tx_page: 0,
            length: 0,
            deadline: 0,
            reply_status: 0,
            reset_generation: 0,
            info: logos_abi::NetworkInfo {
                mac: [0; 6],
                mtu: 0,
                generation: 0,
                link_up: 0,
                configuration: 0,
                ipv4: 0,
                subnet_mask: 0,
                router: 0,
            },
            metadata: [0; 32],
            reserved: [0; logos_abi::PAGE_SIZE - 112],
        }
    }

    /// # Safety
    /// `address` must point to a live, aligned NetworkDevicePage mapping.
    pub unsafe fn reset_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        device_generation: u32,
        rx_page: logos_abi::PageHandle,
        tx_page: logos_abi::PageHandle,
    ) -> bool {
        if !valid_network_page_address::<Self>(address)
            || rx_page.0 == 0
            || tx_page.0 == 0
            || rx_page == tx_page
        {
            return false;
        }
        unsafe {
            (address as *mut Self).write_volatile(Self {
                rx_page: rx_page.0,
                tx_page: tx_page.0,
                ..Self::new(service_generation, endpoint_generation, device_generation)
            })
        };
        true
    }

    /// # Safety
    /// Core replaces the device generation while publishing the matching reset result.
    pub unsafe fn reset_with_reply_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        device_generation: u32,
        rx_page: logos_abi::PageHandle,
        tx_page: logos_abi::PageHandle,
        reply: logos_abi::NetworkDeviceReply,
    ) -> bool {
        if !valid_network_page_address::<Self>(address)
            || device_generation == 0
            || rx_page.0 == 0
            || tx_page.0 == 0
            || rx_page == tx_page
        {
            return false;
        }
        let old = unsafe { (address as *const Self).read_volatile() };
        if old.service_generation == 0
            || old.endpoint_generation == 0
            || old.device_generation == 0
            || NetworkDevicePageState::from_wire(old.state)
                != Some(NetworkDevicePageState::Submitted)
            || old.request_id == 0
            || reply.id != old.request_id
        {
            return false;
        }
        let mut page = Self::new(service_generation, endpoint_generation, device_generation);
        page.request_id = old.request_id;
        page.operation = old.operation;
        page.tx_page = tx_page.0;
        page.rx_page = rx_page.0;
        page.length = old.length;
        page.deadline = old.deadline;
        page.reply_status = reply.status as u32;
        page.reset_generation = u32::from(reply.generation);
        page.info = reply.info;
        page.state = NetworkDevicePageState::Reply as u32;
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    /// # Safety
    /// Core replaces the service generation while retaining Core-owned DMA identities.
    pub unsafe fn reset_generation_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> bool {
        if !valid_network_page_address::<Self>(address) {
            return false;
        }
        let old = unsafe { (address as *const Self).read_volatile() };
        if old.service_generation == 0 || old.endpoint_generation == 0 || old.device_generation == 0
        {
            return false;
        }
        unsafe {
            (address as *mut Self).write_volatile(Self {
                rx_page: old.rx_page,
                tx_page: old.tx_page,
                ..Self::new(service_generation, endpoint_generation, old.device_generation)
            })
        };
        true
    }

    /// # Safety
    /// Core configures a newly mapped page before the service starts.
    pub unsafe fn configure_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        device_generation: u32,
        rx_page: logos_abi::PageHandle,
        tx_page: logos_abi::PageHandle,
    ) -> bool {
        if !valid_network_page_address::<Self>(address)
            || rx_page.0 == 0
            || tx_page.0 == 0
            || rx_page == tx_page
        {
            return false;
        }
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if !valid_network_device_identity(
            &page,
            service_generation,
            endpoint_generation,
            device_generation,
        ) || NetworkDevicePageState::from_wire(page.state) != Some(NetworkDevicePageState::Ready)
            || page.rx_page != 0
            || page.tx_page != 0
        {
            return false;
        }
        page.rx_page = rx_page.0;
        page.tx_page = tx_page.0;
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    /// # Safety
    /// The Network service owns request creation.
    pub unsafe fn request_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        device_generation: u32,
        request: logos_abi::NetworkDeviceRequest,
    ) -> bool {
        if !request.valid_shape()
            || !valid_network_page_address::<Self>(address)
            || request.generation != 0 && u32::from(request.generation) != device_generation
        {
            return false;
        }
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if !valid_network_device_identity(
            &page,
            service_generation,
            endpoint_generation,
            device_generation,
        ) || NetworkDevicePageState::from_wire(page.state) != Some(NetworkDevicePageState::Ready)
            || page.rx_page == 0
            || page.tx_page == 0
        {
            return false;
        }
        page.request_id = request.id;
        page.operation = request.operation as u32;
        page.length = u32::from(request.length);
        page.deadline = request.deadline;
        page.reply_status = 0;
        page.reset_generation = 0;
        page.info = logos_abi::NetworkInfo::default();
        page.state = NetworkDevicePageState::Request as u32;
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    /// # Safety
    /// Core consumes a service-created request.
    pub unsafe fn take_request_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        device_generation: u32,
    ) -> Option<NetworkDeviceMessage> {
        if !valid_network_page_address::<Self>(address) {
            return None;
        }
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if !valid_network_device_identity(
            &page,
            service_generation,
            endpoint_generation,
            device_generation,
        ) || NetworkDevicePageState::from_wire(page.state)
            != Some(NetworkDevicePageState::Request)
        {
            return None;
        }
        let operation =
            logos_abi::NetworkDeviceOperation::from_wire(u8::try_from(page.operation).ok()?)?;
        let length = u16::try_from(page.length).ok()?;
        let generation = match operation {
            logos_abi::NetworkDeviceOperation::Info => 0,
            logos_abi::NetworkDeviceOperation::Transmit
            | logos_abi::NetworkDeviceOperation::Reset => u16::try_from(device_generation).ok()?,
        };
        let request = logos_abi::NetworkDeviceRequest {
            id: page.request_id,
            operation,
            length,
            generation,
            deadline: page.deadline,
        };
        if !request.valid_shape()
            || page.request_id == 0
            || (operation == logos_abi::NetworkDeviceOperation::Transmit && page.tx_page == 0)
        {
            return None;
        }
        page.state = NetworkDevicePageState::Submitted as u32;
        unsafe { (address as *mut Self).write_volatile(page) };
        Some(NetworkDeviceMessage {
            request,
            rx_page: logos_abi::PageHandle(page.rx_page),
            tx_page: logos_abi::PageHandle(page.tx_page),
        })
    }

    /// # Safety
    /// Core completes the current request with a validated driver result.
    pub unsafe fn complete_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        device_generation: u32,
        reply: logos_abi::NetworkDeviceReply,
    ) -> bool {
        if !valid_network_page_address::<Self>(address) {
            return false;
        }
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if !valid_network_device_identity(
            &page,
            service_generation,
            endpoint_generation,
            device_generation,
        ) || NetworkDevicePageState::from_wire(page.state)
            != Some(NetworkDevicePageState::Submitted)
        {
            return false;
        }
        let operation = match logos_abi::NetworkDeviceOperation::from_wire(
            u8::try_from(page.operation).ok().unwrap_or(0),
        ) {
            Some(operation) => operation,
            None => return false,
        };
        let request = logos_abi::NetworkDeviceRequest {
            id: page.request_id,
            operation,
            length: u16::try_from(page.length).ok().unwrap_or(0),
            generation: if operation == logos_abi::NetworkDeviceOperation::Info {
                0
            } else {
                u16::try_from(device_generation).ok().unwrap_or(0)
            },
            deadline: page.deadline,
        };
        if !request.valid_shape() || !reply.valid_for(request) {
            return false;
        }
        page.reply_status = reply.status as u32;
        page.reset_generation = u32::from(reply.generation);
        page.info = reply.info;
        page.state = NetworkDevicePageState::Reply as u32;
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    /// # Safety
    /// The Network service consumes only its matching completion.
    pub unsafe fn take_reply_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        device_generation: u32,
        expected_id: u32,
    ) -> Option<logos_abi::NetworkDeviceReply> {
        if !valid_network_page_address::<Self>(address) {
            return None;
        }
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if !valid_network_device_identity(
            &page,
            service_generation,
            endpoint_generation,
            device_generation,
        ) || NetworkDevicePageState::from_wire(page.state) != Some(NetworkDevicePageState::Reply)
            || page.request_id != expected_id
        {
            return None;
        }
        let status = logos_abi::NetworkStatus::from_wire(u8::try_from(page.reply_status).ok()?)?;
        let reply = logos_abi::NetworkDeviceReply {
            id: page.request_id,
            status,
            generation: u16::try_from(page.reset_generation).ok()?,
            info: page.info,
        };
        page.request_id = 0;
        page.operation = 0;
        page.length = 0;
        page.deadline = 0;
        page.reply_status = 0;
        page.reset_generation = 0;
        page.info = logos_abi::NetworkInfo::default();
        page.state = NetworkDevicePageState::Ready as u32;
        unsafe { (address as *mut Self).write_volatile(page) };
        Some(reply)
    }

    /// # Safety
    /// Core reads only scalar state.
    pub unsafe fn pending_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        device_generation: u32,
    ) -> bool {
        if !valid_network_page_address::<Self>(address) {
            return false;
        }
        let page = unsafe { (address as *const Self).read_volatile() };
        valid_network_device_identity(
            &page,
            service_generation,
            endpoint_generation,
            device_generation,
        ) && matches!(
            NetworkDevicePageState::from_wire(page.state),
            Some(NetworkDevicePageState::Request | NetworkDevicePageState::Submitted)
        )
    }

    /// # Safety
    /// Core reads only the generation and state fields from the endpoint mapping.
    pub unsafe fn active_for_core_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> bool {
        if !valid_network_page_address::<Self>(address) {
            return false;
        }
        let page = unsafe { (address as *const Self).read_volatile() };
        page.service_generation == service_generation
            && page.endpoint_generation == endpoint_generation
            && service_generation != 0
            && endpoint_generation != 0
            && page.device_generation != 0
            && matches!(
                NetworkDevicePageState::from_wire(page.state),
                Some(
                    NetworkDevicePageState::Request
                        | NetworkDevicePageState::Submitted
                        | NetworkDevicePageState::Reply
                )
            )
    }

    /// # Safety
    /// Core reads only scalar state.
    pub unsafe fn reply_pending_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        device_generation: u32,
    ) -> bool {
        if !valid_network_page_address::<Self>(address) {
            return false;
        }
        let page = unsafe { (address as *const Self).read_volatile() };
        valid_network_device_identity(
            &page,
            service_generation,
            endpoint_generation,
            device_generation,
        ) && NetworkDevicePageState::from_wire(page.state) == Some(NetworkDevicePageState::Reply)
    }

    /// # Safety
    /// The Network service reads configured DMA identities.
    pub unsafe fn dma_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        device_generation: u32,
    ) -> Option<(logos_abi::PageHandle, logos_abi::PageHandle)> {
        if !valid_network_page_address::<Self>(address) {
            return None;
        }
        let page = unsafe { (address as *const Self).read_volatile() };
        if !valid_network_device_identity(
            &page,
            service_generation,
            endpoint_generation,
            device_generation,
        ) || page.rx_page == 0
            || page.tx_page == 0
            || page.rx_page == page.tx_page
        {
            return None;
        }
        Some((logos_abi::PageHandle(page.rx_page), logos_abi::PageHandle(page.tx_page)))
    }
}

impl NetworkEventPage {
    pub const fn new(
        service_generation: u32,
        endpoint_generation: u32,
        device_generation: u32,
    ) -> Self {
        Self {
            service_generation,
            endpoint_generation,
            device_generation,
            state: NetworkEventPageState::Ready as u32,
            sequence: 0,
            kind: 0,
            transfer_page: 0,
            length: 0,
            deadline: 0,
            now: 0,
            generation: 0,
            reserved0: 0,
            metadata: [0; 32],
            configured_rx_page: 0,
            reserved: [0; logos_abi::PAGE_SIZE - 88],
        }
    }

    /// # Safety
    /// `address` must point to a live, aligned NetworkEventPage mapping.
    pub unsafe fn reset_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        device_generation: u32,
        rx_page: logos_abi::PageHandle,
    ) -> bool {
        if !valid_network_page_address::<Self>(address) || rx_page.0 == 0 {
            return false;
        }
        unsafe {
            (address as *mut Self).write_volatile(Self {
                configured_rx_page: rx_page.0,
                ..Self::new(service_generation, endpoint_generation, device_generation)
            })
        };
        true
    }

    /// # Safety
    /// Core replaces the service generation while retaining the RX identity.
    pub unsafe fn reset_generation_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> bool {
        if !valid_network_page_address::<Self>(address) {
            return false;
        }
        let old = unsafe { (address as *const Self).read_volatile() };
        if old.service_generation == 0 || old.endpoint_generation == 0 || old.device_generation == 0
        {
            return false;
        }
        unsafe {
            (address as *mut Self).write_volatile(Self {
                configured_rx_page: old.configured_rx_page,
                ..Self::new(service_generation, endpoint_generation, old.device_generation)
            })
        };
        true
    }

    /// # Safety
    /// Core configures a newly mapped page before the service starts.
    pub unsafe fn configure_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        device_generation: u32,
        rx_page: logos_abi::PageHandle,
    ) -> bool {
        if !valid_network_page_address::<Self>(address) || rx_page.0 == 0 {
            return false;
        }
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if !valid_network_event_identity(
            &page,
            service_generation,
            endpoint_generation,
            device_generation,
        ) || NetworkEventPageState::from_wire(page.state) != Some(NetworkEventPageState::Ready)
            || page.configured_rx_page != 0
        {
            return false;
        }
        page.configured_rx_page = rx_page.0;
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    /// # Safety
    /// The Network service owns wait creation.
    pub unsafe fn wait_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        device_generation: u32,
        deadline: u64,
    ) -> bool {
        if !valid_network_page_address::<Self>(address) || deadline == 0 {
            return false;
        }
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if !valid_network_event_identity(
            &page,
            service_generation,
            endpoint_generation,
            device_generation,
        ) || page.configured_rx_page == 0
            || NetworkEventPageState::from_wire(page.state) != Some(NetworkEventPageState::Ready)
        {
            return false;
        }
        page.deadline = deadline;
        page.state = NetworkEventPageState::Waiting as u32;
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    /// # Safety
    /// Core reads only scalar state.
    pub unsafe fn waiting_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        device_generation: u32,
    ) -> bool {
        if !valid_network_page_address::<Self>(address) {
            return false;
        }
        let page = unsafe { (address as *const Self).read_volatile() };
        valid_network_event_identity(
            &page,
            service_generation,
            endpoint_generation,
            device_generation,
        ) && NetworkEventPageState::from_wire(page.state) == Some(NetworkEventPageState::Waiting)
    }

    /// # Safety
    /// Core reads only scalar state.
    pub unsafe fn deadline_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        device_generation: u32,
    ) -> Option<u64> {
        if !valid_network_page_address::<Self>(address) {
            return None;
        }
        let page = unsafe { (address as *const Self).read_volatile() };
        (valid_network_event_identity(
            &page,
            service_generation,
            endpoint_generation,
            device_generation,
        ) && NetworkEventPageState::from_wire(page.state) == Some(NetworkEventPageState::Waiting))
        .then_some(page.deadline)
        .filter(|deadline| *deadline != 0)
    }

    /// # Safety
    /// Core delivers one event only while the service is waiting.
    pub unsafe fn deliver_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        device_generation: u32,
        event: logos_abi::NetworkEvent,
    ) -> bool {
        if !event.valid() || !valid_network_page_address::<Self>(address) {
            return false;
        }
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if !valid_network_event_identity(
            &page,
            service_generation,
            endpoint_generation,
            device_generation,
        ) || NetworkEventPageState::from_wire(page.state) != Some(NetworkEventPageState::Waiting)
            || event.device_generation != device_generation
            || event.generation != u16::try_from(device_generation).ok().unwrap_or(0)
        {
            return false;
        }
        if event.kind == logos_abi::NetworkEventKind::Frame {
            if event.page.0 != page.configured_rx_page {
                return false;
            }
        } else if event.page.0 != 0 || event.length != 0 {
            return false;
        }
        page.sequence = event.id;
        page.kind = event.kind as u32;
        page.transfer_page = event.page.0;
        page.length = u32::from(event.length);
        page.now = event.now;
        page.generation = event.generation;
        page.metadata = [0; 32];
        page.metadata[..event.metadata.len()].copy_from_slice(&event.metadata);
        page.state = NetworkEventPageState::Event as u32;
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    /// # Safety
    /// The Network service consumes the single delivered event.
    pub unsafe fn take_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        device_generation: u32,
    ) -> Option<logos_abi::NetworkEvent> {
        if !valid_network_page_address::<Self>(address) {
            return None;
        }
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if !valid_network_event_identity(
            &page,
            service_generation,
            endpoint_generation,
            device_generation,
        ) || NetworkEventPageState::from_wire(page.state) != Some(NetworkEventPageState::Event)
        {
            return None;
        }
        let kind = logos_abi::NetworkEventKind::from_wire(u8::try_from(page.kind).ok()?)?;
        let event = logos_abi::NetworkEvent {
            id: page.sequence,
            kind,
            generation: page.generation,
            device_generation,
            page: logos_abi::PageHandle(page.transfer_page),
            length: u16::try_from(page.length).ok()?,
            now: page.now,
            metadata: page.metadata[..16].try_into().ok()?,
        };
        if !event.valid() {
            return None;
        }
        page.state = NetworkEventPageState::Consumed as u32;
        unsafe { (address as *mut Self).write_volatile(page) };
        Some(event)
    }

    /// # Safety
    /// The Network service acknowledges and releases the event slot.
    pub unsafe fn acknowledge_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        device_generation: u32,
    ) -> bool {
        if !valid_network_page_address::<Self>(address) {
            return false;
        }
        let mut page = unsafe { (address as *mut Self).read_volatile() };
        if !valid_network_event_identity(
            &page,
            service_generation,
            endpoint_generation,
            device_generation,
        ) || NetworkEventPageState::from_wire(page.state)
            != Some(NetworkEventPageState::Consumed)
        {
            return false;
        }
        page.sequence = 0;
        page.kind = 0;
        page.transfer_page = 0;
        page.length = 0;
        page.deadline = 0;
        page.now = 0;
        page.generation = 0;
        page.metadata = [0; 32];
        page.state = NetworkEventPageState::Ready as u32;
        unsafe { (address as *mut Self).write_volatile(page) };
        true
    }

    /// # Safety
    /// Core reads only scalar state.
    pub unsafe fn pending_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
        device_generation: u32,
    ) -> bool {
        if !valid_network_page_address::<Self>(address) {
            return false;
        }
        let page = unsafe { (address as *const Self).read_volatile() };
        valid_network_event_identity(
            &page,
            service_generation,
            endpoint_generation,
            device_generation,
        ) && NetworkEventPageState::from_wire(page.state) == Some(NetworkEventPageState::Event)
    }

    /// # Safety
    /// Core reads only the generation and state fields from the endpoint mapping.
    pub unsafe fn active_for_core_at(
        address: u64,
        service_generation: u32,
        endpoint_generation: u32,
    ) -> bool {
        if !valid_network_page_address::<Self>(address) {
            return false;
        }
        let page = unsafe { (address as *const Self).read_volatile() };
        page.service_generation == service_generation
            && page.endpoint_generation == endpoint_generation
            && service_generation != 0
            && endpoint_generation != 0
            && page.device_generation != 0
            && matches!(
                NetworkEventPageState::from_wire(page.state),
                Some(
                    NetworkEventPageState::Waiting
                        | NetworkEventPageState::Event
                        | NetworkEventPageState::Consumed
                )
            )
    }
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

trait GenerationPage {
    fn service_generation(&self) -> u32;
    fn endpoint_generation(&self) -> u32;
}

impl GenerationPage for SessionClientPage {
    fn service_generation(&self) -> u32 {
        self.service_generation
    }

    fn endpoint_generation(&self) -> u32 {
        self.endpoint_generation
    }
}

impl GenerationPage for StoreClientPage {
    fn service_generation(&self) -> u32 {
        self.service_generation
    }

    fn endpoint_generation(&self) -> u32 {
        self.endpoint_generation
    }
}

impl GenerationPage for BlockClientPage {
    fn service_generation(&self) -> u32 {
        self.service_generation
    }

    fn endpoint_generation(&self) -> u32 {
        self.endpoint_generation
    }
}

impl GenerationPage for SessionServerPage {
    fn service_generation(&self) -> u32 {
        self.service_generation
    }

    fn endpoint_generation(&self) -> u32 {
        self.endpoint_generation
    }
}

impl GenerationPage for StoreServerPage {
    fn service_generation(&self) -> u32 {
        self.service_generation
    }

    fn endpoint_generation(&self) -> u32 {
        self.endpoint_generation
    }
}

impl GenerationPage for NetworkClientPage {
    fn service_generation(&self) -> u32 {
        self.service_generation
    }
    fn endpoint_generation(&self) -> u32 {
        self.endpoint_generation
    }
}

impl GenerationPage for NetworkServerPage {
    fn service_generation(&self) -> u32 {
        self.service_generation
    }
    fn endpoint_generation(&self) -> u32 {
        self.endpoint_generation
    }
}

impl GenerationPage for RemotePage {
    fn service_generation(&self) -> u32 {
        self.service_generation
    }

    fn endpoint_generation(&self) -> u32 {
        self.endpoint_generation
    }
}

fn client_identity<T: GenerationPage>(
    page: &T,
    service_generation: u32,
    endpoint_generation: u32,
) -> bool {
    page.service_generation() == service_generation
        && page.endpoint_generation() == endpoint_generation
}

fn server_identity<T: GenerationPage>(
    page: &T,
    service_generation: u32,
    endpoint_generation: u32,
) -> bool {
    page.service_generation() == service_generation
        && page.endpoint_generation() == endpoint_generation
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
const _: () = assert!(size_of::<StoreClientPage>() <= logos_abi::PAGE_SIZE);
const _: () = assert!(size_of::<StoreServerPage>() <= logos_abi::PAGE_SIZE);
const _: () = assert!(size_of::<BlockClientPage>() <= logos_abi::PAGE_SIZE);
const _: () = assert!(size_of::<NetworkDevicePage>() == logos_abi::PAGE_SIZE);
const _: () = assert!(size_of::<NetworkEventPage>() == logos_abi::PAGE_SIZE);
const _: () = assert!(size_of::<StreamPage>() == logos_abi::PAGE_SIZE);
const _: () = assert!(size_of::<RemotePage>() == logos_abi::PAGE_SIZE);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NetworkDmaResources {
    pub rx_handle: logos_abi::PageHandle,
    pub rx_address: u64,
    pub tx_handle: logos_abi::PageHandle,
    pub tx_address: u64,
}

#[allow(clippy::missing_safety_doc, clippy::too_many_arguments)]
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
            store_client_page: 0,
            store_server_page: 0,
            block_client_page: 0,
            remote_page: 0,
            network_client_page: 0,
            network_server_page: 0,
            slot0: 0,
            slot1: 0,
            network_device_page: 0,
            network_event_page: 0,
            network_stream_page: 0,
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
        reset.store_client_page = current.store_client_page;
        reset.store_server_page = current.store_server_page;
        reset.block_client_page = current.block_client_page;
        reset.remote_page = current.remote_page;
        reset.network_client_page = current.network_client_page;
        reset.network_server_page = current.network_server_page;
        reset.network_device_page = current.network_device_page;
        reset.network_event_page = current.network_event_page;
        reset.network_stream_page = current.network_stream_page;
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
        store_client_page: Option<u64>,
        store_server_page: Option<u64>,
        block_client_page: Option<u64>,
        remote_page: Option<u64>,
        network_client_page: Option<u64>,
        network_server_page: Option<u64>,
        network_device_page: Option<u64>,
        network_event_page: Option<u64>,
        network_stream_page: Option<u64>,
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
        context.store_client_page = store_client_page.unwrap_or(0);
        context.store_server_page = store_server_page.unwrap_or(0);
        context.block_client_page = block_client_page.unwrap_or(0);
        context.remote_page = remote_page.unwrap_or(0);
        context.network_client_page = network_client_page.unwrap_or(0);
        context.network_server_page = network_server_page.unwrap_or(0);
        context.network_device_page = network_device_page.unwrap_or(0);
        context.network_event_page = network_event_page.unwrap_or(0);
        context.network_stream_page = network_stream_page.unwrap_or(0);
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
        // Keep typed endpoint addresses and network page configuration across reset.
        unsafe { (address as *mut Self).write_volatile(context) };
        true
    }

    /// # Safety
    /// `address` must point to a live, aligned `ControlPage` mapping owned by Core.
    pub unsafe fn notify_at(address: u64, operation: u32) -> bool {
        if !matches!(
            operation,
            STORE_REQUEST | STORE_REPLY | BLOCK_REQUEST | BLOCK_REPLY | NETWORK_REQUEST
        ) {
            return false;
        }
        let mut context = unsafe { (address as *mut Self).read_volatile() };
        if context.abi != ABI || context.reserved != 0 || context.status != ACKNOWLEDGED {
            return false;
        }
        context.operation = operation;
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

    pub unsafe fn store_client_page_at(address: u64) -> Option<u64> {
        let context = unsafe { (address as *const Self).read_volatile() };
        (context.abi == ABI && context.reserved == 0 && context.store_client_page != 0)
            .then_some(context.store_client_page)
    }

    pub unsafe fn store_server_page_at(address: u64) -> Option<u64> {
        let context = unsafe { (address as *const Self).read_volatile() };
        (context.abi == ABI && context.reserved == 0 && context.store_server_page != 0)
            .then_some(context.store_server_page)
    }

    pub unsafe fn block_client_page_at(address: u64) -> Option<u64> {
        let context = unsafe { (address as *const Self).read_volatile() };
        (context.abi == ABI && context.reserved == 0 && context.block_client_page != 0)
            .then_some(context.block_client_page)
    }

    pub unsafe fn network_client_page_at(address: u64) -> Option<u64> {
        let context = unsafe { (address as *const Self).read_volatile() };
        (context.abi == ABI && context.reserved == 0 && context.network_client_page != 0)
            .then_some(context.network_client_page)
    }

    pub unsafe fn network_server_page_at(address: u64) -> Option<u64> {
        let context = unsafe { (address as *const Self).read_volatile() };
        (context.abi == ABI && context.reserved == 0 && context.network_server_page != 0)
            .then_some(context.network_server_page)
    }

    pub unsafe fn network_stream_page_at(address: u64) -> Option<u64> {
        let context = unsafe { (address as *const Self).read_volatile() };
        (context.abi == ABI && context.reserved == 0 && context.network_stream_page != 0)
            .then_some(context.network_stream_page)
    }

    pub unsafe fn remote_pending_at(address: u64) -> bool {
        let context = unsafe { (address as *const Self).read_volatile() };
        context.abi == ABI
            && context.reserved == 0
            && context.remote_page != 0
            && context.generation != 0
            && unsafe {
                RemotePage::pending_at(context.remote_page, context.generation, context.generation)
            }
    }

    pub unsafe fn network_client_pending_at(address: u64) -> bool {
        let context = unsafe { (address as *const Self).read_volatile() };
        context.abi == ABI
            && context.reserved == 0
            && context.operation == NETWORK_REQUEST
            && context.status == ACKNOWLEDGED
            && context.network_client_page != 0
            && unsafe {
                NetworkClientPage::pending_at(
                    context.network_client_page,
                    context.generation,
                    context.generation,
                )
            }
    }

    pub unsafe fn network_server_pending_at(address: u64) -> bool {
        let context = unsafe { (address as *const Self).read_volatile() };
        context.abi == ABI
            && context.reserved == 0
            && context.operation == NETWORK_REQUEST
            && context.status == ACKNOWLEDGED
            && context.network_server_page != 0
            && unsafe {
                NetworkServerPage::pending_at(
                    context.network_server_page,
                    context.generation,
                    context.generation,
                )
            }
    }

    pub unsafe fn store_client_pending_at(address: u64) -> bool {
        let context = unsafe { (address as *const Self).read_volatile() };
        context.abi == ABI
            && context.reserved == 0
            && context.operation == STORE_REQUEST
            && context.status == ACKNOWLEDGED
            && context.store_client_page != 0
            && unsafe {
                StoreClientPage::pending_at(
                    context.store_client_page,
                    context.generation,
                    context.generation,
                )
            }
    }

    pub unsafe fn block_client_pending_at(address: u64) -> bool {
        let context = unsafe { (address as *const Self).read_volatile() };
        context.abi == ABI
            && context.reserved == 0
            && context.operation == BLOCK_REQUEST
            && context.status == ACKNOWLEDGED
            && context.block_client_page != 0
            && unsafe {
                BlockClientPage::pending_at(
                    context.block_client_page,
                    context.generation,
                    context.generation,
                )
            }
    }

    pub unsafe fn network_server_reply_pending_at(address: u64) -> bool {
        let context = unsafe { (address as *const Self).read_volatile() };
        context.abi == ABI
            && context.reserved == 0
            && context.operation == NETWORK_REPLY
            && context.status == ACKNOWLEDGED
            && context.network_server_page != 0
            && unsafe {
                NetworkServerPage::reply_pending_at(
                    context.network_server_page,
                    context.generation,
                    context.generation,
                )
            }
    }

    pub unsafe fn store_client_reply_pending_at(address: u64) -> bool {
        let context = unsafe { (address as *const Self).read_volatile() };
        context.abi == ABI
            && context.reserved == 0
            && context.operation == STORE_REPLY
            && context.status == ACKNOWLEDGED
            && context.store_client_page != 0
            && unsafe {
                StoreClientPage::reply_pending_at(
                    context.store_client_page,
                    context.generation,
                    context.generation,
                )
            }
    }

    pub unsafe fn store_server_reply_pending_at(address: u64) -> bool {
        let context = unsafe { (address as *const Self).read_volatile() };
        context.abi == ABI
            && context.reserved == 0
            && context.operation == STORE_REPLY
            && context.status == ACKNOWLEDGED
            && context.store_server_page != 0
            && unsafe {
                StoreServerPage::reply_pending_at(
                    context.store_server_page,
                    context.generation,
                    context.generation,
                )
            }
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

    pub unsafe fn network_device_pending_at(address: u64) -> bool {
        let context = unsafe { (address as *const Self).read_volatile() };
        context.abi == ABI
            && context.reserved == 0
            && context.operation == NETWORK_DEVICE_REQUEST
            && context.status == ACKNOWLEDGED
    }

    pub unsafe fn network_event_pending_at(address: u64) -> bool {
        let context = unsafe { (address as *const Self).read_volatile() };
        context.abi == ABI
            && context.reserved == 0
            && context.status == ACKNOWLEDGED
            && context.operation == NETWORK_WAIT
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
    use logos_abi::{NetworkEndpoint, NetworkProtocol, NetworkScope, PageHandle};

    fn bind_request(id: u32) -> logos_abi::NetworkRequest {
        logos_abi::NetworkRequest {
            id,
            operation: logos_abi::NetworkOperation::Bind,
            endpoint: NetworkEndpoint(0),
            peer: NetworkScope::new(NetworkProtocol::Udp, 0, 4000),
            page: PageHandle(0),
            length: 0,
            generation: 0,
            deadline: 100,
        }
    }

    fn send_request(id: u32, page: PageHandle) -> logos_abi::NetworkRequest {
        logos_abi::NetworkRequest {
            id,
            operation: logos_abi::NetworkOperation::SendTo,
            endpoint: NetworkEndpoint::new(1, 1).unwrap(),
            peer: NetworkScope::new(NetworkProtocol::Udp, 0x0a00_0202, 4001),
            page,
            length: 4,
            generation: 1,
            deadline: 100,
        }
    }

    fn error_reply(
        request: logos_abi::NetworkRequest,
        status: logos_abi::NetworkStatus,
    ) -> logos_abi::NetworkReply {
        logos_abi::NetworkReply {
            id: request.id,
            status,
            endpoint: NetworkEndpoint(0),
            generation: 0,
            source_address: 0,
            source_port: 0,
            length: 0,
            stream_readiness: 0,
            stream_reserved: 0,
            stream_accepted_bytes: 0,
            stream_acknowledged_bytes: 0,
            info: logos_abi::NetworkInfo::default(),
            counters: logos_abi::NetworkCounters::default(),
        }
    }

    fn bind_reply(request: logos_abi::NetworkRequest) -> logos_abi::NetworkReply {
        logos_abi::NetworkReply {
            id: request.id,
            status: logos_abi::NetworkStatus::Complete,
            endpoint: NetworkEndpoint::new(1, 1).unwrap(),
            generation: 1,
            source_address: 0,
            source_port: 0,
            length: 0,
            stream_readiness: 0,
            stream_reserved: 0,
            stream_accepted_bytes: 0,
            stream_acknowledged_bytes: 0,
            info: logos_abi::NetworkInfo::default(),
            counters: logos_abi::NetworkCounters::default(),
        }
    }

    #[test]
    fn abi_self_check_covers_header_and_control_reset() {
        assert!(self_check());
    }

    #[test]
    fn typed_store_pages_round_trip_and_reject_stale_state() {
        let request = logos_abi::StoreRequest {
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
        let mut client = StoreClientPage::new(2, 5);
        let client_address = (&mut client as *mut StoreClientPage) as u64;
        assert!(unsafe { StoreClientPage::request_at(client_address, 2, 5, request) });
        assert!(unsafe { StoreClientPage::current_request_at(client_address, 1, 5) }.is_none());
        assert_eq!(
            unsafe { StoreClientPage::current_request_at(client_address, 2, 5) }
                .map(|request| (request.id, request.operation)),
            Some((request.id, request.operation))
        );
        let reply = logos_abi::StoreReply {
            id: 7,
            status: logos_abi::PersistenceStatus::Complete,
            version: 3,
            length: 0,
        };
        assert!(!unsafe {
            StoreClientPage::reply_at(
                client_address,
                2,
                5,
                logos_abi::StoreReply { id: 8, ..reply },
            )
        });
        assert!(unsafe { StoreClientPage::reply_at(client_address, 2, 5, reply) });
        assert!(unsafe { StoreClientPage::finish_at(client_address, 2, 4, 7) }.is_none());
        assert_eq!(unsafe { StoreClientPage::finish_at(client_address, 2, 5, 7) }, Some(reply));

        let mut server = StoreServerPage::new(4, 9);
        let server_address = (&mut server as *mut StoreServerPage) as u64;
        assert!(unsafe { StoreServerPage::wait_at(server_address, 4, 9) });
        assert!(unsafe { StoreServerPage::deliver_at(server_address, 4, 9, 0x1234, request) });
        let delivered = unsafe { StoreServerPage::take_at(server_address, 4, 9) }.unwrap();
        assert_eq!(delivered.caller, 0x1234);
        assert_eq!(delivered.request.id, request.id);
        assert_eq!(delivered.request.operation, request.operation);
        assert!(!unsafe { StoreServerPage::reply_at(server_address, 4, 8, reply) });
        assert!(unsafe { StoreServerPage::reply_at(server_address, 4, 9, reply) });
        assert!(unsafe { StoreServerPage::take_reply_at(server_address, 4, 8, 7) }.is_none());
        assert_eq!(unsafe { StoreServerPage::take_reply_at(server_address, 4, 9, 7) }, Some(reply));
    }

    #[test]
    fn typed_block_page_round_trip_and_rejects_malformed_state() {
        let request = logos_abi::BlockRequest {
            id: 9,
            operation: logos_abi::BlockOperation::Flush,
            lba: 0,
            blocks: 0,
            page: logos_abi::PageHandle(0),
            deadline: 1,
        };
        let mut page = BlockClientPage::new(3, 7);
        let address = (&mut page as *mut BlockClientPage) as u64;
        assert!(unsafe { BlockClientPage::request_at(address, 3, 7, request) });
        assert!(unsafe { BlockClientPage::take_at(address, 2, 7) }.is_none());
        assert_eq!(unsafe { BlockClientPage::take_at(address, 3, 7) }, Some(request));
        let reply = logos_abi::BlockReply {
            id: request.id,
            status: logos_abi::PersistenceStatus::Complete,
            info: logos_abi::BlockInfo::default(),
        };
        assert!(unsafe { BlockClientPage::reply_at(address, 3, 7, reply) });
        assert_eq!(unsafe { BlockClientPage::finish_at(address, 3, 7, request.id) }, Some(reply));
        page.state = u32::MAX;
        unsafe { (address as *mut BlockClientPage).write_volatile(page) };
        assert!(unsafe { BlockClientPage::take_at(address, 3, 7) }.is_none());

        let mut context = ControlPage::new();
        context.status = ACKNOWLEDGED;
        let context_address = (&mut context as *mut ControlPage) as u64;
        assert!(unsafe { ControlPage::notify_at(context_address, BLOCK_REQUEST) });
        assert_eq!(context.operation, BLOCK_REQUEST);
    }

    #[test]
    fn remote_page_is_scalar_generation_safe_and_replay_bound() {
        let mut page = RemotePage::new(4, 9);
        let address = (&mut page as *mut RemotePage) as u64;
        let request = RemotePageRequest {
            id: 7,
            operation: RemoteGateOperation::Invoke,
            page: logos_abi::PageHandle(3),
            length: 12,
            deadline: 99,
        };
        assert!(unsafe { RemotePage::request_at(address, 4, 9, request) });
        assert!(unsafe { RemotePage::take_at(address, 4, 8) }.is_none());
        assert_eq!(unsafe { RemotePage::take_at(address, 4, 9) }, Some(request));
        let reply = RemotePageReply {
            id: request.id,
            status: RemoteGateStatus::Complete,
            length: 5,
            cursor: 11,
        };
        assert!(!unsafe {
            RemotePage::reply_at(address, 4, 9, RemotePageReply { id: 8, ..reply })
        });
        assert!(unsafe { RemotePage::reply_at(address, 4, 9, reply) });
        assert!(unsafe { RemotePage::finish_at(address, 4, 8, request.id) }.is_none());
        assert_eq!(unsafe { RemotePage::finish_at(address, 4, 9, request.id) }, Some(reply));
        page.state = u32::MAX;
        unsafe { (address as *mut RemotePage).write_volatile(page) };
        assert!(unsafe { RemotePage::take_at(address, 4, 9) }.is_none());
    }

    #[test]
    fn network_client_server_pages_associate_replies() {
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
        let mut client = NetworkClientPage::new(1, 1);
        let mut server = NetworkServerPage::new(1, 1);
        let client_address = (&mut client as *mut NetworkClientPage) as u64;
        let server_address = (&mut server as *mut NetworkServerPage) as u64;
        assert!(unsafe { NetworkClientPage::request_at(client_address, 1, 1, request) });
        assert!(unsafe { NetworkClientPage::mark_processing_at(client_address, 1, 1) });
        assert!(unsafe {
            NetworkServerPage::deliver_at(server_address, 1, 1, 0x1234_5678, request)
        });
        let message = unsafe { NetworkServerPage::take_at(server_address, 1, 1) }.unwrap();
        assert_eq!(message.caller, 0x1234_5678);
        let reply = logos_abi::NetworkReply {
            id: request.id,
            status: logos_abi::NetworkStatus::Complete,
            endpoint: logos_abi::NetworkEndpoint::new(1, 1).unwrap(),
            generation: 1,
            source_address: 0,
            source_port: 0,
            length: 0,
            stream_readiness: logos_abi::NetworkStreamReadiness::Writable.bits(),
            stream_reserved: 0,
            stream_accepted_bytes: 5,
            stream_acknowledged_bytes: 3,
            info: logos_abi::NetworkInfo::default(),
            counters: logos_abi::NetworkCounters::default(),
        };
        assert!(unsafe { NetworkServerPage::reply_at(server_address, 1, 1, reply) });
        assert_eq!(
            unsafe { NetworkServerPage::finish_at(server_address, 1, 1, request.id) },
            Some(reply)
        );
    }

    #[test]
    fn network_client_transfer_page_is_generation_bound() {
        let mut page = NetworkClientPage::new(1, 2);
        let address = (&mut page as *mut NetworkClientPage) as u64;
        assert!(!unsafe { NetworkClientPage::configure_transfer_at(address, 1, 2, PageHandle(0)) });
        assert!(!unsafe { NetworkClientPage::configure_transfer_at(address, 2, 2, PageHandle(3)) });
        assert!(unsafe { NetworkClientPage::configure_transfer_at(address, 1, 2, PageHandle(3)) });
        assert_eq!(
            unsafe { NetworkClientPage::transfer_page_at(address, 1, 2) },
            Some(PageHandle(3))
        );
        assert_eq!(unsafe { NetworkClientPage::transfer_page_at(address, 1, 3) }, None);
    }

    #[test]
    fn stream_page_coalesces_and_reports_bounded_loss() {
        let mut page = StreamPage::new(3, 7);
        let address = (&mut page as *mut StreamPage) as u64;
        let endpoint = logos_abi::NetworkEndpoint::new(1, 7).unwrap();
        let record = |owner, endpoint, accepted| logos_abi::NetworkStreamRecord {
            owner,
            endpoint,
            generation: 7,
            readiness: logos_abi::NetworkStreamReadiness::Writable.bits(),
            status: logos_abi::NetworkStatus::Complete,
            reserved: 0,
            sequence: 0,
            accepted_bytes: accepted,
            acknowledged_bytes: accepted / 2,
        };
        assert!(unsafe { StreamPage::publish_at(address, 3, 7, record(11, endpoint, 3)) });
        assert!(unsafe { StreamPage::publish_at(address, 3, 7, record(11, endpoint, 6)) });
        assert_eq!(unsafe { StreamPage::take_next_at(address, 3, 7) }.unwrap().accepted_bytes, 6);
        for slot in 1..=logos_abi::NETWORK_MAX_STREAM_RECORDS as u16 {
            assert!(unsafe {
                StreamPage::publish_at(
                    address,
                    3,
                    7,
                    record(u64::from(slot), logos_abi::NetworkEndpoint::new(slot, 7).unwrap(), 1),
                )
            });
        }
        assert!(!unsafe {
            StreamPage::publish_at(
                address,
                3,
                7,
                record(99, logos_abi::NetworkEndpoint::new(99, 7).unwrap(), 1),
            )
        });
        assert!(unsafe { StreamPage::overflow_at(address, 3, 7) });
        assert!(unsafe { StreamPage::clear_overflow_at(address, 3, 7) });
        assert!(!unsafe { StreamPage::overflow_at(address, 3, 7) });
    }

    #[test]
    fn network_client_rejects_data_request_on_wrong_transfer_page() {
        let request = send_request(1, PageHandle(4));
        let mut page = NetworkClientPage::new(1, 2);
        let address = (&mut page as *mut NetworkClientPage) as u64;
        assert!(!unsafe { NetworkClientPage::request_at(address, 1, 2, request) });
        assert!(unsafe { NetworkClientPage::configure_transfer_at(address, 1, 2, PageHandle(3)) });
        assert!(!unsafe { NetworkClientPage::request_at(address, 1, 2, request) });
        assert!(unsafe {
            NetworkClientPage::request_at(address, 1, 2, send_request(request.id, PageHandle(3)))
        });
    }

    #[test]
    fn network_client_rejects_oversized_data_request() {
        let mut page = NetworkClientPage::new(1, 2);
        let address = (&mut page as *mut NetworkClientPage) as u64;
        assert!(unsafe { NetworkClientPage::configure_transfer_at(address, 1, 2, PageHandle(3)) });
        let request = logos_abi::NetworkRequest {
            length: (logos_abi::MAX_NETWORK_PAYLOAD + 1) as u16,
            ..send_request(1, PageHandle(3))
        };
        assert!(!unsafe { NetworkClientPage::request_at(address, 1, 2, request) });
    }

    #[test]
    fn network_client_rejects_duplicate_request_while_processing() {
        let request = bind_request(1);
        let mut page = NetworkClientPage::new(1, 2);
        let address = (&mut page as *mut NetworkClientPage) as u64;
        assert!(unsafe { NetworkClientPage::request_at(address, 1, 2, request) });
        assert!(unsafe { NetworkClientPage::mark_processing_at(address, 1, 2) });
        assert!(!unsafe { NetworkClientPage::request_at(address, 1, 2, bind_request(2)) });
    }

    #[test]
    fn network_client_can_rollback_request_before_processing() {
        let request = bind_request(1);
        let mut page = NetworkClientPage::new(1, 2);
        let address = (&mut page as *mut NetworkClientPage) as u64;
        let reply = error_reply(request, logos_abi::NetworkStatus::TimedOut);
        assert!(unsafe { NetworkClientPage::request_at(address, 1, 2, request) });
        assert!(unsafe { NetworkClientPage::reply_request_at(address, 1, 2, reply) });
        assert_eq!(unsafe { NetworkClientPage::finish_at(address, 1, 2, request.id) }, Some(reply));
    }

    #[test]
    fn network_client_reply_requires_exact_request_id() {
        let request = bind_request(1);
        let mut page = NetworkClientPage::new(1, 2);
        let address = (&mut page as *mut NetworkClientPage) as u64;
        assert!(unsafe { NetworkClientPage::request_at(address, 1, 2, request) });
        assert!(unsafe { NetworkClientPage::mark_processing_at(address, 1, 2) });
        assert!(!unsafe {
            NetworkClientPage::reply_at(address, 1, 2, bind_reply(bind_request(2)))
        });
        assert!(unsafe { NetworkClientPage::reply_at(address, 1, 2, bind_reply(request)) });
    }

    #[test]
    fn network_client_reply_requires_exact_page_identity() {
        let request = bind_request(1);
        let mut page = NetworkClientPage::new(1, 2);
        let address = (&mut page as *mut NetworkClientPage) as u64;
        assert!(unsafe { NetworkClientPage::request_at(address, 1, 2, request) });
        assert!(unsafe { NetworkClientPage::mark_processing_at(address, 1, 2) });
        assert!(!unsafe { NetworkClientPage::reply_at(address, 1, 3, bind_reply(request)) });
        assert!(unsafe { NetworkClientPage::reply_at(address, 1, 2, bind_reply(request)) });
    }

    #[test]
    fn network_client_finish_requires_exact_identity() {
        let request = bind_request(1);
        let mut page = NetworkClientPage::new(1, 2);
        let address = (&mut page as *mut NetworkClientPage) as u64;
        assert!(unsafe { NetworkClientPage::request_at(address, 1, 2, request) });
        assert!(unsafe { NetworkClientPage::mark_processing_at(address, 1, 2) });
        let reply = bind_reply(request);
        assert!(unsafe { NetworkClientPage::reply_at(address, 1, 2, reply) });
        assert!(unsafe { NetworkClientPage::finish_at(address, 2, 2, request.id) }.is_none());
        assert!(unsafe { NetworkClientPage::finish_at(address, 1, 3, request.id) }.is_none());
        assert_eq!(unsafe { NetworkClientPage::finish_at(address, 1, 2, request.id) }, Some(reply));
    }

    #[test]
    fn network_client_finish_requires_exact_request_id() {
        let request = bind_request(1);
        let mut page = NetworkClientPage::new(1, 2);
        let address = (&mut page as *mut NetworkClientPage) as u64;
        assert!(unsafe { NetworkClientPage::request_at(address, 1, 2, request) });
        assert!(unsafe { NetworkClientPage::mark_processing_at(address, 1, 2) });
        let reply = bind_reply(request);
        assert!(unsafe { NetworkClientPage::reply_at(address, 1, 2, reply) });
        assert!(unsafe { NetworkClientPage::finish_at(address, 1, 2, 2) }.is_none());
        assert_eq!(unsafe { NetworkClientPage::finish_at(address, 1, 2, request.id) }, Some(reply));
    }

    #[test]
    fn network_client_finish_preserves_configured_transfer_page() {
        let request = send_request(1, PageHandle(3));
        let mut page = NetworkClientPage::new(1, 2);
        let address = (&mut page as *mut NetworkClientPage) as u64;
        assert!(unsafe { NetworkClientPage::configure_transfer_at(address, 1, 2, PageHandle(3)) });
        assert!(unsafe { NetworkClientPage::request_at(address, 1, 2, request) });
        assert!(unsafe { NetworkClientPage::mark_processing_at(address, 1, 2) });
        let reply = error_reply(request, logos_abi::NetworkStatus::Cancelled);
        assert!(unsafe { NetworkClientPage::reply_at(address, 1, 2, reply) });
        assert_eq!(unsafe { NetworkClientPage::finish_at(address, 1, 2, request.id) }, Some(reply));
        assert_eq!(
            unsafe { NetworkClientPage::transfer_page_at(address, 1, 2) },
            Some(PageHandle(3))
        );
    }

    #[test]
    fn network_server_accepts_only_one_request_until_reset() {
        let mut page = NetworkServerPage::new(1, 2);
        let address = (&mut page as *mut NetworkServerPage) as u64;
        assert!(unsafe { NetworkServerPage::deliver_at(address, 1, 2, 7, bind_request(1)) });
        assert!(!unsafe { NetworkServerPage::deliver_at(address, 1, 2, 7, bind_request(2)) });
        assert!(unsafe { NetworkServerPage::take_at(address, 1, 2) }.is_some());
        assert!(!unsafe { NetworkServerPage::deliver_at(address, 1, 2, 7, bind_request(2)) });
        assert!(unsafe { NetworkServerPage::reset_at(address, 1, 2) });
        assert!(unsafe { NetworkServerPage::deliver_at(address, 1, 2, 7, bind_request(2)) });
    }

    #[test]
    fn network_server_preserves_caller_identity() {
        let request = bind_request(1);
        let mut page = NetworkServerPage::new(1, 2);
        let address = (&mut page as *mut NetworkServerPage) as u64;
        assert!(unsafe {
            NetworkServerPage::deliver_at(address, 1, 2, 0x1234_5678_9abc_def0, request)
        });
        let message = unsafe { NetworkServerPage::take_at(address, 1, 2) }.unwrap();
        assert_eq!(message.caller, 0x1234_5678_9abc_def0);
        assert_eq!(message.request, request);
    }

    #[test]
    fn network_server_reply_rejects_invalid_reply_identity() {
        let request = bind_request(1);
        let mut page = NetworkServerPage::new(1, 2);
        let address = (&mut page as *mut NetworkServerPage) as u64;
        assert!(unsafe { NetworkServerPage::deliver_at(address, 1, 2, 7, request) });
        assert!(unsafe { NetworkServerPage::take_at(address, 1, 2) }.is_some());
        assert!(!unsafe {
            NetworkServerPage::reply_at(address, 1, 2, bind_reply(bind_request(2)))
        });
        assert!(unsafe { NetworkServerPage::reply_at(address, 1, 2, bind_reply(request)) });
    }

    #[test]
    fn network_server_finish_requires_exact_request_id() {
        let request = bind_request(1);
        let mut page = NetworkServerPage::new(1, 2);
        let address = (&mut page as *mut NetworkServerPage) as u64;
        assert!(unsafe { NetworkServerPage::deliver_at(address, 1, 2, 7, request) });
        assert!(unsafe { NetworkServerPage::take_at(address, 1, 2) }.is_some());
        let reply = bind_reply(request);
        assert!(unsafe { NetworkServerPage::reply_at(address, 1, 2, reply) });
        assert!(unsafe { NetworkServerPage::finish_at(address, 1, 2, 2) }.is_none());
        assert_eq!(unsafe { NetworkServerPage::finish_at(address, 1, 2, request.id) }, Some(reply));
    }

    #[test]
    fn network_server_reset_clears_previous_transaction() {
        let mut page = NetworkServerPage::new(1, 2);
        let address = (&mut page as *mut NetworkServerPage) as u64;
        assert!(unsafe { NetworkServerPage::deliver_at(address, 1, 2, 7, bind_request(1)) });
        assert!(unsafe { NetworkServerPage::reset_at(address, 1, 2) });
        assert!(unsafe { NetworkServerPage::deliver_at(address, 1, 2, 7, bind_request(2)) });
        assert_eq!(unsafe { NetworkServerPage::take_at(address, 1, 2) }.unwrap().id, 2);
    }

    #[test]
    fn network_client_cancel_and_timeout_replies_are_typed() {
        let mut page = NetworkClientPage::new(1, 2);
        let address = (&mut page as *mut NetworkClientPage) as u64;
        for (id, status) in
            [(1, logos_abi::NetworkStatus::Cancelled), (2, logos_abi::NetworkStatus::TimedOut)]
        {
            let request = bind_request(id);
            let reply = error_reply(request, status);
            assert!(unsafe { NetworkClientPage::request_at(address, 1, 2, request) });
            assert!(unsafe { NetworkClientPage::reply_request_at(address, 1, 2, reply) });
            assert_eq!(unsafe { NetworkClientPage::finish_at(address, 1, 2, id) }, Some(reply));
        }
    }

    #[test]
    fn network_device_and_event_pages_reject_stale_and_unconsumed_transitions() {
        let mut device = NetworkDevicePage::new(1, 2, 3);
        let device_address = (&mut device as *mut NetworkDevicePage) as u64;
        let rx = logos_abi::PageHandle(10);
        let tx = logos_abi::PageHandle(11);
        assert!(unsafe { NetworkDevicePage::configure_at(device_address, 1, 2, 3, rx, tx) });
        let request = logos_abi::NetworkDeviceRequest {
            id: 9,
            operation: logos_abi::NetworkDeviceOperation::Info,
            length: 0,
            generation: 0,
            deadline: 1,
        };
        assert!(unsafe { NetworkDevicePage::request_at(device_address, 1, 2, 3, request) });
        let message = unsafe { NetworkDevicePage::take_request_at(device_address, 1, 2, 3) };
        assert_eq!(message.map(|message| message.request), Some(request));
        let reply = logos_abi::NetworkDeviceReply {
            id: request.id,
            status: logos_abi::NetworkStatus::Complete,
            generation: 3,
            info: logos_abi::NetworkInfo { generation: 3, ..Default::default() },
        };
        assert!(!unsafe { NetworkDevicePage::complete_at(device_address, 1, 2, 4, reply) });
        assert!(unsafe { NetworkDevicePage::complete_at(device_address, 1, 2, 3, reply) });
        assert_eq!(
            unsafe { NetworkDevicePage::take_reply_at(device_address, 1, 2, 3, request.id) },
            Some(reply)
        );
        assert!(unsafe { NetworkDevicePage::reset_generation_at(device_address, 4, 5) });

        let mut event_page = NetworkEventPage::new(1, 2, 3);
        let event_address = (&mut event_page as *mut NetworkEventPage) as u64;
        assert!(unsafe { NetworkEventPage::configure_at(event_address, 1, 2, 3, rx) });
        assert!(unsafe { NetworkEventPage::wait_at(event_address, 1, 2, 3, 7) });
        let event = logos_abi::NetworkEvent {
            id: 12,
            kind: logos_abi::NetworkEventKind::Frame,
            generation: 3,
            device_generation: 3,
            page: rx,
            length: 64,
            now: 7,
            metadata: [0; 16],
        };
        assert!(unsafe { NetworkEventPage::deliver_at(event_address, 1, 2, 3, event) });
        assert!(!unsafe { NetworkEventPage::deliver_at(event_address, 1, 2, 3, event) });
        assert_eq!(unsafe { NetworkEventPage::take_at(event_address, 1, 2, 3) }, Some(event));
        assert!(unsafe { NetworkEventPage::acknowledge_at(event_address, 1, 2, 3) });
        assert!(!unsafe { NetworkEventPage::deliver_at(event_address, 1, 2, 4, event) });
        let mut fresh_event_page = NetworkEventPage::new(1, 2, 3);
        let fresh_event_address = (&mut fresh_event_page as *mut NetworkEventPage) as u64;
        assert!(unsafe { NetworkEventPage::reset_generation_at(fresh_event_address, 4, 5) });
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
        assert!(unsafe { InputPage::take_at(address, 7) }.is_none());
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
