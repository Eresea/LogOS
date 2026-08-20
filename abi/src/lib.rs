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

mod device_api;
mod package_ipc;
mod service_manager;
mod storage_api;
mod user_api;

pub use device_api::{
    DEVICE_ABI_VERSION, DeviceKind, DeviceOperation, DeviceRecord, DeviceRequest, DeviceResponse,
    DeviceState, DeviceStatus, MAX_DEVICES,
};

pub use package_ipc::{
    MAX_PACKAGE_NAME_BYTES, PACKAGE_ABI_VERSION, PACKAGE_TRANSFER_BYTES, PackageOperation,
    PackageRequest, PackageResponse, PackageStatus, PackageTarget, PackageTargetKind,
};
pub use service_manager::{
    MANAGER_ABI_VERSION, ManagerCapability, ManagerCapabilityPage, ManagerOperation,
    ManagerRequest, ManagerResponse, ManagerRights, ManagerState, ManagerStatus, ManagerTargetKind,
    ServiceManagerRecord,
};
pub use storage_api::{
    STORAGE_API_EXTENSION_VERSION, STORAGE_API_FLAG_REPLACE, STORAGE_API_MAP_DESCRIPTOR_BYTES,
    STORAGE_API_MAP_LENGTH_BYTES, STORAGE_API_RESPONSE_DATA_BYTES, STORAGE_API_VERSION,
    StorageApiError, StorageApiOperation, StorageApiRequest, StorageApiResponse, StorageApiStatus,
};
pub use user_api::{
    CapabilityHandle, NamespaceCapability, NamespaceRights, NamespaceRoot, RoleId, SessionHandle,
    USER_ABI_VERSION, USER_ARGON2_OUTPUT_BYTES, USER_ARGON2_SALT_BYTES, USER_KDF_WORKSPACE_BASE,
    USER_KDF_WORKSPACE_BYTES, USER_KDF_WORKSPACE_PAGES, USER_MAX_PASSWORD_BYTES,
    USER_MAX_ROLE_NAME_BYTES, USER_MAX_USER_NAME_BYTES, USER_STORAGE_CHUNK_BYTES,
    USER_STORAGE_FLAG_BEGIN, USER_STORAGE_FLAG_END, UserAdminCapability, UserId, UserOperation,
    UserRequest, UserResponse, UserStatus, UserStorageOperation, UserStorageRequest,
    UserStorageResponse, UserStorageStatus,
};

pub const ABI_VERSION: u16 = 4;
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
pub const CELL_ATTR_BOLD: u16 = 1;
pub const CELL_ATTR_DIM: u16 = 1 << 1;
pub const CELL_ATTR_UNDERLINE: u16 = 1 << 2;
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
pub const MAX_DEVICE_NAME_BYTES: usize = 16;
pub const COMPLETION_ABI_VERSION: u8 = 1;
pub const MAX_COMPLETION_LINE_BYTES: usize = 248;
pub const MAX_COMPLETION_CANDIDATES: usize = 8;
pub const MAX_COMPLETION_ITEM_BYTES: usize = 24;
pub const IPC_FLAG_MORE: u8 = 1 << 0;
pub const RENDER_FLAG_MORE: u8 = IPC_FLAG_MORE;
pub const SERVICE_IPC_BASE: usize = 0x0000_0100_0200_0000;
pub const IPC_STAGING_BASE: usize = SERVICE_IPC_BASE + 0x20_000;
pub const STORAGE_DATA_BASE: usize = SERVICE_IPC_BASE + 0x21_000;
pub const STORAGE_CACHE_PAGES: usize = 32;
pub const STORAGE_CACHE_BASE: usize = STORAGE_DATA_BASE + IPC_PAGE_BYTES;
pub const STORAGE_DATA_PAGES: usize = STORAGE_CACHE_PAGES + 1;
pub const IPC_CAPABILITY_BASE: usize = STORAGE_CACHE_BASE + STORAGE_CACHE_PAGES * IPC_PAGE_BYTES;
pub const MANAGER_CAPABILITY_BASE: usize = IPC_CAPABILITY_BASE + IPC_PAGE_BYTES;
pub const MANAGER_CAPABILITY_SLOT: usize = 0;
pub const NETWORK_CONFIG_BASE: usize = SERVICE_IPC_BASE + 0x3a_000;
pub const MAX_IPC_CAPABILITIES: usize = 12;
pub const MAX_MANAGER_SERVICES: usize = 10;
pub const MAX_SERVICE_NAME_BYTES: usize = 16;
pub const SERVICE_HEARTBEAT_INTERVAL_TICKS: u64 = 100;
pub const STORAGE_BLOCK_BYTES: u16 = 4096;
pub const STORAGE_MAX_BLOCKS_PER_REQUEST: u16 = 1;
pub const NETWORK_ABI_VERSION: u16 = 2;
pub const NETWORK_INLINE_PAYLOAD_BYTES: usize = 192;
pub const NETWORK_MAX_SOCKET_SLOTS: usize = 8;
pub const NETWORK_MAX_LISTENER_SLOTS: usize = 2;
pub const NETWORK_MAX_FRAME_BYTES: usize = 1536;
pub const NETWORK_DMA_BUFFER_BYTES: usize = 2048;
pub const NETWORK_QUEUE_DESCRIPTORS: usize = 64;
pub const NETWORK_PACKET_PAGE_COUNT: usize = 32;
pub const NETWORK_PACKET_PAGE_BYTES: usize = NETWORK_DMA_BUFFER_BYTES;
pub const NETWORK_RX_PACKET_PAGES: usize = 16;
pub const NETWORK_TX_PACKET_PAGES: usize = 16;
pub const NETWORK_PACKET_BASE: usize = SERVICE_IPC_BASE + 0x3b_000;
pub const NETWORK_GATEWAY_ARP_DEADLINE_TICKS: u32 = 5_000;
pub const NETWORK_DHCP_DEADLINE_TICKS: u32 = 10_000;
// Keep interactive network probes below the bounded Flow receive budget.
pub const NETWORK_PING_TIMEOUT_TICKS: u32 = 128;
pub const NETWORK_TCP_CONNECT_TIMEOUT_TICKS: u32 = 128;
pub const NETWORK_REQUEST_FLAG_LISTENER: u8 = 1 << 0;

pub const IPC_SYSCALL_SEND: usize = 4;
pub const IPC_SYSCALL_RECEIVE: usize = 5;
pub const MANAGER_SYSCALL: usize = 12;
pub const PROGRAM_EXIT_SYSCALL: usize = 14;
pub const POWER_SYSCALL: usize = 11;
pub const POWER_SHUTDOWN: usize = 1;
pub const POWER_REBOOT: usize = 2;

pub const IPC_ENDPOINT_COUNT: usize = 32;
pub const IPC_READ_EVENT_BASE: usize = 0;
pub const IPC_WRITE_EVENT_BASE: usize = IPC_READ_EVENT_BASE + IPC_ENDPOINT_COUNT;
// Keep the event mask within the u64 syscall contract. Endpoint 31 has no
// distinct write-edge bit; its producer uses bounded retry when that queue is
// full, while all readable edges retain distinct notifications.
pub const KEYBOARD_READ_EVENT: usize = 63;
pub const EVENT_COUNT: usize = KEYBOARD_READ_EVENT + 1;

pub const fn ipc_read_event_mask(endpoint: usize) -> u64 {
    if endpoint < IPC_ENDPOINT_COUNT { 1u64 << (IPC_READ_EVENT_BASE + endpoint) } else { 0 }
}

