#![cfg_attr(not(test), no_main)]
#![cfg_attr(not(test), no_std)]

use logos_abi::{
    NetworkDeviceOperation, NetworkDeviceRequest, NetworkEndpoint, NetworkInfo, NetworkOperation,
    NetworkReply, NetworkRequest, NetworkStatus,
};
use logos_net::{
    Arp, DHCP_ACK, DHCP_NAK, DHCP_OFFER, DHCP_OPTION_LEASE_TIME, DHCP_OPTION_MESSAGE_TYPE,
    DHCP_OPTION_ROUTER, DHCP_OPTION_SERVER_ID, DHCP_OPTION_SUBNET_MASK, DHCP_OPTION_T1,
    DHCP_OPTION_T2, DhcpAction, Ipv4, Mac, NetworkConfig, NetworkState, StateError, encode_arp,
    encode_dhcp_discover, encode_dhcp_request, encode_ethernet, encode_ipv4, encode_udp, parse_arp,
    parse_dhcp, parse_ethernet, parse_icmp_echo, parse_ipv4, parse_udp,
};
use logos_service_rt::{Context, Header, ProtocolVersion};

#[cfg(feature = "test-hooks")]
fn trace(message: &[u8]) {
    logos_service_rt::debug(message);
}

#[cfg(not(feature = "test-hooks"))]
fn trace(_: &[u8]) {}

const DHCP_CLIENT: u16 = 68;
const DHCP_SERVER: u16 = 67;
const DEVICE_DEADLINE: u64 = u64::MAX / 2;
const BROADCAST: Ipv4 = Ipv4([255; 4]);
const CLIENT_PAYLOAD_OFFSET: usize = 2048;
const ICMP_PAYLOAD: usize = logos_abi::MAX_NETWORK_PAYLOAD;

#[derive(Clone, Copy)]
struct IcmpReply {
    destination_mac: Mac,
    source: Ipv4,
    identifier: u16,
    sequence: u16,
    length: u16,
    payload: [u8; ICMP_PAYLOAD],
}

#[used]
#[unsafe(link_section = ".logos")]
static HEADER: Header =
    Header::new(*b"network\0\0\0\0\0\0\0\0\0", ProtocolVersion::V1, logos_service_entry);

#[unsafe(no_mangle)]
extern "C" fn logos_service_entry(context: logos_service_rt::EntryContext) -> ! {
    logos_service_rt::entry(context, run)
}

