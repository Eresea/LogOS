use crate as logos_abi;
use core::mem::{align_of, size_of};

pub mod block;
pub mod display;
pub mod input;
pub mod network;
pub mod remote;
pub mod session;
pub mod storage;
pub use block::*;
pub use display::*;
pub use input::*;
pub use network::*;
pub use remote::{
    RemoteGateOperation, RemoteGateStatus, RemotePage, RemotePageReply, RemotePageRequest,
    RemotePageState,
};
pub use session::*;
pub use storage::*;

pub const MAGIC: [u8; 4] = *b"LGSV";
pub const ABI: u16 = 5;
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

/// ABI-v5 operation phases shared by every bounded cross-boundary task.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u16)]
pub enum OperationPhase {
    Accepted = 1,
    Pending = 2,
    Blocked = 3,
    Completing = 4,
    Complete = 5,
    Failed = 6,
    TimedOut = 7,
    Cancelled = 8,
}

impl OperationPhase {
    pub const fn from_wire(value: u16) -> Option<Self> {
        Some(match value {
            1 => Self::Accepted,
            2 => Self::Pending,
            3 => Self::Blocked,
            4 => Self::Completing,
            5 => Self::Complete,
            6 => Self::Failed,
            7 => Self::TimedOut,
            8 => Self::Cancelled,
            _ => return None,
        })
    }

    pub const fn terminal(self) -> bool {
        matches!(self, Self::Complete | Self::Failed | Self::TimedOut | Self::Cancelled)
    }
}

/// Owner/generation/request identity carried by ABI-v5 operation pages.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct OperationToken {
    pub owner: u64,
    pub generation: u32,
    pub request_id: u32,
    pub deadline: u64,
    pub sequence: u64,
}

impl OperationToken {
    pub const fn new(
        owner: u64,
        generation: u32,
        request_id: u32,
        deadline: u64,
        sequence: u64,
    ) -> Option<Self> {
        if owner == 0 || generation == 0 || request_id == 0 || deadline == 0 || sequence == 0 {
            None
        } else {
            Some(Self { owner, generation, request_id, deadline, sequence })
        }
    }

    pub const fn matches(self, owner: u64, generation: u32, request_id: u32) -> bool {
        self.owner == owner && self.generation == generation && self.request_id == request_id
    }

    pub const fn expired(self, tick: u64) -> bool {
        tick >= self.deadline
    }
}

/// Fixed-size completion envelope; notifications never replace this state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct CompletionEnvelope {
    pub token: OperationToken,
    pub phase: OperationPhase,
    pub status: u32,
}

/// Core-owned control page shared by one native service.
///
/// ABI v5 keeps the control header compact and puts service-specific request
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
        if context.abi != ABI || context.reserved != 0 {
            return false;
        }
        if context.status != ACKNOWLEDGED {
            return context.operation == operation;
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
    pub const V2: Self = Self { major: 2, minor: 0 };

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
    Header::new(*b"terminal\0\0\0\0\0\0\0\0", ProtocolVersion::V2, self_check_entry)
        .valid_for(b"terminal", ProtocolVersion::V2)
        && !Header::new(*b"terminal\0\0\0\0\0\0\0\0", ProtocolVersion::V1, self_check_entry)
            .valid_for(b"terminal", ProtocolVersion::V2)
        && !Header::new(*b"terminal\0\0\0\0\0\0\0\0", ProtocolVersion::V2, self_check_entry)
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

    #[test]
    fn v5_operation_tokens_are_bounded_and_terminal() {
        assert!(OperationToken::new(0, 1, 1, 1, 1).is_none());
        let token = OperationToken::new(7, 3, 9, 10, 2).unwrap();
        assert!(token.matches(7, 3, 9));
        assert!(!token.matches(7, 4, 9));
        assert!(!token.expired(9));
        assert!(token.expired(10));
        assert!(OperationPhase::Complete.terminal());
        assert!(!OperationPhase::Pending.terminal());
        assert_eq!(OperationPhase::from_wire(8), Some(OperationPhase::Cancelled));
        assert_eq!(OperationPhase::from_wire(0), None);
    }

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
        context.status = 0;
        unsafe { (context_address as *mut ControlPage).write_volatile(context) };
        assert!(unsafe { ControlPage::notify_at(context_address, BLOCK_REQUEST) });
        assert!(!unsafe { ControlPage::notify_at(context_address, NETWORK_REQUEST) });
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
