#![no_std]

use core::ops::{Deref, DerefMut};

pub const ETHERNET_MIN_FRAME: usize = 60;
pub const ETHERNET_HEADER: usize = 14;
pub const ETHERNET_MAX_FRAME: usize = 1514;
pub const IPV4_HEADER: usize = 20;
pub const UDP_HEADER: usize = 8;
pub const TCP_HEADER: usize = 20;
pub const MAX_UDP_PAYLOAD: usize = 1472;
pub const STREAM_READABLE: u16 = 1;
pub const STREAM_WRITABLE: u16 = 2;
pub const STREAM_CLOSED: u16 = 4;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Short,
    Length,
    Unsupported,
    Checksum,
    Fragmented,
    Malformed,
    Destination,
    TooLarge,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Mac(pub [u8; 6]);

impl Mac {
    pub const BROADCAST: Self = Self([0xff; 6]);
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Ipv4(pub [u8; 4]);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Ethernet<'a> {
    pub destination: Mac,
    pub source: Mac,
    pub ether_type: u16,
    pub payload: &'a [u8],
}

pub fn parse_ethernet<'a>(frame: &'a [u8], local: Mac) -> Result<Ethernet<'a>, Error> {
    if frame.len() < ETHERNET_HEADER || frame.len() > ETHERNET_MAX_FRAME {
        return Err(Error::Length);
    }
    let destination = Mac(frame[0..6].try_into().map_err(|_| Error::Short)?);
    if destination != local && destination != Mac::BROADCAST {
        return Err(Error::Destination);
    }
    let source = Mac(frame[6..12].try_into().map_err(|_| Error::Short)?);
    let ether_type = u16::from_be_bytes([frame[12], frame[13]]);
    if matches!(ether_type, 0x8100 | 0x88a8) {
        return Err(Error::Unsupported);
    }
    if !matches!(ether_type, 0x0800 | 0x0806) {
        return Err(Error::Unsupported);
    }
    Ok(Ethernet { destination, source, ether_type, payload: &frame[ETHERNET_HEADER..] })
}

pub fn encode_ethernet(
    output: &mut [u8],
    destination: Mac,
    source: Mac,
    ether_type: u16,
    payload: &[u8],
) -> Result<usize, Error> {
    let length = ETHERNET_HEADER.checked_add(payload.len()).ok_or(Error::TooLarge)?;
    if length > ETHERNET_MAX_FRAME || payload.is_empty() {
        return Err(Error::Length);
    }
    let frame_length = length.max(ETHERNET_MIN_FRAME);
    if output.len() < frame_length {
        return Err(Error::Short);
    }
    output[..frame_length].fill(0);
    output[0..6].copy_from_slice(&destination.0);
    output[6..12].copy_from_slice(&source.0);
    output[12..14].copy_from_slice(&ether_type.to_be_bytes());
    output[14..length].copy_from_slice(payload);
    Ok(frame_length)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Arp {
    pub reply: bool,
    pub sender_mac: Mac,
    pub sender_ip: Ipv4,
    pub target_mac: Mac,
    pub target_ip: Ipv4,
}

pub fn parse_arp(bytes: &[u8]) -> Result<Arp, Error> {
    if bytes.len() < 28 {
        return Err(Error::Short);
    }
    if u16::from_be_bytes([bytes[0], bytes[1]]) != 1
        || u16::from_be_bytes([bytes[2], bytes[3]]) != 0x0800
        || bytes[4] != 6
        || bytes[5] != 4
    {
        return Err(Error::Unsupported);
    }
    let opcode = u16::from_be_bytes([bytes[6], bytes[7]]);
    if !matches!(opcode, 1 | 2) {
        return Err(Error::Unsupported);
    }
    Ok(Arp {
        reply: opcode == 2,
        sender_mac: Mac(bytes[8..14].try_into().map_err(|_| Error::Short)?),
        sender_ip: Ipv4(bytes[14..18].try_into().map_err(|_| Error::Short)?),
        target_mac: Mac(bytes[18..24].try_into().map_err(|_| Error::Short)?),
        target_ip: Ipv4(bytes[24..28].try_into().map_err(|_| Error::Short)?),
    })
}

pub fn encode_arp(output: &mut [u8], packet: Arp) -> Result<usize, Error> {
    if output.len() < 28 {
        return Err(Error::Short);
    }
    output[..28].fill(0);
    output[0..2].copy_from_slice(&1u16.to_be_bytes());
    output[2..4].copy_from_slice(&0x0800u16.to_be_bytes());
    output[4] = 6;
    output[5] = 4;
    output[6..8].copy_from_slice(&(if packet.reply { 2u16 } else { 1 }).to_be_bytes());
    output[8..14].copy_from_slice(&packet.sender_mac.0);
    output[14..18].copy_from_slice(&packet.sender_ip.0);
    output[18..24].copy_from_slice(&packet.target_mac.0);
    output[24..28].copy_from_slice(&packet.target_ip.0);
    Ok(28)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Ipv4Packet<'a> {
    pub source: Ipv4,
    pub destination: Ipv4,
    pub identification: u16,
    pub ttl: u8,
    pub protocol: u8,
    pub payload: &'a [u8],
}

pub fn parse_ipv4<'a>(bytes: &'a [u8], local: Ipv4) -> Result<Ipv4Packet<'a>, Error> {
    if bytes.len() < IPV4_HEADER {
        return Err(Error::Short);
    }
    if bytes[0] >> 4 != 4 {
        return Err(Error::Unsupported);
    }
    let header_length = usize::from(bytes[0] & 0x0f) * 4;
    if header_length < IPV4_HEADER || bytes.len() < header_length {
        return Err(Error::Length);
    }
    let total_length = usize::from(u16::from_be_bytes([bytes[2], bytes[3]]));
    if total_length < header_length || total_length > bytes.len() {
        return Err(Error::Length);
    }
    if u16::from_be_bytes([bytes[6], bytes[7]]) & 0x3fff != 0 {
        return Err(Error::Fragmented);
    }
    if checksum(&bytes[..header_length]) != 0 {
        return Err(Error::Checksum);
    }
    let destination = Ipv4(bytes[16..20].try_into().map_err(|_| Error::Short)?);
    if destination != local {
        return Err(Error::Destination);
    }
    Ok(Ipv4Packet {
        source: Ipv4(bytes[12..16].try_into().map_err(|_| Error::Short)?),
        destination,
        identification: u16::from_be_bytes([bytes[4], bytes[5]]),
        ttl: bytes[8],
        protocol: bytes[9],
        payload: &bytes[header_length..total_length],
    })
}

pub fn encode_ipv4(
    output: &mut [u8],
    source: Ipv4,
    destination: Ipv4,
    identification: u16,
    protocol: u8,
    payload: &[u8],
) -> Result<usize, Error> {
    let length = IPV4_HEADER.checked_add(payload.len()).ok_or(Error::TooLarge)?;
    if length > usize::from(u16::MAX) || output.len() < length {
        return Err(Error::TooLarge);
    }
    output[..length].fill(0);
    output[0] = 0x45;
    output[2..4].copy_from_slice(&(length as u16).to_be_bytes());
    output[4..6].copy_from_slice(&identification.to_be_bytes());
    output[8] = 64;
    output[9] = protocol;
    output[12..16].copy_from_slice(&source.0);
    output[16..20].copy_from_slice(&destination.0);
    let header_checksum = checksum_bytes(&output[..IPV4_HEADER]);
    output[10..12].copy_from_slice(&header_checksum);
    output[IPV4_HEADER..length].copy_from_slice(payload);
    Ok(length)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Udp<'a> {
    pub source_port: u16,
    pub destination_port: u16,
    pub payload: &'a [u8],
}

pub fn parse_udp<'a>(bytes: &'a [u8], source: Ipv4, destination: Ipv4) -> Result<Udp<'a>, Error> {
    if bytes.len() < UDP_HEADER {
        return Err(Error::Short);
    }
    let length = usize::from(u16::from_be_bytes([bytes[4], bytes[5]]));
    if !(UDP_HEADER..=bytes.len()).contains(&length) {
        return Err(Error::Length);
    }
    let checksum_value = u16::from_be_bytes([bytes[6], bytes[7]]);
    if checksum_value != 0 && pseudo_checksum(source, destination, 17, &bytes[..length]) != 0 {
        return Err(Error::Checksum);
    }
    Ok(Udp {
        source_port: u16::from_be_bytes([bytes[0], bytes[1]]),
        destination_port: u16::from_be_bytes([bytes[2], bytes[3]]),
        payload: &bytes[UDP_HEADER..length],
    })
}

pub fn encode_udp(
    output: &mut [u8],
    source: Ipv4,
    destination: Ipv4,
    source_port: u16,
    destination_port: u16,
    payload: &[u8],
) -> Result<usize, Error> {
    let length = UDP_HEADER.checked_add(payload.len()).ok_or(Error::TooLarge)?;
    if length > usize::from(u16::MAX) || output.len() < length {
        return Err(Error::TooLarge);
    }
    output[..length].fill(0);
    output[0..2].copy_from_slice(&source_port.to_be_bytes());
    output[2..4].copy_from_slice(&destination_port.to_be_bytes());
    output[4..6].copy_from_slice(&(length as u16).to_be_bytes());
    output[UDP_HEADER..length].copy_from_slice(payload);
    let value = pseudo_checksum(source, destination, 17, &output[..length]);
    output[6..8].copy_from_slice(&(if value == 0 { 0xffff } else { value }).to_be_bytes());
    Ok(length)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Tcp<'a> {
    pub source_port: u16,
    pub destination_port: u16,
    pub sequence: u32,
    pub acknowledgement: u32,
    pub flags: u8,
    pub window: u16,
    pub payload: &'a [u8],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TcpHeader {
    pub source_port: u16,
    pub destination_port: u16,
    pub sequence: u32,
    pub acknowledgement: u32,
    pub flags: u8,
    pub window: u16,
}

pub fn parse_tcp<'a>(bytes: &'a [u8], source: Ipv4, destination: Ipv4) -> Result<Tcp<'a>, Error> {
    if bytes.len() < TCP_HEADER {
        return Err(Error::Short);
    }
    let header = usize::from(bytes[12] >> 4) * 4;
    if header < TCP_HEADER || header > bytes.len() {
        return Err(Error::Length);
    }
    if pseudo_checksum(source, destination, 6, bytes) != 0 {
        return Err(Error::Checksum);
    }
    Ok(Tcp {
        source_port: u16::from_be_bytes([bytes[0], bytes[1]]),
        destination_port: u16::from_be_bytes([bytes[2], bytes[3]]),
        sequence: u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
        acknowledgement: u32::from_be_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]),
        flags: bytes[13],
        window: u16::from_be_bytes([bytes[14], bytes[15]]),
        payload: &bytes[header..],
    })
}