#[allow(clippy::collapsible_if)]
fn run(context: &mut Context) -> ! {
    if !context.ready() {
        spin();
    }
    let mut state = NetworkState::new();
    let mut info = NetworkInfo::default();
    let mut offer = Ipv4([0; 4]);
    let mut server = Ipv4([0; 4]);
    let mut arp_reply: Option<Arp> = None;
    let mut pending = 1;
    let mut pending_info = true;
    let mut next_id = 2u32;
    let mut now = 1u64;
    let mut xid = 0x4c4f_474fu32;
    let mut waiting_receive: Option<NetworkRequest> = None;
    let mut waiting_send: Option<NetworkRequest> = None;
    let mut waiting_send_arp = false;
    let mut waiting_echo: Option<NetworkRequest> = None;
    let mut waiting_echo_arp = false;
    let mut next_echo = 1u16;
    let mut icmp_reply: Option<IcmpReply> = None;
    let mut counters = logos_abi::NetworkCounters::default();

    if !issue_info(context, pending) {
        spin();
    }
    while context.acknowledged() {
        if pending != 0 {
            if let Some(device_reply) = context.network_device_reply(pending) {
                if pending_info {
                    if device_reply.status != NetworkStatus::Complete
                        || device_reply.info.mac == [0; 6]
                    {
                        spin();
                    }
                    info = device_reply.info;
                    xid ^= u32::from_be_bytes([info.mac[2], info.mac[3], info.mac[4], info.mac[5]]);
                    xid = xid.max(1);
                    state.dhcp_start(now, xid);
                    pending_info = false;
                    if !submit_action(
                        context,
                        &state,
                        &info,
                        DhcpAction::Discover,
                        offer,
                        server,
                        arp_reply,
                        icmp_reply,
                        next_id,
                    ) {
                        spin();
                    }
                    pending = next_id;
                    next_id = next_id.wrapping_add(1).max(1);
                    continue;
                }
                pending = 0;
                if !pending_info && device_reply.status == NetworkStatus::Complete {
                    counters.tx_frames = counters.tx_frames.saturating_add(1);
                }
                if device_reply.status == NetworkStatus::Reset {
                    counters.resets = counters.resets.saturating_add(1);
                    info = device_reply.info;
                    state.reset();
                    state.dhcp_start(now, xid);
                    waiting_receive = None;
                    waiting_send = None;
                    waiting_send_arp = false;
                    waiting_echo = None;
                    waiting_echo_arp = false;
                }
                if let Some(request) = waiting_send {
                    if waiting_send_arp {
                        if device_reply.status != NetworkStatus::Complete {
                            waiting_send = None;
                            waiting_send_arp = false;
                            if !context.request_network(request) {
                                spin();
                            }
                            if !context.network_reply(NetworkReply {
                                id: request.id,
                                status: NetworkStatus::Io,
                                endpoint: NetworkEndpoint(0),
                                generation: info.generation,
                                source_address: 0,
                                source_port: 0,
                                length: 0,
                                info,
                                counters,
                            }) {
                                spin();
                            }
                        }
                    } else {
                        waiting_send = None;
                        let _ = state.finish_pending(request.id);
                        let reply = NetworkReply {
                            id: request.id,
                            status: if device_reply.status == NetworkStatus::Complete {
                                NetworkStatus::Complete
                            } else {
                                NetworkStatus::Io
                            },
                            endpoint: if device_reply.status == NetworkStatus::Complete {
                                request.endpoint
                            } else {
                                NetworkEndpoint(0)
                            },
                            generation: info.generation,
                            source_address: 0,
                            source_port: 0,
                            length: if device_reply.status == NetworkStatus::Complete {
                                request.length
                            } else {
                                0
                            },
                            info,
                            counters,
                        };
                        if !context.network_reply_after_device(request, reply) {
                            spin();
                        }
                    }
                }
                if let Some(request) = waiting_echo {
                    if waiting_echo_arp {
                        if device_reply.status != NetworkStatus::Complete {
                            waiting_echo = None;
                            waiting_echo_arp = false;
                            let _ = state.expire_echo(u64::MAX);
                            if !context.request_network(request) {
                                spin();
                            }
                            if !context.network_reply(NetworkReply {
                                id: request.id,
                                status: NetworkStatus::Io,
                                endpoint: NetworkEndpoint(0),
                                generation: info.generation,
                                source_address: 0,
                                source_port: 0,
                                length: 0,
                                info,
                                counters,
                            }) {
                                spin();
                            }
                        }
                    } else if device_reply.status != NetworkStatus::Complete {
                        waiting_echo = None;
                        let _ = state.expire_echo(u64::MAX);
                        if !context.request_network(request) {
                            spin();
                        }
                        if !context.network_reply(NetworkReply {
                            id: request.id,
                            status: NetworkStatus::Io,
                            endpoint: NetworkEndpoint(0),
                            generation: info.generation,
                            source_address: 0,
                            source_port: 0,
                            length: 0,
                            info,
                            counters,
                        }) {
                            spin();
                        }
                    }
                }
            }
        }

        if let Some(request) = context.network_request() {
            #[cfg(feature = "test-hooks")]
            inject_failure(request.id);
            if matches!(request.operation, NetworkOperation::Cancel | NetworkOperation::Close) {
                counters.cancellations = counters.cancellations.saturating_add(1);
                let endpoint = logos_net::EndpointId::from_wire(request.endpoint.0);
                let cancels_receive = waiting_receive.is_some_and(|pending| {
                    endpoint == logos_net::EndpointId::from_wire(pending.endpoint.0)
                });
                let cancels_send = waiting_send.is_some_and(|pending| {
                    endpoint == logos_net::EndpointId::from_wire(pending.endpoint.0)
                });
                let cancels_echo = waiting_echo.is_some();
                if cancels_receive || cancels_send || cancels_echo {
                    if let Some(pending) = waiting_receive.take() {
                        let _ = state.cancel_pending(pending.id);
                    }
                    if let Some(pending) = waiting_send.take() {
                        let _ = state.cancel_pending(pending.id);
                    }
                    if cancels_echo {
                        waiting_echo = None;
                        waiting_echo_arp = false;
                        let _ = state.expire_echo(u64::MAX);
                    }
                    waiting_send_arp = false;
                }
                let status = if request.operation == NetworkOperation::Close {
                    endpoint
                        .and_then(|endpoint| state.close(0, endpoint).ok())
                        .map_or(NetworkStatus::Invalid, |_| NetworkStatus::Complete)
                } else {
                    NetworkStatus::Cancelled
                };
                if !context.network_reply(NetworkReply {
                    id: request.id,
                    status,
                    endpoint: NetworkEndpoint(0),
                    generation: info.generation,
                    source_address: 0,
                    source_port: 0,
                    length: 0,
                    info,
                    counters,
                }) {
                    spin();
                }
                continue;
            }
            if request.operation == NetworkOperation::SendTo {
                let Some(endpoint) = logos_net::EndpointId::from_wire(request.endpoint.0) else {
                    if !context.network_reply(error_reply(
                        request,
                        NetworkStatus::Invalid,
                        info,
                        counters,
                    )) {
                        spin();
                    }
                    continue;
                };
                if let Err(error) = state.begin_pending(logos_net::Pending {
                    id: request.id,
                    endpoint,
                    kind: logos_net::PendingKind::Send,
                    deadline: request.deadline,
                }) {
                    if !context.network_reply(error_reply(
                        request,
                        map_state_error(error),
                        info,
                        counters,
                    )) {
                        spin();
                    }
                    continue;
                }
                match submit_datagram(context, &mut state, info, request, now, next_id) {
                    Ok(SubmitDatagram::Sent) => {
                        waiting_send = Some(request);
                        waiting_send_arp = false;
                        pending = next_id;
                        next_id = next_id.wrapping_add(1).max(1);
                        continue;
                    }
                    Ok(SubmitDatagram::Arp) => {
                        waiting_send = Some(request);
                        waiting_send_arp = true;
                        pending = next_id;
                        next_id = next_id.wrapping_add(1).max(1);
                        continue;
                    }
                    Err(status) => {
                        let _ = state.cancel_pending(request.id);
                        if !context.network_reply(NetworkReply {
                            id: request.id,
                            status,
                            endpoint: NetworkEndpoint(0),
                            generation: info.generation,
                            source_address: 0,
                            source_port: 0,
                            length: 0,
                            info,
                            counters,
                        }) {
                            spin();
                        }
                        continue;
                    }
                }
            }
            if request.operation == NetworkOperation::Echo {
                #[cfg(feature = "test-hooks")]
                let identifier = match request.id {
                    0x9000_0130 => 2,
                    0x9000_0131 => 3,
                    _ => next_echo,
                };
                #[cfg(not(feature = "test-hooks"))]
                let identifier = next_echo;
                next_echo = next_echo.wrapping_add(1).max(1);
                let echo = logos_net::EchoMatch {
                    peer: Ipv4(request.peer.address().to_be_bytes()),
                    identifier,
                    sequence: 1,
                    generation: info.generation,
                    deadline: request.deadline,
                };
                if let Err(error) = state.begin_echo(echo) {
                    if !context.network_reply(error_reply(
                        request,
                        map_state_error(error),
                        info,
                        counters,
                    )) {
                        spin();
                    }
                    continue;
                }
                match submit_echo(context, &mut state, info, request, echo, now, next_id) {
                    Ok(SubmitDatagram::Sent) => {
                        waiting_echo = Some(request);
                        waiting_echo_arp = false;
                        pending = next_id;
                        next_id = next_id.wrapping_add(1).max(1);
                        continue;
                    }
                    Ok(SubmitDatagram::Arp) => {
                        waiting_echo = Some(request);
                        waiting_echo_arp = true;
                        pending = next_id;
                        next_id = next_id.wrapping_add(1).max(1);
                        continue;
                    }
                    Err(status) => {
                        let _ = state.expire_echo(u64::MAX);
                        if !context.network_reply(error_reply(request, status, info, counters)) {
                            spin();
                        }
                        continue;
                    }
                }
            }
            let reply =
                handle_request(&mut state, info, request, context.network_pages(), counters);
            if reply.status == NetworkStatus::Busy
                && request.operation == NetworkOperation::ReceiveFrom
            {
                if let Some(endpoint) = logos_net::EndpointId::from_wire(request.endpoint.0) {
                    let _ = state.begin_pending(logos_net::Pending {
                        id: request.id,
                        endpoint,
                        kind: logos_net::PendingKind::Receive,
                        deadline: request.deadline,
                    });
                    waiting_receive = Some(request);
                    if !context.network_wait(request.deadline) {
                        spin();
                    }
                    continue;
                }
            }
            if !context.network_reply(reply) {
                spin();
            }
            continue;
        }

        if let Some(event) = context.network_event() {
            now = event.now.max(now);
            if event.generation != info.generation {
                continue;
            }
            let action = match event.kind {
                logos_abi::NetworkEventKind::Frame => {
                    counters.rx_frames = counters.rx_frames.saturating_add(1);
                    counters.rx_bytes = counters.rx_bytes.saturating_add(u64::from(event.length));
                    accept_dhcp(
                        context,
                        event.length,
                        now,
                        &mut state,
                        info,
                        &mut offer,
                        &mut server,
                        &mut arp_reply,
                        &mut icmp_reply,
                        &mut counters,
                    )
                }
                logos_abi::NetworkEventKind::Timer => state.dhcp_tick(now),
                logos_abi::NetworkEventKind::Reset => {
                    counters.resets = counters.resets.saturating_add(1);
                    state.reset();
                    state.dhcp_start(now, xid);
                    DhcpAction::Discover
                }
                logos_abi::NetworkEventKind::Cancel => DhcpAction::None,
            };
            if let Some(request) = waiting_echo
                && !waiting_echo_arp
                && state.echo().is_none()
            {
                waiting_echo = None;
                if !context.network_reply_after_event(
                    request,
                    NetworkReply {
                        id: request.id,
                        status: NetworkStatus::Complete,
                        endpoint: NetworkEndpoint(0),
                        generation: info.generation,
                        source_address: request.peer.address(),
                        source_port: 0,
                        length: 0,
                        info,
                        counters,
                    },
                ) {
                    spin();
                }
                continue;
            }
            if let Some(request) = waiting_receive {
                let received = context.network_pages().and_then(|pages| {
                    let output = unsafe {
                        core::slice::from_raw_parts_mut(
                            (pages.tx_address + CLIENT_PAYLOAD_OFFSET as u64) as *mut u8,
                            logos_abi::MAX_NETWORK_PAYLOAD,
                        )
                    };
                    let endpoint = logos_net::EndpointId::from_wire(request.endpoint.0)?;
                    state
                        .receive(0, endpoint, output)
                        .ok()
                        .map(|receive| (receive.source, receive.source_port, receive.payload.len()))
                });
                if let Some(receive) = received {
                    trace(b"LogOS: network receive complete\r\n");
                    let _ = state.finish_pending(request.id);
                    waiting_receive = None;
                    if !context.network_reply_after_event(
                        request,
                        NetworkReply {
                            id: request.id,
                            status: NetworkStatus::Complete,
                            endpoint: request.endpoint,
                            generation: info.generation,
                            source_address: u32::from_be_bytes(receive.0.0),
                            source_port: receive.1,
                            length: receive.2 as u16,
                            info,
                            counters,
                        },
                    ) {
                        spin();
                    }
                    continue;
                }
                if now >= request.deadline {
                    counters.timeouts = counters.timeouts.saturating_add(1);
                    let _ = state.cancel_pending(request.id);
                    waiting_receive = None;
                    if !context.network_reply_after_event(
                        request,
                        NetworkReply {
                            id: request.id,
                            status: NetworkStatus::TimedOut,
                            endpoint: NetworkEndpoint(0),
                            generation: info.generation,
                            source_address: 0,
                            source_port: 0,
                            length: 0,
                            info,
                            counters,
                        },
                    ) {
                        spin();
                    }
                    continue;
                }
            }
            if let Some(request) = waiting_send
                && waiting_send_arp
                && state.arp_target().is_none()
            {
                if now >= request.deadline {
                    counters.timeouts = counters.timeouts.saturating_add(1);
                    waiting_send = None;
                    waiting_send_arp = false;
                    let _ = state.cancel_pending(request.id);
                    if !context.network_reply_after_event(
                        request,
                        NetworkReply {
                            id: request.id,
                            status: NetworkStatus::TimedOut,
                            endpoint: NetworkEndpoint(0),
                            generation: info.generation,
                            source_address: 0,
                            source_port: 0,
                            length: 0,
                            info,
                            counters,
                        },
                    ) {
                        spin();
                    }
                    continue;
                }
                if let Ok(SubmitDatagram::Sent) =
                    submit_datagram(context, &mut state, info, request, now, next_id)
                {
                    waiting_send_arp = false;
                    pending = next_id;
                    next_id = next_id.wrapping_add(1).max(1);
                    continue;
                }
            }
            if let Some(request) = waiting_echo
                && waiting_echo_arp
                && state.arp_target().is_none()
            {
                if now >= request.deadline {
                    counters.timeouts = counters.timeouts.saturating_add(1);
                    waiting_echo = None;
                    waiting_echo_arp = false;
                    let _ = state.expire_echo(now);
                    if !context.network_reply_after_event(
                        request,
                        error_reply(request, NetworkStatus::TimedOut, info, counters),
                    ) {
                        spin();
                    }
                    continue;
                }
                if let Some(echo) = state.echo()
                    && let Ok(SubmitDatagram::Sent) =
                        submit_echo(context, &mut state, info, request, echo, now, next_id)
                {
                    waiting_echo_arp = false;
                    pending = next_id;
                    next_id = next_id.wrapping_add(1).max(1);
                    continue;
                }
            }
            if action != DhcpAction::None && action != DhcpAction::Expired {
                if !submit_action(
                    context, &state, &info, action, offer, server, arp_reply, icmp_reply, next_id,
                ) {
                    spin();
                }
                pending = next_id;
                next_id = next_id.wrapping_add(1).max(1);
                continue;
            }
        }

        let deadline = state.dhcp_deadline().max(now.saturating_add(1));
        if !context.network_wait(deadline) {
            spin();
        }
    }
    spin()
}

