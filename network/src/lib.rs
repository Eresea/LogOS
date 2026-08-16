#![no_std]

//! Fixed Network service state. Packet parsing and device ownership stay at
//! this boundary; callers never receive a DMA address or an unbounded queue.

pub use smoltcp;
pub mod pci;

use smoltcp::{
    iface::{Config, Interface},
    phy::{Device, DeviceCapabilities, Medium, RxToken, TxToken},
    time::Instant,
    wire::{EthernetAddress, HardwareAddress, IpCidr, Ipv4Address, Ipv4Cidr},
};

/// Fixed Core/Network packet adapter. It copies frames into private storage;
/// no VirtIO DMA address is exposed to the protocol stack.
pub struct PacketDevice {
    rx: [[u8; logos_abi::NETWORK_PACKET_PAGE_BYTES]; logos_abi::NETWORK_RX_PACKET_PAGES],
    rx_lengths: [u16; logos_abi::NETWORK_RX_PACKET_PAGES],
    rx_head: usize,
    rx_tail: usize,
    tx: [[u8; logos_abi::NETWORK_PACKET_PAGE_BYTES]; logos_abi::NETWORK_TX_PACKET_PAGES],
    tx_lengths: [u16; logos_abi::NETWORK_TX_PACKET_PAGES],
    tx_head: usize,
    tx_tail: usize,
}

impl Default for PacketDevice {
    fn default() -> Self {
        Self::new()
    }
}

impl PacketDevice {
    pub const fn new() -> Self {
        Self {
            rx: [[0; logos_abi::NETWORK_PACKET_PAGE_BYTES]; logos_abi::NETWORK_RX_PACKET_PAGES],
            rx_lengths: [0; logos_abi::NETWORK_RX_PACKET_PAGES],
            rx_head: 0,
            rx_tail: 0,
            tx: [[0; logos_abi::NETWORK_PACKET_PAGE_BYTES]; logos_abi::NETWORK_TX_PACKET_PAGES],
            tx_lengths: [0; logos_abi::NETWORK_TX_PACKET_PAGES],
            tx_head: 0,
            tx_tail: 0,
        }
    }

    pub fn enqueue_rx(&mut self, frame: &[u8]) -> bool {
        if frame.len() > logos_abi::NETWORK_MAX_FRAME_BYTES
            || self.rx_head.wrapping_sub(self.rx_tail) >= self.rx.len()
        {
            return false;
        }
        let slot = self.rx_head % self.rx.len();
        self.rx[slot][..frame.len()].copy_from_slice(frame);
        self.rx_lengths[slot] = frame.len() as u16;
        self.rx_head = self.rx_head.wrapping_add(1);
        true
    }

    pub fn take_tx(&mut self, output: &mut [u8]) -> Option<usize> {
        if self.tx_tail == self.tx_head {
            return None;
        }
        let slot = self.tx_tail % self.tx.len();
        let length = usize::from(self.tx_lengths[slot]);
        if output.len() < length {
            return None;
        }
        output[..length].copy_from_slice(&self.tx[slot][..length]);
        self.tx_lengths[slot] = 0;
        self.tx_tail = self.tx_tail.wrapping_add(1);
        Some(length)
    }

    pub const fn pending_rx(&self) -> usize {
        self.rx_head.wrapping_sub(self.rx_tail)
    }
}

pub struct NetworkRxToken<'a> {
    buffer: &'a [u8],
}

impl RxToken for NetworkRxToken<'_> {
    fn consume<R, F>(self, f: F) -> R
    where
        F: FnOnce(&[u8]) -> R,
    {
        f(self.buffer)
    }
}

pub struct NetworkTxToken<'a> {
    buffer: &'a mut [u8],
    length: &'a mut u16,
    head: &'a mut usize,
}

impl TxToken for NetworkTxToken<'_> {
    fn consume<R, F>(self, length: usize, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        let result = f(&mut self.buffer[..length]);
        *self.length = length as u16;
        *self.head = (*self.head).wrapping_add(1);
        result
    }
}

impl Device for PacketDevice {
    type RxToken<'a> = NetworkRxToken<'a>;
    type TxToken<'a> = NetworkTxToken<'a>;

