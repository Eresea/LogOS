#![no_std]

pub mod endpoint_v5;
pub mod service;

pub const MAX_SESSION_TEXT: usize = 256;
pub const PAGE_SIZE: usize = 4096;
pub const MAX_OBJECT_NAME: usize = 64;
pub const MAX_PERSISTENCE_OPERATIONS: usize = 8;
pub const MAX_NETWORK_PAYLOAD: usize = 1472;
pub const MAX_TCP_PAYLOAD: usize = 1024;
pub const NETWORK_MAX_ENDPOINTS: usize = 8;
pub const NETWORK_MAX_TCP_LISTENERS: usize = 1;
pub const NETWORK_MAX_TCP_CONNECTIONS: usize = 8;
pub const NETWORK_MAX_STREAM_TX_BYTES: usize = 4 * MAX_TCP_PAYLOAD;
pub const NETWORK_MAX_STREAM_RECORDS: usize = 8;
pub const NETWORK_MAX_ARP_ENTRIES: usize = 8;
pub const NETWORK_MAX_DATAGRAMS: usize = 4;
pub const NETWORK_MIN_FRAME: usize = 60;
pub const NETWORK_MAX_FRAME: usize = 1514;
pub const REMOTE_TCP_PORT: u16 = 7443;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct PageHandle(pub u32);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct NamespaceId(pub u32);

pub const TERMINAL_NAMESPACE: NamespaceId = NamespaceId(1);
pub const TRUST_NAMESPACE: NamespaceId = NamespaceId(5);
pub const TRUST_ENROLLMENT_NAME: &[u8] = b"enrollment";
pub const TRUST_SESSION_NAME: &[u8] = b"remote-session";

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(C)]
pub struct BlockInfo {
    pub logical_block_size: u32,
    pub blocks: u64,
    pub max_transfer_blocks: u32,
}