#[cfg(feature = "test-hooks")]
fn inject_failure(id: u32) {
    if id == u32::MAX - 1 {
        panic!("test panic");
    }
    if id == u32::MAX - 2 {
        let address = core::hint::black_box(1usize);
        unsafe { (address as *mut u8).write_volatile(1) };
    }
}

fn issue_info(context: &mut Context, id: u32) -> bool {
    context.network_device_request(NetworkDeviceRequest {
        id,
        operation: NetworkDeviceOperation::Info,
        length: 0,
        generation: 0,
        deadline: DEVICE_DEADLINE,
    })
}

enum SubmitDatagram {
    Sent,
    Arp,
}

fn submit_datagram(
    context: &mut Context,
    state: &mut NetworkState,
    info: NetworkInfo,
    request: NetworkRequest,
    now: u64,
    id: u32,
) -> Result<SubmitDatagram, NetworkStatus> {
    if info.generation == 0 || id == 0 || request.length == 0 {
        return Err(NetworkStatus::Invalid);
    }
    let Some(config) = state.dhcp_config() else {
        return Err(NetworkStatus::Offline);
    };
    let Some(endpoint) = logos_net::EndpointId::from_wire(request.endpoint.0) else {
        return Err(NetworkStatus::Invalid);
    };
    let Ok(source_port) = state.endpoint_port(0, endpoint) else {
        return Err(NetworkStatus::Invalid);
    };
    let destination = Ipv4(request.peer.address().to_be_bytes());
    let next_hop = logos_net::route_target(config.address, config.mask, config.router, destination)
        .map_err(|error| match error {
            StateError::NoRoute => NetworkStatus::NoRoute,
            _ => NetworkStatus::Invalid,
        })?;
    let Some(pages) = context.network_pages() else {
        return Err(NetworkStatus::Offline);
    };
    let mac = Mac(info.mac);
    let remote = if let Some(mac) = state.resolve_arp(next_hop, now) {
        mac
    } else {
        let requested = if !state.arp_target().is_some_and(|target| target == next_hop) {
            let mut arp_bytes = [0; 64];
            let arp_length = encode_arp(
                &mut arp_bytes,
                Arp {
                    reply: false,
                    sender_mac: mac,
                    sender_ip: config.address,
                    target_mac: Mac([0; 6]),
                    target_ip: next_hop,
                },
            )
            .map_err(|_| NetworkStatus::Io)?;
            let tx = unsafe { core::slice::from_raw_parts_mut(pages.tx_address as *mut u8, 4096) };
            let frame_length =
                encode_ethernet(tx, Mac::BROADCAST, mac, 0x0806, &arp_bytes[..arp_length])
                    .map_err(|_| NetworkStatus::Io)?;
            let _ = state.expect_arp(next_hop);
            if !context.network_device_request(NetworkDeviceRequest {
                id,
                operation: NetworkDeviceOperation::Transmit,
                length: frame_length as u16,
                generation: info.generation,
                deadline: DEVICE_DEADLINE,
            }) {
                return Err(NetworkStatus::Io);
            }
            true
        } else {
            false
        };
        return if requested { Ok(SubmitDatagram::Arp) } else { Err(NetworkStatus::Busy) };
    };
    let payload = unsafe {
        core::slice::from_raw_parts(
            (pages.tx_address + CLIENT_PAYLOAD_OFFSET as u64) as *const u8,
            usize::from(request.length),
        )
    };
    let mut udp = [0; 1480];
    let udp_length = encode_udp(
        &mut udp,
        config.address,
        destination,
        source_port,
        request.peer.port(),
        payload,
    )
    .map_err(|_| NetworkStatus::MessageTooLarge)?;
    let mut ipv4 = [0; 1500];
    let ipv4_length = encode_ipv4(
        &mut ipv4,
        config.address,
        destination,
        request.id as u16,
        17,
        &udp[..udp_length],
    )
    .map_err(|_| NetworkStatus::MessageTooLarge)?;
    let tx = unsafe { core::slice::from_raw_parts_mut(pages.tx_address as *mut u8, 4096) };
    let frame_length = encode_ethernet(tx, remote, mac, 0x0800, &ipv4[..ipv4_length])
        .map_err(|_| NetworkStatus::MessageTooLarge)?;
    if !context.network_device_request(NetworkDeviceRequest {
        id,
        operation: NetworkDeviceOperation::Transmit,
        length: frame_length as u16,
        generation: info.generation,
        deadline: DEVICE_DEADLINE,
    }) {
        return Err(NetworkStatus::Io);
    }
    Ok(SubmitDatagram::Sent)
}

