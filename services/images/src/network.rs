#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", no_main)]
#![cfg_attr(not(target_os = "none"), allow(dead_code, unused_imports, unused_variables))]

mod common;

use core::{mem, ptr};
use logos_abi::{IpcBytes, IpcStatus, MessageKind, NetworkRequest, NetworkResponse, ServiceId};

const FLOW_RECEIVE: usize = common::capability_slot(
    ServiceId::Network,
    logos_abi::IpcEndpointId::FlowToNetwork,
    logos_abi::IpcRights::Receive,
);
const FLOW_SEND: usize = common::capability_slot(
    ServiceId::Network,
    logos_abi::IpcEndpointId::NetworkToFlow,
    logos_abi::IpcRights::Send,
);
const FETCH_RECEIVE: usize = common::capability_slot(
    ServiceId::Network,
    logos_abi::IpcEndpointId::FetchToNetwork,
    logos_abi::IpcRights::Receive,
);
const FETCH_SEND: usize = common::capability_slot(
    ServiceId::Network,
    logos_abi::IpcEndpointId::NetworkToFetch,
    logos_abi::IpcRights::Send,
);
const CORE_RECEIVE: usize = common::capability_slot(
    ServiceId::Network,
    logos_abi::IpcEndpointId::CoreToNetwork,
    logos_abi::IpcRights::Receive,
);
const CORE_SEND: usize = common::capability_slot(
    ServiceId::Network,
    logos_abi::IpcEndpointId::NetworkToCore,
    logos_abi::IpcRights::Send,
);

#[cfg(target_os = "none")]
mod stack {
    use core::{
        mem::MaybeUninit,
        ptr,
        sync::atomic::{AtomicBool, Ordering},
    };

    use logos_abi::{NetworkConfig, NetworkPacketDescriptor, NetworkPacketOperation};
    use logos_network::PacketDevice;
    use logos_network::smoltcp::{
        iface::{Interface, SocketSet, SocketStorage},
        phy::ChecksumCapabilities,
        time::Instant,
        wire::{Icmpv4Packet, Icmpv4Repr, IpAddress, IpEndpoint, IpListenEndpoint, Ipv4Address},
    };