pub fn encode_tcp(
    output: &mut [u8],
    source: Ipv4,
    destination: Ipv4,
    header: TcpHeader,
    payload: &[u8],
) -> Result<usize, Error> {
    let length = TCP_HEADER.checked_add(payload.len()).ok_or(Error::TooLarge)?;
    if length > usize::from(u16::MAX) || output.len() < length {
        return Err(Error::TooLarge);
    }
    output[..length].fill(0);
    output[0..2].copy_from_slice(&header.source_port.to_be_bytes());
    output[2..4].copy_from_slice(&header.destination_port.to_be_bytes());
    output[4..8].copy_from_slice(&header.sequence.to_be_bytes());
    output[8..12].copy_from_slice(&header.acknowledgement.to_be_bytes());
    output[12] = 5 << 4;
    output[13] = header.flags;
    output[14..16].copy_from_slice(&header.window.to_be_bytes());
    output[TCP_HEADER..length].copy_from_slice(payload);
    let checksum = pseudo_checksum(source, destination, 6, &output[..length]).to_be_bytes();
    output[16..18].copy_from_slice(&checksum);
    Ok(length)
}

pub const TCP_FLAG_FIN: u8 = 0x01;
pub const TCP_FLAG_SYN: u8 = 0x02;
pub const TCP_FLAG_RST: u8 = 0x04;
pub const TCP_FLAG_ACK: u8 = 0x10;
pub const MAX_TCP_STREAM: usize = 1024;
pub const MAX_TCP_LISTENERS: usize = 1;
pub const MAX_TCP_CONNECTIONS: usize = 8;
pub const MAX_TCP_ENDPOINTS: usize = MAX_TCP_LISTENERS + MAX_TCP_CONNECTIONS;
pub const MAX_TCP_TX_BYTES: usize = 4 * MAX_TCP_STREAM;
pub const MAX_TCP_RX_BYTES: usize = 4 * MAX_TCP_STREAM;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TcpStateError {
    Full,
    AddressInUse,
    Invalid,
    Busy,
    NotFound,
    NoData,
    MessageTooLarge,
    Owner,
    Reset,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TcpPhase {
    SynReceived,
    Established,
    CloseWait,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TcpTx {
    pub source: Ipv4,
    pub destination: Ipv4,
    pub header: TcpHeader,
    pub length: u16,
    pub payload: [u8; MAX_TCP_STREAM],
}

#[derive(Clone, Copy)]
pub struct TcpConnection {
    active: bool,
    owner: u64,
    endpoint: EndpointId,
    local_port: u16,
    peer: Ipv4,
    peer_port: u16,
    phase: TcpPhase,
    local_seq: u32,
    remote_seq: u32,
    accepted: bool,
    read_length: u16,
    read_buffer: [u8; MAX_TCP_RX_BYTES],
    tx_length: u16,
    tx_buffer: [u8; MAX_TCP_TX_BYTES],
    accepted_bytes: u64,
    acknowledged_bytes: u64,
    in_flight: bool,
    in_flight_sequence: u32,
    in_flight_length: u16,
    outgoing: Option<TcpTx>,
    last: Option<TcpTx>,
    deadline: u64,
    retries: u8,
}

impl TcpConnection {
    const EMPTY: Self = Self {
        active: false,
        owner: 0,
        endpoint: EndpointId(0),
        local_port: 0,
        peer: Ipv4([0; 4]),
        peer_port: 0,
        phase: TcpPhase::SynReceived,
        local_seq: 0,
        remote_seq: 0,
        accepted: false,
        read_length: 0,
        read_buffer: [0; MAX_TCP_RX_BYTES],
        tx_length: 0,
        tx_buffer: [0; MAX_TCP_TX_BYTES],
        accepted_bytes: 0,
        acknowledged_bytes: 0,
        in_flight: false,
        in_flight_sequence: 0,
        in_flight_length: 0,
        outgoing: None,
        last: None,
        deadline: 0,
        retries: 0,
    };
}

#[derive(Clone, Copy)]
pub struct TcpListener {
    owner: u64,
    endpoint: EndpointId,
    local_port: u16,
    sequence: u32,
    active: bool,
}

impl TcpListener {
    const EMPTY: Self =
        Self { owner: 0, endpoint: EndpointId(0), local_port: 0, sequence: 0, active: false };
}

pub struct ListenerTable<const N: usize> {
    slots: [TcpListener; N],
}

impl<const N: usize> ListenerTable<N> {
    pub const fn new() -> Self {
        Self { slots: [TcpListener::EMPTY; N] }
    }
}

impl<const N: usize> Default for ListenerTable<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> Deref for ListenerTable<N> {
    type Target = [TcpListener; N];

    fn deref(&self) -> &Self::Target {
        &self.slots
    }
}

impl<const N: usize> DerefMut for ListenerTable<N> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.slots
    }
}

pub struct ConnectionTable<const N: usize> {
    slots: [TcpConnection; N],
}

impl<const N: usize> ConnectionTable<N> {
    pub const fn new() -> Self {
        Self { slots: [TcpConnection::EMPTY; N] }
    }
}

impl<const N: usize> Default for ConnectionTable<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> Deref for ConnectionTable<N> {
    type Target = [TcpConnection; N];

    fn deref(&self) -> &Self::Target {
        &self.slots
    }
}

impl<const N: usize> DerefMut for ConnectionTable<N> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.slots
    }
}

pub struct TcpState {
    generation: u16,
    listeners: ListenerTable<MAX_TCP_LISTENERS>,
    connections: ConnectionTable<MAX_TCP_CONNECTIONS>,
    next_schedule: usize,
    now: u64,
}

impl Default for TcpState {
    fn default() -> Self {
        Self::new()
    }
}

impl TcpState {
    pub const fn new() -> Self {
        Self {
            generation: 1,
            listeners: ListenerTable::new(),
            connections: ConnectionTable::new(),
            next_schedule: 0,
            now: 1,
        }
    }

    pub const fn generation(&self) -> u16 {
        self.generation
    }

    pub fn listen(
        &mut self,
        owner: u64,
        port: u16,
        sequence: u32,
    ) -> Result<EndpointId, TcpStateError> {
        if port == 0 {
            return Err(TcpStateError::Invalid);
        }
        if let Some(listener) =
            self.listeners.iter().find(|listener| listener.active && listener.local_port == port)
        {
            return (listener.owner == owner)
                .then_some(listener.endpoint)
                .ok_or(TcpStateError::AddressInUse);
        }
        let index = self
            .listeners
            .iter()
            .position(|listener| !listener.active)
            .ok_or(TcpStateError::Full)?;
        let endpoint = self.endpoint_for_listener(index);
        self.now = u64::from(sequence).max(1);
        self.listeners[index] =
            TcpListener { owner, endpoint, local_port: port, sequence, active: true };
        Ok(endpoint)
    }

    pub fn accept(
        &mut self,
        owner: u64,
        listener: EndpointId,
    ) -> Result<EndpointId, TcpStateError> {
        let index = self.listener_index(owner, listener)?;
        let port = self.listeners[index].local_port;
        self.connections
            .iter_mut()
            .find(|connection| {
                connection.active
                    && connection.owner == owner
                    && connection.local_port == port
                    && connection.phase == TcpPhase::Established
                    && !connection.accepted
            })
            .map(|connection| {
                connection.accepted = true;
                connection.endpoint
            })
            .ok_or(TcpStateError::NoData)
    }

    pub fn peer(&self, owner: u64, endpoint: EndpointId) -> Result<(Ipv4, u16), TcpStateError> {
        let connection = self.connection(owner, endpoint)?;
        Ok((connection.peer, connection.peer_port))
    }

    pub fn ingest(&mut self, source: Ipv4, packet: Tcp<'_>) -> Result<(), TcpStateError> {
        self.now = self.now.max(1);
        if let Some(index) =
            self.connection_index(source, packet.source_port, packet.destination_port)
        {
            if packet.flags & TCP_FLAG_RST != 0 {
                self.connections[index] = TcpConnection::EMPTY;
                return Err(TcpStateError::Reset);
            }
            if self.connections[index].phase == TcpPhase::SynReceived {
                if packet.flags & TCP_FLAG_ACK != 0
                    && packet.acknowledgement == self.connections[index].local_seq + 1
                {
                    self.connections[index].phase = TcpPhase::Established;
                    self.connections[index].local_seq =
                        self.connections[index].local_seq.wrapping_add(1);
                    self.connections[index].last = None;
                }
                return Ok(());
            }
            if self.connections[index].in_flight
                && packet.flags & TCP_FLAG_ACK != 0
                && packet.acknowledgement.wrapping_sub(self.connections[index].in_flight_sequence)
                    >= u32::from(self.connections[index].in_flight_length)
            {
                let length = self.connections[index].in_flight_length;
                self.connections[index].in_flight = false;
                self.connections[index].last = None;
                self.connections[index].acknowledged_bytes =
                    self.connections[index].acknowledged_bytes.saturating_add(u64::from(length));
                self.connections[index].in_flight_length = 0;
                self.arm_next_tx(index);
            }
            let mut acknowledge = false;
            if packet.sequence == self.connections[index].remote_seq {
                if packet.payload.len()
                    > MAX_TCP_RX_BYTES - usize::from(self.connections[index].read_length)
                {
                    self.arm_ack(
                        index,
                        source,
                        packet.source_port,
                        self.connections[index].local_seq,
                        self.connections[index].remote_seq,
                    );
                    return Err(TcpStateError::MessageTooLarge);
                }
                let start = usize::from(self.connections[index].read_length);
                self.connections[index].read_buffer[start..start + packet.payload.len()]
                    .copy_from_slice(packet.payload);
                self.connections[index].read_length = (start + packet.payload.len()) as u16;
                self.connections[index].remote_seq =
                    self.connections[index].remote_seq.wrapping_add(packet.payload.len() as u32);
                acknowledge = !packet.payload.is_empty();
            }
            if packet.flags & TCP_FLAG_FIN != 0
                && packet.sequence == self.connections[index].remote_seq
            {
                self.connections[index].remote_seq =
                    self.connections[index].remote_seq.wrapping_add(1);
                self.connections[index].phase = TcpPhase::CloseWait;
                acknowledge = true;
            }
            if acknowledge {
                self.arm_ack(
                    index,
                    source,
                    packet.source_port,
                    self.connections[index].local_seq,
                    self.connections[index].remote_seq,
                );
            }
            return Ok(());
        }
        if let Some(listener_index) = self.listener_for_port(packet.destination_port)
            && packet.flags & TCP_FLAG_SYN != 0
        {
            let index = self
                .connections
                .iter()
                .position(|connection| !connection.active)
                .ok_or(TcpStateError::Full)?;
            let listener = self.listeners[listener_index];
            let endpoint = self.endpoint_for_connection(index);
            let connection = TcpConnection {
                active: true,
                owner: listener.owner,
                endpoint,
                local_port: packet.destination_port,
                peer: source,
                peer_port: packet.source_port,
                phase: TcpPhase::SynReceived,
                local_seq: listener.sequence,
                remote_seq: packet.sequence.wrapping_add(1),
                ..TcpConnection::EMPTY
            };
            self.connections[index] = connection;
            self.listeners[listener_index].sequence = listener.sequence.wrapping_add(1);
            self.arm_control(
                index,
                TcpTx {
                    source: Ipv4([0; 4]),
                    destination: source,
                    header: TcpHeader {
                        source_port: packet.destination_port,
                        destination_port: packet.source_port,
                        sequence: connection.local_seq,
                        acknowledgement: connection.remote_seq,
                        flags: TCP_FLAG_SYN | TCP_FLAG_ACK,
                        window: MAX_TCP_RX_BYTES as u16,
                    },
                    length: 0,
                    payload: [0; MAX_TCP_STREAM],
                },
            );
        }
        Ok(())
    }