fn submit_echo(
    context: &mut Context,
    state: &mut NetworkState,
    info: NetworkInfo,
    request: NetworkRequest,
    echo: logos_net::EchoMatch,
    now: u64,
    id: u32,
) -> Result<SubmitDatagram, NetworkStatus> {
    let Some(config) = state.dhcp_config() else {
        return Err(NetworkStatus::Offline);
    };
    let destination = echo.peer;
    let next_hop = logos_net::route_target(config.address, config.mask, config.router, destination)
        .map_err(|error| match error {
            StateError::NoRoute => NetworkStatus::NoRoute,
            _ => NetworkStatus::Invalid,
        })?;
    let Some(pages) = context.network_pages() else {
        return Err(NetworkStatus::Offline);
    };
    let mac = Mac(info.mac);
    let remote = if let Some(mac) = state.resolve_arp(next_hop, now) {
        mac
    } else {
        let requested = if !state.arp_target().is_some_and(|target| target == next_hop) {
            let mut arp_bytes = [0; 64];
            let arp_length = encode_arp(
                &mut arp_bytes,
                Arp {
                    reply: false,
                    sender_mac: mac,
                    sender_ip: config.address,
                    target_mac: Mac([0; 6]),
                    target_ip: next_hop,
                },
            )
            .map_err(|_| NetworkStatus::Io)?;
            let tx = unsafe { core::slice::from_raw_parts_mut(pages.tx_address as *mut u8, 4096) };
            let frame_length =
                encode_ethernet(tx, Mac::BROADCAST, mac, 0x0806, &arp_bytes[..arp_length])
                    .map_err(|_| NetworkStatus::Io)?;
            let _ = state.expect_arp(next_hop);
            if !context.network_device_request(NetworkDeviceRequest {
                id,
                operation: NetworkDeviceOperation::Transmit,
                length: frame_length as u16,
                generation: info.generation,
                deadline: request.deadline,
            }) {
                return Err(NetworkStatus::Io);
            }
            true
        } else {
            false
        };
        return if requested { Ok(SubmitDatagram::Arp) } else { Err(NetworkStatus::Busy) };
    };
    let mut icmp = [0; 8 + ICMP_PAYLOAD];
    let payload = b"LogOS-ICMP";
    let icmp_length =
        logos_net::encode_icmp_echo(&mut icmp, false, echo.identifier, echo.sequence, payload)
            .map_err(|_| NetworkStatus::MessageTooLarge)?;
    let mut ipv4 = [0; 1500];
    let ipv4_length = encode_ipv4(
        &mut ipv4,
        config.address,
        destination,
        request.id as u16,
        1,
        &icmp[..icmp_length],
    )
    .map_err(|_| NetworkStatus::MessageTooLarge)?;
    let tx = unsafe { core::slice::from_raw_parts_mut(pages.tx_address as *mut u8, 4096) };
    let frame_length = encode_ethernet(tx, remote, mac, 0x0800, &ipv4[..ipv4_length])
        .map_err(|_| NetworkStatus::MessageTooLarge)?;
    if !context.network_device_request(NetworkDeviceRequest {
        id,
        operation: NetworkDeviceOperation::Transmit,
        length: frame_length as u16,
        generation: info.generation,
        deadline: request.deadline,
    }) {
        return Err(NetworkStatus::Io);
    }
    Ok(SubmitDatagram::Sent)
}