    fn receive(&mut self, _timestamp: Instant) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        if self.rx_tail == self.rx_head || self.tx_head.wrapping_sub(self.tx_tail) >= self.tx.len()
        {
            return None;
        }
        let rx_slot = self.rx_tail % self.rx.len();
        let tx_slot = self.tx_head % self.tx.len();
        let rx_length = usize::from(self.rx_lengths[rx_slot]);
        self.rx_tail = self.rx_tail.wrapping_add(1);
        Some((
            NetworkRxToken { buffer: &self.rx[rx_slot][..rx_length] },
            NetworkTxToken {
                buffer: &mut self.tx[tx_slot],
                length: &mut self.tx_lengths[tx_slot],
                head: &mut self.tx_head,
            },
        ))
    }

    fn transmit(&mut self, _timestamp: Instant) -> Option<Self::TxToken<'_>> {
        if self.tx_head.wrapping_sub(self.tx_tail) >= self.tx.len() {
            return None;
        }
        let tx_slot = self.tx_head % self.tx.len();
        Some(NetworkTxToken {
            buffer: &mut self.tx[tx_slot],
            length: &mut self.tx_lengths[tx_slot],
            head: &mut self.tx_head,
        })
    }

    fn capabilities(&self) -> DeviceCapabilities {
        let mut capabilities = DeviceCapabilities::default();
        capabilities.medium = Medium::Ethernet;
        capabilities.max_transmission_unit = logos_abi::NETWORK_MAX_FRAME_BYTES;
        capabilities.max_burst_size = Some(1);
        capabilities
    }
}

pub fn interface(device: &mut PacketDevice, mac: [u8; 6], now_ms: u64) -> Interface {
    let hardware = HardwareAddress::Ethernet(EthernetAddress(mac));
    Interface::new(
        Config::new(hardware),
        device,
        Instant::from_millis(now_ms.min(i64::MAX as u64) as i64),
    )
}

pub fn configure_static_ipv4(interface: &mut Interface, config: NetworkConfig) -> bool {
    let Some(prefix) = prefix_len(config.netmask) else {
        return false;
    };
    let address = Ipv4Address::from_bits(u32::from_be_bytes(config.address));
    let mut added = true;
    interface.update_ip_addrs(|addrs| {
        if addrs.push(IpCidr::Ipv4(Ipv4Cidr::new(address, prefix))).is_err() {
            added = false;
        }
    });
    if !added {
        return false;
    }
    interface
        .routes_mut()
        .add_default_ipv4_route(Ipv4Address::from_bits(u32::from_be_bytes(config.gateway)))
        .is_ok()
}

pub struct ProtocolSocketBuffers<'a> {
    pub udp_rx_metadata: &'a mut [smoltcp::socket::udp::PacketMetadata; 4],
    pub udp_rx_bytes: &'a mut [u8],
    pub udp_tx_metadata: &'a mut [smoltcp::socket::udp::PacketMetadata; 4],
    pub udp_tx_bytes: &'a mut [u8],
    pub icmp_rx_metadata: &'a mut [smoltcp::socket::icmp::PacketMetadata; 4],
    pub icmp_rx_bytes: &'a mut [u8],
    pub icmp_tx_metadata: &'a mut [smoltcp::socket::icmp::PacketMetadata; 4],
    pub icmp_tx_bytes: &'a mut [u8],
    pub tcp_rx_bytes: &'a mut [u8],
    pub tcp_tx_bytes: &'a mut [u8],
    pub dhcp_rx_bytes: &'a mut [u8],
}