    pub fn take_tx(&mut self) -> Option<TcpTx> {
        for offset in 0..MAX_TCP_CONNECTIONS {
            let index = (self.next_schedule + offset) % MAX_TCP_CONNECTIONS;
            if let Some(tx) = self.connections[index].outgoing.take() {
                self.next_schedule = (index + 1) % MAX_TCP_CONNECTIONS;
                return Some(tx);
            }
        }
        None
    }

    /// Retransmit reliable control/data frames a bounded number of times.
    pub fn tick(&mut self, now: u64) -> bool {
        self.now = now.max(1);
        let mut changed = false;
        for index in 0..MAX_TCP_CONNECTIONS {
            if !self.connections[index].active {
                continue;
            }
            if self.connections[index].outgoing.is_none()
                && self.connections[index].last.is_some()
                && now >= self.connections[index].deadline
            {
                if self.connections[index].retries >= 3 {
                    self.connections[index] = TcpConnection::EMPTY;
                    continue;
                }
                self.connections[index].retries += 1;
                self.connections[index].deadline =
                    now.saturating_add(1u64 << self.connections[index].retries);
                self.connections[index].outgoing = self.connections[index].last;
                changed = true;
            }
            self.arm_next_tx(index);
        }
        changed
    }

    pub fn read(
        &mut self,
        owner: u64,
        endpoint: EndpointId,
        output: &mut [u8],
    ) -> Result<usize, TcpStateError> {
        let index = self.connection_index_by_owner(owner, endpoint)?;
        if self.connections[index].read_length == 0 {
            return Err(TcpStateError::NoData);
        }
        let length = usize::from(self.connections[index].read_length).min(output.len());
        output[..length].copy_from_slice(&self.connections[index].read_buffer[..length]);
        let read_length = usize::from(self.connections[index].read_length);
        self.connections[index].read_buffer.copy_within(length..read_length, 0);
        self.connections[index].read_length -= length as u16;
        Ok(length)
    }

    pub fn write(
        &mut self,
        owner: u64,
        endpoint: EndpointId,
        payload: &[u8],
    ) -> Result<(), TcpStateError> {
        let index = self.connection_index_by_owner(owner, endpoint)?;
        if self.connections[index].tx_length != 0 || self.connections[index].in_flight {
            return Err(TcpStateError::Busy);
        }
        self.submit_write(owner, endpoint, payload).map(|_| ())
    }

    pub fn submit_write(
        &mut self,
        owner: u64,
        endpoint: EndpointId,
        payload: &[u8],
    ) -> Result<u64, TcpStateError> {
        if payload.is_empty() || payload.len() > MAX_TCP_STREAM {
            return Err(TcpStateError::MessageTooLarge);
        }
        let index = self.connection_index_by_owner(owner, endpoint)?;
        if self.connections[index].phase != TcpPhase::Established {
            return Err(TcpStateError::Busy);
        }
        if payload.len() > MAX_TCP_TX_BYTES - usize::from(self.connections[index].tx_length) {
            return Err(TcpStateError::Busy);
        }
        let start = usize::from(self.connections[index].tx_length);
        self.connections[index].tx_buffer[start..start + payload.len()].copy_from_slice(payload);
        self.connections[index].tx_length += payload.len() as u16;
        self.connections[index].accepted_bytes =
            self.connections[index].accepted_bytes.saturating_add(payload.len() as u64);
        self.arm_next_tx(index);
        Ok(self.connections[index].accepted_bytes)
    }

    pub fn stream_watermarks(
        &self,
        owner: u64,
        endpoint: EndpointId,
    ) -> Result<(u64, u64), TcpStateError> {
        let connection = self.connection(owner, endpoint)?;
        Ok((connection.accepted_bytes, connection.acknowledged_bytes))
    }

    pub fn stream_state(
        &self,
        owner: u64,
        endpoint: EndpointId,
    ) -> Result<(u16, u64, u64), TcpStateError> {
        let connection = self.connection(owner, endpoint)?;
        let mut readiness = 0;
        if connection.read_length != 0 {
            readiness |= STREAM_READABLE;
        }
        if connection.phase == TcpPhase::Established
            && usize::from(connection.tx_length) < MAX_TCP_TX_BYTES
        {
            readiness |= STREAM_WRITABLE;
        }
        if connection.phase == TcpPhase::CloseWait {
            readiness |= STREAM_CLOSED;
        }
        Ok((readiness, connection.accepted_bytes, connection.acknowledged_bytes))
    }

    pub fn close(&mut self, owner: u64, endpoint: EndpointId) -> Result<(), TcpStateError> {
        if let Some(index) = self.listeners.iter().position(|listener| {
            listener.active && listener.owner == owner && listener.endpoint == endpoint
        }) {
            if self.connections.iter().any(|connection| {
                connection.active && connection.local_port == self.listeners[index].local_port
            }) {
                return Err(TcpStateError::Busy);
            }
            self.listeners[index] = TcpListener::EMPTY;
            return Ok(());
        }
        let index = self.connection_index_by_owner(owner, endpoint)?;
        self.connections[index] = TcpConnection::EMPTY;
        Ok(())
    }

    pub fn reset(&mut self) {
        self.generation = self.generation.wrapping_add(1).max(1);
        self.listeners = ListenerTable::new();
        self.connections = ConnectionTable::new();
        self.next_schedule = 0;
        self.now = 1;
    }

    pub fn listener_count(&self) -> usize {
        self.listeners.iter().filter(|listener| listener.active).count()
    }

    pub fn connection_count(&self) -> usize {
        self.connections.iter().filter(|connection| connection.active).count()
    }

    fn endpoint_for_listener(&self, index: usize) -> EndpointId {
        EndpointId((u32::from(self.generation) << 16) | (index as u32 + 1))
    }

    fn endpoint_for_connection(&self, index: usize) -> EndpointId {
        EndpointId(
            (u32::from(self.generation) << 16) | (index as u32 + MAX_TCP_LISTENERS as u32 + 1),
        )
    }

    fn listener_index(&self, owner: u64, endpoint: EndpointId) -> Result<usize, TcpStateError> {
        self.listeners
            .iter()
            .position(|listener| {
                listener.active && listener.owner == owner && listener.endpoint == endpoint
            })
            .ok_or(TcpStateError::Owner)
    }

    fn listener_for_port(&self, port: u16) -> Option<usize> {
        self.listeners.iter().position(|listener| listener.active && listener.local_port == port)
    }

    fn connection_index(
        &self,
        source: Ipv4,
        source_port: u16,
        destination_port: u16,
    ) -> Option<usize> {
        self.connections.iter().position(|connection| {
            connection.active
                && connection.peer == source
                && connection.peer_port == source_port
                && connection.local_port == destination_port
        })
    }

    fn connection_index_by_owner(
        &self,
        owner: u64,
        endpoint: EndpointId,
    ) -> Result<usize, TcpStateError> {
        self.connections
            .iter()
            .position(|connection| {
                connection.active && connection.owner == owner && connection.endpoint == endpoint
            })
            .ok_or_else(|| {
                if endpoint.generation() != self.generation {
                    TcpStateError::NotFound
                } else {
                    TcpStateError::Owner
                }
            })
    }

    fn connection(&self, owner: u64, endpoint: EndpointId) -> Result<TcpConnection, TcpStateError> {
        Ok(self.connections[self.connection_index_by_owner(owner, endpoint)?])
    }

    fn arm_ack(
        &mut self,
        index: usize,
        destination: Ipv4,
        destination_port: u16,
        sequence: u32,
        acknowledgement: u32,
    ) {
        self.connections[index].outgoing = Some(TcpTx {
            source: Ipv4([0; 4]),
            destination,
            header: TcpHeader {
                source_port: self.connections[index].local_port,
                destination_port,
                sequence,
                acknowledgement,
                flags: TCP_FLAG_ACK,
                window: (MAX_TCP_RX_BYTES as u16)
                    .saturating_sub(self.connections[index].read_length),
            },
            length: 0,
            payload: [0; MAX_TCP_STREAM],
        });
    }

    fn arm_control(&mut self, index: usize, tx: TcpTx) {
        self.connections[index].outgoing = Some(tx);
        self.connections[index].last = Some(tx);
        self.connections[index].retries = 0;
        self.connections[index].deadline = self.now.saturating_add(1).max(1);
    }

    fn arm_next_tx(&mut self, index: usize) {
        let connection = &mut self.connections[index];
        if !connection.active
            || connection.outgoing.is_some()
            || connection.in_flight
            || connection.tx_length == 0
            || connection.phase != TcpPhase::Established
        {
            return;
        }
        let length = usize::from(connection.tx_length).min(MAX_TCP_STREAM);
        let mut payload = [0; MAX_TCP_STREAM];
        payload[..length].copy_from_slice(&connection.tx_buffer[..length]);
        connection.tx_buffer.copy_within(length..usize::from(connection.tx_length), 0);
        connection.tx_length -= length as u16;
        let sequence = connection.local_seq;
        connection.local_seq = connection.local_seq.wrapping_add(length as u32);
        connection.in_flight = true;
        connection.in_flight_sequence = sequence;
        connection.in_flight_length = length as u16;
        let tx = TcpTx {
            source: Ipv4([0; 4]),
            destination: connection.peer,
            header: TcpHeader {
                source_port: connection.local_port,
                destination_port: connection.peer_port,
                sequence,
                acknowledgement: connection.remote_seq,
                flags: TCP_FLAG_ACK,
                window: MAX_TCP_RX_BYTES as u16,
            },
            length: length as u16,
            payload,
        };
        connection.outgoing = Some(tx);
        connection.last = Some(tx);
        connection.retries = 0;
        connection.deadline = self.now.saturating_add(1).max(1);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IcmpEcho<'a> {
    pub reply: bool,
    pub identifier: u16,
    pub sequence: u16,
    pub payload: &'a [u8],
}

pub fn parse_icmp_echo<'a>(bytes: &'a [u8]) -> Result<IcmpEcho<'a>, Error> {
    if bytes.len() < 8 || checksum(bytes) != 0 || bytes[1] != 0 {
        return Err(Error::Malformed);
    }
    if !matches!(bytes[0], 0 | 8) {
        return Err(Error::Unsupported);
    }
    Ok(IcmpEcho {
        reply: bytes[0] == 0,
        identifier: u16::from_be_bytes([bytes[4], bytes[5]]),
        sequence: u16::from_be_bytes([bytes[6], bytes[7]]),
        payload: &bytes[8..],
    })
}

