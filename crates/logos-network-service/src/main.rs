#![cfg_attr(not(test), no_main)]
#![cfg_attr(not(test), no_std)]

use logos_abi::{
    NetworkDeviceOperation, NetworkDeviceRequest, NetworkEndpoint, NetworkInfo, NetworkOperation,
    NetworkReply, NetworkRequest, NetworkStatus,
};
use logos_net::{
    DHCP_ACK, DHCP_NAK, DHCP_OFFER, DHCP_OPTION_LEASE_TIME, DHCP_OPTION_MESSAGE_TYPE,
    DHCP_OPTION_ROUTER, DHCP_OPTION_SERVER_ID, DHCP_OPTION_SUBNET_MASK, DHCP_OPTION_T1,
    DHCP_OPTION_T2, DhcpAction, Ipv4, Mac, NetworkConfig, NetworkState, StateError,
    encode_dhcp_discover, encode_dhcp_request, encode_ethernet, encode_ipv4, encode_udp,
    parse_dhcp, parse_ethernet, parse_ipv4, parse_udp,
};
use logos_service_rt::{Context, Header};

const DHCP_CLIENT: u16 = 68;
const DHCP_SERVER: u16 = 67;
const DEVICE_DEADLINE: u64 = u64::MAX / 2;
const BROADCAST: Ipv4 = Ipv4([255; 4]);

#[used]
#[unsafe(link_section = ".logos")]
static HEADER: Header = Header::new(*b"network\0\0\0\0\0\0\0\0\0", logos_service_entry);

#[unsafe(no_mangle)]
extern "C" fn logos_service_entry(context: logos_service_rt::EntryContext) -> ! {
    logos_service_rt::entry(context, run)
}