#[allow(clippy::too_many_arguments)]
fn submit_action(
    context: &mut Context,
    state: &NetworkState,
    info: &NetworkInfo,
    action: DhcpAction,
    offer: Ipv4,
    server: Ipv4,
    arp_reply: Option<Arp>,
    icmp_reply: Option<IcmpReply>,
    id: u32,
) -> bool {
    let Some(pages) = context.network_pages() else {
        return false;
    };
    if info.generation == 0 || id == 0 {
        return false;
    }
    let mac = Mac(info.mac);
    if action == DhcpAction::ArpReply {
        let Some(arp) = arp_reply else { return false };
        let mut arp_bytes = [0; 64];
        let Ok(arp_length) = encode_arp(&mut arp_bytes, arp) else { return false };
        let mut frame = [0; logos_net::ETHERNET_MAX_FRAME];
        let Ok(frame_length) =
            encode_ethernet(&mut frame, arp.sender_mac, mac, 0x0806, &arp_bytes[..arp_length])
        else {
            return false;
        };
        let tx = unsafe { core::slice::from_raw_parts_mut(pages.tx_address as *mut u8, 4096) };
        tx[..frame_length].copy_from_slice(&frame[..frame_length]);
        return context.network_device_request(NetworkDeviceRequest {
            id,
            operation: NetworkDeviceOperation::Transmit,
            length: frame_length as u16,
            generation: info.generation,
            deadline: DEVICE_DEADLINE,
        });
    }
    if action == DhcpAction::IcmpReply {
        let Some(reply) = icmp_reply else { return false };
        let Some(config) = state.dhcp_config() else { return false };
        let mut icmp = [0; 8 + ICMP_PAYLOAD];
        let Ok(icmp_length) = logos_net::encode_icmp_echo(
            &mut icmp,
            true,
            reply.identifier,
            reply.sequence,
            &reply.payload[..usize::from(reply.length)],
        ) else {
            return false;
        };
        let mut ipv4 = [0; 1500];
        let Ok(ipv4_length) = encode_ipv4(
            &mut ipv4,
            config.address,
            reply.source,
            reply.identifier,
            1,
            &icmp[..icmp_length],
        ) else {
            return false;
        };
        let mut frame = [0; logos_net::ETHERNET_MAX_FRAME];
        let Ok(frame_length) =
            encode_ethernet(&mut frame, reply.destination_mac, mac, 0x0800, &ipv4[..ipv4_length])
        else {
            return false;
        };
        let tx = unsafe { core::slice::from_raw_parts_mut(pages.tx_address as *mut u8, 4096) };
        tx[..frame_length].copy_from_slice(&frame[..frame_length]);
        return context.network_device_request(NetworkDeviceRequest {
            id,
            operation: NetworkDeviceOperation::Transmit,
            length: frame_length as u16,
            generation: info.generation,
            deadline: DEVICE_DEADLINE,
        });
    }
    let mut dhcp = [0; 300];
    let dhcp_length = match action {
        DhcpAction::Discover => encode_dhcp_discover(&mut dhcp, state.dhcp_xid(), mac),
        DhcpAction::Request | DhcpAction::Renew | DhcpAction::Rebind => {
            encode_dhcp_request(&mut dhcp, state.dhcp_xid(), mac, offer, server)
        }
        DhcpAction::None | DhcpAction::Expired | DhcpAction::ArpReply | DhcpAction::IcmpReply => {
            return false;
        }
    }
    .ok();
    let Some(dhcp_length) = dhcp_length else {
        return false;
    };
    let mut udp = [0; 320];
    let udp_length = encode_udp(
        &mut udp,
        Ipv4([0; 4]),
        BROADCAST,
        DHCP_CLIENT,
        DHCP_SERVER,
        &dhcp[..dhcp_length],
    )
    .ok();
    let Some(udp_length) = udp_length else {
        return false;
    };
    udp[6..8].fill(0);
    let mut ipv4 = [0; 340];
    let ipv4_length =
        encode_ipv4(&mut ipv4, Ipv4([0; 4]), BROADCAST, 0, 17, &udp[..udp_length]).ok();
    let Some(ipv4_length) = ipv4_length else {
        return false;
    };
    let tx = unsafe { core::slice::from_raw_parts_mut(pages.tx_address as *mut u8, 4096) };
    let frame_length = encode_ethernet(tx, Mac::BROADCAST, mac, 0x0800, &ipv4[..ipv4_length]).ok();
    let Some(frame_length) = frame_length else {
        return false;
    };
    context.network_device_request(NetworkDeviceRequest {
        id,
        operation: NetworkDeviceOperation::Transmit,
        length: frame_length as u16,
        generation: info.generation,
        deadline: DEVICE_DEADLINE,
    })
}