pub fn encode_icmp_echo(
    output: &mut [u8],
    reply: bool,
    identifier: u16,
    sequence: u16,
    payload: &[u8],
) -> Result<usize, Error> {
    let length = 8usize.checked_add(payload.len()).ok_or(Error::TooLarge)?;
    if output.len() < length {
        return Err(Error::Short);
    }
    output[..length].fill(0);
    output[0] = if reply { 0 } else { 8 };
    output[4..6].copy_from_slice(&identifier.to_be_bytes());
    output[6..8].copy_from_slice(&sequence.to_be_bytes());
    output[8..length].copy_from_slice(payload);
    let packet_checksum = checksum_bytes(&output[..length]);
    output[2..4].copy_from_slice(&packet_checksum);
    Ok(length)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Dhcp<'a> {
    pub xid: u32,
    pub offered: Ipv4,
    pub client_mac: Mac,
    options: &'a [u8],
}

pub const DHCP_DISCOVER: u8 = 1;
pub const DHCP_OFFER: u8 = 2;
pub const DHCP_REQUEST: u8 = 3;
pub const DHCP_ACK: u8 = 5;
pub const DHCP_NAK: u8 = 6;
pub const DHCP_OPTION_MESSAGE_TYPE: u8 = 53;
pub const DHCP_OPTION_REQUESTED_IP: u8 = 50;
pub const DHCP_OPTION_SERVER_ID: u8 = 54;
pub const DHCP_OPTION_SUBNET_MASK: u8 = 1;
pub const DHCP_OPTION_ROUTER: u8 = 3;
pub const DHCP_OPTION_LEASE_TIME: u8 = 51;
pub const DHCP_OPTION_T1: u8 = 58;
pub const DHCP_OPTION_T2: u8 = 59;
pub const DHCP_OPTION_CLIENT_ID: u8 = 61;
pub const DHCP_OPTION_PARAMETER_REQUEST: u8 = 55;

impl<'a> Dhcp<'a> {
    pub fn option(self, wanted: u8) -> Result<Option<&'a [u8]>, Error> {
        let mut offset = 0;
        let mut found = None;
        while offset < self.options.len() {
            let code = self.options[offset];
            offset += 1;
            if code == 255 {
                break;
            }
            if code == 0 {
                continue;
            }
            let length = usize::from(*self.options.get(offset).ok_or(Error::Short)?);
            offset += 1;
            let end = offset.checked_add(length).ok_or(Error::Length)?;
            let value = self.options.get(offset..end).ok_or(Error::Short)?;
            if code == wanted {
                if found.is_some() {
                    return Err(Error::Malformed);
                }
                found = Some(value);
            }
            offset = end;
        }
        Ok(found)
    }
}

pub fn parse_dhcp<'a>(bytes: &'a [u8]) -> Result<Dhcp<'a>, Error> {
    if bytes.len() < 240 || bytes[0] != 2 || bytes[1] != 1 || bytes[2] != 6 {
        return Err(Error::Malformed);
    }
    if bytes[236..240] != [99, 130, 83, 99] {
        return Err(Error::Malformed);
    }
    let mut client_mac = [0; 6];
    client_mac.copy_from_slice(&bytes[28..34]);
    let offered = Ipv4(bytes[16..20].try_into().map_err(|_| Error::Short)?);
    let options = &bytes[240..];
    let mut offset = 0;
    let mut end = false;
    while offset < options.len() {
        let code = options[offset];
        offset += 1;
        if code == 255 {
            end = true;
            break;
        }
        if code == 0 {
            continue;
        }
        let length = usize::from(*options.get(offset).ok_or(Error::Short)?);
        offset += 1;
        offset = offset.checked_add(length).ok_or(Error::Length)?;
        if offset > options.len() {
            return Err(Error::Short);
        }
    }
    if !end {
        return Err(Error::Malformed);
    }
    Ok(Dhcp {
        xid: u32::from_be_bytes(bytes[4..8].try_into().map_err(|_| Error::Short)?),
        offered,
        client_mac: Mac(client_mac),
        options,
    })
}

pub fn encode_dhcp_discover(output: &mut [u8], xid: u32, client_mac: Mac) -> Result<usize, Error> {
    let mut packet = [0; 300];
    encode_dhcp_header(&mut packet, xid, client_mac);
    let mut offset = 240;
    write_option(&mut packet, &mut offset, DHCP_OPTION_MESSAGE_TYPE, &[DHCP_DISCOVER])?;
    write_option(
        &mut packet,
        &mut offset,
        DHCP_OPTION_CLIENT_ID,
        &[
            1,
            client_mac.0[0],
            client_mac.0[1],
            client_mac.0[2],
            client_mac.0[3],
            client_mac.0[4],
            client_mac.0[5],
        ],
    )?;
    write_option(
        &mut packet,
        &mut offset,
        DHCP_OPTION_PARAMETER_REQUEST,
        &[
            DHCP_OPTION_SUBNET_MASK,
            DHCP_OPTION_ROUTER,
            6,
            DHCP_OPTION_LEASE_TIME,
            DHCP_OPTION_T1,
            DHCP_OPTION_T2,
        ],
    )?;
    packet[offset] = 255;
    offset += 1;
    if output.len() < offset {
        return Err(Error::Short);
    }
    output[..offset].copy_from_slice(&packet[..offset]);
    Ok(offset)
}

pub fn encode_dhcp_request(
    output: &mut [u8],
    xid: u32,
    client_mac: Mac,
    requested: Ipv4,
    server: Ipv4,
) -> Result<usize, Error> {
    let mut packet = [0; 300];
    encode_dhcp_header(&mut packet, xid, client_mac);
    let mut offset = 240;
    write_option(&mut packet, &mut offset, DHCP_OPTION_MESSAGE_TYPE, &[DHCP_REQUEST])?;
    write_option(&mut packet, &mut offset, DHCP_OPTION_REQUESTED_IP, &requested.0)?;
    write_option(&mut packet, &mut offset, DHCP_OPTION_SERVER_ID, &server.0)?;
    write_option(
        &mut packet,
        &mut offset,
        DHCP_OPTION_CLIENT_ID,
        &[
            1,
            client_mac.0[0],
            client_mac.0[1],
            client_mac.0[2],
            client_mac.0[3],
            client_mac.0[4],
            client_mac.0[5],
        ],
    )?;
    write_option(
        &mut packet,
        &mut offset,
        DHCP_OPTION_PARAMETER_REQUEST,
        &[
            DHCP_OPTION_SUBNET_MASK,
            DHCP_OPTION_ROUTER,
            6,
            DHCP_OPTION_LEASE_TIME,
            DHCP_OPTION_T1,
            DHCP_OPTION_T2,
        ],
    )?;
    packet[offset] = 255;
    offset += 1;
    if output.len() < offset {
        return Err(Error::Short);
    }
    output[..offset].copy_from_slice(&packet[..offset]);
    Ok(offset)
}

fn encode_dhcp_header(packet: &mut [u8; 300], xid: u32, client_mac: Mac) {
    packet.fill(0);
    packet[0] = 1;
    packet[1] = 1;
    packet[2] = 6;
    packet[4..8].copy_from_slice(&xid.to_be_bytes());
    packet[10..12].copy_from_slice(&0x8000u16.to_be_bytes());
    packet[28..34].copy_from_slice(&client_mac.0);
    packet[236..240].copy_from_slice(&[99, 130, 83, 99]);
}

fn write_option(
    packet: &mut [u8; 300],
    offset: &mut usize,
    code: u8,
    value: &[u8],
) -> Result<(), Error> {
    if value.len() > 255 {
        return Err(Error::TooLarge);
    }
    let end = offset
        .checked_add(2)
        .and_then(|end| end.checked_add(value.len()))
        .ok_or(Error::TooLarge)?;
    if end > packet.len() {
        return Err(Error::TooLarge);
    }
    packet[*offset] = code;
    packet[*offset + 1] = value.len() as u8;
    packet[*offset + 2..end].copy_from_slice(value);
    *offset = end;
    Ok(())
}