/// Install one fixed ICMP, UDP, and TCP socket in the caller-owned SocketSet.
/// DHCP is created by the service only when static configuration falls back to
/// DHCP, so static startup does not emit an unsolicited discover packet.
pub fn add_protocol_sockets<'a>(
    sockets: &mut smoltcp::iface::SocketSet<'a>,
    buffers: ProtocolSocketBuffers<'a>,
) -> [smoltcp::iface::SocketHandle; 3] {
    let udp = smoltcp::socket::udp::Socket::new(
        smoltcp::socket::udp::PacketBuffer::new(
            &mut buffers.udp_rx_metadata[..],
            &mut buffers.udp_rx_bytes[..],
        ),
        smoltcp::socket::udp::PacketBuffer::new(
            &mut buffers.udp_tx_metadata[..],
            &mut buffers.udp_tx_bytes[..],
        ),
    );
    let icmp = smoltcp::socket::icmp::Socket::new(
        smoltcp::socket::icmp::PacketBuffer::new(
            &mut buffers.icmp_rx_metadata[..],
            &mut buffers.icmp_rx_bytes[..],
        ),
        smoltcp::socket::icmp::PacketBuffer::new(
            &mut buffers.icmp_tx_metadata[..],
            &mut buffers.icmp_tx_bytes[..],
        ),
    );
    let tcp = smoltcp::socket::tcp::Socket::new(
        smoltcp::socket::tcp::SocketBuffer::new(&mut buffers.tcp_rx_bytes[..]),
        smoltcp::socket::tcp::SocketBuffer::new(&mut buffers.tcp_tx_bytes[..]),
    );
    let _ = buffers.dhcp_rx_bytes;
    [sockets.add(icmp), sockets.add(udp), sockets.add(tcp)]
}

fn prefix_len(mask: [u8; 4]) -> Option<u8> {
    let bits = u32::from_be_bytes(mask);
    let prefix = bits.count_ones() as u8;
    (bits == if prefix == 0 { 0 } else { u32::MAX << (32 - prefix) }).then_some(prefix)
}