impl BlockInfo {
    pub const fn valid(self) -> bool {
        self.logical_block_size == 512
            && self.blocks > 0
            && self.max_transfer_blocks > 0
            && self.max_transfer_blocks <= (PAGE_SIZE / 512) as u32
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum BlockOperation {
    Read = 1,
    Write,
    Flush,
    Cancel,
    Reset,
    Info,
}

impl BlockOperation {
    pub const fn from_wire(value: u8) -> Option<Self> {
        match value {
            1 => Some(Self::Read),
            2 => Some(Self::Write),
            3 => Some(Self::Flush),
            4 => Some(Self::Cancel),
            5 => Some(Self::Reset),
            6 => Some(Self::Info),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct BlockRequest {
    pub id: u32,
    pub operation: BlockOperation,
    pub lba: u64,
    pub blocks: u32,
    pub page: PageHandle,
    pub deadline: u64,
}

impl BlockRequest {
    pub fn valid(self, info: BlockInfo) -> bool {
        if !info.valid() {
            return false;
        }
        match self.operation {
            BlockOperation::Read | BlockOperation::Write => {
                self.blocks != 0
                    && self.blocks <= info.max_transfer_blocks
                    && self.page.0 != 0
                    && self
                        .lba
                        .checked_add(self.blocks as u64)
                        .is_some_and(|end| end <= info.blocks)
            }
            BlockOperation::Info
            | BlockOperation::Flush
            | BlockOperation::Cancel
            | BlockOperation::Reset => self.lba == 0 && self.blocks == 0 && self.page.0 == 0,
        }
    }

    pub fn valid_shape(self) -> bool {
        match self.operation {
            BlockOperation::Read | BlockOperation::Write => {
                self.blocks != 0
                    && self.page.0 != 0
                    && self.lba.checked_add(self.blocks as u64).is_some()
            }
            BlockOperation::Info
            | BlockOperation::Flush
            | BlockOperation::Cancel
            | BlockOperation::Reset => self.lba == 0 && self.blocks == 0 && self.page.0 == 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum PersistenceStatus {
    Complete = 1,
    Invalid,
    Denied,
    Cancelled,
    TimedOut,
    Io,
    Corrupt,
    Recovered,
    OutOfMemory,
    Full,
    NotFound,
    Unavailable,
}

impl PersistenceStatus {
    pub const fn from_wire(value: u8) -> Option<Self> {
        match value {
            1 => Some(Self::Complete),
            2 => Some(Self::Invalid),
            3 => Some(Self::Denied),
            4 => Some(Self::Cancelled),
            5 => Some(Self::TimedOut),
            6 => Some(Self::Io),
            7 => Some(Self::Corrupt),
            8 => Some(Self::Recovered),
            9 => Some(Self::OutOfMemory),
            10 => Some(Self::Full),
            11 => Some(Self::NotFound),
            12 => Some(Self::Unavailable),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct BlockReply {
    pub id: u32,
    pub status: PersistenceStatus,
    pub info: BlockInfo,
}

impl BlockReply {
    pub fn valid_for(self, request: BlockRequest) -> bool {
        self.id == request.id
            && match request.operation {
                BlockOperation::Info if self.status == PersistenceStatus::Complete => {
                    self.info.valid()
                }
                BlockOperation::Info => self.info == BlockInfo::default(),
                _ => self.info == BlockInfo::default(),
            }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum VersionSelector {
    None = 0,
    Current = 1,
    Previous,
}

impl VersionSelector {
    pub const fn from_wire(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::None),
            1 => Some(Self::Current),
            2 => Some(Self::Previous),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum StoreOperation {
    OpenRead = 1,
    ReadChunk,
    BeginReplace,
    WriteChunk,
    Commit,
    Abort,
    Cancel,
}

impl StoreOperation {
    pub const fn from_wire(value: u8) -> Option<Self> {
        match value {
            1 => Some(Self::OpenRead),
            2 => Some(Self::ReadChunk),
            3 => Some(Self::BeginReplace),
            4 => Some(Self::WriteChunk),
            5 => Some(Self::Commit),
            6 => Some(Self::Abort),
            7 => Some(Self::Cancel),
            _ => None,
        }
    }
}

#[derive(Clone, Copy)]
#[repr(C)]
pub struct StoreRequest {
    pub id: u32,
    pub operation: StoreOperation,
    pub namespace: NamespaceId,
    pub name: [u8; MAX_OBJECT_NAME],
    pub name_length: u8,
    pub version: VersionSelector,
    pub offset: u64,
    pub length: u32,
    pub page: PageHandle,
    pub deadline: u64,
}

impl StoreRequest {
    pub fn valid(self) -> bool {
        let empty_identity =
            self.namespace.0 == 0 && self.name_length == 0 && self.name == [0; MAX_OBJECT_NAME];
        let identity = self.name_length != 0
            && usize::from(self.name_length) <= self.name.len()
            && self.name[usize::from(self.name_length)..].iter().all(|byte| *byte == 0)
            && core::str::from_utf8(&self.name[..usize::from(self.name_length)]).is_ok();
        match self.operation {
            StoreOperation::OpenRead => {
                identity
                    && matches!(self.version, VersionSelector::Current | VersionSelector::Previous)
                    && self.offset == 0
                    && self.length == 0
                    && self.page.0 == 0
            }
            StoreOperation::BeginReplace => {
                identity
                    && self.version == VersionSelector::None
                    && self.offset == 0
                    && self.length as usize <= PAGE_SIZE
                    && self.page.0 == 0
            }
            StoreOperation::ReadChunk | StoreOperation::WriteChunk => {
                empty_identity
                    && self.version == VersionSelector::None
                    && self.page.0 != 0
                    && self
                        .offset
                        .checked_add(self.length as u64)
                        .is_some_and(|end| end <= PAGE_SIZE as u64)
            }
            StoreOperation::Commit | StoreOperation::Abort | StoreOperation::Cancel => {
                empty_identity
                    && self.version == VersionSelector::None
                    && self.offset == 0
                    && self.length == 0
                    && self.page.0 == 0
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct StoreReply {
    pub id: u32,
    pub status: PersistenceStatus,
    pub version: u64,
    pub length: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum NetworkProtocol {
    Udp = 1,
    Icmp = 2,
    Tcp = 3,
}

impl NetworkProtocol {
    pub const fn from_wire(value: u8) -> Option<Self> {
        match value {
            1 => Some(Self::Udp),
            2 => Some(Self::Icmp),
            3 => Some(Self::Tcp),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct NetworkScope(pub u64);

impl NetworkScope {
    pub fn new(protocol: NetworkProtocol, address: u32, port: u16) -> Self {
        Self((u64::from(protocol as u8) << 56) | (u64::from(address) << 16) | u64::from(port))
    }

    pub const fn protocol(self) -> Option<NetworkProtocol> {
        NetworkProtocol::from_wire((self.0 >> 56) as u8)
    }

    pub const fn address(self) -> u32 {
        (self.0 >> 16) as u32
    }

    pub const fn port(self) -> u16 {
        self.0 as u16
    }

    pub const fn valid(self) -> bool {
        match self.protocol() {
            Some(NetworkProtocol::Udp) => self.port() != 0,
            Some(NetworkProtocol::Icmp) => self.address() != 0 && self.port() == 0,
            Some(NetworkProtocol::Tcp) => self.port() == REMOTE_TCP_PORT,
            None => false,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct NetworkEndpoint(pub u32);

impl NetworkEndpoint {
    pub fn new(slot: u16, generation: u16) -> Option<Self> {
        if slot == 0 || generation == 0 {
            None
        } else {
            Some(Self((u32::from(generation) << 16) | u32::from(slot)))
        }
    }

    pub const fn slot(self) -> u16 {
        self.0 as u16
    }

    pub const fn generation(self) -> u16 {
        (self.0 >> 16) as u16
    }

    pub const fn valid(self) -> bool {
        self.slot() != 0 && self.generation() != 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum NetworkOperation {
    Status = 1,
    Bind,
    SendTo,
    ReceiveFrom,
    Echo,
    Cancel,
    Close,
    Listen,
    Accept,
    Read,
    Write,
    SubmitWrite,
    PollStream,
}

impl NetworkOperation {
    pub const fn from_wire(value: u8) -> Option<Self> {
        match value {
            1 => Some(Self::Status),
            2 => Some(Self::Bind),
            3 => Some(Self::SendTo),
            4 => Some(Self::ReceiveFrom),
            5 => Some(Self::Echo),
            6 => Some(Self::Cancel),
            7 => Some(Self::Close),
            8 => Some(Self::Listen),
            9 => Some(Self::Accept),
            10 => Some(Self::Read),
            11 => Some(Self::Write),
            12 => Some(Self::SubmitWrite),
            13 => Some(Self::PollStream),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u16)]
pub enum NetworkStreamReadiness {
    Readable = 1,
    Writable = 2,
    Closed = 4,
}

impl NetworkStreamReadiness {
    pub const ALL_BITS: u16 = Self::Readable.bits() | Self::Writable.bits() | Self::Closed.bits();

    pub const fn bits(self) -> u16 {
        self as u16
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct NetworkStreamRecord {
    pub owner: u64,
    pub endpoint: NetworkEndpoint,
    pub generation: u16,
    pub readiness: u16,
    pub status: NetworkStatus,
    pub reserved: u8,
    pub sequence: u64,
    pub accepted_bytes: u64,
    pub acknowledged_bytes: u64,
}

impl NetworkStreamRecord {
    pub const EMPTY: Self = Self {
        owner: 0,
        endpoint: NetworkEndpoint(0),
        generation: 0,
        readiness: 0,
        status: NetworkStatus::Invalid,
        reserved: 0,
        sequence: 0,
        accepted_bytes: 0,
        acknowledged_bytes: 0,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum NetworkStatus {
    Complete = 1,
    Denied,
    Invalid,
    Busy,
    Full,
    Offline,
    NoRoute,
    AddressInUse,
    MessageTooLarge,
    TimedOut,
    Cancelled,
    Reset,
    Io,
}

impl NetworkStatus {
    pub const fn from_wire(value: u8) -> Option<Self> {
        match value {
            1 => Some(Self::Complete),
            2 => Some(Self::Denied),
            3 => Some(Self::Invalid),
            4 => Some(Self::Busy),
            5 => Some(Self::Full),
            6 => Some(Self::Offline),
            7 => Some(Self::NoRoute),
            8 => Some(Self::AddressInUse),
            9 => Some(Self::MessageTooLarge),
            10 => Some(Self::TimedOut),
            11 => Some(Self::Cancelled),
            12 => Some(Self::Reset),
            13 => Some(Self::Io),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum NetworkDeviceOperation {
    Info = 1,
    Transmit,
    Reset,
}

impl NetworkDeviceOperation {
    pub const fn from_wire(value: u8) -> Option<Self> {
        match value {
            1 => Some(Self::Info),
            2 => Some(Self::Transmit),
            3 => Some(Self::Reset),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(C)]
pub struct NetworkCounters {
    pub rx_frames: u64,
    pub tx_frames: u64,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    pub malformed: u64,
    pub unsupported: u64,
    pub rx_dropped: u64,
    pub udp_no_endpoint: u64,
    pub udp_queue_dropped: u64,
    pub timeouts: u64,
    pub cancellations: u64,
    pub resets: u64,
    pub denied: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(C)]
pub struct NetworkInfo {
    pub mac: [u8; 6],
    pub mtu: u16,
    pub generation: u16,
    pub link_up: u8,
    pub configuration: u8,
    pub ipv4: u32,
    pub subnet_mask: u32,
    pub router: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct NetworkDeviceRequest {
    pub id: u32,
    pub operation: NetworkDeviceOperation,
    pub length: u16,
    pub generation: u16,
    pub deadline: u64,
}

impl NetworkDeviceRequest {
    pub fn valid_shape(self) -> bool {
        if self.id == 0 || self.deadline == 0 {
            return false;
        }
        match self.operation {
            NetworkDeviceOperation::Info => self.length == 0 && self.generation == 0,
            NetworkDeviceOperation::Transmit => {
                (NETWORK_MIN_FRAME..=NETWORK_MAX_FRAME).contains(&(self.length as usize))
                    && self.generation != 0
            }
            NetworkDeviceOperation::Reset => self.length == 0 && self.generation != 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct NetworkDeviceReply {
    pub id: u32,
    pub status: NetworkStatus,
    pub generation: u16,
    pub info: NetworkInfo,
}

impl NetworkDeviceReply {
    pub fn valid_for(self, request: NetworkDeviceRequest) -> bool {
        if self.id != request.id {
            return false;
        }
        if self.status != NetworkStatus::Complete {
            return true;
        }
        match request.operation {
            NetworkDeviceOperation::Info => self.generation != 0,
            NetworkDeviceOperation::Transmit => self.generation == request.generation,
            NetworkDeviceOperation::Reset => {
                self.generation != 0 && self.generation != request.generation
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct NetworkRequest {
    pub id: u32,
    pub operation: NetworkOperation,
    pub endpoint: NetworkEndpoint,
    pub peer: NetworkScope,
    pub page: PageHandle,
    pub length: u16,
    pub generation: u16,
    pub deadline: u64,
}

impl NetworkRequest {
    pub fn valid_shape(self) -> bool {
        if self.id == 0 || self.deadline == 0 || self.length as usize > MAX_NETWORK_PAYLOAD {
            return false;
        }
        match self.operation {
            NetworkOperation::Status => {
                !self.endpoint.valid()
                    && self.peer.0 == 0
                    && self.page.0 == 0
                    && self.length == 0
                    && self.generation == 0
            }
            NetworkOperation::Bind => {
                !self.endpoint.valid()
                    && self.peer.protocol() == Some(NetworkProtocol::Udp)
                    && self.peer.valid()
                    && self.peer.address() == 0
                    && self.page.0 == 0
                    && self.length == 0
                    && self.generation == 0
            }
            NetworkOperation::SendTo => {
                self.endpoint.valid()
                    && self.peer.protocol() == Some(NetworkProtocol::Udp)
                    && self.peer.valid()
                    && self.peer.address() != 0
                    && self.page.0 != 0
                    && (1..=MAX_NETWORK_PAYLOAD).contains(&(self.length as usize))
                    && self.generation != 0
            }
            NetworkOperation::ReceiveFrom => {
                self.endpoint.valid()
                    && self.peer.protocol() == Some(NetworkProtocol::Udp)
                    && self.peer.valid()
                    && self.peer.address() == 0
                    && self.page.0 != 0
                    && self.length as usize == MAX_NETWORK_PAYLOAD
                    && self.generation != 0
            }
            NetworkOperation::Echo => {
                !self.endpoint.valid()
                    && self.peer.protocol() == Some(NetworkProtocol::Icmp)
                    && self.peer.valid()
                    && self.peer.address() != 0
                    && self.page.0 == 0
                    && self.length == 0
                    && self.generation == 0
            }
            NetworkOperation::Listen => {
                !self.endpoint.valid()
                    && self.peer.protocol() == Some(NetworkProtocol::Tcp)
                    && self.peer.valid()
                    && self.peer.address() == 0
                    && self.page.0 == 0
                    && self.length == 0
                    && self.generation == 0
            }
            NetworkOperation::Accept => {
                self.endpoint.valid()
                    && self.peer.protocol() == Some(NetworkProtocol::Tcp)
                    && self.peer.valid()
                    && self.peer.address() == 0
                    && self.page.0 == 0
                    && self.length == 0
                    && self.generation == 0
            }
            NetworkOperation::Read => {
                self.endpoint.valid()
                    && self.peer.protocol() == Some(NetworkProtocol::Tcp)
                    && self.peer.valid()
                    && self.peer.address() == 0
                    && self.page.0 != 0
                    && self.length as usize == MAX_TCP_PAYLOAD
                    && self.generation != 0
            }
            NetworkOperation::Write => {
                self.endpoint.valid()
                    && self.peer.protocol() == Some(NetworkProtocol::Tcp)
                    && self.peer.valid()
                    && self.peer.address() == 0
                    && self.page.0 != 0
                    && (1..=MAX_TCP_PAYLOAD).contains(&(self.length as usize))
                    && self.generation != 0
            }
            NetworkOperation::SubmitWrite => {
                self.endpoint.valid()
                    && self.peer.protocol() == Some(NetworkProtocol::Tcp)
                    && self.peer.valid()
                    && self.peer.address() == 0
                    && self.page.0 != 0
                    && (1..=MAX_TCP_PAYLOAD).contains(&(self.length as usize))
                    && self.generation != 0
            }
            NetworkOperation::PollStream => {
                self.endpoint.valid()
                    && self.peer.protocol() == Some(NetworkProtocol::Tcp)
                    && self.peer.valid()
                    && self.peer.address() == 0
                    && self.page.0 == 0
                    && self.length == 0
                    && self.generation != 0
            }
            NetworkOperation::Cancel | NetworkOperation::Close => {
                self.endpoint.valid()
                    && self.peer.0 == 0
                    && self.page.0 == 0
                    && self.length == 0
                    && self.generation == 0
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct NetworkReply {
    pub id: u32,
    pub status: NetworkStatus,
    pub endpoint: NetworkEndpoint,
    pub generation: u16,
    pub source_address: u32,
    pub source_port: u16,
    pub length: u16,
    pub stream_readiness: u16,
    pub stream_reserved: u16,
    pub stream_accepted_bytes: u64,
    pub stream_acknowledged_bytes: u64,
    pub info: NetworkInfo,
    pub counters: NetworkCounters,
}

impl NetworkReply {
    pub fn valid_for(self, request: NetworkRequest) -> bool {
        if self.id != request.id || self.length as usize > MAX_NETWORK_PAYLOAD {
            return false;
        }
        if self.status != NetworkStatus::Complete {
            return self.endpoint.0 == 0
                && self.source_address == 0
                && self.source_port == 0
                && self.length == 0
                && self.stream_readiness == 0
                && self.stream_reserved == 0
                && self.stream_accepted_bytes == 0
                && self.stream_acknowledged_bytes == 0;
        }
        if self.generation == 0 {
            return false;
        }
        match request.operation {
            NetworkOperation::Status => {
                self.endpoint.0 == 0
                    && self.source_address == 0
                    && self.source_port == 0
                    && self.length == 0
                    && self.generation != 0
            }
            NetworkOperation::Bind => {
                self.endpoint.valid()
                    && self.source_address == 0
                    && self.source_port == 0
                    && self.length == 0
                    && self.generation != 0
            }
            NetworkOperation::SendTo => {
                self.endpoint == request.endpoint
                    && self.source_address == 0
                    && self.source_port == 0
                    && self.length == request.length
                    && self.generation == request.generation
            }
            NetworkOperation::ReceiveFrom => {
                self.endpoint == request.endpoint
                    && self.source_address != 0
                    && self.source_port != 0
                    && self.generation == request.generation
            }
            NetworkOperation::Echo => {
                self.endpoint.0 == 0
                    && self.source_address == request.peer.address()
                    && self.source_port == 0
                    && self.length == 0
                    && self.generation != 0
            }
            NetworkOperation::Listen => {
                self.endpoint.valid()
                    && self.source_address == 0
                    && self.source_port == 0
                    && self.length == 0
                    && self.generation != 0
            }
            NetworkOperation::Accept => {
                self.endpoint.valid()
                    && self.source_address != 0
                    && self.source_port != 0
                    && self.length == 0
                    && self.generation != 0
            }
            NetworkOperation::Read => {
                self.endpoint == request.endpoint
                    && self.source_address == 0
                    && self.source_port == 0
                    && self.length as usize <= MAX_TCP_PAYLOAD
                    && self.generation == request.generation
            }
            NetworkOperation::Write => {
                self.endpoint == request.endpoint
                    && self.source_address == 0
                    && self.source_port == 0
                    && self.length == request.length
                    && self.generation == request.generation
            }
            NetworkOperation::SubmitWrite => {
                self.endpoint == request.endpoint
                    && self.source_address == 0
                    && self.source_port == 0
                    && self.length == request.length
                    && self.generation == request.generation
            }
            NetworkOperation::PollStream => {
                self.endpoint == request.endpoint
                    && self.source_address == 0
                    && self.source_port == 0
                    && self.length == 0
                    && self.stream_readiness & !NetworkStreamReadiness::ALL_BITS == 0
                    && self.stream_reserved == 0
                    && self.stream_acknowledged_bytes <= self.stream_accepted_bytes
                    && self.generation == request.generation
            }
            NetworkOperation::Cancel | NetworkOperation::Close => {
                self.endpoint.0 == 0
                    && self.source_address == 0
                    && self.source_port == 0
                    && self.length == 0
                    && self.generation != 0
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum NetworkEventKind {
    Frame = 1,
    Timer,
    Reset,
    Cancel,
}

impl NetworkEventKind {
    pub const fn from_wire(value: u8) -> Option<Self> {
        match value {
            1 => Some(Self::Frame),
            2 => Some(Self::Timer),
            3 => Some(Self::Reset),
            4 => Some(Self::Cancel),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct NetworkEvent {
    pub id: u32,
    pub kind: NetworkEventKind,
    pub generation: u16,
    pub device_generation: u32,
    pub page: PageHandle,
    pub length: u16,
    pub now: u64,
    pub metadata: [u8; 16],
}

impl NetworkEvent {
    pub fn valid(self) -> bool {
        self.id != 0
            && self.kind as u8 != 0
            && self.generation != 0
            && self.device_generation != 0
            && self.length as usize <= PAGE_SIZE
            && self.now != 0
            && match self.kind {
                NetworkEventKind::Frame => self.page.0 != 0,
                NetworkEventKind::Timer | NetworkEventKind::Reset | NetworkEventKind::Cancel => {
                    self.page.0 == 0 && self.length == 0
                }
            }
    }
}

impl StoreReply {
    pub const fn valid_for(self, request: StoreRequest) -> bool {
        self.id == request.id && self.length as usize <= PAGE_SIZE
    }
}

/// `foundation.session` v1 command request.  The transport stays bounded by
/// `logos_core::native_service::ControlPage`; this is the shared wire contract.
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
    RemoteKey,
    Enroll,
    Unenroll,
    Health,
}

impl Syscall {
    pub fn from_name(name: &[u8]) -> Option<Self> {
        match name {
            b"recovery" => Some(Self::Recovery),
            b"reboot" => Some(Self::Reboot),
            b"poweroff" => Some(Self::PowerOff),
            b"ping" => Some(Self::Ping),
            b"tasks" => Some(Self::Tasks),
            b"services" => Some(Self::Services),
            b"drivers" => Some(Self::Drivers),
            b"trace" => Some(Self::Trace),
            b"inspect" => Some(Self::Inspect),
            b"restart" => Some(Self::Restart),
            b"cancel" => Some(Self::Cancel),
            b"remote-key" => Some(Self::RemoteKey),
            b"enroll" => Some(Self::Enroll),
            b"unenroll" => Some(Self::Unenroll),
            b"health" => Some(Self::Health),
            _ => None,
        }
    }

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
            13 => Some(Self::RemoteKey),
            14 => Some(Self::Enroll),
            15 => Some(Self::Unenroll),
            16 => Some(Self::Health),
            _ => None,
        }
    }

    pub const fn takes_argument(self) -> bool {
        matches!(
            self,
            Self::Inspect | Self::Restart | Self::Cancel | Self::SetInputLayout | Self::Enroll
        )
    }
}

/// Privileged or machine-state operation requested by Sessions from Core.
#[derive(Clone, Copy, Eq, PartialEq)]
#[repr(u32)]
pub enum Effect {
    EnterRecovery = 1,
    ResetMachine,
    PowerOffMachine,
    PingService,
    ReadTasks,
    ReadServices,
    ReadDrivers,
    ReadTrace,
    InspectResource,
    RestartService,
    CancelService,
    SetInputLayout,
    RemoteKey,
    Enroll,
    Unenroll,
    ReadHealth,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct EffectReply {
    pub result: EffectResult,
    pub text: [u8; MAX_SESSION_TEXT],
    pub length: u16,
}

impl EffectReply {
    pub fn new(result: EffectResult, text: &[u8]) -> Self {
        let length = text.len().min(MAX_SESSION_TEXT);
        let mut output = Self { result, text: [0; MAX_SESSION_TEXT], length: length as u16 };
        output.text[..length].copy_from_slice(&text[..length]);
        output
    }

    pub fn valid(self) -> bool {
        self.length as usize <= self.text.len()
            && self.text[self.length as usize..].iter().all(|byte| *byte == 0)
    }
}

impl Effect {
    pub const fn from_wire(value: u32) -> Option<Self> {
        match value {
            1 => Some(Self::EnterRecovery),
            2 => Some(Self::ResetMachine),
            3 => Some(Self::PowerOffMachine),
            4 => Some(Self::PingService),
            5 => Some(Self::ReadTasks),
            6 => Some(Self::ReadServices),
            7 => Some(Self::ReadDrivers),
            8 => Some(Self::ReadTrace),
            9 => Some(Self::InspectResource),
            10 => Some(Self::RestartService),
            11 => Some(Self::CancelService),
            12 => Some(Self::SetInputLayout),
            13 => Some(Self::RemoteKey),
            14 => Some(Self::Enroll),
            15 => Some(Self::Unenroll),
            16 => Some(Self::ReadHealth),
            _ => None,
        }
    }

    pub const fn takes_argument(self) -> bool {
        matches!(
            self,
            Self::InspectResource
                | Self::RestartService
                | Self::CancelService
                | Self::SetInputLayout
                | Self::Enroll
        )
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

    pub fn valid(self) -> bool {
        self.length <= self.argument.len()
            && self.syscall.takes_argument() == (self.length != 0)
            && core::str::from_utf8(&self.argument[..self.length]).is_ok()
    }
}

#[derive(Clone, Copy)]
#[repr(C)]
pub struct EffectRequest {
    pub effect: Effect,
    pub argument: [u8; MAX_SESSION_TEXT],
    pub length: usize,
}

impl EffectRequest {
    pub const fn new(effect: Effect, argument: [u8; MAX_SESSION_TEXT], length: usize) -> Self {
        Self { effect, argument, length }
    }

    pub fn valid(self) -> bool {
        self.length <= self.argument.len()
            && self.effect.takes_argument() == (self.length != 0)
            && core::str::from_utf8(&self.argument[..self.length]).is_ok()
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

/// Typed result of one capability-gated Core effect. Sessions owns rendering
/// this value into a terminal reply.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum EffectResult {
    Completed = 1,
    Recovery,
    Unavailable,
    Pong,
    TasksActive,
    ServiceRunning,
    ServiceOverdue,
    DriverBound,
    TraceNone,
    TraceBoot,
    TraceTaskBlocked,
    TraceTaskWoken,
    TraceVirtioSubmit,
    TraceVirtioComplete,
    TraceDriverBound,
    TraceDriverQuiesced,
    TraceDriverRecovered,
    TraceDriverFailed,
    TraceFault,
    TraceSelfCheck,
    Inspected,
    RestartScheduled,
    CancelRequested,
    LayoutQwerty,
    LayoutAzerty,
    Denied,
    Unknown,
}

impl EffectResult {
    pub const fn from_wire(value: u32) -> Option<Self> {
        match value {
            1 => Some(Self::Completed),
            2 => Some(Self::Recovery),
            3 => Some(Self::Unavailable),
            4 => Some(Self::Pong),
            5 => Some(Self::TasksActive),
            6 => Some(Self::ServiceRunning),
            7 => Some(Self::ServiceOverdue),
            8 => Some(Self::DriverBound),
            9 => Some(Self::TraceNone),
            10 => Some(Self::TraceBoot),
            11 => Some(Self::TraceTaskBlocked),
            12 => Some(Self::TraceTaskWoken),
            13 => Some(Self::TraceVirtioSubmit),
            14 => Some(Self::TraceVirtioComplete),
            15 => Some(Self::TraceDriverBound),
            16 => Some(Self::TraceDriverQuiesced),
            17 => Some(Self::TraceDriverRecovered),
            18 => Some(Self::TraceDriverFailed),
            19 => Some(Self::TraceFault),
            20 => Some(Self::TraceSelfCheck),
            21 => Some(Self::Inspected),
            22 => Some(Self::RestartScheduled),
            23 => Some(Self::CancelRequested),
            24 => Some(Self::LayoutQwerty),
            25 => Some(Self::LayoutAzerty),
            26 => Some(Self::Denied),
            27 => Some(Self::Unknown),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
#[repr(transparent)]
pub struct InputEvent(u8);

impl InputEvent {
    pub const BACKSPACE: Self = Self(0x08);
    pub const ENTER: Self = Self(b'\n');
    pub const ESCAPE: Self = Self(0x1b);
    pub const UP: Self = Self(0x11);
    pub const DOWN: Self = Self(0x12);
    pub const STARTUP: Self = Self(0x13);

    pub const fn from_byte(byte: u8) -> Option<Self> {
        if byte == Self::BACKSPACE.0
            || byte == Self::ENTER.0
            || byte == Self::ESCAPE.0
            || byte == Self::UP.0
            || byte == Self::DOWN.0
            || byte == Self::STARTUP.0
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

pub fn self_check() -> bool {
    BlockInfo { logical_block_size: 512, blocks: 1, max_transfer_blocks: 8 }.valid()
        && !BlockInfo { logical_block_size: 4096, blocks: 1, max_transfer_blocks: 1 }.valid()
        && InputEvent::from_byte(b'a').is_some_and(|event| event.byte() == b'a')
        && InputEvent::from_byte(0).is_none()
        && DisplayColor::from_wire(DisplayColor::GREEN.wire()) == Some(DisplayColor::GREEN)
        && DisplayColor::from_wire(0xff00_0000).is_none()
        && Syscall::from_wire(Syscall::Restart as u32)
            .is_some_and(|call| call == Syscall::Restart && call.takes_argument())
        && Syscall::from_name(b"restart") == Some(Syscall::Restart)
        && Syscall::from_name(b"missing").is_none()
        && Syscall::from_wire(0).is_none()
        && Effect::from_wire(Effect::RestartService as u32) == Some(Effect::RestartService)
        && Effect::from_wire(0).is_none()
        && SessionRequest::new(Syscall::Inspect, [0; MAX_SESSION_TEXT], 1).valid()
        && !SessionRequest::new(Syscall::Reboot, [0; MAX_SESSION_TEXT], 1).valid()
        && EffectRequest::new(Effect::InspectResource, [0; MAX_SESSION_TEXT], 1).valid()
        && SessionReply::from_bytes(b"ok").is_some_and(|reply| reply.valid() && reply.length == 2)
        && EffectResult::from_wire(EffectResult::RestartScheduled as u32)
            == Some(EffectResult::RestartScheduled)
        && EffectResult::from_wire(0).is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    const INFO: BlockInfo =
        BlockInfo { logical_block_size: 512, blocks: 16, max_transfer_blocks: 8 };

    #[test]
    fn accepts_each_persistence_operation_shape() {
        for operation in [
            BlockOperation::Info,
            BlockOperation::Flush,
            BlockOperation::Cancel,
            BlockOperation::Reset,
        ] {
            assert!(
                BlockRequest {
                    id: 0,
                    operation,
                    lba: 0,
                    blocks: 0,
                    page: PageHandle(0),
                    deadline: 0
                }
                .valid(INFO)
            );
        }
        for operation in [BlockOperation::Read, BlockOperation::Write] {
            assert!(
                BlockRequest {
                    id: 0,
                    operation,
                    lba: 8,
                    blocks: 8,
                    page: PageHandle(1),
                    deadline: 0
                }
                .valid(INFO)
            );
        }
        let mut name = [0; MAX_OBJECT_NAME];
        name[..4].copy_from_slice(b"name");
        assert!(
            StoreRequest {
                id: 0,
                operation: StoreOperation::OpenRead,
                namespace: NamespaceId(1),
                name,
                name_length: 4,
                version: VersionSelector::Current,
                offset: 0,
                length: 0,
                page: PageHandle(0),
                deadline: 0
            }
            .valid()
        );
        assert!(
            StoreRequest {
                id: 0,
                operation: StoreOperation::BeginReplace,
                namespace: NamespaceId(1),
                name,
                name_length: 4,
                version: VersionSelector::None,
                offset: 0,
                length: PAGE_SIZE as u32,
                page: PageHandle(0),
                deadline: 0
            }
            .valid()
        );
        for operation in [StoreOperation::ReadChunk, StoreOperation::WriteChunk] {
            assert!(
                StoreRequest {
                    id: 0,
                    operation,
                    namespace: NamespaceId(0),
                    name: [0; MAX_OBJECT_NAME],
                    name_length: 0,
                    version: VersionSelector::None,
                    offset: 0,
                    length: PAGE_SIZE as u32,
                    page: PageHandle(1),
                    deadline: 0
                }
                .valid()
            );
        }
        for operation in [StoreOperation::Commit, StoreOperation::Abort, StoreOperation::Cancel] {
            assert!(
                StoreRequest {
                    id: 0,
                    operation,
                    namespace: NamespaceId(0),
                    name: [0; MAX_OBJECT_NAME],
                    name_length: 0,
                    version: VersionSelector::None,
                    offset: 0,
                    length: 0,
                    page: PageHandle(0),
                    deadline: 0
                }
                .valid()
            );
        }
    }

    #[test]
    fn rejects_unknown_and_malformed_persistence_wires() {
        assert!(BlockOperation::from_wire(0).is_none());
        assert!(StoreOperation::from_wire(0).is_none());
        assert!(PersistenceStatus::from_wire(0).is_none());
        assert!(VersionSelector::from_wire(3).is_none());
        assert!(
            !BlockRequest {
                id: 0,
                operation: BlockOperation::Read,
                lba: u64::MAX,
                blocks: 1,
                page: PageHandle(1),
                deadline: 0
            }
            .valid(INFO)
        );
        assert!(
            !BlockRequest {
                id: 0,
                operation: BlockOperation::Info,
                lba: 1,
                blocks: 0,
                page: PageHandle(0),
                deadline: 0
            }
            .valid(INFO)
        );
        assert!(
            !StoreRequest {
                id: 0,
                operation: StoreOperation::WriteChunk,
                namespace: NamespaceId(0),
                name: [0; MAX_OBJECT_NAME],
                name_length: 0,
                version: VersionSelector::None,
                offset: PAGE_SIZE as u64,
                length: 1,
                page: PageHandle(1),
                deadline: 0
            }
            .valid()
        );
        let mut invalid_name = [0; MAX_OBJECT_NAME];
        invalid_name[0] = 0xff;
        assert!(
            !StoreRequest {
                id: 0,
                operation: StoreOperation::OpenRead,
                namespace: NamespaceId(1),
                name: invalid_name,
                name_length: 1,
                version: VersionSelector::Current,
                offset: 0,
                length: 0,
                page: PageHandle(0),
                deadline: 0
            }
            .valid()
        );
    }

    #[test]
    fn accepts_network_shapes_and_rejects_stale_fields() {
        let bind = NetworkRequest {
            id: 1,
            operation: NetworkOperation::Bind,
            endpoint: NetworkEndpoint(0),
            peer: NetworkScope::new(NetworkProtocol::Udp, 0, 4000),
            page: PageHandle(0),
            length: 0,
            generation: 0,
            deadline: 1,
        };
        assert!(bind.valid_shape());
        let send = NetworkRequest {
            id: 2,
            operation: NetworkOperation::SendTo,
            endpoint: NetworkEndpoint::new(1, 1).unwrap(),
            peer: NetworkScope::new(NetworkProtocol::Udp, 0xc000_0201, 4001),
            page: PageHandle(1),
            length: MAX_NETWORK_PAYLOAD as u16,
            generation: 1,
            deadline: 1,
        };
        assert!(send.valid_shape());
        assert!(!NetworkRequest { id: 0, ..send }.valid_shape());
        assert!(!NetworkRequest { deadline: 0, ..send }.valid_shape());
        assert!(!NetworkRequest { page: PageHandle(0), ..send }.valid_shape());
        let transmit = NetworkDeviceRequest {
            id: 3,
            operation: NetworkDeviceOperation::Transmit,
            length: NETWORK_MIN_FRAME as u16,
            generation: 1,
            deadline: 1,
        };
        assert!(transmit.valid_shape());
        assert!(!NetworkDeviceRequest { id: 0, ..transmit }.valid_shape());
        assert!(!NetworkDeviceRequest { deadline: 0, ..transmit }.valid_shape());
        assert!(!NetworkDeviceRequest { length: 59, ..transmit }.valid_shape());
        assert!(
            !NetworkDeviceRequest { operation: NetworkDeviceOperation::Info, ..transmit }
                .valid_shape()
        );
        let complete = NetworkDeviceReply {
            id: 3,
            status: NetworkStatus::Complete,
            generation: 1,
            info: NetworkInfo { generation: 1, ..NetworkInfo::default() },
        };
        assert!(complete.valid_for(transmit));
        assert!(!NetworkDeviceReply { id: 4, ..complete }.valid_for(transmit));
        assert!(!NetworkDeviceReply { generation: 2, ..complete }.valid_for(transmit));
        assert_eq!(
            NetworkScope::new(NetworkProtocol::Udp, 7, 8).protocol(),
            Some(NetworkProtocol::Udp)
        );
        assert_eq!(NetworkScope::new(NetworkProtocol::Udp, 7, 8).address(), 7);
        assert_eq!(NetworkScope::new(NetworkProtocol::Udp, 7, 8).port(), 8);
        assert!(!NetworkScope::new(NetworkProtocol::Udp, 0, 0).valid());
        assert!(NetworkScope::new(NetworkProtocol::Icmp, 0xc000_0201, 0).valid());
        let echo = NetworkRequest {
            id: 4,
            operation: NetworkOperation::Echo,
            endpoint: NetworkEndpoint(0),
            peer: NetworkScope::new(NetworkProtocol::Icmp, 0xc000_0201, 0),
            page: PageHandle(0),
            length: 0,
            generation: 0,
            deadline: 1,
        };
        assert!(echo.valid_shape());

        let listen = NetworkRequest {
            id: 5,
            operation: NetworkOperation::Listen,
            endpoint: NetworkEndpoint(0),
            peer: NetworkScope::new(NetworkProtocol::Tcp, 0, 7443),
            page: PageHandle(0),
            length: 0,
            generation: 0,
            deadline: 1,
        };
        assert!(listen.valid_shape());
        assert!(!NetworkScope::new(NetworkProtocol::Tcp, 0, REMOTE_TCP_PORT + 1).valid());
        let write = NetworkRequest {
            id: 6,
            operation: NetworkOperation::Write,
            endpoint: NetworkEndpoint::new(1, 1).unwrap(),
            peer: NetworkScope::new(NetworkProtocol::Tcp, 0, 7443),
            page: PageHandle(1),
            length: MAX_TCP_PAYLOAD as u16,
            generation: 1,
            deadline: 1,
        };
        assert!(write.valid_shape());
        assert!(!NetworkRequest { length: MAX_TCP_PAYLOAD as u16 + 1, ..write }.valid_shape());
        let submit = NetworkRequest { operation: NetworkOperation::SubmitWrite, ..write };
        assert!(submit.valid_shape());
        let poll = NetworkRequest {
            id: 7,
            operation: NetworkOperation::PollStream,
            endpoint: write.endpoint,
            peer: write.peer,
            page: PageHandle(0),
            length: 0,
            generation: write.generation,
            deadline: 1,
        };
        assert!(poll.valid_shape());
        assert!(!NetworkRequest { page: write.page, ..poll }.valid_shape());

        let reply = NetworkReply {
            id: send.id,
            status: NetworkStatus::Complete,
            endpoint: send.endpoint,
            generation: send.generation,
            source_address: 0,
            source_port: 0,
            length: send.length,
            stream_readiness: 0,
            stream_reserved: 0,
            stream_accepted_bytes: 0,
            stream_acknowledged_bytes: 0,
            info: NetworkInfo { generation: send.generation, ..NetworkInfo::default() },
            counters: NetworkCounters::default(),
        };
        assert!(reply.valid_for(send));
        assert!(!NetworkReply { endpoint: NetworkEndpoint(0), ..reply }.valid_for(send));
        assert!(!NetworkReply { generation: 2, ..reply }.valid_for(send));
        let tcp_reply = NetworkReply {
            id: write.id,
            status: NetworkStatus::Complete,
            endpoint: write.endpoint,
            generation: write.generation,
            source_address: 0,
            source_port: 0,
            length: write.length,
            stream_readiness: 0,
            stream_reserved: 0,
            stream_accepted_bytes: 0,
            stream_acknowledged_bytes: 0,
            info: NetworkInfo { generation: write.generation, ..NetworkInfo::default() },
            counters: NetworkCounters::default(),
        };
        assert!(tcp_reply.valid_for(write));
        assert!(tcp_reply.valid_for(submit));
        assert!(
            NetworkReply {
                id: poll.id,
                endpoint: poll.endpoint,
                generation: poll.generation,
                length: 0,
                ..tcp_reply
            }
            .valid_for(poll)
        );
        let poll_reply = NetworkReply {
            id: poll.id,
            endpoint: poll.endpoint,
            generation: poll.generation,
            length: 0,
            stream_readiness: NetworkStreamReadiness::Readable.bits()
                | NetworkStreamReadiness::Writable.bits(),
            stream_accepted_bytes: 6,
            stream_acknowledged_bytes: 3,
            ..tcp_reply
        };
        assert!(poll_reply.valid_for(poll));
        assert!(!NetworkReply { stream_readiness: 8, ..poll_reply }.valid_for(poll));
        assert!(!NetworkReply { stream_acknowledged_bytes: 7, ..poll_reply }.valid_for(poll));
        assert!(!NetworkReply { length: send.length - 1, ..reply }.valid_for(send));
        assert!(
            !NetworkReply { status: NetworkStatus::TimedOut, endpoint: send.endpoint, ..reply }
                .valid_for(send)
        );
    }
}