#[allow(clippy::too_many_arguments)]
fn accept_dhcp(
    context: &Context,
    length: u16,
    now: u64,
    state: &mut NetworkState,
    info: NetworkInfo,
    offer: &mut Ipv4,
    server: &mut Ipv4,
    arp_reply: &mut Option<Arp>,
    icmp_reply: &mut Option<IcmpReply>,
    counters: &mut logos_abi::NetworkCounters,
) -> DhcpAction {
    let Some(pages) = context.network_pages() else {
        return DhcpAction::None;
    };
    let length = usize::from(length);
    if !(logos_net::ETHERNET_MIN_FRAME..=logos_net::ETHERNET_MAX_FRAME).contains(&length) {
        counters.malformed = counters.malformed.saturating_add(1);
        return DhcpAction::None;
    }
    let frame = unsafe { core::slice::from_raw_parts(pages.rx_address as *const u8, length) };
    let ethernet = match parse_ethernet(frame, Mac(info.mac)) {
        Ok(ethernet) => ethernet,
        Err(logos_net::Error::Unsupported) => {
            counters.unsupported = counters.unsupported.saturating_add(1);
            return DhcpAction::None;
        }
        Err(_) => {
            counters.malformed = counters.malformed.saturating_add(1);
            return DhcpAction::None;
        }
    };
    if ethernet.ether_type == 0x0806 {
        let arp = match parse_arp(ethernet.payload) {
            Ok(arp) => arp,
            Err(logos_net::Error::Unsupported) => {
                counters.unsupported = counters.unsupported.saturating_add(1);
                return DhcpAction::None;
            }
            Err(_) => {
                counters.malformed = counters.malformed.saturating_add(1);
                return DhcpAction::None;
            }
        };
        let local =
            state.dhcp_config().map_or(Ipv4(info.ipv4.to_be_bytes()), |config| config.address);
        if !arp.reply && arp.target_ip == local {
            *arp_reply = Some(Arp {
                reply: true,
                sender_mac: Mac(info.mac),
                sender_ip: local,
                target_mac: arp.sender_mac,
                target_ip: arp.sender_ip,
            });
            return DhcpAction::ArpReply;
        }
        if arp.reply {
            state.learn_arp_reply(arp.sender_ip, arp.sender_mac, now, 60);
        }
        return DhcpAction::None;
    }
    if ethernet.ether_type != 0x0800 {
        return DhcpAction::None;
    }
    let local = state.dhcp_config().map_or(Ipv4(info.ipv4.to_be_bytes()), |config| config.address);
    let ip =
        parse_ipv4(ethernet.payload, BROADCAST).or_else(|_| parse_ipv4(ethernet.payload, local));
    let ip = match ip {
        Ok(ip) => ip,
        Err(logos_net::Error::Unsupported | logos_net::Error::Fragmented) => {
            counters.unsupported = counters.unsupported.saturating_add(1);
            return DhcpAction::None;
        }
        Err(_) => {
            counters.malformed = counters.malformed.saturating_add(1);
            return DhcpAction::None;
        }
    };
    if ip.protocol == 1 {
        let icmp = match parse_icmp_echo(ip.payload) {
            Ok(icmp) => icmp,
            Err(logos_net::Error::Unsupported) => {
                counters.unsupported = counters.unsupported.saturating_add(1);
                return DhcpAction::None;
            }
            Err(_) => {
                counters.malformed = counters.malformed.saturating_add(1);
                return DhcpAction::None;
            }
        };
        if icmp.reply {
            let _ = state.finish_echo(ip.source, icmp.identifier, icmp.sequence);
            return DhcpAction::None;
        }
        if icmp.payload.len() > ICMP_PAYLOAD {
            return DhcpAction::None;
        }
        let mut payload = [0; ICMP_PAYLOAD];
        payload[..icmp.payload.len()].copy_from_slice(icmp.payload);
        *icmp_reply = Some(IcmpReply {
            destination_mac: ethernet.source,
            source: ip.source,
            identifier: icmp.identifier,
            sequence: icmp.sequence,
            length: icmp.payload.len() as u16,
            payload,
        });
        return DhcpAction::IcmpReply;
    }
    if ip.protocol != 17 {
        counters.unsupported = counters.unsupported.saturating_add(1);
        return DhcpAction::None;
    }
    let udp = match parse_udp(ip.payload, ip.source, ip.destination) {
        Ok(udp) => {
            trace(b"LogOS: network udp parsed\r\n");
            udp
        }
        Err(_) => {
            trace(b"LogOS: network udp rejected\r\n");
            counters.malformed = counters.malformed.saturating_add(1);
            return DhcpAction::None;
        }
    };
    if udp.source_port != DHCP_SERVER || udp.destination_port != DHCP_CLIENT {
        if state.dhcp_config().is_none() {
            counters.udp_no_endpoint = counters.udp_no_endpoint.saturating_add(1);
        } else if let Some(endpoint) = state.endpoint_for_port(udp.destination_port) {
            match state.enqueue(endpoint, ip.source, udp.source_port, udp.payload) {
                Ok(()) => trace(b"LogOS: network udp queued\r\n"),
                Err(StateError::QueueFull) => {
                    trace(b"LogOS: network udp queue-full\r\n");
                    counters.udp_queue_dropped = counters.udp_queue_dropped.saturating_add(1);
                }
                Err(StateError::MessageTooLarge) => {
                    trace(b"LogOS: network udp too-large\r\n");
                    counters.malformed = counters.malformed.saturating_add(1);
                }
                Err(_) => {
                    trace(b"LogOS: network udp queue-error\r\n");
                    counters.malformed = counters.malformed.saturating_add(1);
                }
            }
        } else {
            counters.udp_no_endpoint = counters.udp_no_endpoint.saturating_add(1);
        }
        return DhcpAction::None;
    }
    let dhcp = match parse_dhcp(udp.payload) {
        Ok(dhcp) => dhcp,
        Err(_) => {
            counters.malformed = counters.malformed.saturating_add(1);
            return DhcpAction::None;
        }
    };
    if dhcp.xid != state.dhcp_xid() || dhcp.client_mac != Mac(info.mac) || dhcp.offered.0 == [0; 4]
    {
        return DhcpAction::None;
    }
    let Ok(Some(message)) = dhcp.option(DHCP_OPTION_MESSAGE_TYPE) else {
        return DhcpAction::None;
    };
    if message.len() != 1 {
        return DhcpAction::None;
    }
    let Ok(Some(server_id)) = dhcp.option(DHCP_OPTION_SERVER_ID) else {
        return DhcpAction::None;
    };
    if server_id.len() != 4 {
        return DhcpAction::None;
    }
    let offered_server = Ipv4(server_id.try_into().unwrap_or([0; 4]));
    match message[0] {
        DHCP_OFFER if state.dhcp_phase() == logos_net::DhcpPhase::Selecting => {
            *offer = dhcp.offered;
            *server = offered_server;
            if state.dhcp_offer(now, state.dhcp_xid()) {
                DhcpAction::Request
            } else {
                DhcpAction::None
            }
        }
        DHCP_ACK => {
            if *server != offered_server && *server != Ipv4([0; 4]) {
                return DhcpAction::None;
            }
            let Some(mask) = option_ipv4(dhcp.option(DHCP_OPTION_SUBNET_MASK)) else {
                return DhcpAction::None;
            };
            if !contiguous_mask(mask) {
                return DhcpAction::None;
            }
            let Some(lease) = option_u32(dhcp.option(DHCP_OPTION_LEASE_TIME)) else {
                return DhcpAction::None;
            };
            let t1 = option_u32(dhcp.option(DHCP_OPTION_T1)).unwrap_or(lease / 2);
            let t2 = option_u32(dhcp.option(DHCP_OPTION_T2)).unwrap_or(lease.saturating_mul(7) / 8);
            if lease == 0 || t1 == 0 || t1 >= t2 || t2 >= lease {
                return DhcpAction::None;
            }
            let router = match dhcp.option(DHCP_OPTION_ROUTER) {
                Ok(Some(value)) if value.len() == 4 => {
                    Some(Ipv4(value.try_into().unwrap_or([0; 4])))
                }
                Ok(None) => None,
                _ => return DhcpAction::None,
            };
            let config = NetworkConfig {
                address: dhcp.offered,
                mask,
                router,
                lease_until: now.saturating_add(u64::from(lease)),
                renew_at: now.saturating_add(u64::from(t1)),
                rebind_at: now.saturating_add(u64::from(t2)),
            };
            state.dhcp_acknowledge(now, state.dhcp_xid(), config);
            DhcpAction::None
        }
        DHCP_NAK => {
            state.dhcp_nak();
            state.dhcp_start(now, state.dhcp_xid());
            DhcpAction::Discover
        }
        _ => DhcpAction::None,
    }
}