pub const fn checksum(bytes: &[u8]) -> u16 {
    let mut sum = 0u32;
    let mut index = 0;
    while index + 1 < bytes.len() {
        sum += u16::from_be_bytes([bytes[index], bytes[index + 1]]) as u32;
        index += 2;
    }
    if index < bytes.len() {
        sum += (bytes[index] as u32) << 8;
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

const fn checksum_bytes(bytes: &[u8]) -> [u8; 2] {
    checksum(bytes).to_be_bytes()
}

fn pseudo_checksum(source: Ipv4, destination: Ipv4, protocol: u8, payload: &[u8]) -> u16 {
    let mut sum = u32::from(u16::from_be_bytes([source.0[0], source.0[1]]))
        + u32::from(u16::from_be_bytes([source.0[2], source.0[3]]))
        + u32::from(u16::from_be_bytes([destination.0[0], destination.0[1]]))
        + u32::from(u16::from_be_bytes([destination.0[2], destination.0[3]]))
        + u32::from(protocol)
        + u32::try_from(payload.len()).unwrap_or(u32::MAX);
    let mut index = 0;
    while index + 1 < payload.len() {
        sum += u32::from(u16::from_be_bytes([payload[index], payload[index + 1]]));
        index += 2;
    }
    if index < payload.len() {
        sum += u32::from(payload[index]) << 8;
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StateError {
    Full,
    AddressInUse,
    Invalid,
    Busy,
    NotFound,
    NoData,
    MessageTooLarge,
    Stale,
    Owner,
    QueueFull,
    NoRoute,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EndpointId(u32);

impl EndpointId {
    pub const fn from_wire(value: u32) -> Option<Self> {
        let endpoint = Self(value);
        if endpoint.generation() != 0 && endpoint.slot() < MAX_TCP_ENDPOINTS {
            Some(endpoint)
        } else {
            None
        }
    }

    pub const fn wire(self) -> u32 {
        self.0
    }

    pub const fn slot(self) -> usize {
        (self.0 as u16).wrapping_sub(1) as usize
    }

    pub const fn generation(self) -> u16 {
        (self.0 >> 16) as u16
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Receive<'a> {
    pub source: Ipv4,
    pub source_port: u16,
    pub payload: &'a [u8],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PendingKind {
    Send,
    Receive,
    Echo,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Pending {
    pub id: u32,
    pub endpoint: EndpointId,
    pub kind: PendingKind,
    pub deadline: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EchoMatch {
    pub peer: Ipv4,
    pub identifier: u16,
    pub sequence: u16,
    pub generation: u16,
    pub deadline: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NetworkConfig {
    pub address: Ipv4,
    pub mask: Ipv4,
    pub router: Option<Ipv4>,
    pub lease_until: u64,
    pub renew_at: u64,
    pub rebind_at: u64,
}

pub fn route_target(
    local: Ipv4,
    mask: Ipv4,
    router: Option<Ipv4>,
    destination: Ipv4,
) -> Result<Ipv4, StateError> {
    let local = u32::from_be_bytes(local.0);
    let mask = u32::from_be_bytes(mask.0);
    let destination_value = u32::from_be_bytes(destination.0);
    if local & mask == destination_value & mask {
        Ok(destination)
    } else {
        router.ok_or(StateError::NoRoute)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DhcpPhase {
    Init,
    Selecting,
    Requesting,
    Bound,
    Renewing,
    Rebinding,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DhcpAction {
    None,
    Discover,
    Request,
    Renew,
    Rebind,
    Expired,
    ArpReply,
    IcmpReply,
    TcpReply,
}

#[derive(Clone, Copy)]
struct DhcpMachine {
    phase: DhcpPhase,
    xid: u32,
    deadline: u64,
    retry: u8,
    config: Option<NetworkConfig>,
}

impl DhcpMachine {
    const fn new() -> Self {
        Self { phase: DhcpPhase::Init, xid: 0, deadline: 0, retry: 0, config: None }
    }

    fn start(&mut self, now: u64, xid: u32) {
        self.phase = DhcpPhase::Selecting;
        self.xid = xid;
        self.retry = 0;
        self.deadline = now;
        self.config = None;
    }

    fn offer(&mut self, now: u64, xid: u32) -> bool {
        if self.phase != DhcpPhase::Selecting || self.xid != xid {
            return false;
        }
        self.phase = DhcpPhase::Requesting;
        self.retry = 0;
        self.deadline = now;
        true
    }

    fn acknowledge(&mut self, now: u64, xid: u32, config: NetworkConfig) -> bool {
        if !matches!(self.phase, DhcpPhase::Requesting | DhcpPhase::Renewing | DhcpPhase::Rebinding)
            || self.xid != xid
            || config.address.0 == [0; 4]
            || config.mask.0 == [0; 4]
            || config.lease_until <= now
        {
            return false;
        }
        self.phase = DhcpPhase::Bound;
        self.deadline = config.renew_at;
        self.config = Some(config);
        true
    }

    fn nak(&mut self) {
        self.phase = DhcpPhase::Init;
        self.deadline = 0;
        self.retry = 0;
        self.config = None;
    }

    fn tick(&mut self, now: u64) -> DhcpAction {
        if now < self.deadline {
            return DhcpAction::None;
        }
        match self.phase {
            DhcpPhase::Init => DhcpAction::None,
            DhcpPhase::Selecting => {
                let action = DhcpAction::Discover;
                self.retry = self.retry.saturating_add(1).min(4);
                self.deadline = now.saturating_add(1u64 << self.retry.min(3));
                action
            }
            DhcpPhase::Requesting => {
                let action = DhcpAction::Request;
                self.retry = self.retry.saturating_add(1).min(4);
                self.deadline = now.saturating_add(1u64 << self.retry.min(3));
                action
            }
            DhcpPhase::Bound => {
                let config = self.config.expect("bound DHCP state has configuration");
                if now >= config.lease_until {
                    self.nak();
                    DhcpAction::Expired
                } else if now >= config.rebind_at {
                    self.phase = DhcpPhase::Rebinding;
                    self.deadline = now;
                    DhcpAction::Rebind
                } else if now >= config.renew_at {
                    self.phase = DhcpPhase::Renewing;
                    self.deadline = now;
                    DhcpAction::Renew
                } else {
                    DhcpAction::None
                }
            }
            DhcpPhase::Renewing => {
                let config = self.config.expect("renewing DHCP state has configuration");
                if now >= config.lease_until {
                    self.nak();
                    DhcpAction::Expired
                } else if now >= config.rebind_at {
                    self.phase = DhcpPhase::Rebinding;
                    self.deadline = now;
                    DhcpAction::Rebind
                } else {
                    self.deadline = now.saturating_add(1);
                    DhcpAction::Renew
                }
            }
            DhcpPhase::Rebinding => {
                let config = self.config.expect("rebinding DHCP state has configuration");
                if now >= config.lease_until {
                    self.nak();
                    DhcpAction::Expired
                } else {
                    self.deadline = now.saturating_add(1);
                    DhcpAction::Rebind
                }
            }
        }
    }
}

#[derive(Clone, Copy)]
struct Endpoint {
    owner: u64,
    port: u16,
    active: bool,
}

#[derive(Clone, Copy)]
struct Datagram {
    endpoint: EndpointId,
    source: Ipv4,
    source_port: u16,
    length: u16,
    bytes: [u8; MAX_UDP_PAYLOAD],
    active: bool,
}

impl Datagram {
    const EMPTY: Self = Self {
        endpoint: EndpointId(0),
        source: Ipv4([0; 4]),
        source_port: 0,
        length: 0,
        bytes: [0; MAX_UDP_PAYLOAD],
        active: false,
    };
}

#[derive(Clone, Copy)]
struct ArpEntry {
    ip: Ipv4,
    mac: Mac,
    expires: u64,
    active: bool,
}

pub struct NetworkState {
    generation: u16,
    endpoints: [Endpoint; 8],
    datagrams: [Datagram; 4],
    arp: [ArpEntry; 8],
    pending: Option<Pending>,
    echo: Option<EchoMatch>,
    arp_target: Option<Ipv4>,
    dhcp: DhcpMachine,
    tcp: TcpState,
}

impl NetworkState {
    pub const fn new() -> Self {
        const ENDPOINT: Endpoint = Endpoint { owner: 0, port: 0, active: false };
        const ARP: ArpEntry =
            ArpEntry { ip: Ipv4([0; 4]), mac: Mac([0; 6]), expires: 0, active: false };
        Self {
            generation: 1,
            endpoints: [ENDPOINT; 8],
            datagrams: [Datagram::EMPTY; 4],
            arp: [ARP; 8],
            pending: None,
            echo: None,
            arp_target: None,
            dhcp: DhcpMachine::new(),
            tcp: TcpState::new(),
        }
    }

    pub const fn generation(&self) -> u16 {
        self.generation
    }

    pub const fn tcp(&self) -> &TcpState {
        &self.tcp
    }

    pub fn tcp_mut(&mut self) -> &mut TcpState {
        &mut self.tcp
    }

    pub const fn pending(&self) -> Option<Pending> {
        self.pending
    }

    pub const fn echo(&self) -> Option<EchoMatch> {
        self.echo
    }

    pub const fn arp_target(&self) -> Option<Ipv4> {
        self.arp_target
    }

    pub fn bind(&mut self, owner: u64, port: u16) -> Result<EndpointId, StateError> {
        if port == 0 {
            return Err(StateError::Invalid);
        }
        if self.endpoints.iter().any(|endpoint| endpoint.active && endpoint.port == port) {
            return Err(StateError::AddressInUse);
        }
        let (slot, endpoint) = self
            .endpoints
            .iter_mut()
            .enumerate()
            .find(|(_, endpoint)| !endpoint.active)
            .ok_or(StateError::Full)?;
        endpoint.owner = owner;
        endpoint.port = port;
        endpoint.active = true;
        Ok(EndpointId((u32::from(self.generation) << 16) | (slot as u32 + 1)))
    }

    pub fn endpoint_for_port(&self, port: u16) -> Option<EndpointId> {
        self.endpoints.iter().enumerate().find_map(|(slot, endpoint)| {
            (endpoint.active && endpoint.port == port)
                .then(|| EndpointId((u32::from(self.generation) << 16) | (slot as u32 + 1)))
        })
    }

    pub fn endpoint_port(&self, owner: u64, endpoint: EndpointId) -> Result<u16, StateError> {
        Ok(self.endpoints[self.endpoint_slot(owner, endpoint)?].port)
    }

    pub fn close(&mut self, owner: u64, endpoint: EndpointId) -> Result<(), StateError> {
        let slot = self.endpoint_slot(owner, endpoint)?;
        self.endpoints[slot].active = false;
        for datagram in &mut self.datagrams {
            if datagram.active && datagram.endpoint == endpoint {
                datagram.active = false;
            }
        }
        if self.pending.is_some_and(|pending| pending.endpoint == endpoint) {
            self.pending = None;
        }
        Ok(())
    }

    pub fn enqueue(
        &mut self,
        endpoint: EndpointId,
        source: Ipv4,
        source_port: u16,
        payload: &[u8],
    ) -> Result<(), StateError> {
        if payload.len() > MAX_UDP_PAYLOAD {
            return Err(StateError::MessageTooLarge);
        }
        self.endpoint_slot_any(endpoint)?;
        let datagram = self
            .datagrams
            .iter_mut()
            .find(|datagram| !datagram.active)
            .ok_or(StateError::QueueFull)?;
        datagram.endpoint = endpoint;
        datagram.source = source;
        datagram.source_port = source_port;
        datagram.length = payload.len() as u16;
        datagram.bytes[..payload.len()].copy_from_slice(payload);
        datagram.active = true;
        Ok(())
    }

    pub fn receive<'a>(
        &'a mut self,
        owner: u64,
        endpoint: EndpointId,
        output: &'a mut [u8],
    ) -> Result<Receive<'a>, StateError> {
        self.endpoint_slot(owner, endpoint)?;
        let index = self
            .datagrams
            .iter()
            .position(|datagram| datagram.active && datagram.endpoint == endpoint)
            .ok_or(StateError::NoData)?;
        let datagram = &mut self.datagrams[index];
        let length = usize::from(datagram.length);
        if output.len() < length {
            return Err(StateError::MessageTooLarge);
        }
        output[..length].copy_from_slice(&datagram.bytes[..length]);
        datagram.active = false;
        Ok(Receive {
            source: datagram.source,
            source_port: datagram.source_port,
            payload: &output[..length],
        })
    }

    pub fn begin_pending(&mut self, pending: Pending) -> Result<(), StateError> {
        if self.pending.is_some() || pending.endpoint.0 == 0 {
            return Err(StateError::Busy);
        }
        self.endpoint_slot_any(pending.endpoint)?;
        if pending.id == 0 || pending.deadline == 0 {
            return Err(StateError::Invalid);
        }
        self.pending = Some(pending);
        Ok(())
    }

    pub fn begin_echo(&mut self, echo: EchoMatch) -> Result<(), StateError> {
        if echo.peer.0 == [0; 4] || echo.generation != self.generation {
            return Err(StateError::Invalid);
        }
        if echo.identifier == 0 || echo.deadline == 0 {
            return Err(StateError::Invalid);
        }
        if self.echo.is_some() {
            return Err(StateError::Busy);
        }
        self.echo = Some(echo);
        Ok(())
    }

    pub fn finish_echo(&mut self, peer: Ipv4, identifier: u16, sequence: u16) -> bool {
        if self.echo.is_some_and(|echo| {
            echo.peer == peer
                && echo.identifier == identifier
                && echo.sequence == sequence
                && echo.generation == self.generation
        }) {
            self.echo = None;
            true
        } else {
            false
        }
    }

    pub fn expire_echo(&mut self, now: u64) -> bool {
        if self.echo.is_some_and(|echo| now >= echo.deadline) {
            self.echo = None;
            true
        } else {
            false
        }
    }

    pub fn expect_arp(&mut self, ip: Ipv4) -> bool {
        if ip.0 == [0; 4] || self.arp_target.is_some() {
            return false;
        }
        self.arp_target = Some(ip);
        true
    }

    pub fn finish_pending(&mut self, id: u32) -> Result<Pending, StateError> {
        if self.pending.is_some_and(|pending| pending.id == id) {
            return self.pending.take().ok_or(StateError::NotFound);
        }
        Err(StateError::NotFound)
    }

    pub fn cancel_pending(&mut self, id: u32) -> Result<Pending, StateError> {
        self.finish_pending(id)
    }

    pub fn learn_arp(&mut self, ip: Ipv4, mac: Mac, now: u64, ttl: u64) {
        let index = self
            .arp
            .iter()
            .position(|entry| entry.active && entry.ip == ip)
            .or_else(|| self.arp.iter().position(|entry| !entry.active || entry.expires <= now))
            .unwrap_or_else(|| {
                self.arp
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, entry)| entry.expires)
                    .map_or(0, |(index, _)| index)
            });
        let entry = &mut self.arp[index];
        *entry = ArpEntry { ip, mac, expires: now.saturating_add(ttl), active: true };
        self.arp_target = None;
    }

    pub fn learn_arp_reply(&mut self, ip: Ipv4, mac: Mac, now: u64, ttl: u64) -> bool {
        if self.arp_target != Some(ip) {
            return false;
        }
        self.learn_arp(ip, mac, now, ttl);
        true
    }

    pub fn resolve_arp(&self, ip: Ipv4, now: u64) -> Option<Mac> {
        self.arp
            .iter()
            .find(|entry| entry.active && entry.ip == ip && entry.expires > now)
            .map(|entry| entry.mac)
    }

    pub fn endpoint_count(&self) -> usize {
        self.endpoints.iter().filter(|endpoint| endpoint.active).count()
    }

    pub fn datagram_count(&self) -> usize {
        self.datagrams.iter().filter(|datagram| datagram.active).count()
    }

    pub fn arp_count(&self, now: u64) -> usize {
        self.arp.iter().filter(|entry| entry.active && entry.expires > now).count()
    }

    pub fn reset(&mut self) {
        self.generation = self.generation.wrapping_add(1).max(1);
        for endpoint in &mut self.endpoints {
            endpoint.active = false;
        }
        for datagram in &mut self.datagrams {
            datagram.active = false;
        }
        for entry in &mut self.arp {
            entry.active = false;
        }
        self.pending = None;
        self.echo = None;
        self.arp_target = None;
        self.dhcp = DhcpMachine::new();
        self.tcp.reset();
    }

    pub fn dhcp_start(&mut self, now: u64, xid: u32) {
        self.dhcp.start(now, xid);
    }

    pub const fn dhcp_phase(&self) -> DhcpPhase {
        self.dhcp.phase
    }

    pub const fn dhcp_xid(&self) -> u32 {
        self.dhcp.xid
    }

    pub const fn dhcp_config(&self) -> Option<NetworkConfig> {
        self.dhcp.config
    }

    pub const fn dhcp_deadline(&self) -> u64 {
        self.dhcp.deadline
    }

    pub fn dhcp_offer(&mut self, now: u64, xid: u32) -> bool {
        self.dhcp.offer(now, xid)
    }

    pub fn dhcp_acknowledge(&mut self, now: u64, xid: u32, config: NetworkConfig) -> bool {
        self.dhcp.acknowledge(now, xid, config)
    }

    pub fn dhcp_nak(&mut self) {
        self.dhcp.nak();
        self.reset_protocol_state();
    }

    pub fn dhcp_tick(&mut self, now: u64) -> DhcpAction {
        let action = self.dhcp.tick(now);
        if action == DhcpAction::Expired {
            self.reset_protocol_state();
        }
        action
    }

    fn reset_protocol_state(&mut self) {
        for endpoint in &mut self.endpoints {
            endpoint.active = false;
        }
        for datagram in &mut self.datagrams {
            datagram.active = false;
        }
        for entry in &mut self.arp {
            entry.active = false;
        }
        self.pending = None;
        self.echo = None;
        self.arp_target = None;
    }

    fn endpoint_slot(&self, owner: u64, endpoint: EndpointId) -> Result<usize, StateError> {
        let slot = self.endpoint_slot_any(endpoint)?;
        if self.endpoints[slot].owner != owner {
            return Err(StateError::Owner);
        }
        Ok(slot)
    }

    fn endpoint_slot_any(&self, endpoint: EndpointId) -> Result<usize, StateError> {
        if endpoint.generation() != self.generation || endpoint.slot() >= self.endpoints.len() {
            return Err(StateError::Stale);
        }
        let slot = endpoint.slot();
        self.endpoints[slot].active.then_some(slot).ok_or(StateError::NotFound)
    }
}

impl Default for NetworkState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOCAL_MAC: Mac = Mac([2, 0, 0, 0, 0, 1]);
    const LOCAL_IP: Ipv4 = Ipv4([192, 0, 2, 2]);
    const PEER_IP: Ipv4 = Ipv4([192, 0, 2, 1]);

    #[test]
    fn ethernet_and_arp_round_trip() {
        let arp = Arp {
            reply: true,
            sender_mac: LOCAL_MAC,
            sender_ip: LOCAL_IP,
            target_mac: Mac::BROADCAST,
            target_ip: PEER_IP,
        };
        let mut payload = [0; 28];
        let length = encode_arp(&mut payload, arp).unwrap();
        assert_eq!(parse_arp(&payload[..length]).unwrap(), arp);
        let mut frame = [0; ETHERNET_MAX_FRAME];
        let length = encode_ethernet(&mut frame, LOCAL_MAC, LOCAL_MAC, 0x0806, &payload).unwrap();
        assert_eq!(parse_ethernet(&frame[..length], LOCAL_MAC).unwrap().ether_type, 0x0806);
        assert_eq!(parse_ethernet(&frame[..length], Mac([9; 6])), Err(Error::Destination));
    }

    #[test]
    fn independent_rfc_wire_vectors_parse() {
        let ethernet_arp = [
            0x02, 0x00, 0x00, 0x00, 0x00, 0x01, 0x02, 0x00, 0x00, 0x00, 0x00, 0x02, 0x08, 0x06,
            0x00, 0x01, 0x08, 0x00, 0x06, 0x04, 0x00, 0x01, 0x02, 0x00, 0x00, 0x00, 0x00, 0x01,
            0xc0, 0x00, 0x02, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xc0, 0x00, 0x02, 0x02,
        ];
        let ethernet = parse_ethernet(&ethernet_arp, LOCAL_MAC).unwrap();
        let arp = parse_arp(ethernet.payload).unwrap();
        assert!(!arp.reply);
        assert_eq!(arp.sender_ip, PEER_IP);
        assert_eq!(arp.target_ip, LOCAL_IP);

        let ethernet_udp = [
            0x02, 0x00, 0x00, 0x00, 0x00, 0x01, 0x02, 0x00, 0x00, 0x00, 0x00, 0x02, 0x08, 0x00,
            0x45, 0x00, 0x00, 0x21, 0x00, 0x00, 0x00, 0x00, 0x40, 0x11, 0xf6, 0xc8, 0xc0, 0x00,
            0x02, 0x01, 0xc0, 0x00, 0x02, 0x02, 0x0f, 0xa1, 0x0f, 0xa0, 0x00, 0x0d, 0x18, 0xbd,
            0x68, 0x65, 0x6c, 0x6c, 0x6f,
        ];
        let ethernet = parse_ethernet(&ethernet_udp, LOCAL_MAC).unwrap();
        let ip = parse_ipv4(ethernet.payload, LOCAL_IP).unwrap();
        let udp = parse_udp(ip.payload, ip.source, ip.destination).unwrap();
        assert_eq!(udp.source_port, 4001);
        assert_eq!(udp.destination_port, 4000);
        assert_eq!(udp.payload, b"hello");

        let ethernet_icmp = [
            0x02, 0x00, 0x00, 0x00, 0x00, 0x01, 0x02, 0x00, 0x00, 0x00, 0x00, 0x02, 0x08, 0x00,
            0x45, 0x00, 0x00, 0x20, 0x00, 0x00, 0x00, 0x00, 0x40, 0x01, 0xf6, 0xd9, 0xc0, 0x00,
            0x02, 0x01, 0xc0, 0x00, 0x02, 0x02, 0x08, 0x00, 0x19, 0x27, 0x00, 0x07, 0x00, 0x01,
            0x70, 0x69, 0x6e, 0x67,
        ];
        let ethernet = parse_ethernet(&ethernet_icmp, LOCAL_MAC).unwrap();
        let ip = parse_ipv4(ethernet.payload, LOCAL_IP).unwrap();
        let icmp = parse_icmp_echo(ip.payload).unwrap();
        assert!(!icmp.reply);
        assert_eq!(icmp.identifier, 7);
        assert_eq!(icmp.sequence, 1);
        assert_eq!(icmp.payload, b"ping");
    }

    #[test]
    fn ipv4_udp_and_icmp_round_trip() {
        let mut udp = [0; UDP_HEADER + MAX_UDP_PAYLOAD];
        let udp_length = encode_udp(&mut udp, LOCAL_IP, PEER_IP, 4000, 4001, b"hello").unwrap();
        let mut ip = [0; IPV4_HEADER + UDP_HEADER + 5];
        let ip_length = encode_ipv4(&mut ip, LOCAL_IP, PEER_IP, 7, 17, &udp[..udp_length]).unwrap();
        assert_eq!(parse_ipv4(&ip[..ip_length], PEER_IP).unwrap().payload.len(), udp_length);
        assert_eq!(parse_udp(&udp[..udp_length], LOCAL_IP, PEER_IP).unwrap().payload, b"hello");
        let mut icmp = [0; 16];
        let icmp_length = encode_icmp_echo(&mut icmp, false, 3, 4, b"ping").unwrap();
        assert_eq!(parse_icmp_echo(&icmp[..icmp_length]).unwrap().sequence, 4);
        assert_eq!(parse_icmp_echo(&icmp[..icmp_length]).unwrap().payload, b"ping");
    }

    #[test]
    fn tcp_round_trip_rejects_truncation_and_tampering() {
        let mut bytes = [0; TCP_HEADER + 5];
        let length = encode_tcp(
            &mut bytes,
            LOCAL_IP,
            PEER_IP,
            TcpHeader {
                source_port: 7443,
                destination_port: 50000,
                sequence: 7,
                acknowledgement: 3,
                flags: 0x18,
                window: 1024,
            },
            b"hello",
        )
        .unwrap();
        let tcp = parse_tcp(&bytes[..length], LOCAL_IP, PEER_IP).unwrap();
        assert_eq!(tcp.payload, b"hello");
        assert_eq!(tcp.flags, 0x18);
        for prefix in 0..length {
            assert!(parse_tcp(&bytes[..prefix], LOCAL_IP, PEER_IP).is_err());
        }
        bytes[19] ^= 1;
        assert_eq!(parse_tcp(&bytes[..length], LOCAL_IP, PEER_IP), Err(Error::Checksum));
    }

    #[test]
    fn tcp_stream_handshake_owner_and_bounded_io() {
        let mut state = TcpState::new();
        let listener = state.listen(7, 7443, 100).unwrap();
        state
            .ingest(
                PEER_IP,
                Tcp {
                    source_port: 50000,
                    destination_port: 7443,
                    sequence: 200,
                    acknowledgement: 0,
                    flags: TCP_FLAG_SYN,
                    window: 1024,
                    payload: &[],
                },
            )
            .unwrap();
        let syn_ack = state.take_tx().unwrap();
        assert_eq!(syn_ack.header.flags, TCP_FLAG_SYN | TCP_FLAG_ACK);
        state
            .ingest(
                PEER_IP,
                Tcp {
                    source_port: 50000,
                    destination_port: 7443,
                    sequence: 201,
                    acknowledgement: 101,
                    flags: TCP_FLAG_ACK,
                    window: 1024,
                    payload: &[],
                },
            )
            .unwrap();
        let stream = state.accept(7, listener).unwrap();
        assert_eq!(state.accept(8, listener), Err(TcpStateError::Owner));
        state
            .ingest(
                PEER_IP,
                Tcp {
                    source_port: 50000,
                    destination_port: 7443,
                    sequence: 201,
                    acknowledgement: 101,
                    flags: TCP_FLAG_ACK,
                    window: 1024,
                    payload: b"hello",
                },
            )
            .unwrap();
        let mut output = [0; 5];
        assert_eq!(state.read(7, stream, &mut output), Ok(5));
        assert_eq!(&output, b"hello");
        assert_eq!(state.write(7, stream, b"pong"), Ok(()));
        assert_eq!(state.write(7, stream, b"again"), Err(TcpStateError::Busy));
        state.reset();
        assert_ne!(state.generation(), 1);
        assert_eq!(state.read(7, stream, &mut output), Err(TcpStateError::NotFound));
    }

    #[test]
    fn tcp_state_tracks_sequence_and_acknowledgement_numbers() {
        let mut state = TcpState::new();
        let listener = state.listen(7, 7443, 100).unwrap();

        state
            .ingest(
                PEER_IP,
                Tcp {
                    source_port: 50000,
                    destination_port: 7443,
                    sequence: 200,
                    acknowledgement: 0,
                    flags: TCP_FLAG_SYN,
                    window: 1024,
                    payload: &[],
                },
            )
            .unwrap();
        let syn_ack = state.take_tx().unwrap();
        assert_eq!(syn_ack.header.sequence, 100);
        assert_eq!(syn_ack.header.acknowledgement, 201);
        assert_eq!(syn_ack.header.flags, TCP_FLAG_SYN | TCP_FLAG_ACK);

        state
            .ingest(
                PEER_IP,
                Tcp {
                    source_port: 50000,
                    destination_port: 7443,
                    sequence: 201,
                    acknowledgement: 101,
                    flags: TCP_FLAG_ACK,
                    window: 1024,
                    payload: &[],
                },
            )
            .unwrap();
        assert_eq!(state.take_tx(), None);
        let stream = state.accept(7, listener).unwrap();
        assert_eq!(state.stream_state(7, stream), Ok((STREAM_WRITABLE, 0, 0)));

        state
            .ingest(
                PEER_IP,
                Tcp {
                    source_port: 50000,
                    destination_port: 7443,
                    sequence: 201,
                    acknowledgement: 101,
                    flags: TCP_FLAG_ACK,
                    window: 1024,
                    payload: b"hello",
                },
            )
            .unwrap();
        let payload_ack = state.take_tx().unwrap();
        assert_eq!(payload_ack.header.sequence, 101);
        assert_eq!(payload_ack.header.acknowledgement, 206);
        assert_eq!(payload_ack.header.flags, TCP_FLAG_ACK);
        assert_eq!(state.stream_state(7, stream), Ok((STREAM_READABLE | STREAM_WRITABLE, 0, 0)));

        let mut output = [0; 5];
        assert_eq!(state.read(7, stream, &mut output), Ok(5));
        assert_eq!(&output, b"hello");
        assert_eq!(state.stream_state(7, stream), Ok((STREAM_WRITABLE, 0, 0)));
        state.write(7, stream, b"world").unwrap();
        let server_write = state.take_tx().unwrap();
        assert_eq!(server_write.header.sequence, 101);
        assert_eq!(server_write.header.acknowledgement, 206);
        assert_eq!(server_write.header.flags, TCP_FLAG_ACK);
        assert_eq!(&server_write.payload[..usize::from(server_write.length)], b"world");
        assert_eq!(state.stream_state(7, stream), Ok((STREAM_WRITABLE, 5, 0)));

        state
            .ingest(
                PEER_IP,
                Tcp {
                    source_port: 50000,
                    destination_port: 7443,
                    sequence: 206,
                    acknowledgement: 106,
                    flags: TCP_FLAG_ACK,
                    window: 1024,
                    payload: &[],
                },
            )
            .unwrap();
        assert_eq!(state.take_tx(), None);
        assert_eq!(state.stream_state(7, stream), Ok((STREAM_WRITABLE, 5, 5)));

        state.write(7, stream, b"again").unwrap();
        let second_write = state.take_tx().unwrap();
        assert_eq!(second_write.header.sequence, 106);
        assert_eq!(second_write.header.acknowledgement, 206);
        assert_eq!(&second_write.payload[..usize::from(second_write.length)], b"again");

        for _ in 0..2 {
            state
                .ingest(
                    PEER_IP,
                    Tcp {
                        source_port: 50000,
                        destination_port: 7443,
                        sequence: 206,
                        acknowledgement: 111,
                        flags: TCP_FLAG_ACK,
                        window: 1024,
                        payload: &[],
                    },
                )
                .unwrap();
            assert_eq!(state.take_tx(), None);
        }

        state
            .ingest(
                PEER_IP,
                Tcp {
                    source_port: 50000,
                    destination_port: 7443,
                    sequence: 206,
                    acknowledgement: 111,
                    flags: TCP_FLAG_FIN | TCP_FLAG_ACK,
                    window: 1024,
                    payload: &[],
                },
            )
            .unwrap();
        let fin_ack = state.take_tx().unwrap();
        assert_eq!(fin_ack.header.sequence, 111);
        assert_eq!(fin_ack.header.acknowledgement, 207);
        assert_eq!(fin_ack.header.flags, TCP_FLAG_ACK);
        assert_eq!(state.stream_state(7, stream), Ok((STREAM_CLOSED, 10, 10)));
    }

    #[test]
    fn tcp_state_retransmits_boundedly_and_resets_on_rst() {
        let mut state = TcpState::new();
        let listener = state.listen(7, 7443, 100).unwrap();
        state
            .ingest(
                PEER_IP,
                Tcp {
                    source_port: 50000,
                    destination_port: 7443,
                    sequence: 200,
                    acknowledgement: 0,
                    flags: TCP_FLAG_SYN,
                    window: 1024,
                    payload: &[],
                },
            )
            .unwrap();
        let syn_ack = state.take_tx().unwrap();
        assert!(!state.tick(100));
        assert!(state.tick(101));
        assert_eq!(state.take_tx(), Some(syn_ack));
        assert!(state.tick(103));
        assert_eq!(state.take_tx(), Some(syn_ack));
        assert!(state.tick(107));
        assert_eq!(state.take_tx(), Some(syn_ack));
        assert!(!state.tick(115));
        assert_eq!(state.accept(7, listener), Err(TcpStateError::NoData));

        let mut state = TcpState::new();
        let listener = state.listen(7, 7443, 100).unwrap();
        state
            .ingest(
                PEER_IP,
                Tcp {
                    source_port: 50000,
                    destination_port: 7443,
                    sequence: 200,
                    acknowledgement: 0,
                    flags: TCP_FLAG_SYN,
                    window: 1024,
                    payload: &[],
                },
            )
            .unwrap();
        let _ = state.take_tx();
        assert_eq!(
            state.ingest(
                PEER_IP,
                Tcp {
                    source_port: 50000,
                    destination_port: 7443,
                    sequence: 201,
                    acknowledgement: 101,
                    flags: TCP_FLAG_ACK | TCP_FLAG_RST,
                    window: 1024,
                    payload: &[],
                },
            ),
            Err(TcpStateError::Reset)
        );
        assert_eq!(state.accept(7, listener), Err(TcpStateError::NoData));
    }

    #[test]
    fn tcp_stream_writes_are_byte_queued_and_connection_local() {
        let mut state = TcpState::new();
        let listener = state.listen(7, 7443, 100).unwrap();
        for (port, sequence) in [(50000, 200), (50001, 300)] {
            state
                .ingest(
                    PEER_IP,
                    Tcp {
                        source_port: port,
                        destination_port: 7443,
                        sequence,
                        acknowledgement: 0,
                        flags: TCP_FLAG_SYN,
                        window: 1024,
                        payload: &[],
                    },
                )
                .unwrap();
            let _ = state.take_tx();
            state
                .ingest(
                    PEER_IP,
                    Tcp {
                        source_port: port,
                        destination_port: 7443,
                        sequence: sequence + 1,
                        acknowledgement: if port == 50000 { 101 } else { 102 },
                        flags: TCP_FLAG_ACK,
                        window: 1024,
                        payload: &[],
                    },
                )
                .unwrap();
        }
        let first = state.accept(7, listener).unwrap();
        let second = state.accept(7, listener).unwrap();
        assert_ne!(first, second);

        assert_eq!(state.submit_write(7, first, b"abc"), Ok(3));
        assert_eq!(state.submit_write(7, first, b"def"), Ok(6));
        let first_tx = state.take_tx().unwrap();
        assert_eq!(&first_tx.payload[..usize::from(first_tx.length)], b"abc");
        assert_eq!(state.stream_watermarks(7, first), Ok((6, 0)));
        assert_eq!(state.submit_write(7, second, b"peer"), Ok(4));
        assert_eq!(state.submit_write(8, first, b"x"), Err(TcpStateError::Owner));

        state
            .ingest(
                PEER_IP,
                Tcp {
                    source_port: 50000,
                    destination_port: 7443,
                    sequence: 201,
                    acknowledgement: first_tx.header.sequence + u32::from(first_tx.length),
                    flags: TCP_FLAG_ACK,
                    window: 1024,
                    payload: &[],
                },
            )
            .unwrap();
        let mut second_tx = state.take_tx().unwrap();
        if &second_tx.payload[..usize::from(second_tx.length)] == b"peer" {
            state
                .ingest(
                    PEER_IP,
                    Tcp {
                        source_port: 50001,
                        destination_port: 7443,
                        sequence: 301,
                        acknowledgement: second_tx.header.sequence + u32::from(second_tx.length),
                        flags: TCP_FLAG_ACK,
                        window: 1024,
                        payload: &[],
                    },
                )
                .unwrap();
            second_tx = state.take_tx().unwrap();
        }
        assert_eq!(&second_tx.payload[..usize::from(second_tx.length)], b"def");
        state
            .ingest(
                PEER_IP,
                Tcp {
                    source_port: 50000,
                    destination_port: 7443,
                    sequence: 201,
                    acknowledgement: second_tx.header.sequence + u32::from(second_tx.length),
                    flags: TCP_FLAG_ACK,
                    window: 1024,
                    payload: &[],
                },
            )
            .unwrap();
        assert_eq!(state.stream_watermarks(7, first), Ok((6, 6)));
        assert_eq!(state.stream_watermarks(7, second), Ok((4, 4)));
    }

    #[test]
    fn listener_and_connection_tables_have_independent_bounds() {
        let mut state = TcpState::new();
        let listener = state.listen(7, 7443, 100).unwrap();
        assert_eq!(state.listen(7, 7444, 100), Err(TcpStateError::Full));
        for port in 50000..50000 + MAX_TCP_CONNECTIONS as u16 {
            assert_eq!(
                state.ingest(
                    PEER_IP,
                    Tcp {
                        source_port: port,
                        destination_port: 7443,
                        sequence: 200,
                        acknowledgement: 0,
                        flags: TCP_FLAG_SYN,
                        window: 1024,
                        payload: &[],
                    },
                ),
                Ok(())
            );
            let _ = state.take_tx();
        }
        assert_eq!(state.listener_count(), 1);
        assert_eq!(state.connection_count(), MAX_TCP_CONNECTIONS);
        assert_eq!(
            state.ingest(
                PEER_IP,
                Tcp {
                    source_port: 51000,
                    destination_port: 7443,
                    sequence: 200,
                    acknowledgement: 0,
                    flags: TCP_FLAG_SYN,
                    window: 1024,
                    payload: &[],
                },
            ),
            Err(TcpStateError::Full)
        );
        assert_eq!(listener.slot(), 0);
    }

    #[test]
    fn strict_prefixes_fail_and_bad_protocol_fields_do_not_parse() {
        let mut udp = [0; UDP_HEADER + 5];
        let length = encode_udp(&mut udp, LOCAL_IP, PEER_IP, 1, 2, b"hello").unwrap();
        for prefix in 0..length {
            assert!(parse_udp(&udp[..prefix], LOCAL_IP, PEER_IP).is_err());
        }
        let mut ip = [0; IPV4_HEADER + UDP_HEADER + 5];
        let length = encode_ipv4(&mut ip, LOCAL_IP, PEER_IP, 1, 17, &udp[..]).unwrap();
        for prefix in 0..length {
            assert!(parse_ipv4(&ip[..prefix], PEER_IP).is_err());
        }
        ip[8] = 0;
        assert_eq!(parse_ipv4(&ip, PEER_IP), Err(Error::Checksum));
        udp[4..6].copy_from_slice(&3u16.to_be_bytes());
        assert_eq!(parse_udp(&udp, LOCAL_IP, PEER_IP), Err(Error::Length));
    }

    #[test]
    fn dhcp_options_are_bounded_and_duplicates_rejected() {
        let mut packet = [0; 247];
        packet[0] = 2;
        packet[1] = 1;
        packet[2] = 6;
        packet[4..8].copy_from_slice(&7u32.to_be_bytes());
        packet[28..34].copy_from_slice(&LOCAL_MAC.0);
        packet[236..240].copy_from_slice(&[99, 130, 83, 99]);
        packet[240..247].copy_from_slice(&[53, 1, 2, 53, 1, 2, 255]);
        let dhcp = parse_dhcp(&packet).unwrap();
        assert_eq!(dhcp.option(53), Err(Error::Malformed));
        packet[240..247].copy_from_slice(&[53, 5, 2, 0, 255, 0, 0]);
        assert_eq!(parse_dhcp(&packet[..245]), Err(Error::Short));
    }

    #[test]
    fn dhcp_discover_and_request_encoders_are_bounded() {
        let mut output = [0; 300];
        let discover = encode_dhcp_discover(&mut output, 7, LOCAL_MAC).unwrap();
        assert!(discover > 240);
        assert_eq!(&output[..4], &[1, 1, 6, 0]);
        assert_eq!(&output[4..8], &7u32.to_be_bytes());
        assert_eq!(&output[28..34], &LOCAL_MAC.0);
        assert_eq!(output[240..243], [53, 1, DHCP_DISCOVER]);
        let request = encode_dhcp_request(&mut output, 7, LOCAL_MAC, LOCAL_IP, PEER_IP).unwrap();
        assert!(request > discover);
        assert!(
            output[..request]
                .windows(3)
                .any(|window| { window == [DHCP_OPTION_MESSAGE_TYPE, 1, DHCP_REQUEST] })
        );
        assert_eq!(encode_dhcp_discover(&mut output[..240], 7, LOCAL_MAC), Err(Error::Short));
    }

    #[test]
    fn bounded_endpoints_queue_pending_and_reset() {
        let mut state = NetworkState::new();
        let endpoint = state.bind(7, 4000).unwrap();
        assert_eq!(state.bind(8, 4000), Err(StateError::AddressInUse));
        assert_eq!(state.bind(7, 0), Err(StateError::Invalid));
        state.enqueue(endpoint, PEER_IP, 4001, b"hello").unwrap();
        let mut output = [0; 5];
        let received = state.receive(7, endpoint, &mut output).unwrap();
        assert_eq!(received.source, PEER_IP);
        assert_eq!(received.payload, b"hello");
        assert_eq!(state.receive(7, endpoint, &mut output), Err(StateError::NoData));
        assert_eq!(
            state.begin_pending(Pending {
                id: 1,
                endpoint,
                kind: PendingKind::Receive,
                deadline: 10
            }),
            Ok(())
        );
        assert_eq!(
            state.begin_pending(Pending { id: 2, endpoint, kind: PendingKind::Send, deadline: 10 }),
            Err(StateError::Busy)
        );
        assert_eq!(state.cancel_pending(1).unwrap().kind, PendingKind::Receive);
        state.learn_arp(PEER_IP, Mac([3; 6]), 1, 10);
        assert_eq!(state.resolve_arp(PEER_IP, 2), Some(Mac([3; 6])));
        state.reset();
        assert_eq!(state.generation(), 2);
        assert_eq!(state.receive(7, endpoint, &mut output), Err(StateError::Stale));
    }

    #[test]
    fn routing_and_echo_matching_are_exact() {
        assert_eq!(
            route_target(LOCAL_IP, Ipv4([255, 255, 255, 0]), Some(PEER_IP), PEER_IP),
            Ok(PEER_IP)
        );
        let off_subnet = Ipv4([198, 51, 100, 1]);
        assert_eq!(
            route_target(LOCAL_IP, Ipv4([255, 255, 255, 0]), None, off_subnet),
            Err(StateError::NoRoute)
        );
        let mut state = NetworkState::new();
        let endpoint = state.bind(7, 4000).unwrap();
        assert!(
            state
                .begin_echo(EchoMatch {
                    peer: PEER_IP,
                    identifier: 1,
                    sequence: 2,
                    generation: state.generation(),
                    deadline: 10,
                })
                .is_ok()
        );
        assert!(!state.finish_echo(PEER_IP, 1, 3));
        assert!(state.finish_echo(PEER_IP, 1, 2));
        assert!(
            state
                .begin_pending(Pending {
                    id: 1,
                    endpoint,
                    kind: PendingKind::Receive,
                    deadline: 10
                })
                .is_ok()
        );
        assert!(!state.expire_echo(20));
    }

    #[test]
    fn arp_resolution_is_single_flight_and_generation_scoped() {
        let mut state = NetworkState::new();
        assert!(state.expect_arp(PEER_IP));
        assert!(!state.expect_arp(Ipv4([198, 51, 100, 1])));
        assert!(!state.learn_arp_reply(Ipv4([198, 51, 100, 1]), Mac([4; 6]), 1, 10));
        assert!(state.learn_arp_reply(PEER_IP, Mac([4; 6]), 1, 10));
        assert_eq!(state.resolve_arp(PEER_IP, 2), Some(Mac([4; 6])));
        state.reset();
        assert_eq!(state.resolve_arp(PEER_IP, 2), None);
        assert!(state.expect_arp(PEER_IP));
    }

    #[test]
    fn bounded_slots_release_on_failure_and_expire_before_live_arp() {
        let mut state = NetworkState::new();
        let mut endpoints = [EndpointId(0); 8];
        for (port, endpoint) in endpoints.iter_mut().enumerate() {
            *endpoint = state.bind(7, 4000 + port as u16).unwrap();
        }
        assert_eq!(state.endpoint_count(), 8);
        assert_eq!(state.bind(7, 5000), Err(StateError::Full));
        assert_eq!(state.close(7, endpoints[0]), Ok(()));
        assert_eq!(state.endpoint_count(), 7);
        let endpoint = state.bind(7, 5000).unwrap();
        assert_eq!(state.enqueue(endpoint, PEER_IP, 4001, &[1]), Ok(()));
        assert_eq!(state.enqueue(endpoint, PEER_IP, 4001, &[2]), Ok(()));
        assert_eq!(state.enqueue(endpoint, PEER_IP, 4001, &[3]), Ok(()));
        assert_eq!(state.enqueue(endpoint, PEER_IP, 4001, &[4]), Ok(()));
        assert_eq!(state.enqueue(endpoint, PEER_IP, 4001, &[5]), Err(StateError::QueueFull));
        assert_eq!(state.datagram_count(), 4);
        state.close(7, endpoint).unwrap();
        assert_eq!(state.datagram_count(), 0);

        for index in 0..8 {
            state.learn_arp(Ipv4([192, 0, 2, index as u8 + 10]), Mac([index as u8 + 1; 6]), 0, 10);
        }
        state.learn_arp(PEER_IP, Mac([9; 6]), 20, 10);
        assert_eq!(state.resolve_arp(PEER_IP, 21), Some(Mac([9; 6])));
        assert_eq!(state.arp_count(21), 1);
    }

    #[test]
    fn dhcp_retries_and_lease_transitions_are_bounded() {
        let mut state = NetworkState::new();
        state.dhcp_start(0, 7);
        assert_eq!(state.dhcp_tick(0), DhcpAction::Discover);
        assert!(state.dhcp_offer(2, 7));
        assert_eq!(state.dhcp_tick(2), DhcpAction::Request);
        let config = NetworkConfig {
            address: LOCAL_IP,
            mask: Ipv4([255, 255, 255, 0]),
            router: Some(PEER_IP),
            lease_until: 20,
            renew_at: 10,
            rebind_at: 17,
        };
        assert!(state.dhcp_acknowledge(3, 7, config));
        assert_eq!(state.dhcp_tick(9), DhcpAction::None);
        assert_eq!(state.dhcp_tick(10), DhcpAction::Renew);
        assert_eq!(state.dhcp_tick(17), DhcpAction::Rebind);
        assert_eq!(state.dhcp_tick(20), DhcpAction::Expired);
    }
}