pub const fn ipc_write_event_mask(endpoint: usize) -> u64 {
    if endpoint >= IPC_ENDPOINT_COUNT {
        0
    } else if endpoint + IPC_WRITE_EVENT_BASE < KEYBOARD_READ_EVENT {
        1u64 << (IPC_WRITE_EVENT_BASE + endpoint)
    } else {
        0
    }
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
    Flow = 5,
    Storage = 6,
    Network = 7,
    Fetch = 8,
    Device = 9,
    User = 10,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum IpcRights {
    Send = 1,
    Receive = 2,
}

impl IpcRights {
    pub const fn allows(self, requested: Self) -> bool {
        matches!((self, requested), (Self::Send, Self::Send) | (Self::Receive, Self::Receive))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct IpcCapability {
    pub endpoint: u8,
    pub rights: IpcRights,
    pub generation: u16,
    pub service_epoch: u64,
}

impl IpcCapability {
    pub const EMPTY: Self =
        Self { endpoint: u8::MAX, rights: IpcRights::Send, generation: 0, service_epoch: 0 };

    pub const fn new(
        endpoint: usize,
        rights: IpcRights,
        generation: u16,
        service_epoch: u64,
    ) -> Option<Self> {
        if endpoint >= IPC_ENDPOINT_COUNT || generation == 0 || service_epoch == 0 {
            return None;
        }
        Some(Self { endpoint: endpoint as u8, rights, generation, service_epoch })
    }

    pub const fn is_empty(self) -> bool {
        self.endpoint == u8::MAX
    }

    pub const fn endpoint_index(self) -> Option<usize> {
        if self.endpoint < IPC_ENDPOINT_COUNT as u8 { Some(self.endpoint as usize) } else { None }
    }
}

#[repr(C, align(16))]
pub struct IpcCapabilityPage {
    pub capabilities: [IpcCapability; MAX_IPC_CAPABILITIES],
}

impl IpcCapabilityPage {
    pub const fn empty() -> Self {
        Self { capabilities: [IpcCapability::EMPTY; MAX_IPC_CAPABILITIES] }
    }

    pub const fn get(&self, index: usize) -> Option<IpcCapability> {
        if index < MAX_IPC_CAPABILITIES {
            let capability = self.capabilities[index];
            if !capability.is_empty() { Some(capability) } else { None }
        } else {
            None
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum IpcStatus {
    Ok = 0,
    Full = 1,
    Empty = 2,
    Stale = 3,
    Disconnected = 4,
    Unauthorized = 5,
    Malformed = 6,
}

impl IpcStatus {
    pub const fn from_raw(raw: usize) -> Option<Self> {
        match raw {
            0 => Some(Self::Ok),
            1 => Some(Self::Full),
            2 => Some(Self::Empty),
            3 => Some(Self::Stale),
            4 => Some(Self::Disconnected),
            5 => Some(Self::Unauthorized),
            6 => Some(Self::Malformed),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum StorageOperation {
    Read = 1,
    Write = 2,
    Flush = 3,
    /// Reserved for the service-owned format command; Core currently rejects it.
    Format = 4,
    Reopen = 5,
    /// Reserved for the service-owned transaction lifecycle.
    BeginTransaction = 6,
    AppendRecord = 7,
    CommitTransaction = 8,
    AbortTransaction = 9,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum StorageStatus {
    Ok = 0,
    Io = 1,
    OutOfBounds = 2,
    ReadOnly = 3,
    Invalid = 4,
    Stale = 5,
    Unauthorized = 6,
    Full = 7,
    Recovery = 8,
    Unsupported = 9,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct StorageRequest {
    pub operation: StorageOperation,
    pub flags: u8,
    pub request_id: u32,
    pub generation: u16,
    pub capability_slot: u16,
    pub service_epoch: u64,
    pub start_block: u64,
    pub blocks: u16,
    pub payload_bytes: u16,
    pub transaction_id: u64,
}

impl StorageRequest {
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        operation: StorageOperation,
        request_id: u32,
        generation: u16,
        capability_slot: u16,
        service_epoch: u64,
        start_block: u64,
        blocks: u16,
        payload_bytes: u16,
        transaction_id: u64,
    ) -> Option<Self> {
        if request_id == 0
            || generation == 0
            || service_epoch == 0
            || blocks > STORAGE_MAX_BLOCKS_PER_REQUEST
            || payload_bytes as usize > IPC_PAGE_BYTES
        {
            return None;
        }
        if matches!(operation, StorageOperation::Read | StorageOperation::Write) {
            if blocks != 1 || payload_bytes != STORAGE_BLOCK_BYTES {
                return None;
            }
        } else if matches!(operation, StorageOperation::AppendRecord) {
            if blocks != 0 || payload_bytes == 0 {
                return None;
            }
        } else if blocks != 0 || payload_bytes != 0 {
            return None;
        }
        Some(Self {
            operation,
            flags: 0,
            request_id,
            generation,
            capability_slot,
            service_epoch,
            start_block,
            blocks,
            payload_bytes,
            transaction_id,
        })
    }

    pub const fn is_block_io(self) -> bool {
        matches!(self.operation, StorageOperation::Read | StorageOperation::Write)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct StorageResponse {
    pub request_id: u32,
    pub status: StorageStatus,
    pub reserved: u8,
    pub generation: u16,
    pub blocks_completed: u16,
    pub payload_bytes: u16,
    pub transaction_id: u64,
    pub block_count: u64,
}

/// Private Storage-to-Core request for a read-only cache mapping.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct StorageMapRequest {
    pub operation: u8,
    pub flags: u8,
    pub reserved: u16,
    pub request_id: u32,
    pub generation: u64,
    pub client: u16,
    pub pages: u16,
    pub source_page: u64,
    pub target_page: u64,
    pub window_generation: u32,
    pub reserved_tail: u32,
}

/// Private Core-to-Storage result for a read-only cache mapping.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct StorageMapResponse {
    pub request_id: u32,
    pub status: StorageStatus,
    pub reserved: [u8; 3],
    pub generation: u64,
    pub target_page: u64,
    pub pages: u8,
    pub reserved_tail: [u8; 7],
    pub window_generation: u32,
    pub reserved_end: [u8; 4],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum NetworkProfile {
    Disabled = 0,
    StaticThenDhcp = 1,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct NetworkConfig {
    pub abi_version: u16,
    pub profile: NetworkProfile,
    pub reserved: u8,
    pub address: [u8; 4],
    pub netmask: [u8; 4],
    pub gateway: [u8; 4],
    pub gateway_deadline_ticks: u32,
    pub dhcp_deadline_ticks: u32,
    pub service_epoch: u64,
}

impl NetworkConfig {
    pub const fn disabled() -> Self {
        Self {
            abi_version: NETWORK_ABI_VERSION,
            profile: NetworkProfile::Disabled,
            reserved: 0,
            address: [0; 4],
            netmask: [0; 4],
            gateway: [0; 4],
            gateway_deadline_ticks: NETWORK_GATEWAY_ARP_DEADLINE_TICKS,
            dhcp_deadline_ticks: NETWORK_DHCP_DEADLINE_TICKS,
            service_epoch: 1,
        }
    }

    pub const fn is_enabled(self) -> bool {
        matches!(self.profile, NetworkProfile::StaticThenDhcp)
    }

    pub fn is_valid(self) -> bool {
        self.abi_version == NETWORK_ABI_VERSION
            && self.reserved == 0
            && self.gateway_deadline_ticks != 0
            && self.dhcp_deadline_ticks != 0
            && self.service_epoch != 0
            && match self.profile {
                NetworkProfile::Disabled => true,
                NetworkProfile::StaticThenDhcp => {
                    self.address != [0; 4]
                        && self.gateway != [0; 4]
                        && valid_ipv4_netmask(self.netmask)
                }
            }
    }
}

fn valid_ipv4_netmask(mask: [u8; 4]) -> bool {
    let bits = u32::from_be_bytes(mask);
    bits != 0 && (!bits & (!bits).wrapping_add(1)) == 0
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum NetworkOperation {
    Status = 1,
    IcmpPing = 2,
    UdpBind = 3,
    UdpSend = 4,
    UdpReceive = 5,
    TcpConnect = 6,
    TcpListen = 7,
    TcpAccept = 8,
    TcpRead = 9,
    TcpWrite = 10,
    Close = 11,
    Cancel = 12,
}

impl NetworkOperation {
    pub const fn from_raw(raw: u8) -> Option<Self> {
        match raw {
            1 => Some(Self::Status),
            2 => Some(Self::IcmpPing),
            3 => Some(Self::UdpBind),
            4 => Some(Self::UdpSend),
            5 => Some(Self::UdpReceive),
            6 => Some(Self::TcpConnect),
            7 => Some(Self::TcpListen),
            8 => Some(Self::TcpAccept),
            9 => Some(Self::TcpRead),
            10 => Some(Self::TcpWrite),
            11 => Some(Self::Close),
            12 => Some(Self::Cancel),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum NetworkState {
    Disabled = 0,
    Unavailable = 1,
    Configuring = 2,
    Ready = 3,
    Restarting = 4,
    Faulted = 5,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum NetworkResult {
    Ok = 0,
    Full = 1,
    WouldBlock = 2,
    Invalid = 3,
    Stale = 4,
    Timeout = 5,
    Disabled = 6,
    Unavailable = 7,
    NotFound = 8,
    Refused = 9,
    Checksum = 10,
    Unsupported = 11,
    Cancelled = 12,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct NetworkRequest {
    pub abi_version: u16,
    pub operation: NetworkOperation,
    pub flags: u8,
    pub request_id: u32,
    pub handle: u32,
    pub generation: u16,
    pub reserved: u16,
    pub service_epoch: u64,
    pub address: [u8; 4],
    pub port: u16,
    pub payload_len: u16,
    pub timeout_ticks: u32,
    pub reserved_tail: [u8; 16],
    pub payload: [u8; NETWORK_INLINE_PAYLOAD_BYTES],
}

impl NetworkRequest {
    pub const fn new(operation: NetworkOperation, request_id: u32) -> Self {
        Self {
            abi_version: NETWORK_ABI_VERSION,
            operation,
            flags: 0,
            request_id,
            handle: 0,
            generation: 0,
            reserved: 0,
            service_epoch: 0,
            address: [0; 4],
            port: 0,
            payload_len: 0,
            timeout_ticks: 0,
            reserved_tail: [0; 16],
            payload: [0; NETWORK_INLINE_PAYLOAD_BYTES],
        }
    }

    pub fn is_valid(self) -> bool {
        self.abi_version == NETWORK_ABI_VERSION
            && NetworkOperation::from_raw(self.operation as u8).is_some()
            && self.request_id != 0
            && self.flags & !NETWORK_REQUEST_FLAG_LISTENER == 0
            && self.reserved == 0
            && self.reserved_tail.iter().all(|byte| *byte == 0)
            && self.payload_len as usize <= NETWORK_INLINE_PAYLOAD_BYTES
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct NetworkResponse {
    pub abi_version: u16,
    pub operation: NetworkOperation,
    pub result: NetworkResult,
    pub state: NetworkState,
    pub request_id: u32,
    pub handle: u32,
    pub generation: u16,
    pub reserved: u16,
    pub service_epoch: u64,
    pub payload_len: u16,
    pub detail: [u8; 16],
    pub payload: [u8; NETWORK_INLINE_PAYLOAD_BYTES],
}

impl NetworkResponse {
    pub const fn new(
        operation: NetworkOperation,
        result: NetworkResult,
        state: NetworkState,
        request_id: u32,
    ) -> Self {
        Self {
            abi_version: NETWORK_ABI_VERSION,
            operation,
            result,
            state,
            request_id,
            handle: 0,
            generation: 0,
            reserved: 0,
            service_epoch: 0,
            payload_len: 0,
            detail: [0; 16],
            payload: [0; NETWORK_INLINE_PAYLOAD_BYTES],
        }
    }

    pub fn is_valid_for(self, request: NetworkRequest) -> bool {
        self.abi_version == NETWORK_ABI_VERSION
            && self.operation as u8 == request.operation as u8
            && self.request_id == request.request_id
            && self.reserved == 0
            && self.payload_len as usize <= NETWORK_INLINE_PAYLOAD_BYTES
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum NetworkPacketOperation {
    SubmitTx = 1,
    RecycleRx = 2,
    LinkState = 3,
    Reset = 4,
}

impl NetworkPacketOperation {
    pub const fn from_raw(raw: u8) -> Option<Self> {
        match raw {
            1 => Some(Self::SubmitTx),
            2 => Some(Self::RecycleRx),
            3 => Some(Self::LinkState),
            4 => Some(Self::Reset),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct NetworkPacketDescriptor {
    pub abi_version: u16,
    pub operation: NetworkPacketOperation,
    pub result: NetworkResult,
    pub page: u16,
    pub length: u16,
    pub generation: u16,
    pub reserved: u16,
    pub service_epoch: u64,
    pub sequence: u32,
    pub mac: [u8; 6],
    pub reserved_tail: [u8; 8],
}

impl NetworkPacketDescriptor {
    pub const fn new(operation: NetworkPacketOperation, page: u16, sequence: u32) -> Self {
        Self {
            abi_version: NETWORK_ABI_VERSION,
            operation,
            result: NetworkResult::Ok,
            page,
            length: 0,
            generation: 0,
            reserved: 0,
            service_epoch: 0,
            sequence,
            mac: [0; 6],
            reserved_tail: [0; 8],
        }
    }

    pub fn is_valid(self) -> bool {
        self.abi_version == NETWORK_ABI_VERSION
            && NetworkPacketOperation::from_raw(self.operation as u8).is_some()
            && self.page < NETWORK_PACKET_PAGE_COUNT as u16
            && self.length as usize <= NETWORK_PACKET_PAGE_BYTES
            && self.reserved == 0
            && self.reserved_tail.iter().all(|byte| *byte == 0)
    }
}

impl StorageResponse {
    pub const fn new(
        request_id: u32,
        status: StorageStatus,
        generation: u16,
        blocks_completed: u16,
        payload_bytes: u16,
        transaction_id: u64,
    ) -> Self {
        Self {
            request_id,
            status,
            reserved: 0,
            generation,
            blocks_completed,
            payload_bytes,
            transaction_id,
            block_count: 0,
        }
    }

    pub const fn with_block_count(mut self, block_count: u64) -> Self {
        self.block_count = block_count;
        self
    }
}

impl ServiceId {
    pub const fn index(self) -> usize {
        match self {
            Self::Input => 0,
            Self::Display => 1,
            Self::Terminal => 2,
            Self::Session => 3,
            Self::Flow => 4,
            Self::Storage => 5,
            Self::Network => 6,
            Self::Fetch => 7,
            Self::Device => 8,
            Self::User => 9,
        }
    }

    pub const fn from_index(index: usize) -> Option<Self> {
        match index {
            0 => Some(Self::Input),
            1 => Some(Self::Display),
            2 => Some(Self::Terminal),
            3 => Some(Self::Session),
            4 => Some(Self::Flow),
            5 => Some(Self::Storage),
            6 => Some(Self::Network),
            7 => Some(Self::Fetch),
            8 => Some(Self::Device),
            9 => Some(Self::User),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum IpcEndpointId {
    InputToTerminal = 0,
    TerminalToDisplay = 1,
    TerminalToSession = 2,
    SessionToTerminal = 3,
    SessionToFlow = 4,
    FlowToSession = 5,
    StorageToCore = 6,
    CoreToStorage = 7,
    FlowToStorage = 8,
    StorageToFlow = 9,
    NetworkToCore = 10,
    CoreToNetwork = 11,
    FlowToNetwork = 12,
    NetworkToFlow = 13,
    FlowToFetch = 14,
    FetchToFlow = 15,
    FetchToStorage = 16,
    StorageToFetch = 17,
    FetchToNetwork = 18,
    NetworkToFetch = 19,
    CoreToStoragePackage = 20,
    StoragePackageToCore = 21,
    StorageMapToCore = 22,
    CoreToStorageMap = 23,
    FlowToDevice = 24,
    DeviceToFlow = 25,
    DeviceToCore = 26,
    CoreToDevice = 27,
    FlowToUser = 28,
    UserToFlow = 29,
    UserToStorage = 30,
    StorageToUser = 31,
}

impl IpcEndpointId {
    pub const COUNT: usize = IPC_ENDPOINT_COUNT;

    pub const fn index(self) -> usize {
        self as usize
    }

    pub const fn from_index(index: usize) -> Option<Self> {
        match index {
            0 => Some(Self::InputToTerminal),
            1 => Some(Self::TerminalToDisplay),
            2 => Some(Self::TerminalToSession),
            3 => Some(Self::SessionToTerminal),
            4 => Some(Self::SessionToFlow),
            5 => Some(Self::FlowToSession),
            6 => Some(Self::StorageToCore),
            7 => Some(Self::CoreToStorage),
            8 => Some(Self::FlowToStorage),
            9 => Some(Self::StorageToFlow),
            10 => Some(Self::NetworkToCore),
            11 => Some(Self::CoreToNetwork),
            12 => Some(Self::FlowToNetwork),
            13 => Some(Self::NetworkToFlow),
            14 => Some(Self::FlowToFetch),
            15 => Some(Self::FetchToFlow),
            16 => Some(Self::FetchToStorage),
            17 => Some(Self::StorageToFetch),
            18 => Some(Self::FetchToNetwork),
            19 => Some(Self::NetworkToFetch),
            20 => Some(Self::CoreToStoragePackage),
            21 => Some(Self::StoragePackageToCore),
            22 => Some(Self::StorageMapToCore),
            23 => Some(Self::CoreToStorageMap),
            24 => Some(Self::FlowToDevice),
            25 => Some(Self::DeviceToFlow),
            26 => Some(Self::DeviceToCore),
            27 => Some(Self::CoreToDevice),
            28 => Some(Self::FlowToUser),
            29 => Some(Self::UserToFlow),
            30 => Some(Self::UserToStorage),
            31 => Some(Self::StorageToUser),
            _ => None,
        }
    }

    pub const fn producer(self) -> ServiceId {
        match self {
            Self::InputToTerminal => ServiceId::Input,
            Self::TerminalToDisplay | Self::TerminalToSession => ServiceId::Terminal,
            Self::SessionToTerminal | Self::SessionToFlow => ServiceId::Session,
            Self::FlowToSession => ServiceId::Flow,
            Self::StorageToCore | Self::CoreToStorage | Self::StorageToFlow => ServiceId::Storage,
            Self::FlowToStorage => ServiceId::Flow,
            Self::NetworkToCore | Self::NetworkToFlow | Self::CoreToNetwork => ServiceId::Network,
            Self::FlowToNetwork => ServiceId::Flow,
            Self::FlowToFetch => ServiceId::Flow,
            Self::FetchToFlow | Self::FetchToStorage | Self::FetchToNetwork => ServiceId::Fetch,
            Self::StorageToFetch => ServiceId::Storage,
            Self::CoreToStoragePackage
            | Self::StoragePackageToCore
            | Self::StorageMapToCore
            | Self::CoreToStorageMap => ServiceId::Storage,
            Self::NetworkToFetch => ServiceId::Network,
            Self::FlowToDevice => ServiceId::Flow,
            Self::DeviceToFlow | Self::DeviceToCore | Self::CoreToDevice => ServiceId::Device,
            Self::FlowToUser => ServiceId::Flow,
            Self::UserToFlow | Self::UserToStorage => ServiceId::User,
            Self::StorageToUser => ServiceId::Storage,
        }
    }

    pub const fn consumer(self) -> ServiceId {
        match self {
            Self::InputToTerminal | Self::SessionToTerminal => ServiceId::Terminal,
            Self::TerminalToDisplay => ServiceId::Display,
            Self::TerminalToSession | Self::FlowToSession => ServiceId::Session,
            Self::SessionToFlow => ServiceId::Flow,
            Self::StorageToCore | Self::CoreToStorage | Self::FlowToStorage => ServiceId::Storage,
            Self::StorageToFlow => ServiceId::Flow,
            Self::CoreToStoragePackage
            | Self::StoragePackageToCore
            | Self::StorageMapToCore
            | Self::CoreToStorageMap => ServiceId::Storage,
            Self::NetworkToCore | Self::CoreToNetwork | Self::FlowToNetwork => ServiceId::Network,
            Self::NetworkToFlow => ServiceId::Flow,
            Self::FlowToFetch | Self::FetchToStorage | Self::FetchToNetwork => ServiceId::Fetch,
            Self::FetchToFlow => ServiceId::Flow,
            Self::StorageToFetch | Self::NetworkToFetch => ServiceId::Fetch,
            Self::DeviceToFlow => ServiceId::Flow,
            Self::FlowToDevice | Self::DeviceToCore | Self::CoreToDevice => ServiceId::Device,
            Self::FlowToUser => ServiceId::User,
            Self::UserToFlow => ServiceId::Flow,
            Self::UserToStorage => ServiceId::Storage,
            Self::StorageToUser => ServiceId::User,
        }
    }

    pub const fn read_event_mask(self) -> u64 {
        ipc_read_event_mask(self.index())
    }

    pub const fn write_event_mask(self) -> u64 {
        ipc_write_event_mask(self.index())
    }
}

const _: () = assert!(IpcEndpointId::COUNT == IPC_ENDPOINT_COUNT);

/// Fixed capability-page slot for one edge of the service graph.
pub const fn ipc_capability_slot(
    service: ServiceId,
    endpoint: IpcEndpointId,
    rights: IpcRights,
) -> Option<usize> {
    match (service, endpoint, rights) {
        (ServiceId::Input, IpcEndpointId::InputToTerminal, IpcRights::Send) => Some(0),
        (ServiceId::Display, IpcEndpointId::TerminalToDisplay, IpcRights::Receive) => Some(0),
        (ServiceId::Terminal, IpcEndpointId::InputToTerminal, IpcRights::Receive) => Some(0),
        (ServiceId::Terminal, IpcEndpointId::TerminalToDisplay, IpcRights::Send) => Some(1),
        (ServiceId::Terminal, IpcEndpointId::TerminalToSession, IpcRights::Send) => Some(2),
        (ServiceId::Terminal, IpcEndpointId::SessionToTerminal, IpcRights::Receive) => Some(3),
        (ServiceId::Session, IpcEndpointId::TerminalToSession, IpcRights::Receive) => Some(0),
        (ServiceId::Session, IpcEndpointId::SessionToTerminal, IpcRights::Send) => Some(1),
        (ServiceId::Session, IpcEndpointId::SessionToFlow, IpcRights::Send) => Some(2),
        (ServiceId::Session, IpcEndpointId::FlowToSession, IpcRights::Receive) => Some(3),
        (ServiceId::Flow, IpcEndpointId::SessionToFlow, IpcRights::Receive) => Some(0),
        (ServiceId::Flow, IpcEndpointId::FlowToSession, IpcRights::Send) => Some(1),
        (ServiceId::Flow, IpcEndpointId::FlowToStorage, IpcRights::Send) => Some(2),
        (ServiceId::Flow, IpcEndpointId::StorageToFlow, IpcRights::Receive) => Some(3),
        (ServiceId::Storage, IpcEndpointId::FlowToStorage, IpcRights::Receive) => Some(0),
        (ServiceId::Storage, IpcEndpointId::StorageToFlow, IpcRights::Send) => Some(1),
        (ServiceId::Storage, IpcEndpointId::StorageToCore, IpcRights::Send) => Some(2),
        (ServiceId::Storage, IpcEndpointId::CoreToStorage, IpcRights::Receive) => Some(3),
        (ServiceId::Storage, IpcEndpointId::CoreToStoragePackage, IpcRights::Receive) => Some(6),
        (ServiceId::Storage, IpcEndpointId::StoragePackageToCore, IpcRights::Send) => Some(7),
        (ServiceId::Network, IpcEndpointId::NetworkToCore, IpcRights::Send) => Some(0),
        (ServiceId::Network, IpcEndpointId::CoreToNetwork, IpcRights::Receive) => Some(1),
        (ServiceId::Network, IpcEndpointId::FlowToNetwork, IpcRights::Receive) => Some(2),
        (ServiceId::Network, IpcEndpointId::NetworkToFlow, IpcRights::Send) => Some(3),
        (ServiceId::Flow, IpcEndpointId::FlowToNetwork, IpcRights::Send) => Some(4),
        (ServiceId::Flow, IpcEndpointId::NetworkToFlow, IpcRights::Receive) => Some(5),
        (ServiceId::Flow, IpcEndpointId::FlowToFetch, IpcRights::Send) => Some(6),
        (ServiceId::Flow, IpcEndpointId::FetchToFlow, IpcRights::Receive) => Some(7),
        (ServiceId::Fetch, IpcEndpointId::FetchToFlow, IpcRights::Send) => Some(0),
        (ServiceId::Fetch, IpcEndpointId::FlowToFetch, IpcRights::Receive) => Some(1),
        (ServiceId::Fetch, IpcEndpointId::FetchToStorage, IpcRights::Send) => Some(2),
        (ServiceId::Fetch, IpcEndpointId::StorageToFetch, IpcRights::Receive) => Some(3),
        (ServiceId::Fetch, IpcEndpointId::FetchToNetwork, IpcRights::Send) => Some(4),
        (ServiceId::Fetch, IpcEndpointId::NetworkToFetch, IpcRights::Receive) => Some(5),
        (ServiceId::Storage, IpcEndpointId::FetchToStorage, IpcRights::Receive) => Some(4),
        (ServiceId::Storage, IpcEndpointId::StorageToFetch, IpcRights::Send) => Some(5),
        (ServiceId::Network, IpcEndpointId::FetchToNetwork, IpcRights::Receive) => Some(4),
        (ServiceId::Network, IpcEndpointId::NetworkToFetch, IpcRights::Send) => Some(5),
        (ServiceId::Storage, IpcEndpointId::StorageMapToCore, IpcRights::Send) => Some(8),
        (ServiceId::Storage, IpcEndpointId::CoreToStorageMap, IpcRights::Receive) => Some(9),
        (ServiceId::Flow, IpcEndpointId::FlowToDevice, IpcRights::Send) => Some(8),
        (ServiceId::Flow, IpcEndpointId::DeviceToFlow, IpcRights::Receive) => Some(9),
        (ServiceId::Device, IpcEndpointId::FlowToDevice, IpcRights::Receive) => Some(2),
        (ServiceId::Device, IpcEndpointId::DeviceToFlow, IpcRights::Send) => Some(3),
        (ServiceId::Device, IpcEndpointId::DeviceToCore, IpcRights::Send) => Some(0),
        (ServiceId::Device, IpcEndpointId::CoreToDevice, IpcRights::Receive) => Some(1),
        (ServiceId::Flow, IpcEndpointId::FlowToUser, IpcRights::Send) => Some(10),
        (ServiceId::Flow, IpcEndpointId::UserToFlow, IpcRights::Receive) => Some(11),
        (ServiceId::User, IpcEndpointId::FlowToUser, IpcRights::Receive) => Some(0),
        (ServiceId::User, IpcEndpointId::UserToFlow, IpcRights::Send) => Some(1),
        (ServiceId::User, IpcEndpointId::UserToStorage, IpcRights::Send) => Some(2),
        (ServiceId::User, IpcEndpointId::StorageToUser, IpcRights::Receive) => Some(3),
        (ServiceId::Storage, IpcEndpointId::UserToStorage, IpcRights::Receive) => Some(10),
        (ServiceId::Storage, IpcEndpointId::StorageToUser, IpcRights::Send) => Some(11),
        _ => None,
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
    StorageRequest = 8,
    StorageResponse = 9,
    NetworkRequest = 10,
    NetworkResponse = 11,
    CompletionRequest = 12,
    CompletionResponse = 13,
    FetchRequest = 14,
    FetchControl = 15,
    FetchResponse = 16,
    FlowProgress = 17,
    FlowControl = 18,
    FetchBodyChunk = 19,
    DeviceRequest = 20,
    DeviceResponse = 21,
    UserRequest = 22,
    UserResponse = 23,
    UserStorageRequest = 24,
    UserStorageResponse = 25,
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
    pub const INSERT: Self = Self::Insert;
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
    pub const NUM_LOCK: Self = Self(0x305);

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
    pub flags: u8,
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
            flags: 0,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IpcMessageType {
    Input,
    Render,
    Bytes,
    Packet,
}

pub const fn ipc_message_type(endpoint: usize) -> Option<IpcMessageType> {
    match endpoint {
        0 => Some(IpcMessageType::Input),
        1 => Some(IpcMessageType::Render),
        2..=5 | 8..=9 | 12..=19 | 24..=25 => Some(IpcMessageType::Bytes),
        10..=11 => Some(IpcMessageType::Packet),
        28..=31 => Some(IpcMessageType::Bytes),
        _ => None,
    }
}

pub const fn ipc_message_size(endpoint: usize) -> Option<usize> {
    if endpoint == IpcEndpointId::StorageToCore as usize {
        return Some(core::mem::size_of::<StorageRequest>());
    }
    if endpoint == IpcEndpointId::CoreToStorage as usize {
        return Some(core::mem::size_of::<StorageResponse>());
    }
    if endpoint == IpcEndpointId::CoreToStoragePackage as usize {
        return Some(core::mem::size_of::<PackageRequest>());
    }
    if endpoint == IpcEndpointId::StoragePackageToCore as usize {
        return Some(core::mem::size_of::<PackageResponse>());
    }
    if endpoint == IpcEndpointId::StorageMapToCore as usize {
        return Some(core::mem::size_of::<StorageMapRequest>());
    }
    if endpoint == IpcEndpointId::CoreToStorageMap as usize {
        return Some(core::mem::size_of::<StorageMapResponse>());
    }
    if endpoint == IpcEndpointId::DeviceToCore as usize
        || endpoint == IpcEndpointId::FlowToDevice as usize
    {
        return Some(core::mem::size_of::<DeviceRequest>());
    }
    if endpoint == IpcEndpointId::CoreToDevice as usize
        || endpoint == IpcEndpointId::DeviceToFlow as usize
    {
        return Some(core::mem::size_of::<DeviceResponse>());
    }
    match ipc_message_type(endpoint) {
        Some(IpcMessageType::Input) => Some(core::mem::size_of::<InputMessage>()),
        Some(IpcMessageType::Render) => Some(core::mem::size_of::<RenderMessage>()),
        Some(IpcMessageType::Bytes) => Some(core::mem::size_of::<IpcBytes>()),
        Some(IpcMessageType::Packet) => Some(core::mem::size_of::<NetworkPacketDescriptor>()),
        None => None,
    }
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

pub const FETCH_REQUEST_DATA_BYTES: usize = 240;
pub const FETCH_BODY_CHUNK_BYTES: usize = 240;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct FetchRequest {
    pub request_id: u32,
    pub url_len: u16,
    pub destination_len: u16,
    pub reserved: u16,
    pub data: [u8; FETCH_REQUEST_DATA_BYTES],
}

impl FetchRequest {
    pub fn new(request_id: u32, url: &[u8], destination: &[u8]) -> Option<Self> {
        if request_id == 0 || url.is_empty() {
            return None;
        }
        let total = url.len().checked_add(destination.len())?;
        if total > FETCH_REQUEST_DATA_BYTES || url.len() > u16::MAX as usize {
            return None;
        }
        let mut request = Self {
            request_id,
            url_len: url.len() as u16,
            destination_len: destination.len() as u16,
            reserved: 0,
            data: [0; FETCH_REQUEST_DATA_BYTES],
        };
        request.data[..url.len()].copy_from_slice(url);
        request.data[url.len()..total].copy_from_slice(destination);
        Some(request)
    }

    pub fn is_valid(self) -> bool {
        self.request_id != 0
            && self.reserved == 0
            && usize::from(self.url_len) + usize::from(self.destination_len)
                <= FETCH_REQUEST_DATA_BYTES
            && self.url_len != 0
    }

    pub fn url(&self) -> Option<&[u8]> {
        self.is_valid().then(|| &self.data[..usize::from(self.url_len)])
    }

    pub fn destination(&self) -> Option<&[u8]> {
        self.is_valid().then(|| {
            let start = usize::from(self.url_len);
            &self.data[start..start + usize::from(self.destination_len)]
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum FetchControlOperation {
    Cancel = 1,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct FetchControl {
    pub request_id: u32,
    pub operation: FetchControlOperation,
    pub reserved: [u8; 3],
}

impl FetchControl {
    pub const fn cancel(request_id: u32) -> Self {
        Self { request_id, operation: FetchControlOperation::Cancel, reserved: [0; 3] }
    }

    pub const fn is_valid(self) -> bool {
        self.request_id != 0
            && matches!(self.operation, FetchControlOperation::Cancel)
            && self.reserved[0] == 0
            && self.reserved[1] == 0
            && self.reserved[2] == 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum FetchPhase {
    Idle = 0,
    Connect = 1,
    SendRequest = 2,
    ReadResponse = 3,
    StageStorage = 4,
    Commit = 5,
    Complete = 6,
    Failed = 7,
    Cancelled = 8,
}

impl FetchPhase {
    pub const fn is_valid(self) -> bool {
        matches!(
            self,
            Self::Idle
                | Self::Connect
                | Self::SendRequest
                | Self::ReadResponse
                | Self::StageStorage
                | Self::Commit
                | Self::Complete
                | Self::Failed
                | Self::Cancelled
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum FetchStatus {
    Ok = 0,
    InProgress = 1,
    Cancelled = 2,
    Invalid = 3,
    Network = 4,
    Storage = 5,
    Timeout = 6,
    Malformed = 7,
    Oversized = 8,
    Busy = 9,
    Stale = 10,
}

impl FetchStatus {
    pub const fn is_valid(self) -> bool {
        matches!(
            self,
            Self::Ok
                | Self::InProgress
                | Self::Cancelled
                | Self::Invalid
                | Self::Network
                | Self::Storage
                | Self::Timeout
                | Self::Malformed
                | Self::Oversized
                | Self::Busy
                | Self::Stale
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct FetchResponse {
    pub request_id: u32,
    pub phase: FetchPhase,
    pub status: FetchStatus,
    pub response_status: u16,
    pub downloaded_bytes: u32,
    pub total_bytes: u32,
}

impl FetchResponse {
    pub const fn new(
        request_id: u32,
        phase: FetchPhase,
        status: FetchStatus,
        downloaded_bytes: u32,
        total_bytes: Option<u32>,
    ) -> Self {
        Self {
            request_id,
            phase,
            status,
            response_status: 0,
            downloaded_bytes,
            total_bytes: match total_bytes {
                Some(value) => value,
                None => u32::MAX,
            },
        }
    }

    pub const fn with_response_status(mut self, status: u16) -> Self {
        self.response_status = status;
        self
    }

    pub const fn is_valid(self) -> bool {
        self.request_id != 0
            && self.phase.is_valid()
            && self.status.is_valid()
            && (self.response_status == 0
                || (self.response_status >= 200 && self.response_status < 300))
    }

    pub const fn total(self) -> Option<u32> {
        if self.total_bytes == u32::MAX { None } else { Some(self.total_bytes) }
    }
}

pub type FlowProgress = FetchResponse;

/// One bounded response-body fragment owned by Fetch and consumed by Flow.
///
/// Chunks use the same request correlation as progress messages.  The offset
/// makes delivery idempotent and lets Flow reject stale or reordered data.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct FetchBodyChunk {
    pub request_id: u32,
    pub offset: u32,
    pub len: u16,
    pub reserved: u16,
    pub bytes: [u8; FETCH_BODY_CHUNK_BYTES],
}

impl FetchBodyChunk {
    pub const fn new(request_id: u32, offset: u32, bytes: &[u8]) -> Option<Self> {
        if request_id == 0 || bytes.is_empty() || bytes.len() > FETCH_BODY_CHUNK_BYTES {
            return None;
        }
        let mut chunk = Self {
            request_id,
            offset,
            len: bytes.len() as u16,
            reserved: 0,
            bytes: [0; FETCH_BODY_CHUNK_BYTES],
        };
        let mut index = 0;
        while index < bytes.len() {
            chunk.bytes[index] = bytes[index];
            index += 1;
        }
        Some(chunk)
    }

    pub fn is_valid(self) -> bool {
        self.request_id != 0
            && self.reserved == 0
            && self.len != 0
            && usize::from(self.len) <= FETCH_BODY_CHUNK_BYTES
    }

    pub fn bytes(&self) -> Option<&[u8]> {
        self.is_valid().then(|| &self.bytes[..usize::from(self.len)])
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct FlowControl {
    pub request_id: u32,
    pub operation: FetchControlOperation,
    pub reserved: [u8; 3],
}

impl FlowControl {
    pub const fn cancel(request_id: u32) -> Self {
        Self { request_id, operation: FetchControlOperation::Cancel, reserved: [0; 3] }
    }

    pub const fn is_valid(self) -> bool {
        // Zero is the bounded "cancel the active Flow" wildcard.
        // resolves it only against its one active Fetch operation.
        matches!(self.operation, FetchControlOperation::Cancel)
            && self.reserved[0] == 0
            && self.reserved[1] == 0
            && self.reserved[2] == 0
    }
}

const _: () = {
    assert!(core::mem::size_of::<NetworkRequest>() <= MAX_IPC_BYTES);
    assert!(core::mem::size_of::<NetworkResponse>() <= MAX_IPC_BYTES);
    assert!(core::mem::size_of::<FetchRequest>() <= MAX_IPC_BYTES);
    assert!(core::mem::size_of::<FetchControl>() <= MAX_IPC_BYTES);
    assert!(core::mem::size_of::<FetchResponse>() <= MAX_IPC_BYTES);
    assert!(core::mem::size_of::<FetchBodyChunk>() <= MAX_IPC_BYTES);
    assert!(core::mem::size_of::<FlowControl>() <= MAX_IPC_BYTES);
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct CompletionRequest {
    pub version: u8,
    pub reserved: u8,
    pub request_id: u16,
    pub cursor: u16,
    pub line_len: u8,
    pub line_revision: u8,
    pub line: [u8; MAX_COMPLETION_LINE_BYTES],
}

impl CompletionRequest {
    pub fn new(request_id: u16, line: &[u8], cursor: usize) -> Option<Self> {
        if request_id == 0 || line.len() > MAX_COMPLETION_LINE_BYTES || cursor > line.len() {
            return None;
        }
        let mut request = Self {
            version: COMPLETION_ABI_VERSION,
            reserved: 0,
            request_id,
            cursor: cursor as u16,
            line_len: line.len() as u8,
            line_revision: 0,
            line: [0; MAX_COMPLETION_LINE_BYTES],
        };
        request.line[..line.len()].copy_from_slice(line);
        Some(request)
    }

    pub fn is_valid(self) -> bool {
        self.version == COMPLETION_ABI_VERSION
            && self.reserved == 0
            && self.request_id != 0
            && usize::from(self.line_len) <= MAX_COMPLETION_LINE_BYTES
            && usize::from(self.cursor) <= usize::from(self.line_len)
    }

    pub fn line(&self) -> Option<&[u8]> {
        self.is_valid().then(|| &self.line[..usize::from(self.line_len)])
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum CompletionStatus {
    Ok = 0,
    NoMatch = 1,
    Unavailable = 2,
    Malformed = 3,
}

impl CompletionStatus {
    pub const fn is_valid(self) -> bool {
        matches!(self, Self::Ok | Self::NoMatch | Self::Unavailable | Self::Malformed)
    }
}

pub const COMPLETION_FLAG_TRUNCATED: u8 = 1 << 0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct CompletionResponse {
    pub version: u8,
    pub status: CompletionStatus,
    pub request_id: u16,
    pub line_revision: u8,
    pub replace_start: u8,
    pub replace_end: u8,
    pub candidate_count: u8,
    pub flags: u8,
    pub lengths: [u8; MAX_COMPLETION_CANDIDATES],
    pub cursor_offsets: [u8; MAX_COMPLETION_CANDIDATES],
    pub candidates: [[u8; MAX_COMPLETION_ITEM_BYTES]; MAX_COMPLETION_CANDIDATES],
}

impl CompletionResponse {
    pub const fn empty(request_id: u16, status: CompletionStatus) -> Self {
        Self {
            version: COMPLETION_ABI_VERSION,
            status,
            request_id,
            line_revision: 0,
            replace_start: 0,
            replace_end: 0,
            candidate_count: 0,
            flags: 0,
            lengths: [0; MAX_COMPLETION_CANDIDATES],
            cursor_offsets: [0; MAX_COMPLETION_CANDIDATES],
            candidates: [[0; MAX_COMPLETION_ITEM_BYTES]; MAX_COMPLETION_CANDIDATES],
        }
    }

    pub fn push_candidate(&mut self, candidate: &[u8]) -> bool {
        self.push_candidate_with_cursor(candidate, candidate.len())
    }

    pub fn push_candidate_with_cursor(&mut self, candidate: &[u8], cursor_offset: usize) -> bool {
        let index = usize::from(self.candidate_count);
        if index >= MAX_COMPLETION_CANDIDATES
            || candidate.len() > MAX_COMPLETION_ITEM_BYTES
            || cursor_offset > candidate.len()
        {
            return false;
        }
        self.candidates[index][..candidate.len()].copy_from_slice(candidate);
        self.lengths[index] = candidate.len() as u8;
        self.cursor_offsets[index] = cursor_offset as u8;
        self.candidate_count += 1;
        true
    }

    pub fn candidate(&self, index: usize) -> Option<&[u8]> {
        if index >= usize::from(self.candidate_count)
            || index >= MAX_COMPLETION_CANDIDATES
            || usize::from(self.lengths[index]) > MAX_COMPLETION_ITEM_BYTES
        {
            return None;
        }
        Some(&self.candidates[index][..usize::from(self.lengths[index])])
    }

    pub fn is_valid_for(self, request: CompletionRequest) -> bool {
        self.version == COMPLETION_ABI_VERSION
            && self.status.is_valid()
            && self.request_id == request.request_id
            && self.line_revision == request.line_revision
            && self.flags & !COMPLETION_FLAG_TRUNCATED == 0
            && usize::from(self.candidate_count) <= MAX_COMPLETION_CANDIDATES
            && self.replace_start <= self.replace_end
            && usize::from(self.replace_end) <= usize::from(request.line_len)
            && self.lengths.iter().take(usize::from(self.candidate_count)).enumerate().all(
                |(index, length)| {
                    usize::from(*length) <= MAX_COMPLETION_ITEM_BYTES
                        && usize::from(self.cursor_offsets[index]) <= usize::from(*length)
                },
            )
    }
}

const _: () = assert!(core::mem::size_of::<CompletionRequest>() == MAX_IPC_BYTES);
const _: () = assert!(core::mem::size_of::<CompletionResponse>() <= MAX_IPC_BYTES);

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
const _: () = assert!(core::mem::size_of::<IpcCapabilityPage>() <= IPC_PAGE_BYTES);

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
    fn flow_fetch_body_chunks_are_bounded_and_correlated() {
        let chunk = FetchBodyChunk::new(7, 240, &[b'x'; FETCH_BODY_CHUNK_BYTES]).unwrap();
        assert!(chunk.is_valid());
        assert_eq!(chunk.bytes(), Some(&[b'x'; FETCH_BODY_CHUNK_BYTES][..]));
        assert!(FetchBodyChunk::new(0, 0, b"x").is_none());
        assert!(FetchBodyChunk::new(7, 0, &[]).is_none());
        assert!(FetchBodyChunk::new(7, 0, &[b'x'; FETCH_BODY_CHUNK_BYTES + 1]).is_none());
        assert_eq!(ServiceId::Flow as u8, 5);
        assert_eq!(IpcEndpointId::SessionToFlow as u8, 4);
        assert_eq!(IpcEndpointId::FlowToFetch as u8, 14);
        assert_eq!(MessageKind::FlowProgress as u8, 17);
        assert_eq!(MessageKind::FlowControl as u8, 18);
        assert!(
            FetchResponse::new(7, FetchPhase::Complete, FetchStatus::Ok, 1, Some(1))
                .with_response_status(200)
                .is_valid()
        );
        assert!(
            !FetchResponse::new(7, FetchPhase::Complete, FetchStatus::Ok, 1, Some(1))
                .with_response_status(500)
                .is_valid()
        );
    }

    #[test]
    fn completion_messages_are_bounded_and_correlated() {
        let request = CompletionRequest::new(7, b"service[\"st", 11).unwrap();
        assert!(request.is_valid());
        assert_eq!(request.line(), Some(&b"service[\"st"[..]));
        assert!(CompletionRequest::new(0, b"x", 1).is_none());
        assert!(CompletionRequest::new(8, &[b'x'; MAX_COMPLETION_LINE_BYTES + 1], 0).is_none());

        let mut response = CompletionResponse::empty(7, CompletionStatus::Ok);
        response.replace_start = 9;
        response.replace_end = 11;
        assert!(response.push_candidate(b"orage\"]"));
        assert!(response.is_valid_for(request));
        assert_eq!(response.candidate(0), Some(&b"orage\"]"[..]));

        let mut helper = CompletionResponse::empty(7, CompletionStatus::Ok);
        helper.replace_end = 4;
        assert!(helper.push_candidate_with_cursor(b"echo(\"\")", 6));
        assert_eq!(helper.cursor_offsets[0], 6);
        assert!(helper.is_valid_for(request));

        let mut stale_revision = response;
        stale_revision.line_revision = request.line_revision.wrapping_add(1);
        assert!(!stale_revision.is_valid_for(request));

        let mut stale = response;
        stale.request_id = 8;
        assert!(!stale.is_valid_for(request));
    }

    #[test]
    fn ipc_metadata_matches_the_fixed_service_graph() {
        assert_eq!(IpcEndpointId::TerminalToSession.producer(), ServiceId::Terminal);
        assert_eq!(IpcEndpointId::TerminalToSession.consumer(), ServiceId::Session);
        assert_eq!(IpcEndpointId::TerminalToSession.write_event_mask(), ipc_write_event_mask(2));
        assert_eq!(ipc_message_type(0), Some(IpcMessageType::Input));
        assert_eq!(ipc_message_type(1), Some(IpcMessageType::Render));
        assert_eq!(ipc_message_type(5), Some(IpcMessageType::Bytes));
        assert_eq!(ipc_message_type(6), None);
        assert_eq!(ipc_message_type(8), Some(IpcMessageType::Bytes));
        assert_eq!(ipc_message_size(0), Some(core::mem::size_of::<InputMessage>()));
        assert_eq!(ipc_message_size(1), Some(core::mem::size_of::<RenderMessage>()));
        assert_eq!(ipc_message_size(5), Some(core::mem::size_of::<IpcBytes>()));
        assert_eq!(
            ipc_capability_slot(
                ServiceId::Terminal,
                IpcEndpointId::TerminalToSession,
                IpcRights::Send,
            ),
            Some(2)
        );
        assert_eq!(
            ipc_capability_slot(
                ServiceId::Terminal,
                IpcEndpointId::TerminalToSession,
                IpcRights::Receive,
            ),
            None
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
    fn capabilities_are_bounded_and_generation_stamped() {
        let capability = IpcCapability::new(2, IpcRights::Send, 3, 9).unwrap();
        assert_eq!(capability.endpoint_index(), Some(2));
        assert_eq!(capability.rights, IpcRights::Send);
        assert!(IpcCapability::new(IPC_ENDPOINT_COUNT, IpcRights::Receive, 1, 1).is_none());
        assert!(IpcCapability::new(0, IpcRights::Receive, 0, 1).is_none());
    }

    #[test]
    fn manager_capability_and_requests_are_bounded() {
        let capability = ManagerCapability::new(3, ManagerRights::ALL, 9).unwrap();
        assert!(capability.rights.contains(ManagerRights::INSPECT));
        assert!(capability.rights.contains(ManagerRights::LIFECYCLE));
        assert!(ManagerCapability::new(0, ManagerRights::ALL, 9).is_none());
        assert!(ManagerCapability::new(3, ManagerRights::NONE, 9).is_none());
        assert!(ManagerCapability::new(3, ManagerRights(0x80), 9).is_none());
        assert!(core::mem::size_of::<ManagerRequest>() <= IPC_PAGE_BYTES);
        assert!(core::mem::size_of::<ManagerResponse>() <= IPC_PAGE_BYTES);
        assert_eq!(MANAGER_CAPABILITY_BASE, IPC_CAPABILITY_BASE + IPC_PAGE_BYTES);
        assert_eq!(STORAGE_CACHE_BASE, STORAGE_DATA_BASE + IPC_PAGE_BYTES);
        assert_eq!(STORAGE_DATA_PAGES, STORAGE_CACHE_PAGES + 1);
    }

    #[test]
    fn manager_responses_validate_the_fixed_envelope() {
        let request = ManagerRequest::new(ManagerOperation::List, 7);
        let response = ManagerResponse::new(ManagerOperation::List, ManagerStatus::Ok, 7);
        assert!(response.is_valid_for(request));

        let mut wrong_operation = response;
        wrong_operation.operation = ManagerOperation::Status;
        assert!(!wrong_operation.is_valid_for(request));

        let mut wrong_request_id = response;
        wrong_request_id.request_id = 8;
        assert!(!wrong_request_id.is_valid_for(request));

        let mut wrong_cursor = response;
        wrong_cursor.cursor = (MAX_MANAGER_SERVICES + 1) as u8;
        assert!(!wrong_cursor.is_valid_for(request));

        let mut wrong_reserved = response;
        wrong_reserved.reserved[0] = 1;
        assert!(!wrong_reserved.is_valid_for(request));
    }

    #[test]
    fn network_ping_timeout_is_explicit_and_bounded() {
        let mut request = NetworkRequest::new(NetworkOperation::IcmpPing, 7);
        request.timeout_ticks = NETWORK_PING_TIMEOUT_TICKS;
        assert!(request.is_valid());
        assert_eq!(request.timeout_ticks, 128);
    }

    #[test]
    fn command_cancel_can_target_the_active_operation_without_its_id() {
        assert!(FlowControl::cancel(0).is_valid());
        assert!(!FetchControl::cancel(0).is_valid());
        assert!(FlowControl::cancel(7).is_valid());
    }

    #[test]
    fn network_inline_payloads_are_bounded_and_round_trip() {
        assert!(core::mem::size_of::<NetworkRequest>() <= MAX_IPC_BYTES);
        assert!(core::mem::size_of::<NetworkResponse>() <= MAX_IPC_BYTES);
        let mut request = NetworkRequest::new(NetworkOperation::TcpWrite, 9);
        request.payload_len = NETWORK_INLINE_PAYLOAD_BYTES as u16;
        request.payload[0] = 0x5a;
        assert!(request.is_valid());
        request.payload_len += 1;
        assert!(!request.is_valid());
    }

    #[test]
    fn capability_page_rejects_empty_and_out_of_range_slots() {
        let mut page = IpcCapabilityPage::empty();
        assert_eq!(page.get(0), None);
        page.capabilities[0] = IpcCapability::new(0, IpcRights::Send, 1, 1).unwrap();
        assert!(page.get(0).is_some());
        assert_eq!(page.get(MAX_IPC_CAPABILITIES), None);
    }

    #[test]
    fn ipc_status_round_trips_bounded_results() {
        assert_eq!(IpcStatus::from_raw(IpcStatus::Ok as usize), Some(IpcStatus::Ok));
        assert_eq!(IpcStatus::from_raw(IpcStatus::Malformed as usize), Some(IpcStatus::Malformed));
        assert_eq!(IpcStatus::from_raw(usize::MAX), None);
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
    fn shared_ring_rejects_disconnected_operations() {
        let ring = RenderIpc::new(EndpointHeader::new(1, 1));
        let identity = ring.endpoint().identity();
        ring.disconnect();
        let message = RenderMessage::empty(MessageKind::RenderCells);
        assert_eq!(ring.send(identity, message), Err(SharedSendError::Disconnected));
        assert_eq!(ring.receive(identity), Err(SharedReceiveError::Disconnected));
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
        assert_eq!(EVENT_COUNT, 64);
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

    #[test]
    fn storage_requests_are_bounded_and_generation_stamped() {
        let request =
            StorageRequest::new(StorageOperation::Read, 7, 3, 1, 9, 12, 1, 4096, 4).unwrap();
        assert!(request.is_block_io());
        assert!(StorageRequest::new(StorageOperation::Read, 7, 3, 1, 9, 12, 2, 4096, 4).is_none());
        assert!(StorageRequest::new(StorageOperation::Read, 7, 3, 1, 9, 12, 0, 0, 4).is_none());
        assert!(
            StorageRequest::new(
                StorageOperation::Write,
                7,
                3,
                1,
                9,
                12,
                STORAGE_MAX_BLOCKS_PER_REQUEST + 1,
                0,
                4,
            )
            .is_none()
        );
    }

    #[test]
    fn storage_response_preserves_request_and_transaction_identity() {
        let response = StorageResponse::new(7, StorageStatus::Ok, 3, 2, 4096, 4);
        assert_eq!(response.request_id, 7);
        assert_eq!(response.generation, 3);
        assert_eq!(response.transaction_id, 4);
    }
}