fn option_ipv4(value: Result<Option<&[u8]>, logos_net::Error>) -> Option<Ipv4> {
    let value = value.ok()??;
    if value.len() != 4 {
        return None;
    }
    Some(Ipv4(value.try_into().ok()?))
}

fn option_u32(value: Result<Option<&[u8]>, logos_net::Error>) -> Option<u32> {
    let value = value.ok()??;
    if value.len() != 4 {
        return None;
    }
    Some(u32::from_be_bytes(value.try_into().ok()?))
}

fn contiguous_mask(mask: Ipv4) -> bool {
    let value = u32::from_be_bytes(mask.0);
    let inverted = !value;
    inverted == 0 || inverted.wrapping_add(1).is_power_of_two()
}

fn map_state_error(error: StateError) -> NetworkStatus {
    match error {
        StateError::Full => NetworkStatus::Full,
        StateError::AddressInUse => NetworkStatus::AddressInUse,
        StateError::Busy | StateError::QueueFull => NetworkStatus::Busy,
        StateError::MessageTooLarge => NetworkStatus::MessageTooLarge,
        StateError::NoRoute => NetworkStatus::NoRoute,
        StateError::Invalid
        | StateError::NotFound
        | StateError::NoData
        | StateError::Stale
        | StateError::Owner => NetworkStatus::Invalid,
    }
}