fn run(context: &mut Context) -> ! {
    if !context.ready() {
        spin();
    }
    let mut state = NetworkState::new();
    let mut info = NetworkInfo::default();
    let mut offer = Ipv4([0; 4]);
    let mut server = Ipv4([0; 4]);
    let mut pending = 1;
    let mut pending_info = true;
    let mut next_id = 2u32;
    let mut now = 1u64;
    let mut xid = 0x4c4f_474fu32;

    if !issue_info(context, pending) {
        spin();
    }
    while context.acknowledged() {
        if pending != 0 {
            if let Some(reply) = context.network_device_reply(pending) {
                if pending_info {
                    if reply.status != NetworkStatus::Complete || reply.info.mac == [0; 6] {
                        spin();
                    }
                    info = reply.info;
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
                        next_id,
                    ) {
                        spin();
                    }
                    pending = next_id;
                    next_id = next_id.wrapping_add(1).max(1);
                    continue;
                }
                pending = 0;
                if reply.status == NetworkStatus::Reset {
                    info = reply.info;
                    state.reset();
                    state.dhcp_start(now, xid);
                }
            }
        }

        if let Some(request) = context.network_request() {
            let reply = handle_request(&mut state, info, request);
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
                logos_abi::NetworkEventKind::Frame => accept_dhcp(
                    context,
                    event.length,
                    now,
                    &mut state,
                    info,
                    &mut offer,
                    &mut server,
                ),
                logos_abi::NetworkEventKind::Timer => state.dhcp_tick(now),
                logos_abi::NetworkEventKind::Reset => {
                    state.reset();
                    state.dhcp_start(now, xid);
                    DhcpAction::Discover
                }
                logos_abi::NetworkEventKind::Cancel => DhcpAction::None,
            };
            if action != DhcpAction::None && action != DhcpAction::Expired {
                if !submit_action(context, &state, &info, action, offer, server, next_id) {
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

fn issue_info(context: &mut Context, id: u32) -> bool {
    context.network_device_request(NetworkDeviceRequest {
        id,
        operation: NetworkDeviceOperation::Info,
        length: 0,
        generation: 0,
        deadline: DEVICE_DEADLINE,
    })
}

fn submit_action(
    context: &mut Context,
    state: &NetworkState,
    info: &NetworkInfo,
    action: DhcpAction,
    offer: Ipv4,
    server: Ipv4,
    id: u32,
) -> bool {
    let Some(pages) = context.network_pages() else {
        return false;
    };
    if info.generation == 0 || id == 0 {
        return false;
    }
    let mac = Mac(info.mac);
    let mut dhcp = [0; 300];
    let dhcp_length = match action {
        DhcpAction::Discover => encode_dhcp_discover(&mut dhcp, state.dhcp_xid(), mac),
        DhcpAction::Request | DhcpAction::Renew | DhcpAction::Rebind => {
            encode_dhcp_request(&mut dhcp, state.dhcp_xid(), mac, offer, server)
        }
        DhcpAction::None | DhcpAction::Expired => return false,
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

fn accept_dhcp(
    context: &Context,
    length: u16,
    now: u64,
    state: &mut NetworkState,
    info: NetworkInfo,
    offer: &mut Ipv4,
    server: &mut Ipv4,
) -> DhcpAction {
    let Some(pages) = context.network_pages() else {
        return DhcpAction::None;
    };
    let length = usize::from(length);
    if !(logos_net::ETHERNET_MIN_FRAME..=logos_net::ETHERNET_MAX_FRAME).contains(&length) {
        return DhcpAction::None;
    }
    let frame = unsafe { core::slice::from_raw_parts(pages.rx_address as *const u8, length) };
    let Ok(ethernet) = parse_ethernet(frame, Mac(info.mac)) else {
        return DhcpAction::None;
    };
    if ethernet.ether_type != 0x0800 {
        return DhcpAction::None;
    }
    let ip = parse_ipv4(ethernet.payload, BROADCAST)
        .or_else(|_| parse_ipv4(ethernet.payload, Ipv4(info.ipv4.to_be_bytes())));
    let Ok(ip) = ip else {
        return DhcpAction::None;
    };
    if ip.protocol != 17 {
        return DhcpAction::None;
    }
    let Ok(udp) = parse_udp(ip.payload, ip.source, ip.destination) else {
        return DhcpAction::None;
    };
    if udp.source_port != DHCP_SERVER || udp.destination_port != DHCP_CLIENT {
        return DhcpAction::None;
    }
    let Ok(dhcp) = parse_dhcp(udp.payload) else {
        return DhcpAction::None;
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
            state
                .dhcp_offer(now, state.dhcp_xid())
                .then_some(DhcpAction::Request)
                .unwrap_or(DhcpAction::None)
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

fn handle_request(
    state: &mut NetworkState,
    info: NetworkInfo,
    request: NetworkRequest,
) -> NetworkReply {
    let config = state.dhcp_config();
    let status_info = config.map_or(info, |config| NetworkInfo {
        configuration: 1,
        ipv4: u32::from_be_bytes(config.address.0),
        subnet_mask: u32::from_be_bytes(config.mask.0),
        router: config.router.map_or(0, |router| u32::from_be_bytes(router.0)),
        ..info
    });
    let (status, endpoint) = match request.operation {
        NetworkOperation::Status => {
            (config.map_or(NetworkStatus::Offline, |_| NetworkStatus::Complete), NetworkEndpoint(0))
        }
        NetworkOperation::Bind if config.is_some() => match state.bind(1, request.peer.port()) {
            Ok(endpoint) => (NetworkStatus::Complete, NetworkEndpoint(endpoint.wire())),
            Err(StateError::AddressInUse) => (NetworkStatus::AddressInUse, NetworkEndpoint(0)),
            Err(StateError::Full) => (NetworkStatus::Full, NetworkEndpoint(0)),
            Err(_) => (NetworkStatus::Invalid, NetworkEndpoint(0)),
        },
        NetworkOperation::Bind => (NetworkStatus::Offline, NetworkEndpoint(0)),
        NetworkOperation::Close => match logos_net::EndpointId::from_wire(request.endpoint.0)
            .and_then(|endpoint| state.close(1, endpoint).ok())
        {
            Some(()) => (NetworkStatus::Complete, NetworkEndpoint(0)),
            None => (NetworkStatus::Invalid, NetworkEndpoint(0)),
        },
        NetworkOperation::Cancel => (NetworkStatus::Cancelled, NetworkEndpoint(0)),
        NetworkOperation::SendTo | NetworkOperation::ReceiveFrom | NetworkOperation::Echo => {
            (NetworkStatus::Offline, NetworkEndpoint(0))
        }
    };
    NetworkReply {
        id: request.id,
        status,
        endpoint,
        generation: info.generation,
        source_address: 0,
        source_port: 0,
        length: 0,
        info: status_info,
        counters: logos_abi::NetworkCounters::default(),
    }
}

fn spin() -> ! {
    loop {
        core::hint::spin_loop();
    }
}