    static mut DEVICE: PacketDevice = PacketDevice::new();
    static mut INTERFACE: MaybeUninit<Interface> = MaybeUninit::uninit();
    const UDP_SOCKET_COUNT: usize = logos_abi::NETWORK_MAX_SOCKET_SLOTS;
    const TCP_SOCKET_COUNT: usize =
        logos_abi::NETWORK_MAX_SOCKET_SLOTS + logos_abi::NETWORK_MAX_LISTENER_SLOTS;
    const SOCKET_STORAGE_COUNT: usize = 1 + UDP_SOCKET_COUNT + TCP_SOCKET_COUNT + 1;
    static mut SOCKETS: MaybeUninit<SocketSet<'static>> = MaybeUninit::uninit();
    static mut STORAGE: [SocketStorage<'static>; SOCKET_STORAGE_COUNT] =
        [const { SocketStorage::EMPTY }; SOCKET_STORAGE_COUNT];
    static mut UDP_RX_META: [[logos_network::smoltcp::socket::udp::PacketMetadata; 4];
        UDP_SOCKET_COUNT] =
        [[logos_network::smoltcp::socket::udp::PacketMetadata::EMPTY; 4]; UDP_SOCKET_COUNT];
    static mut UDP_TX_META: [[logos_network::smoltcp::socket::udp::PacketMetadata; 4];
        UDP_SOCKET_COUNT] =
        [[logos_network::smoltcp::socket::udp::PacketMetadata::EMPTY; 4]; UDP_SOCKET_COUNT];
    static mut ICMP_RX_META: [logos_network::smoltcp::socket::icmp::PacketMetadata; 4] =
        [logos_network::smoltcp::socket::icmp::PacketMetadata::EMPTY; 4];
    static mut ICMP_TX_META: [logos_network::smoltcp::socket::icmp::PacketMetadata; 4] =
        [logos_network::smoltcp::socket::icmp::PacketMetadata::EMPTY; 4];
    static mut UDP_RX: [[u8; 2048]; UDP_SOCKET_COUNT] = [[0; 2048]; UDP_SOCKET_COUNT];
    static mut UDP_TX: [[u8; 2048]; UDP_SOCKET_COUNT] = [[0; 2048]; UDP_SOCKET_COUNT];
    static mut ICMP_RX: [u8; 2048] = [0; 2048];
    static mut ICMP_TX: [u8; 2048] = [0; 2048];
    static mut TCP_RX: [[u8; 4096]; TCP_SOCKET_COUNT] = [[0; 4096]; TCP_SOCKET_COUNT];
    static mut TCP_TX: [[u8; 4096]; TCP_SOCKET_COUNT] = [[0; 4096]; TCP_SOCKET_COUNT];
    static mut DHCP_RX: [u8; 1536] = [0; 1536];
    static mut ICMP_HANDLE: Option<logos_network::smoltcp::iface::SocketHandle> = None;
    static mut UDP_HANDLES: [Option<logos_network::smoltcp::iface::SocketHandle>;
        UDP_SOCKET_COUNT] = [None; UDP_SOCKET_COUNT];
    static mut TCP_HANDLES: [Option<logos_network::smoltcp::iface::SocketHandle>;
        TCP_SOCKET_COUNT] = [None; TCP_SOCKET_COUNT];
    static mut TCP_ACCEPTED_FROM: [Option<u8>; logos_abi::NETWORK_MAX_SOCKET_SLOTS] =
        [None; logos_abi::NETWORK_MAX_SOCKET_SLOTS];
    static mut DHCP_HANDLE: Option<logos_network::smoltcp::iface::SocketHandle> = None;
    static READY: AtomicBool = AtomicBool::new(false);
    static GATEWAY_PROBED: AtomicBool = AtomicBool::new(false);

    pub fn initialize(mac: [u8; 6], config: NetworkConfig) -> bool {
        if READY.load(Ordering::Acquire) {
            return true;
        }
        unsafe {
            let device = &mut *ptr::addr_of_mut!(DEVICE);
            let mut interface = logos_network::interface(device, mac, 0);
            if config.is_enabled() && !logos_network::configure_static_ipv4(&mut interface, config)
            {
                return false;
            }
            let storage = core::slice::from_raw_parts_mut(
                ptr::addr_of_mut!(STORAGE).cast(),
                SOCKET_STORAGE_COUNT,
            );
            let mut sockets = SocketSet::new(storage);
            let mut icmp = logos_network::smoltcp::socket::icmp::Socket::new(
                logos_network::smoltcp::socket::icmp::PacketBuffer::new(
                    (&mut *ptr::addr_of_mut!(ICMP_RX_META))[..].as_mut(),
                    (&mut *ptr::addr_of_mut!(ICMP_RX))[..].as_mut(),
                ),
                logos_network::smoltcp::socket::icmp::PacketBuffer::new(
                    (&mut *ptr::addr_of_mut!(ICMP_TX_META))[..].as_mut(),
                    (&mut *ptr::addr_of_mut!(ICMP_TX))[..].as_mut(),
                ),
            );
            if icmp.bind(logos_network::smoltcp::socket::icmp::Endpoint::Ident(0x4c4f)).is_err() {
                return false;
            }
            ICMP_HANDLE = Some(sockets.add(icmp));
            for (slot, handle) in (&mut *ptr::addr_of_mut!(UDP_HANDLES)).iter_mut().enumerate() {
                let udp = logos_network::smoltcp::socket::udp::Socket::new(
                    logos_network::smoltcp::socket::udp::PacketBuffer::new(
                        (&mut (*ptr::addr_of_mut!(UDP_RX_META))[slot])[..].as_mut(),
                        (&mut (*ptr::addr_of_mut!(UDP_RX))[slot])[..].as_mut(),
                    ),
                    logos_network::smoltcp::socket::udp::PacketBuffer::new(
                        (&mut (*ptr::addr_of_mut!(UDP_TX_META))[slot])[..].as_mut(),
                        (&mut (*ptr::addr_of_mut!(UDP_TX))[slot])[..].as_mut(),
                    ),
                );
                *handle = Some(sockets.add(udp));
            }
            for (slot, handle) in (&mut *ptr::addr_of_mut!(TCP_HANDLES)).iter_mut().enumerate() {
                let tcp = logos_network::smoltcp::socket::tcp::Socket::new(
                    logos_network::smoltcp::socket::tcp::SocketBuffer::new(
                        (&mut (*ptr::addr_of_mut!(TCP_RX))[slot])[..].as_mut(),
                    ),
                    logos_network::smoltcp::socket::tcp::SocketBuffer::new(
                        (&mut (*ptr::addr_of_mut!(TCP_TX))[slot])[..].as_mut(),
                    ),
                );
                *handle = Some(sockets.add(tcp));
            }
            (&mut *ptr::addr_of_mut!(TCP_ACCEPTED_FROM)).fill(None);
            DHCP_HANDLE = None;
            ptr::write(ptr::addr_of_mut!(INTERFACE), MaybeUninit::new(interface));
            ptr::write(ptr::addr_of_mut!(SOCKETS), MaybeUninit::new(sockets));
            GATEWAY_PROBED.store(false, Ordering::Release);
            READY.store(true, Ordering::Release);
            true
        }
    }

    pub fn ready() -> bool {
        READY.load(Ordering::Acquire)
    }

    pub fn enqueue_rx(page: u16, length: u16) -> bool {
        if !ready() || page >= logos_abi::NETWORK_PACKET_PAGE_COUNT as u16 {
            return false;
        }
        let address = logos_abi::NETWORK_PACKET_BASE + usize::from(page) * 4096;
        unsafe {
            (&mut *ptr::addr_of_mut!(DEVICE))
                .enqueue_rx(core::slice::from_raw_parts(address as *const u8, usize::from(length)))
        }
    }

    pub fn poll_network(now: u64, dhcp_active: bool) -> bool {
        if !ready() {
            return false;
        }
        let mut configured = false;
        unsafe {
            let interface = &mut *ptr::addr_of_mut!(INTERFACE).cast::<Interface>();
            let device = &mut *ptr::addr_of_mut!(DEVICE);
            let sockets = &mut *ptr::addr_of_mut!(SOCKETS).cast::<SocketSet<'static>>();
            let timestamp = Instant::from_millis((now.min(i64::MAX as u64)) as i64);
            let _ = interface.poll_ingress_single(timestamp, device, sockets);
            if dhcp_active {
                if let Some(handle) = DHCP_HANDLE {
                    if let Some(logos_network::smoltcp::socket::dhcpv4::Event::Configured(config)) =
                        sockets
                            .get_mut::<logos_network::smoltcp::socket::dhcpv4::Socket>(handle)
                            .poll()
                    {
                        interface.update_ip_addrs(|addrs| {
                            addrs.clear();
                            let _ = addrs.push(config.address.into());
                        });
                        interface.routes_mut().update(|routes| {
                            routes.clear();
                            if let Some(router) = config.router {
                                let _ = routes.push(
                                    logos_network::smoltcp::iface::Route::new_ipv4_gateway(router),
                                );
                            }
                        });
                        configured = true;
                    }
                }
            }
            let _ = interface.poll_egress(timestamp, device, sockets);
        }
        configured
    }

    pub fn probe_gateway(address: [u8; 4]) {
        if !ready() || GATEWAY_PROBED.swap(true, Ordering::AcqRel) {
            return;
        }
        let _ = ping(address);
    }

    pub fn gateway_reachable() -> bool {
        if !ready() {
            return false;
        }
        unsafe {
            let sockets = &mut *ptr::addr_of_mut!(SOCKETS).cast::<SocketSet<'static>>();
            let socket = sockets
                .get_mut::<logos_network::smoltcp::socket::icmp::Socket>(ICMP_HANDLE.unwrap());
            if !socket.can_recv() {
                return false;
            }
            let _ = socket.recv();
            true
        }
    }

    pub fn start_dhcp() {
        if !ready() {
            return;
        }
        unsafe {
            let interface = &mut *ptr::addr_of_mut!(INTERFACE).cast::<Interface>();
            interface.update_ip_addrs(|addrs| addrs.clear());
            interface.routes_mut().update(|routes| routes.clear());
            let sockets = &mut *ptr::addr_of_mut!(SOCKETS).cast::<SocketSet<'static>>();
            let handle = if let Some(handle) = DHCP_HANDLE {
                sockets.get_mut::<logos_network::smoltcp::socket::dhcpv4::Socket>(handle).reset();
                handle
            } else {
                let mut dhcp = logos_network::smoltcp::socket::dhcpv4::Socket::new();
                dhcp.set_receive_packet_buffer(&mut *ptr::addr_of_mut!(DHCP_RX));
                let handle = sockets.add(dhcp);
                DHCP_HANDLE = Some(handle);
                handle
            };
            DHCP_HANDLE = Some(handle);
        }
    }

    pub fn ping(address: [u8; 4]) -> bool {
        if !ready() {
            return false;
        }
        unsafe {
            let sockets = &mut *ptr::addr_of_mut!(SOCKETS).cast::<SocketSet<'static>>();
            let mut packet = [0; 8];
            Icmpv4Repr::EchoRequest { ident: 0x4c4f, seq_no: 1, data: &[] }.emit(
                &mut Icmpv4Packet::new_unchecked(&mut packet),
                &ChecksumCapabilities::default(),
            );
            sockets
                .get_mut::<logos_network::smoltcp::socket::icmp::Socket>(ICMP_HANDLE.unwrap())
                .send_slice(
                    &packet,
                    IpAddress::Ipv4(Ipv4Address::from_bits(u32::from_be_bytes(address))),
                )
                .is_ok()
        }
    }

    pub fn udp_bind(slot: u32, port: u16) -> bool {
        let slot = slot as usize;
        if !ready() || slot >= UDP_SOCKET_COUNT || port == 0 {
            return false;
        }
        unsafe {
            let sockets = &mut *ptr::addr_of_mut!(SOCKETS).cast::<SocketSet<'static>>();
            sockets
                .get_mut::<logos_network::smoltcp::socket::udp::Socket>(UDP_HANDLES[slot].unwrap())
                .bind(port)
                .is_ok()
        }
    }

    pub fn udp_send(slot: u32, address: [u8; 4], port: u16, payload: &[u8]) -> bool {
        let slot = slot as usize;
        if !ready() || slot >= UDP_SOCKET_COUNT || port == 0 {
            return false;
        }
        unsafe {
            let sockets = &mut *ptr::addr_of_mut!(SOCKETS).cast::<SocketSet<'static>>();
            sockets
                .get_mut::<logos_network::smoltcp::socket::udp::Socket>(UDP_HANDLES[slot].unwrap())
                .send_slice(
                    payload,
                    IpEndpoint::new(
                        Ipv4Address::from_bits(u32::from_be_bytes(address)).into(),
                        port,
                    ),
                )
                .is_ok()
        }
    }

    pub fn udp_receive(slot: u32, output: &mut [u8]) -> Option<(usize, [u8; 4], u16)> {
        let slot = slot as usize;
        if !ready() || slot >= UDP_SOCKET_COUNT {
            return None;
        }
        unsafe {
            let sockets = &mut *ptr::addr_of_mut!(SOCKETS).cast::<SocketSet<'static>>();
            let socket = sockets
                .get_mut::<logos_network::smoltcp::socket::udp::Socket>(UDP_HANDLES[slot].unwrap());
            let (length, metadata) = socket.recv_slice(output).ok()?;
            let IpAddress::Ipv4(address) = metadata.endpoint.addr;
            let address = address.octets();
            Some((length, address, metadata.endpoint.port))
        }
    }

    pub fn tcp_connect(slot: u32, address: [u8; 4], port: u16) -> bool {
        let slot = slot as usize;
        if !ready() || slot >= logos_abi::NETWORK_MAX_SOCKET_SLOTS || port == 0 {
            return false;
        }
        unsafe {
            let interface = &mut *ptr::addr_of_mut!(INTERFACE).cast::<Interface>();
            let sockets = &mut *ptr::addr_of_mut!(SOCKETS).cast::<SocketSet<'static>>();
            sockets
                .get_mut::<logos_network::smoltcp::socket::tcp::Socket>(TCP_HANDLES[slot].unwrap())
                .connect(
                    interface.context(),
                    IpEndpoint::new(
                        Ipv4Address::from_bits(u32::from_be_bytes(address)).into(),
                        port,
                    ),
                    IpListenEndpoint { addr: None, port: 49152 },
                )
                .is_ok()
        }
    }

    pub fn tcp_active(slot: u32) -> bool {
        let slot = slot as usize;
        if !ready() || slot >= logos_abi::NETWORK_MAX_SOCKET_SLOTS {
            return false;
        }
        unsafe {
            let sockets = &mut *ptr::addr_of_mut!(SOCKETS).cast::<SocketSet<'static>>();
            sockets
                .get::<logos_network::smoltcp::socket::tcp::Socket>(TCP_HANDLES[slot].unwrap())
                .state()
                == logos_network::smoltcp::socket::tcp::State::Established
        }
    }

    pub fn tcp_listen(slot: u32, port: u16) -> bool {
        let slot = slot as usize;
        if !ready() || slot >= logos_abi::NETWORK_MAX_LISTENER_SLOTS || port == 0 {
            return false;
        }
        unsafe {
            let sockets = &mut *ptr::addr_of_mut!(SOCKETS).cast::<SocketSet<'static>>();
            sockets
                .get_mut::<logos_network::smoltcp::socket::tcp::Socket>(
                    TCP_HANDLES[logos_abi::NETWORK_MAX_SOCKET_SLOTS + slot].unwrap(),
                )
                .listen(port)
                .is_ok()
        }
    }

    pub fn tcp_accept(slot: u32) -> Option<u8> {
        let slot = slot as usize;
        if !ready() || slot >= logos_abi::NETWORK_MAX_LISTENER_SLOTS {
            return None;
        }
        unsafe {
            let sockets = &mut *ptr::addr_of_mut!(SOCKETS).cast::<SocketSet<'static>>();
            let active = sockets
                .get::<logos_network::smoltcp::socket::tcp::Socket>(
                    TCP_HANDLES[logos_abi::NETWORK_MAX_SOCKET_SLOTS + slot].unwrap(),
                )
                .state()
                == logos_network::smoltcp::socket::tcp::State::Established;
            active.then_some(slot as u8)
        }
    }

    pub fn bind_accepted(slot: u32, listener_slot: u8) {
        let slot = slot as usize;
        if slot < logos_abi::NETWORK_MAX_SOCKET_SLOTS
            && usize::from(listener_slot) < logos_abi::NETWORK_MAX_LISTENER_SLOTS
        {
            unsafe {
                (*ptr::addr_of_mut!(TCP_ACCEPTED_FROM))[slot] = Some(listener_slot);
            }
        }
    }

    fn tcp_index(slot: u32) -> Option<usize> {
        let slot = slot as usize;
        if slot >= logos_abi::NETWORK_MAX_SOCKET_SLOTS {
            return None;
        }
        unsafe {
            Some(match (*ptr::addr_of!(TCP_ACCEPTED_FROM))[slot] {
                Some(listener_slot) => {
                    logos_abi::NETWORK_MAX_SOCKET_SLOTS + usize::from(listener_slot)
                }
                None => slot,
            })
        }
    }

    pub fn tcp_read(slot: u32, output: &mut [u8]) -> Option<usize> {
        let index = tcp_index(slot)?;
        if !ready() {
            return None;
        }
        unsafe {
            let sockets = &mut *ptr::addr_of_mut!(SOCKETS).cast::<SocketSet<'static>>();
            sockets
                .get_mut::<logos_network::smoltcp::socket::tcp::Socket>(TCP_HANDLES[index].unwrap())
                .recv_slice(output)
                .ok()
        }
    }

    pub fn tcp_write(slot: u32, payload: &[u8]) -> bool {
        let Some(index) = tcp_index(slot) else {
            return false;
        };
        if !ready() {
            return false;
        }
        unsafe {
            let sockets = &mut *ptr::addr_of_mut!(SOCKETS).cast::<SocketSet<'static>>();
            sockets
                .get_mut::<logos_network::smoltcp::socket::tcp::Socket>(TCP_HANDLES[index].unwrap())
                .send_slice(payload)
                .is_ok()
        }
    }

    pub fn close(handle: u32, listener: bool) {
        let slot = handle as usize;
        if !ready() {
            return;
        }
        let index = if listener {
            if slot >= logos_abi::NETWORK_MAX_LISTENER_SLOTS {
                return;
            }
            logos_abi::NETWORK_MAX_SOCKET_SLOTS + slot
        } else {
            let Some(index) = tcp_index(handle) else { return };
            index
        };
        unsafe {
            let sockets = &mut *ptr::addr_of_mut!(SOCKETS).cast::<SocketSet<'static>>();
            sockets
                .get_mut::<logos_network::smoltcp::socket::tcp::Socket>(TCP_HANDLES[index].unwrap())
                .abort();
            if listener {
                for accepted in (&mut *ptr::addr_of_mut!(TCP_ACCEPTED_FROM)).iter_mut() {
                    if *accepted == Some(slot as u8) {
                        *accepted = None;
                    }
                }
            } else if slot < logos_abi::NETWORK_MAX_SOCKET_SLOTS {
                (*ptr::addr_of_mut!(TCP_ACCEPTED_FROM))[slot] = None;
            }
        }
    }

    pub fn take_tx(page: u16, output: &mut [u8]) -> Option<usize> {
        if !ready() || page < logos_abi::NETWORK_RX_PACKET_PAGES as u16 {
            return None;
        }
        unsafe {
            let length = (&mut *ptr::addr_of_mut!(DEVICE)).take_tx(output)?;
            let address = logos_abi::NETWORK_PACKET_BASE + usize::from(page) * 4096;
            ptr::copy_nonoverlapping(output.as_ptr(), address as *mut u8, length);
            Some(length)
        }
    }

    pub fn make_tx(
        page: u16,
        length: usize,
        sequence: u32,
        generation: u16,
        epoch: u64,
    ) -> NetworkPacketDescriptor {
        let mut descriptor =
            NetworkPacketDescriptor::new(NetworkPacketOperation::SubmitTx, page, sequence);
        descriptor.length = length as u16;
        descriptor.generation = generation;
        descriptor.service_epoch = epoch;
        descriptor
    }
}

fn response_message(response: NetworkResponse) -> IpcBytes {
    let bytes = unsafe {
        core::slice::from_raw_parts(
            (&response as *const NetworkResponse).cast::<u8>(),
            mem::size_of::<NetworkResponse>(),
        )
    };
    IpcBytes::from_bytes(MessageKind::NetworkResponse, bytes)
        .unwrap_or_else(|| IpcBytes::empty(MessageKind::NetworkResponse))
}

fn request_from_message(message: &IpcBytes) -> Option<NetworkRequest> {
    if message.kind != MessageKind::NetworkRequest
        || message.len as usize != mem::size_of::<NetworkRequest>()
    {
        return None;
    }
    Some(unsafe { ptr::read_unaligned(message.bytes.as_ptr().cast()) })
}

#[cfg(target_os = "none")]
#[derive(Clone, Copy)]
struct PendingRequest {
    request: NetworkRequest,
    response: NetworkResponse,
    started: u32,
    peer: Peer,
}

#[cfg(target_os = "none")]
#[derive(Clone, Copy, Eq, PartialEq)]
enum Peer {
    Flow,
    Fetch,
}

#[cfg(target_os = "none")]
fn send_network_response(peer: Peer, response: NetworkResponse) {
    let response = response_message(response);
    let (capability, endpoint) = match peer {
        Peer::Flow => (FLOW_SEND, logos_abi::IpcEndpointId::NetworkToFlow),
        Peer::Fetch => (FETCH_SEND, logos_abi::IpcEndpointId::NetworkToFetch),
    };
    loop {
        match common::ipc_send(capability, &response) {
            IpcStatus::Ok => break,
            IpcStatus::Full => common::wait(common::ipc_write_event(endpoint), ServiceId::Network),
            _ => break,
        }
    }
}

#[cfg(target_os = "none")]
#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    let config = unsafe {
        if logos_abi::NETWORK_CONFIG_BASE == 0 {
            logos_abi::NetworkConfig::disabled()
        } else {
            ptr::read_unaligned(logos_abi::NETWORK_CONFIG_BASE as *const logos_abi::NetworkConfig)
        }
    };
    let mut service = logos_network::NetworkService::new(config);
    let mut ticks = 0;
    let mut elapsed_ticks = 0u32;
    let mut sequence = 1u32;
    let mut pending_tx_bytes = [0u8; logos_abi::NETWORK_PACKET_PAGE_BYTES];
    let mut pending_tx_len = 0usize;
    let mut pending_tx = false;
    let mut pending_request: Option<PendingRequest> = None;
    loop {
        common::heartbeat_tick(&mut ticks, ServiceId::Network);
        elapsed_ticks = elapsed_ticks.saturating_add(1);
        let mut icmp_reply_received = false;
        if stack::ready() {
            if !service.dhcp_active() {
                stack::probe_gateway(config.gateway);
            }
            let dhcp_bound = stack::poll_network(u64::from(elapsed_ticks), service.dhcp_active());
            let gateway_reachable = stack::gateway_reachable();
            icmp_reply_received = gateway_reachable;
            let was_dhcp_active = service.dhcp_active();
            service.poll_configuration(elapsed_ticks, gateway_reachable, dhcp_bound);
            if !was_dhcp_active && service.dhcp_active() {
                stack::start_dhcp();
            }
            if dhcp_bound {
                service.set_ready();
            }
        }
        let mut packet = logos_abi::NetworkPacketDescriptor::new(
            logos_abi::NetworkPacketOperation::LinkState,
            0,
            sequence,
        );
        if common::ipc_receive(CORE_RECEIVE, &mut packet) == IpcStatus::Ok {
            if packet.operation == logos_abi::NetworkPacketOperation::LinkState {
                if packet.result == logos_abi::NetworkResult::NotFound {
                    service.set_unavailable();
                } else {
                    let initialized = stack::initialize(packet.mac, config);
                    if !initialized {
                        service.set_unavailable();
                    }
                }
            } else if packet.operation == logos_abi::NetworkPacketOperation::RecycleRx {
                let page = packet.page;
                let address = logos_abi::NETWORK_PACKET_BASE + usize::from(page) * 4096;
                if stack::ready() {
                    let length = packet.length as usize;
                    let _ = stack::enqueue_rx(
                        page,
                        packet.length.min(logos_abi::NETWORK_MAX_FRAME_BYTES as u16),
                    );
                    let _ = stack::poll_network(u64::from(elapsed_ticks), service.dhcp_active());
                    if stack::gateway_reachable() {
                        icmp_reply_received = true;
                        service.poll_configuration(elapsed_ticks, true, false);
                    }
                    let mut recycle = packet;
                    recycle.operation = logos_abi::NetworkPacketOperation::RecycleRx;
                    recycle.length = length as u16;
                    recycle.sequence = sequence;
                    let _ = address;
                    sequence = sequence.wrapping_add(1).max(1);
                    let _ = common::ipc_send(CORE_SEND, &recycle);
                }
            }
        }
        if let Some(pending) = pending_request {
            let complete = match pending.request.operation {
                logos_abi::NetworkOperation::IcmpPing => icmp_reply_received,
                logos_abi::NetworkOperation::TcpConnect => {
                    stack::tcp_active(pending.response.handle)
                }
                _ => true,
            };
            let deadline = if pending.request.timeout_ticks == 0 {
                config.gateway_deadline_ticks
            } else {
                pending.request.timeout_ticks
            };
            if complete || elapsed_ticks.wrapping_sub(pending.started) >= deadline {
                let mut response = pending.response;
                response.result = if complete {
                    logos_abi::NetworkResult::Ok
                } else {
                    logos_abi::NetworkResult::Timeout
                };
                send_network_response(pending.peer, response);
                pending_request = None;
            }
        }
        let mut message = IpcBytes::empty(MessageKind::NetworkRequest);
        let status = common::ipc_receive(FLOW_RECEIVE, &mut message);
        let (command_status, peer) = if status == IpcStatus::Ok {
            (status, Peer::Flow)
        } else {
            (common::ipc_receive(FETCH_RECEIVE, &mut message), Peer::Fetch)
        };
        if command_status == IpcStatus::Ok {
            if let Some(request) = request_from_message(&message) {
                if request.operation == logos_abi::NetworkOperation::Cancel {
                    let mut response = NetworkResponse::new(
                        logos_abi::NetworkOperation::Cancel,
                        logos_abi::NetworkResult::Stale,
                        service.state(),
                        request.request_id,
                    );
                    if let Some(pending) = pending_request
                        && pending.peer == peer
                        && pending.request.request_id == request.request_id
                    {
                        pending_request = None;
                        if pending.request.operation == logos_abi::NetworkOperation::TcpConnect {
                            stack::close(pending.response.handle, false);
                        }
                        response.result = logos_abi::NetworkResult::Cancelled;
                    }
                    if peer == Peer::Flow {
                        send_network_response(peer, response);
                    }
                } else {
                    let mut response = service.handle(request);
                    let mut wait_for_result = false;
                    if response.result == logos_abi::NetworkResult::Ok
                        && request.operation == logos_abi::NetworkOperation::UdpBind
                        && !stack::udp_bind(response.handle, request.port)
                    {
                        response.result = logos_abi::NetworkResult::Full;
                    } else if response.result == logos_abi::NetworkResult::WouldBlock
                        && request.operation == logos_abi::NetworkOperation::UdpSend
                    {
                        response.result = if stack::udp_send(
                            request.handle,
                            request.address,
                            request.port,
                            &request.payload[..usize::from(request.payload_len)],
                        ) {
                            logos_abi::NetworkResult::Ok
                        } else {
                            logos_abi::NetworkResult::WouldBlock
                        };
                    } else if response.result == logos_abi::NetworkResult::WouldBlock
                        && request.operation == logos_abi::NetworkOperation::IcmpPing
                    {
                        if stack::ping(request.address) {
                            wait_for_result = true;
                        } else {
                            response.result = logos_abi::NetworkResult::Full;
                        }
                    } else if response.result == logos_abi::NetworkResult::WouldBlock
                        && request.operation == logos_abi::NetworkOperation::TcpConnect
                    {
                        if stack::tcp_connect(response.handle, request.address, request.port) {
                            wait_for_result = true;
                        } else {
                            response.result = logos_abi::NetworkResult::Full;
                        }
                    } else if response.result == logos_abi::NetworkResult::Ok
                        && request.operation == logos_abi::NetworkOperation::TcpListen
                        && !stack::tcp_listen(response.handle, request.port)
                    {
                        response.result = logos_abi::NetworkResult::Full;
                    } else if request.operation == logos_abi::NetworkOperation::TcpAccept
                        && response.result == logos_abi::NetworkResult::WouldBlock
                    {
                        response.result = match stack::tcp_accept(request.handle) {
                            Some(listener_slot) => {
                                let listener = logos_network::SocketHandle {
                                    slot: request.handle as u8,
                                    generation: request.generation,
                                    service_epoch: request.service_epoch,
                                };
                                match service.accept(listener) {
                                    Ok(accepted) => {
                                        stack::bind_accepted(accepted.slot as u32, listener_slot);
                                        response.handle = u32::from(accepted.slot);
                                        response.generation = accepted.generation;
                                        response.service_epoch = accepted.service_epoch;
                                        logos_abi::NetworkResult::Ok
                                    }
                                    Err(logos_network::SocketError::Full) => {
                                        logos_abi::NetworkResult::Full
                                    }
                                    Err(logos_network::SocketError::Stale) => {
                                        logos_abi::NetworkResult::Stale
                                    }
                                    Err(logos_network::SocketError::Invalid) => {
                                        logos_abi::NetworkResult::Invalid
                                    }
                                }
                            }
                            None => logos_abi::NetworkResult::WouldBlock,
                        };
                    } else if request.operation == logos_abi::NetworkOperation::TcpRead
                        && response.result == logos_abi::NetworkResult::WouldBlock
                    {
                        response.result = match stack::tcp_read(
                            request.handle,
                            &mut response.payload[..logos_abi::NETWORK_INLINE_PAYLOAD_BYTES],
                        ) {
                            Some(length) => {
                                response.payload_len = length as u16;
                                logos_abi::NetworkResult::Ok
                            }
                            None => logos_abi::NetworkResult::WouldBlock,
                        };
                    } else if request.operation == logos_abi::NetworkOperation::TcpWrite
                        && response.result == logos_abi::NetworkResult::WouldBlock
                    {
                        response.result = if stack::tcp_write(
                            request.handle,
                            &request.payload[..usize::from(request.payload_len)],
                        ) {
                            logos_abi::NetworkResult::Ok
                        } else {
                            logos_abi::NetworkResult::WouldBlock
                        };
                    } else if request.operation == logos_abi::NetworkOperation::UdpReceive
                        && response.result == logos_abi::NetworkResult::WouldBlock
                    {
                        response.result = match stack::udp_receive(
                            request.handle,
                            &mut response.payload[..logos_abi::NETWORK_INLINE_PAYLOAD_BYTES],
                        ) {
                            Some((length, address, port)) => {
                                response.payload_len = length as u16;
                                response.detail[..4].copy_from_slice(&address);
                                response.detail[4..6].copy_from_slice(&port.to_be_bytes());
                                logos_abi::NetworkResult::Ok
                            }
                            None => logos_abi::NetworkResult::WouldBlock,
                        };
                    } else if request.operation == logos_abi::NetworkOperation::Close
                        && response.result == logos_abi::NetworkResult::Ok
                    {
                        stack::close(
                            request.handle,
                            request.flags & logos_abi::NETWORK_REQUEST_FLAG_LISTENER != 0,
                        );
                    }
                    if wait_for_result {
                        pending_request = Some(PendingRequest {
                            request,
                            response,
                            started: elapsed_ticks,
                            peer,
                        });
                    } else {
                        send_network_response(peer, response);
                    }
                }
            } else {
                send_network_response(
                    peer,
                    NetworkResponse::new(
                        logos_abi::NetworkOperation::Status,
                        logos_abi::NetworkResult::Invalid,
                        service.state(),
                        0,
                    ),
                );
            }
        }
        if !pending_tx {
            pending_tx_len = stack::take_tx(16, &mut pending_tx_bytes).unwrap_or(0);
            pending_tx = pending_tx_len != 0;
        }
        if pending_tx {
            let descriptor = stack::make_tx(
                16,
                pending_tx_len,
                sequence,
                packet.generation,
                packet.service_epoch,
            );
            if common::ipc_send(CORE_SEND, &descriptor) == IpcStatus::Ok {
                pending_tx = false;
                sequence = sequence.wrapping_add(1).max(1);
            }
        }
        common::wait(
            common::ipc_read_event(logos_abi::IpcEndpointId::FlowToNetwork)
                | common::ipc_read_event(logos_abi::IpcEndpointId::FetchToNetwork)
                | common::ipc_read_event(logos_abi::IpcEndpointId::CoreToNetwork),
            ServiceId::Network,
        );
    }
}

#[cfg(target_os = "none")]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo<'_>) -> ! {
    common::idle()
}

#[cfg(not(target_os = "none"))]
fn main() {}