fn error_reply(
    request: NetworkRequest,
    status: NetworkStatus,
    info: NetworkInfo,
    counters: logos_abi::NetworkCounters,
) -> NetworkReply {
    NetworkReply {
        id: request.id,
        status,
        endpoint: NetworkEndpoint(0),
        generation: info.generation,
        source_address: 0,
        source_port: 0,
        length: 0,
        info,
        counters,
    }
}

fn handle_request(
    state: &mut NetworkState,
    info: NetworkInfo,
    request: NetworkRequest,
    pages: Option<logos_service_rt::NetworkPages>,
    counters: logos_abi::NetworkCounters,
) -> NetworkReply {
    let config = state.dhcp_config();
    let status_info = config.map_or(info, |config| NetworkInfo {
        configuration: 1,
        ipv4: u32::from_be_bytes(config.address.0),
        subnet_mask: u32::from_be_bytes(config.mask.0),
        router: config.router.map_or(0, |router| u32::from_be_bytes(router.0)),
        ..info
    });
    let mut source_address = 0;
    let mut source_port = 0;
    let mut length = 0;
    let (status, endpoint) = match request.operation {
        NetworkOperation::Status => {
            (config.map_or(NetworkStatus::Offline, |_| NetworkStatus::Complete), NetworkEndpoint(0))
        }
        NetworkOperation::Bind if config.is_some() => match state.bind(0, request.peer.port()) {
            Ok(endpoint) => (NetworkStatus::Complete, NetworkEndpoint(endpoint.wire())),
            Err(StateError::AddressInUse) => (NetworkStatus::AddressInUse, NetworkEndpoint(0)),
            Err(StateError::Full) => (NetworkStatus::Full, NetworkEndpoint(0)),
            Err(_) => (NetworkStatus::Invalid, NetworkEndpoint(0)),
        },
        NetworkOperation::Bind => (NetworkStatus::Offline, NetworkEndpoint(0)),
        NetworkOperation::Close => match logos_net::EndpointId::from_wire(request.endpoint.0)
            .and_then(|endpoint| state.close(0, endpoint).ok())
        {
            Some(()) => (NetworkStatus::Complete, NetworkEndpoint(0)),
            None => (NetworkStatus::Invalid, NetworkEndpoint(0)),
        },
        NetworkOperation::Cancel => (NetworkStatus::Cancelled, NetworkEndpoint(0)),
        NetworkOperation::ReceiveFrom if config.is_some() => {
            let result = pages.and_then(|pages| {
                let output = unsafe {
                    core::slice::from_raw_parts_mut(
                        (pages.tx_address + CLIENT_PAYLOAD_OFFSET as u64) as *mut u8,
                        logos_abi::MAX_NETWORK_PAYLOAD,
                    )
                };
                let endpoint = logos_net::EndpointId::from_wire(request.endpoint.0)?;
                if state.endpoint_port(0, endpoint).ok()? != request.peer.port() {
                    return None;
                }
                state.receive(0, endpoint, output).ok()
            });
            match result {
                Some(receive) => {
                    source_address = u32::from_be_bytes(receive.source.0);
                    source_port = receive.source_port;
                    length = receive.payload.len() as u16;
                    (NetworkStatus::Complete, request.endpoint)
                }
                None => (NetworkStatus::Busy, NetworkEndpoint(0)),
            }
        }
        NetworkOperation::ReceiveFrom => (NetworkStatus::Offline, NetworkEndpoint(0)),
        NetworkOperation::SendTo | NetworkOperation::Echo => {
            (NetworkStatus::Offline, NetworkEndpoint(0))
        }
    };
    NetworkReply {
        id: request.id,
        status,
        endpoint,
        generation: info.generation,
        source_address,
        source_port,
        length,
        info: status_info,
        counters,
    }
}

fn spin() -> ! {
    loop {
        core::hint::spin_loop();
    }
}