use logos_abi::{
    NETWORK_MAX_LISTENER_SLOTS, NETWORK_MAX_SOCKET_SLOTS, NetworkConfig, NetworkOperation,
    NetworkRequest, NetworkResponse, NetworkResult, NetworkState,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SocketHandle {
    pub slot: u8,
    pub generation: u16,
    pub service_epoch: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SocketKind {
    Udp,
    Tcp,
    Listener,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SocketSlot {
    kind: Option<SocketKind>,
    generation: u16,
    service_epoch: u64,
    peer_slot: Option<u8>,
}

impl SocketSlot {
    const EMPTY: Self = Self { kind: None, generation: 1, service_epoch: 1, peer_slot: None };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SocketError {
    Full,
    Stale,
    Invalid,
}

pub struct NetworkService {
    config: NetworkConfig,
    state: NetworkState,
    generation: u16,
    service_epoch: u64,
    sockets: [SocketSlot; NETWORK_MAX_SOCKET_SLOTS],
    listeners: [SocketSlot; NETWORK_MAX_LISTENER_SLOTS],
    dhcp_active: bool,
}

impl NetworkService {
    pub const fn new(config: NetworkConfig) -> Self {
        Self {
            config,
            state: if config.is_enabled() {
                NetworkState::Configuring
            } else {
                NetworkState::Disabled
            },
            generation: 1,
            service_epoch: config.service_epoch,
            sockets: [SocketSlot::EMPTY; NETWORK_MAX_SOCKET_SLOTS],
            listeners: [SocketSlot::EMPTY; NETWORK_MAX_LISTENER_SLOTS],
            dhcp_active: false,
        }
    }

    pub const fn config(&self) -> NetworkConfig {
        self.config
    }

    pub const fn state(&self) -> NetworkState {
        self.state
    }

    pub const fn generation(&self) -> u16 {
        self.generation
    }

    pub const fn service_epoch(&self) -> u64 {
        self.service_epoch
    }

    pub const fn dhcp_active(&self) -> bool {
        self.dhcp_active
    }

    pub fn poll_configuration(
        &mut self,
        elapsed_ticks: u32,
        gateway_arp_reachable: bool,
        dhcp_bound: bool,
    ) {
        if !self.config.is_enabled() || self.state != NetworkState::Configuring {
            return;
        }
        if gateway_arp_reachable {
            self.state = NetworkState::Ready;
            return;
        }
        if elapsed_ticks >= self.config.gateway_deadline_ticks {
            self.dhcp_active = true;
        }
        if self.dhcp_active && dhcp_bound {
            self.state = NetworkState::Ready;
        } else if self.dhcp_active
            && elapsed_ticks
                >= self
                    .config
                    .gateway_deadline_ticks
                    .saturating_add(self.config.dhcp_deadline_ticks)
        {
            self.state = NetworkState::Unavailable;
        }
    }

    pub fn set_ready(&mut self) {
        if self.config.is_enabled() {
            self.state = NetworkState::Ready;
        }
    }

    pub fn set_unavailable(&mut self) {
        if self.config.is_enabled() {
            self.state = NetworkState::Unavailable;
        }
    }

    pub fn allocate_socket(&mut self, listener: bool) -> Result<SocketHandle, SocketError> {
        self.allocate_socket_kind(
            listener,
            if listener { SocketKind::Listener } else { SocketKind::Tcp },
        )
    }

    pub fn allocate_udp(&mut self) -> Result<SocketHandle, SocketError> {
        self.allocate_socket_kind(false, SocketKind::Udp)
    }

    fn allocate_socket_kind(
        &mut self,
        listener: bool,
        kind: SocketKind,
    ) -> Result<SocketHandle, SocketError> {
        if !matches!(self.state, NetworkState::Ready | NetworkState::Configuring) {
            return Err(if self.state == NetworkState::Disabled {
                SocketError::Invalid
            } else {
                SocketError::Stale
            });
        }
        let slots = if listener { &mut self.listeners[..] } else { &mut self.sockets[..] };
        let (slot, entry) = slots
            .iter_mut()
            .enumerate()
            .find(|(_, entry)| entry.kind.is_none())
            .ok_or(SocketError::Full)?;
        entry.kind = Some(kind);
        entry.generation = entry.generation.wrapping_add(1).max(1);
        entry.service_epoch = self.service_epoch;
        entry.peer_slot = None;
        Ok(SocketHandle {
            slot: slot as u8,
            generation: entry.generation,
            service_epoch: entry.service_epoch,
        })
    }

    pub fn close(&mut self, handle: SocketHandle, listener: bool) -> Result<(), SocketError> {
        let slot = handle.slot as usize;
        let valid = if listener {
            self.listeners.get(slot).is_some_and(|entry| {
                entry.kind == Some(SocketKind::Listener)
                    && entry.generation == handle.generation
                    && entry.service_epoch == handle.service_epoch
            })
        } else {
            self.sockets.get(slot).is_some_and(|entry| {
                entry.kind.is_some_and(|kind| kind == SocketKind::Tcp)
                    && entry.generation == handle.generation
                    && entry.service_epoch == handle.service_epoch
            })
        };
        if (listener && self.listeners.get(slot).is_none())
            || (!listener && self.sockets.get(slot).is_none())
        {
            return Err(SocketError::Invalid);
        }
        if !valid {
            return Err(SocketError::Stale);
        }
        let peer_slot =
            if listener { self.listeners[slot].peer_slot } else { self.sockets[slot].peer_slot };
        if listener {
            if let Some(peer_slot) = peer_slot {
                if let Some(peer) = self.sockets.get_mut(peer_slot as usize) {
                    peer.kind = None;
                    peer.peer_slot = None;
                    peer.generation = peer.generation.wrapping_add(1).max(1);
                }
            }
            let entry = &mut self.listeners[slot];
            entry.kind = None;
            entry.peer_slot = None;
            entry.generation = entry.generation.wrapping_add(1).max(1);
        } else {
            if let Some(listener_slot) = peer_slot {
                if let Some(listener) = self.listeners.get_mut(listener_slot as usize) {
                    listener.peer_slot = None;
                }
            }
            let entry = &mut self.sockets[slot];
            entry.kind = None;
            entry.peer_slot = None;
            entry.generation = entry.generation.wrapping_add(1).max(1);
        }
        Ok(())
    }

    pub fn accept(&mut self, handle: SocketHandle) -> Result<SocketHandle, SocketError> {
        let listener_slot = handle.slot as usize;
        let Some(listener) = self.listeners.get(listener_slot) else {
            return Err(SocketError::Invalid);
        };
        if listener.kind != Some(SocketKind::Listener)
            || listener.generation != handle.generation
            || listener.service_epoch != handle.service_epoch
        {
            return Err(SocketError::Stale);
        }
        if listener.peer_slot.is_some() {
            return Err(SocketError::Full);
        }
        let slot =
            self.sockets.iter().position(|entry| entry.kind.is_none()).ok_or(SocketError::Full)?;
        let entry = &mut self.sockets[slot];
        entry.kind = Some(SocketKind::Tcp);
        entry.generation = entry.generation.wrapping_add(1).max(1);
        entry.service_epoch = self.service_epoch;
        entry.peer_slot = Some(handle.slot);
        self.listeners[listener_slot].peer_slot = Some(slot as u8);
        Ok(SocketHandle {
            slot: slot as u8,
            generation: entry.generation,
            service_epoch: entry.service_epoch,
        })
    }

    pub fn reset(&mut self) {
        self.state = if self.config.is_enabled() {
            NetworkState::Restarting
        } else {
            NetworkState::Disabled
        };
        self.generation = self.generation.wrapping_add(1).max(1);
        self.service_epoch = self.service_epoch.wrapping_add(1).max(1);
        self.dhcp_active = false;
        for entry in &mut self.sockets {
            entry.kind = None;
            entry.peer_slot = None;
            entry.generation = entry.generation.wrapping_add(1).max(1);
            entry.service_epoch = self.service_epoch;
        }
        for entry in &mut self.listeners {
            entry.kind = None;
            entry.peer_slot = None;
            entry.generation = entry.generation.wrapping_add(1).max(1);
            entry.service_epoch = self.service_epoch;
        }
        if self.config.is_enabled() {
            self.state = NetworkState::Configuring;
        }
    }

    pub fn handle(&mut self, request: NetworkRequest) -> NetworkResponse {
        let mut response = NetworkResponse::new(
            request.operation,
            NetworkResult::Invalid,
            self.state,
            request.request_id,
        );
        if !request.is_valid() {
            return response;
        }
        if self.state == NetworkState::Disabled {
            response.result = NetworkResult::Disabled;
            return response;
        }
        if self.state != NetworkState::Ready && request.operation != NetworkOperation::Status {
            response.result = NetworkResult::Unavailable;
            return response;
        }
        response.generation = self.generation;
        response.service_epoch = self.service_epoch;
        match request.operation {
            NetworkOperation::Status => response.result = NetworkResult::Ok,
            NetworkOperation::IcmpPing => response.result = NetworkResult::WouldBlock,
            NetworkOperation::UdpBind => match self.allocate_udp() {
                Ok(handle) => {
                    response.handle = u32::from(handle.slot);
                    response.generation = handle.generation;
                    response.service_epoch = handle.service_epoch;
                    response.result = NetworkResult::Ok;
                }
                Err(error) => response.result = socket_error(error),
            },
            NetworkOperation::TcpConnect => match self.allocate_socket(false) {
                Ok(handle) => {
                    response.handle = u32::from(handle.slot);
                    response.generation = handle.generation;
                    response.service_epoch = handle.service_epoch;
                    response.result = NetworkResult::WouldBlock;
                }
                Err(error) => response.result = socket_error(error),
            },
            NetworkOperation::TcpListen => match self.allocate_socket(true) {
                Ok(handle) => {
                    response.handle = u32::from(handle.slot);
                    response.generation = handle.generation;
                    response.service_epoch = handle.service_epoch;
                    response.result = NetworkResult::Ok;
                }
                Err(error) => response.result = socket_error(error),
            },
            NetworkOperation::UdpSend
            | NetworkOperation::UdpReceive
            | NetworkOperation::TcpAccept
            | NetworkOperation::TcpRead
            | NetworkOperation::TcpWrite
            | NetworkOperation::Close => {
                let listener = request.operation == NetworkOperation::TcpAccept
                    || (request.operation == NetworkOperation::Close
                        && request.flags & logos_abi::NETWORK_REQUEST_FLAG_LISTENER != 0);
                let handle = SocketHandle {
                    slot: request.handle as u8,
                    generation: request.generation,
                    service_epoch: request.service_epoch,
                };
                let expected_kind = match request.operation {
                    NetworkOperation::UdpSend | NetworkOperation::UdpReceive => {
                        Some(SocketKind::Udp)
                    }
                    NetworkOperation::TcpAccept => Some(SocketKind::Listener),
                    NetworkOperation::TcpRead | NetworkOperation::TcpWrite => Some(SocketKind::Tcp),
                    NetworkOperation::Close => {
                        Some(if listener { SocketKind::Listener } else { SocketKind::Tcp })
                    }
                    _ => None,
                };
                let valid = self.valid_handle(handle, listener, expected_kind);
                if !valid {
                    response.result = NetworkResult::Stale;
                } else if request.operation == NetworkOperation::Close {
                    response.result = self
                        .close(handle, listener)
                        .map_or_else(socket_error, |_| NetworkResult::Ok);
                } else {
                    response.result = NetworkResult::WouldBlock;
                }
            }
        }
        response.state = self.state;
        response
    }

    fn valid_handle(
        &self,
        handle: SocketHandle,
        listener: bool,
        expected_kind: Option<SocketKind>,
    ) -> bool {
        let slots = if listener { &self.listeners[..] } else { &self.sockets[..] };
        slots.get(handle.slot as usize).is_some_and(|entry| {
            entry.kind.is_some()
                && expected_kind.is_none_or(|kind| entry.kind == Some(kind))
                && entry.generation == handle.generation
                && entry.service_epoch == handle.service_epoch
        })
    }
}

fn socket_error(error: SocketError) -> NetworkResult {
    match error {
        SocketError::Full => NetworkResult::Full,
        SocketError::Stale => NetworkResult::Stale,
        SocketError::Invalid => NetworkResult::Invalid,
    }
}

pub fn internet_checksum(bytes: &[u8]) -> u16 {
    let mut sum = 0u32;
    let mut index = 0;
    while index + 1 < bytes.len() {
        sum += u32::from(u16::from_be_bytes([bytes[index], bytes[index + 1]]));
        index += 2;
    }
    if index < bytes.len() {
        sum += u32::from(bytes[index]) << 8;
    }
    while sum > 0xffff {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

pub fn checksum_valid(bytes: &[u8]) -> bool {
    internet_checksum(bytes) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> NetworkConfig {
        NetworkConfig {
            profile: logos_abi::NetworkProfile::StaticThenDhcp,
            address: [10, 0, 2, 15],
            netmask: [255, 255, 255, 0],
            gateway: [10, 0, 2, 2],
            ..NetworkConfig::disabled()
        }
    }

    #[test]
    fn disabled_state_does_not_allocate_or_start() {
        let mut service = NetworkService::new(NetworkConfig::disabled());
        assert_eq!(service.state(), NetworkState::Disabled);
        assert_eq!(service.allocate_socket(false), Err(SocketError::Invalid));
        assert_eq!(
            service.handle(NetworkRequest::new(NetworkOperation::Status, 1)).result,
            NetworkResult::Disabled
        );
    }

    #[test]
    fn reset_makes_existing_socket_handles_stale() {
        let mut service = NetworkService::new(config());
        service.set_ready();
        let handle = service.allocate_socket(false).unwrap();
        service.reset();
        assert_eq!(service.close(handle, false), Err(SocketError::Stale));
        assert_eq!(service.state(), NetworkState::Configuring);
    }

    #[test]
    fn static_configuration_falls_back_to_dhcp_at_a_fixed_deadline() {
        let mut service = NetworkService::new(config());
        service.poll_configuration(4_999, false, false);
        assert!(!service.dhcp_active());
        service.poll_configuration(5_000, false, false);
        assert!(service.dhcp_active());
        assert_eq!(service.state(), NetworkState::Configuring);
        service.poll_configuration(5_001, false, true);
        assert_eq!(service.state(), NetworkState::Ready);
    }

    #[test]
    fn socket_and_listener_bounds_are_fixed() {
        let mut service = NetworkService::new(config());
        service.set_ready();
        for _ in 0..NETWORK_MAX_SOCKET_SLOTS {
            assert!(service.allocate_socket(false).is_ok());
        }
        assert_eq!(service.allocate_socket(false), Err(SocketError::Full));
        for _ in 0..NETWORK_MAX_LISTENER_SLOTS {
            assert!(service.allocate_socket(true).is_ok());
        }
        assert_eq!(service.allocate_socket(true), Err(SocketError::Full));
    }

    #[test]
    fn operations_reject_handles_of_the_wrong_socket_kind() {
        let mut service = NetworkService::new(config());
        service.set_ready();
        let tcp = service.allocate_socket(false).unwrap();
        let mut request = NetworkRequest::new(NetworkOperation::UdpSend, 7);
        request.handle = u32::from(tcp.slot);
        request.generation = tcp.generation;
        request.service_epoch = tcp.service_epoch;
        assert_eq!(service.handle(request).result, NetworkResult::Stale);
    }

    #[test]
    fn listener_accept_allocates_a_paired_stale_safe_socket() {
        let mut service = NetworkService::new(config());
        service.set_ready();
        let listener = service.allocate_socket(true).unwrap();
        let accepted = service.accept(listener).unwrap();
        assert_eq!(service.accept(listener), Err(SocketError::Full));
        assert_eq!(service.close(accepted, false), Ok(()));
        let accepted_again = service.accept(listener).unwrap();
        assert_ne!(accepted, accepted_again);
        assert_eq!(service.close(listener, true), Ok(()));
        assert_eq!(service.close(accepted_again, false), Err(SocketError::Stale));
    }

    #[test]
    fn malformed_checksum_is_rejected() {
        let valid = [0x00, 0x01, 0xff, 0xfe];
        assert!(checksum_valid(&valid));
        assert!(!checksum_valid(&[valid[0], valid[1], valid[2], 0]));
    }

    #[test]
    fn smoltcp_adapter_copies_private_frames_and_static_ipv4_is_bounded() {
        let mut device = PacketDevice::new();
        assert!(device.enqueue_rx(&[1, 2, 3, 4]));
        let now = smoltcp::time::Instant::from_millis(0);
        let (rx, tx) = smoltcp::phy::Device::receive(&mut device, now).unwrap();
        assert!(rx.consume(|bytes| bytes == [1, 2, 3, 4]));
        tx.consume(4, |bytes| bytes.copy_from_slice(&[5, 6, 7, 8]));
        let mut output = [0; 8];
        assert_eq!(device.take_tx(&mut output), Some(4));
        assert_eq!(&output[..4], &[5, 6, 7, 8]);

        let mut interface_device = PacketDevice::new();
        let mut interface = interface(&mut interface_device, [2, 0, 0, 0, 0, 1], 0);
        assert!(configure_static_ipv4(&mut interface, config()));
    }

    #[test]
    fn static_icmp_probe_reaches_the_private_tx_queue() {
        let mut device = PacketDevice::new();
        let mut interface = interface(&mut device, [2, 0, 0, 0, 0, 1], 0);
        assert!(configure_static_ipv4(&mut interface, config()));
        let mut storage = [const { smoltcp::iface::SocketStorage::EMPTY }; 3];
        let mut sockets = smoltcp::iface::SocketSet::new(&mut storage[..]);
        let mut rx_meta = [smoltcp::socket::icmp::PacketMetadata::EMPTY; 1];
        let mut tx_meta = [smoltcp::socket::icmp::PacketMetadata::EMPTY; 1];
        let mut rx_bytes = [0; 128];
        let mut tx_bytes = [0; 128];
        let icmp = smoltcp::socket::icmp::Socket::new(
            smoltcp::socket::icmp::PacketBuffer::new(&mut rx_meta[..], &mut rx_bytes[..]),
            smoltcp::socket::icmp::PacketBuffer::new(&mut tx_meta[..], &mut tx_bytes[..]),
        );
        let handle = sockets.add(icmp);
        let mut packet = [0; 8];
        smoltcp::wire::Icmpv4Repr::EchoRequest { ident: 0x4c4f, seq_no: 1, data: &[] }.emit(
            &mut smoltcp::wire::Icmpv4Packet::new_unchecked(&mut packet),
            &smoltcp::phy::ChecksumCapabilities::default(),
        );
        sockets
            .get_mut::<smoltcp::socket::icmp::Socket>(handle)
            .send_slice(
                &packet,
                smoltcp::wire::IpAddress::Ipv4(smoltcp::wire::Ipv4Address::new(10, 0, 2, 2)),
            )
            .unwrap();
        interface.poll(smoltcp::time::Instant::from_millis(1), &mut device, &mut sockets);
        assert!(device.pending_rx() == 0);
        let mut output = [0; 1536];
        assert!(device.take_tx(&mut output).is_some());
    }
}
