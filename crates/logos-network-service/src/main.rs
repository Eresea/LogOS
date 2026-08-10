#![cfg_attr(not(test), no_main)]
#![cfg_attr(not(test), no_std)]

use logos_abi::{
    NetworkDeviceOperation, NetworkDeviceRequest, NetworkEndpoint, NetworkEvent, NetworkInfo,
    NetworkOperation, NetworkReply, NetworkRequest, NetworkStatus,
};
use logos_core::event::EventQueue;
use logos_net::{
    Arp, DHCP_ACK, DHCP_NAK, DHCP_OFFER, DHCP_OPTION_LEASE_TIME, DHCP_OPTION_MESSAGE_TYPE,
    DHCP_OPTION_ROUTER, DHCP_OPTION_SERVER_ID, DHCP_OPTION_SUBNET_MASK, DHCP_OPTION_T1,
    DHCP_OPTION_T2, DhcpAction, Ipv4, Mac, NetworkConfig, NetworkState, StateError, TcpTx,
    encode_arp, encode_dhcp_discover, encode_dhcp_request, encode_ethernet, encode_ipv4,
    encode_tcp, encode_udp, parse_arp, parse_dhcp, parse_ethernet, parse_icmp_echo, parse_ipv4,
    parse_tcp, parse_udp,
};
use logos_service_rt::{Header, NetworkServerRequest, ProtocolVersion, ServiceContext};

fn trace(_: &[u8]) {}

const DHCP_CLIENT: u16 = 68;
const DHCP_SERVER: u16 = 67;
const DEVICE_DEADLINE: u64 = u64::MAX / 2;
const BROADCAST: Ipv4 = Ipv4([255; 4]);
const CLIENT_PAYLOAD_OFFSET: usize = 2048;
const ICMP_PAYLOAD: usize = logos_abi::MAX_NETWORK_PAYLOAD;
const TCP_SERVICE_BUDGET: usize = 1;

#[derive(Clone, Copy)]
struct IcmpReply {
    destination_mac: Mac,
    source: Ipv4,
    identifier: u16,
    sequence: u16,
    length: u16,
    payload: [u8; ICMP_PAYLOAD],
}

#[derive(Clone, Copy, Default)]
struct TcpTxStage(Option<TcpTx>);

impl TcpTxStage {
    fn peek(&self) -> Option<TcpTx> {
        self.0
    }

    fn stage_from_state(&mut self, state: &mut NetworkState) {
        if self.0.is_none() {
            self.0 = state.tcp_mut().take_tx();
        }
    }

    fn submit<F>(&mut self, submit: F) -> bool
    where
        F: FnOnce(TcpTx) -> bool,
    {
        let Some(tx) = self.0 else { return false };
        if submit(tx) {
            self.0 = None;
            true
        } else {
            false
        }
    }
}

#[derive(Clone, Copy)]
struct ReceiveOperation {
    request: NetworkRequest,
    owner: u64,
}

#[derive(Clone, Copy)]
struct AcceptOperation {
    request: NetworkRequest,
    owner: u64,
}

#[derive(Clone, Copy)]
struct SendOperation {
    request: NetworkRequest,
    awaiting_arp: bool,
    submitted: bool,
}

#[derive(Clone, Copy)]
struct EchoOperation {
    request: NetworkRequest,
    awaiting_arp: bool,
}

#[derive(Default)]
struct PendingOperations {
    receive: Option<ReceiveOperation>,
    accept: Option<AcceptOperation>,
    send: Option<SendOperation>,
    echo: Option<EchoOperation>,
}

impl PendingOperations {
    fn reset(&mut self) {
        *self = Self::default();
    }
}

#[derive(Clone, Copy)]
enum NetworkStepEvent {
    ClientRequest(NetworkServerRequest),
    NetworkEvent(NetworkEvent),
}

struct NetworkReactor {
    events: EventQueue<NetworkStepEvent, 16>,
    /// Pending outbound operations shared between event-source and dispatch.
    operations: PendingOperations,
}

impl NetworkReactor {
    fn new() -> Self {
        Self { events: EventQueue::new(), operations: PendingOperations::default() }
    }

    fn push_event(&mut self, event: NetworkStepEvent) -> bool {
        self.events.push(event).is_ok()
    }

    fn pop_client_request(&mut self) -> Option<NetworkServerRequest> {
        if matches!(self.events.peek(), Some(NetworkStepEvent::ClientRequest(_))) {
            match self.events.pop() {
                Some(NetworkStepEvent::ClientRequest(request)) => Some(request),
                _ => None,
            }
        } else {
            None
        }
    }

    fn pop_network_event(&mut self) -> Option<NetworkEvent> {
        if matches!(self.events.peek(), Some(NetworkStepEvent::NetworkEvent(_))) {
            match self.events.pop() {
                Some(NetworkStepEvent::NetworkEvent(event)) => Some(event),
                _ => None,
            }
        } else {
            None
        }
    }
}

#[used]
#[unsafe(link_section = ".logos")]
static HEADER: Header =
    Header::new(*b"network\0\0\0\0\0\0\0\0\0", ProtocolVersion::V1, logos_service_entry);

#[unsafe(no_mangle)]
extern "C" fn logos_service_entry(context: logos_service_rt::EntryControlPage) -> ! {
    logos_service_rt::entry(context, run)
}

#[allow(clippy::collapsible_if)]
fn run(context: &mut ServiceContext) -> ! {
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
    let mut reactor = NetworkReactor::new();
    let mut next_echo = 1u16;
    let mut icmp_reply: Option<IcmpReply> = None;
    let mut tcp_stage = TcpTxStage::default();
    let mut counters = logos_abi::NetworkCounters::default();

    #[cfg(feature = "test-usernet")]
    configure_test_usernet(&mut state, now, 1);

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
                    #[cfg(feature = "test-usernet")]
                    {
                        configure_test_usernet(&mut state, now, xid);
                        pending_info = false;
                        pending = 0;
                        continue;
                    }
                    #[cfg(not(feature = "test-usernet"))]
                    {
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
                            tcp_stage.peek(),
                            next_id,
                        ) {
                            spin();
                        }
                        pending = next_id;
                        next_id = next_id.wrapping_add(1).max(1);
                        continue;
                    }
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
                    reactor.operations.reset();
                }
                if let Some(operation) = reactor.operations.send {
                    let request = operation.request;
                    if operation.awaiting_arp {
                        if device_reply.status != NetworkStatus::Complete {
                            reactor.operations.send = None;
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
                                stream_readiness: 0,
                                stream_reserved: 0,
                                stream_accepted_bytes: 0,
                                stream_acknowledged_bytes: 0,
                                info,
                                counters,
                            }) {
                                spin();
                            }
                        }
                    } else if operation.submitted {
                        reactor.operations.send = None;
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
                            stream_readiness: 0,
                            stream_reserved: 0,
                            stream_accepted_bytes: 0,
                            stream_acknowledged_bytes: 0,
                            counters,
                        };
                        if !context.network_reply_after_device(request, reply) {
                            spin();
                        }
                    }
                }
                if let Some(operation) = reactor.operations.echo {
                    let request = operation.request;
                    if operation.awaiting_arp {
                        if device_reply.status != NetworkStatus::Complete {
                            reactor.operations.echo = None;
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
                                stream_readiness: 0,
                                stream_reserved: 0,
                                stream_accepted_bytes: 0,
                                stream_acknowledged_bytes: 0,
                                info,
                                counters,
                            }) {
                                spin();
                            }
                        }
                    } else if device_reply.status != NetworkStatus::Complete {
                        reactor.operations.echo = None;
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
                            stream_readiness: 0,
                            stream_reserved: 0,
                            stream_accepted_bytes: 0,
                            stream_acknowledged_bytes: 0,
                            info,
                            counters,
                        }) {
                            spin();
                        }
                    }
                }
            }
        }

        for _ in 0..TCP_SERVICE_BUDGET {
            if pending != 0 {
                break;
            }
            let from_stage = tcp_stage.peek().is_some();
            let reply = match tcp_stage.peek() {
                Some(reply) => reply,
                None => {
                    let Some(reply) = state.tcp().peek_tx() else { break };
                    reply
                }
            };
            if from_stage {
                if !submit_staged_tcp(
                    context,
                    &state,
                    &info,
                    offer,
                    server,
                    arp_reply,
                    icmp_reply,
                    &mut tcp_stage,
                    next_id,
                ) {
                    break;
                }
                if let Some(operation) = reactor.operations.send
                    && !operation.awaiting_arp
                    && !operation.submitted
                {
                    reactor.operations.send = Some(SendOperation { submitted: true, ..operation });
                }
            } else {
                if !submit_action(
                    context,
                    &state,
                    &info,
                    DhcpAction::TcpReply,
                    offer,
                    server,
                    arp_reply,
                    icmp_reply,
                    Some(reply),
                    next_id,
                ) {
                    break;
                }
                let _ = state.tcp_mut().take_tx();
            }
            pending = next_id;
            next_id = next_id.wrapping_add(1).max(1);
        }

        if let Some(message) = context.network_server_request() {
            if !reactor.push_event(NetworkStepEvent::ClientRequest(message)) {
                spin();
            }
        }
        if let Some(message) = reactor.pop_client_request() {
            let request = message.request;
            let owner = message.caller;
            #[cfg(feature = "test-hooks")]
            inject_failure(request.id);
            if matches!(request.operation, NetworkOperation::Cancel | NetworkOperation::Close) {
                counters.cancellations = counters.cancellations.saturating_add(1);
                let endpoint = logos_net::EndpointId::from_wire(request.endpoint.0);
                let cancels_receive = reactor.operations.receive.is_some_and(|pending| {
                    owner == pending.owner
                        && endpoint == logos_net::EndpointId::from_wire(pending.request.endpoint.0)
                });
                let cancels_send = reactor.operations.send.is_some_and(|pending| {
                    endpoint == logos_net::EndpointId::from_wire(pending.request.endpoint.0)
                });
                let cancels_echo = reactor.operations.echo.is_some();
                if cancels_receive || cancels_send || cancels_echo {
                    if let Some(pending) = reactor.operations.receive.take() {
                        let _ = state.cancel_pending(pending.request.id);
                    }
                    if let Some(pending) = reactor.operations.send.take() {
                        let _ = state.cancel_pending(pending.request.id);
                    }
                    if cancels_echo {
                        reactor.operations.echo = None;
                        let _ = state.expire_echo(u64::MAX);
                    }
                }
                let status = if request.operation == NetworkOperation::Close {
                    endpoint.map_or(NetworkStatus::Invalid, |endpoint| {
                        if state.tcp_mut().close(owner, endpoint).is_ok()
                            || state.close(0, endpoint).is_ok()
                        {
                            NetworkStatus::Complete
                        } else {
                            NetworkStatus::Invalid
                        }
                    })
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
                    stream_readiness: 0,
                    stream_reserved: 0,
                    stream_accepted_bytes: 0,
                    stream_acknowledged_bytes: 0,
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
                        reactor.operations.send =
                            Some(SendOperation { request, awaiting_arp: false, submitted: true });
                        pending = next_id;
                        next_id = next_id.wrapping_add(1).max(1);
                        continue;
                    }
                    Ok(SubmitDatagram::Arp) => {
                        reactor.operations.send =
                            Some(SendOperation { request, awaiting_arp: true, submitted: false });
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
                            stream_readiness: 0,
                            stream_reserved: 0,
                            stream_accepted_bytes: 0,
                            stream_acknowledged_bytes: 0,
                            info,
                            counters,
                        }) {
                            spin();
                        }
                        continue;
                    }
                }
            }
            if request.operation == NetworkOperation::Listen {
                #[cfg(feature = "test-usernet")]
                if state.dhcp_config().is_none() {
                    let xid = state.dhcp_xid().max(1);
                    configure_test_usernet(&mut state, 1, xid);
                }
                let config = state.dhcp_config();
                let result = if cfg!(feature = "test-usernet") {
                    state.tcp_mut().listen(owner, request.peer.port(), request.id).ok()
                } else {
                    config.and_then(|_| {
                        state.tcp_mut().listen(owner, request.peer.port(), request.id).ok()
                    })
                }
                .map(|endpoint| NetworkReply {
                    id: request.id,
                    status: NetworkStatus::Complete,
                    endpoint: NetworkEndpoint(endpoint.wire()),
                    generation: info.generation,
                    source_address: 0,
                    source_port: 0,
                    length: 0,
                    stream_readiness: 0,
                    stream_reserved: 0,
                    stream_accepted_bytes: 0,
                    stream_acknowledged_bytes: 0,
                    info,
                    counters,
                })
                .unwrap_or_else(|| {
                    error_reply(
                        request,
                        if config.is_some() {
                            NetworkStatus::AddressInUse
                        } else {
                            NetworkStatus::Offline
                        },
                        info,
                        counters,
                    )
                });
                if !context.network_reply(result) {
                    spin();
                }
                continue;
            }
            if request.operation == NetworkOperation::Accept {
                let result = logos_net::EndpointId::from_wire(request.endpoint.0)
                    .ok_or(logos_net::TcpStateError::Invalid)
                    .and_then(|endpoint| state.tcp_mut().accept(owner, endpoint));
                if matches!(
                    result,
                    Err(logos_net::TcpStateError::Busy | logos_net::TcpStateError::NoData)
                ) {
                    reactor.operations.accept = Some(AcceptOperation { request, owner });
                    if !context.network_wait(request.deadline) {
                        spin();
                    }
                    continue;
                }
                let reply = match result {
                    Ok(endpoint) => {
                        let (source, source_port) =
                            state.tcp().peer(owner, endpoint).unwrap_or((Ipv4([0; 4]), 0));
                        NetworkReply {
                            id: request.id,
                            status: NetworkStatus::Complete,
                            endpoint: NetworkEndpoint(endpoint.wire()),
                            generation: info.generation,
                            source_address: u32::from_be_bytes(source.0),
                            source_port,
                            length: 0,
                            stream_readiness: 0,
                            stream_reserved: 0,
                            stream_accepted_bytes: 0,
                            stream_acknowledged_bytes: 0,
                            info,
                            counters,
                        }
                    }
                    Err(_) => error_reply(request, NetworkStatus::Busy, info, counters),
                };
                if !context.network_reply(reply) {
                    spin();
                }
                continue;
            }
            if request.operation == NetworkOperation::Read {
                let received = context.network_pages().and_then(|pages| {
                    let output = unsafe {
                        core::slice::from_raw_parts_mut(
                            (pages.tx_address + CLIENT_PAYLOAD_OFFSET as u64) as *mut u8,
                            logos_abi::MAX_TCP_PAYLOAD,
                        )
                    };
                    let endpoint = logos_net::EndpointId::from_wire(request.endpoint.0)?;
                    state.tcp_mut().read(owner, endpoint, output).ok()
                });
                if received.is_none() {
                    reactor.operations.receive = Some(ReceiveOperation { request, owner });
                    if !context.network_wait(request.deadline) {
                        spin();
                    }
                    continue;
                }
                let reply = NetworkReply {
                    id: request.id,
                    status: NetworkStatus::Complete,
                    endpoint: request.endpoint,
                    generation: request.generation,
                    source_address: 0,
                    source_port: 0,
                    length: received.unwrap_or(0) as u16,
                    info,
                    stream_readiness: 0,
                    stream_reserved: 0,
                    stream_accepted_bytes: 0,
                    stream_acknowledged_bytes: 0,
                    counters,
                };
                if !context.network_reply(reply) {
                    spin();
                }
                continue;
            }
            if request.operation == NetworkOperation::SubmitWrite {
                let Some(pages) = context.network_pages() else {
                    if !context.network_reply(error_reply(
                        request,
                        NetworkStatus::Offline,
                        info,
                        counters,
                    )) {
                        spin();
                    }
                    continue;
                };
                let payload = unsafe {
                    core::slice::from_raw_parts(
                        (pages.tx_address + CLIENT_PAYLOAD_OFFSET as u64) as *const u8,
                        usize::from(request.length),
                    )
                };
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
                let result = state.tcp_mut().submit_write(owner, endpoint, payload);
                let (status, readiness, accepted_bytes, acknowledged_bytes) = match result {
                    Ok(accepted) => state.tcp().stream_watermarks(owner, endpoint).map_or(
                        (NetworkStatus::Complete, 0, accepted, 0),
                        |(_, acknowledged)| {
                            let readiness = state
                                .tcp()
                                .stream_state(owner, endpoint)
                                .map_or(0, |(readiness, _, _)| readiness);
                            (NetworkStatus::Complete, readiness, accepted, acknowledged)
                        },
                    ),
                    Err(error) => (map_tcp_error(error), 0, 0, 0),
                };
                if status == NetworkStatus::Complete {
                    let _ = context.publish_stream(logos_abi::NetworkStreamRecord {
                        owner,
                        endpoint: request.endpoint,
                        generation: request.generation,
                        readiness,
                        status,
                        reserved: 0,
                        sequence: 0,
                        accepted_bytes,
                        acknowledged_bytes,
                    });
                }
                if !context.network_reply(NetworkReply {
                    id: request.id,
                    status,
                    endpoint: if status == NetworkStatus::Complete {
                        request.endpoint
                    } else {
                        NetworkEndpoint(0)
                    },
                    generation: request.generation,
                    source_address: 0,
                    source_port: 0,
                    length: if status == NetworkStatus::Complete { request.length } else { 0 },
                    stream_readiness: if status == NetworkStatus::Complete { readiness } else { 0 },
                    stream_reserved: 0,
                    stream_accepted_bytes: if status == NetworkStatus::Complete {
                        accepted_bytes
                    } else {
                        0
                    },
                    stream_acknowledged_bytes: if status == NetworkStatus::Complete {
                        acknowledged_bytes
                    } else {
                        0
                    },
                    info,
                    counters,
                }) {
                    spin();
                }
                continue;
            }
            if request.operation == NetworkOperation::PollStream {
                let result = logos_net::EndpointId::from_wire(request.endpoint.0)
                    .ok_or(logos_net::TcpStateError::Invalid)
                    .and_then(|endpoint| state.tcp().stream_state(owner, endpoint));
                let (status, readiness, accepted_bytes, acknowledged_bytes) = match result {
                    Ok((readiness, accepted_bytes, acknowledged_bytes)) => {
                        (NetworkStatus::Complete, readiness, accepted_bytes, acknowledged_bytes)
                    }
                    Err(error) => (map_tcp_error(error), 0, 0, 0),
                };
                if !context.network_reply(NetworkReply {
                    id: request.id,
                    status,
                    endpoint: if status == NetworkStatus::Complete {
                        request.endpoint
                    } else {
                        NetworkEndpoint(0)
                    },
                    generation: request.generation,
                    source_address: 0,
                    source_port: 0,
                    length: 0,
                    stream_readiness: readiness,
                    stream_reserved: 0,
                    stream_accepted_bytes: accepted_bytes,
                    stream_acknowledged_bytes: acknowledged_bytes,
                    info,
                    counters,
                }) {
                    spin();
                }
                continue;
            }
            if request.operation == NetworkOperation::Write {
                let Some(pages) = context.network_pages() else {
                    if !context.network_reply(error_reply(
                        request,
                        NetworkStatus::Offline,
                        info,
                        counters,
                    )) {
                        spin();
                    }
                    continue;
                };
                let payload = unsafe {
                    core::slice::from_raw_parts(
                        (pages.tx_address + CLIENT_PAYLOAD_OFFSET as u64) as *const u8,
                        usize::from(request.length),
                    )
                };
                let status =
                    if let Some(endpoint) = logos_net::EndpointId::from_wire(request.endpoint.0) {
                        if state.tcp_mut().write(owner, endpoint, payload).is_ok() {
                            tcp_stage.stage_from_state(&mut state);
                            if tcp_stage.peek().is_some() {
                                if pending != 0 {
                                    reactor.operations.send = Some(SendOperation {
                                        request,
                                        awaiting_arp: false,
                                        submitted: false,
                                    });
                                    continue;
                                }
                                if submit_staged_tcp(
                                    context,
                                    &state,
                                    &info,
                                    offer,
                                    server,
                                    arp_reply,
                                    icmp_reply,
                                    &mut tcp_stage,
                                    next_id,
                                ) {
                                    reactor.operations.send = Some(SendOperation {
                                        request,
                                        awaiting_arp: false,
                                        submitted: true,
                                    });
                                    pending = next_id;
                                    next_id = next_id.wrapping_add(1).max(1);
                                    continue;
                                }
                            }
                            NetworkStatus::Io
                        } else {
                            NetworkStatus::Busy
                        }
                    } else {
                        NetworkStatus::Invalid
                    };
                if !context.network_reply(NetworkReply {
                    id: request.id,
                    status,
                    endpoint: if status == NetworkStatus::Complete {
                        request.endpoint
                    } else {
                        NetworkEndpoint(0)
                    },
                    generation: request.generation,
                    source_address: 0,
                    source_port: 0,
                    length: if status == NetworkStatus::Complete { request.length } else { 0 },
                    stream_readiness: 0,
                    stream_reserved: 0,
                    stream_accepted_bytes: 0,
                    stream_acknowledged_bytes: 0,
                    info,
                    counters,
                }) {
                    spin();
                }
                continue;
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
                        reactor.operations.echo =
                            Some(EchoOperation { request, awaiting_arp: false });
                        pending = next_id;
                        next_id = next_id.wrapping_add(1).max(1);
                        continue;
                    }
                    Ok(SubmitDatagram::Arp) => {
                        reactor.operations.echo =
                            Some(EchoOperation { request, awaiting_arp: true });
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
                    reactor.operations.receive = Some(ReceiveOperation { request, owner });
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
            if !reactor.push_event(NetworkStepEvent::NetworkEvent(event)) {
                spin();
            }
        }
        if let Some(event) = reactor.pop_network_event() {
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
                        &mut tcp_stage,
                        &mut counters,
                    )
                }
                logos_abi::NetworkEventKind::Timer => {
                    if state.tcp_mut().tick(now) {
                        tcp_stage.stage_from_state(&mut state);
                        DhcpAction::TcpReply
                    } else {
                        state.dhcp_tick(now)
                    }
                }
                logos_abi::NetworkEventKind::Reset => {
                    counters.resets = counters.resets.saturating_add(1);
                    state.reset();
                    state.dhcp_start(now, xid);
                    DhcpAction::Discover
                }
                logos_abi::NetworkEventKind::Cancel => DhcpAction::None,
            };
            if let Some(operation) = reactor.operations.echo
                && !operation.awaiting_arp
                && state.echo().is_none()
            {
                let request = operation.request;
                reactor.operations.echo = None;
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
                        stream_readiness: 0,
                        stream_reserved: 0,
                        stream_accepted_bytes: 0,
                        stream_acknowledged_bytes: 0,
                        info,
                        counters,
                    },
                ) {
                    spin();
                }
                continue;
            }
            if let Some(operation) = reactor.operations.receive {
                let request = operation.request;
                let owner = operation.owner;
                if request.operation == NetworkOperation::Read {
                    let length = context.network_pages().and_then(|pages| {
                        let output = unsafe {
                            core::slice::from_raw_parts_mut(
                                (pages.tx_address + CLIENT_PAYLOAD_OFFSET as u64) as *mut u8,
                                logos_abi::MAX_TCP_PAYLOAD,
                            )
                        };
                        let endpoint = logos_net::EndpointId::from_wire(request.endpoint.0)?;
                        state.tcp_mut().read(owner, endpoint, output).ok()
                    });
                    if let Some(length) = length {
                        reactor.operations.receive = None;
                        if !context.network_reply_after_event(
                            request,
                            NetworkReply {
                                id: request.id,
                                status: NetworkStatus::Complete,
                                endpoint: request.endpoint,
                                generation: info.generation,
                                source_address: 0,
                                source_port: 0,
                                length: length as u16,
                                stream_readiness: 0,
                                stream_reserved: 0,
                                stream_accepted_bytes: 0,
                                stream_acknowledged_bytes: 0,
                                info,
                                counters,
                            },
                        ) {
                            spin();
                        }
                        continue;
                    }
                    if now >= request.deadline {
                        reactor.operations.receive = None;
                        if !context.network_reply_after_event(
                            request,
                            error_reply(request, NetworkStatus::TimedOut, info, counters),
                        ) {
                            spin();
                        }
                        continue;
                    }
                    continue;
                }
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
                    reactor.operations.receive = None;
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
                            stream_readiness: 0,
                            stream_reserved: 0,
                            stream_accepted_bytes: 0,
                            stream_acknowledged_bytes: 0,
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
                    reactor.operations.receive = None;
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
                            stream_readiness: 0,
                            stream_reserved: 0,
                            stream_accepted_bytes: 0,
                            stream_acknowledged_bytes: 0,
                            info,
                            counters,
                        },
                    ) {
                        spin();
                    }
                    continue;
                }
            }
            if let Some(operation) = reactor.operations.accept {
                let request = operation.request;
                let owner = operation.owner;
                let result = logos_net::EndpointId::from_wire(request.endpoint.0)
                    .ok_or(logos_net::TcpStateError::Invalid)
                    .and_then(|endpoint| state.tcp_mut().accept(owner, endpoint));
                if let Ok(endpoint) = result {
                    reactor.operations.accept = None;
                    let (source, source_port) =
                        state.tcp().peer(owner, endpoint).unwrap_or((Ipv4([0; 4]), 0));
                    if !context.network_reply_after_event(
                        request,
                        NetworkReply {
                            id: request.id,
                            status: NetworkStatus::Complete,
                            endpoint: NetworkEndpoint(endpoint.wire()),
                            generation: info.generation,
                            source_address: u32::from_be_bytes(source.0),
                            source_port,
                            length: 0,
                            stream_readiness: 0,
                            stream_reserved: 0,
                            stream_accepted_bytes: 0,
                            stream_acknowledged_bytes: 0,
                            info,
                            counters,
                        },
                    ) {
                        spin();
                    }
                    continue;
                }
                if now >= request.deadline {
                    reactor.operations.accept = None;
                    if !context.network_reply_after_event(
                        request,
                        error_reply(request, NetworkStatus::TimedOut, info, counters),
                    ) {
                        spin();
                    }
                    continue;
                }
            }
            if let Some(operation) = reactor.operations.send
                && operation.awaiting_arp
                && state.arp_target().is_none()
            {
                let request = operation.request;
                if now >= request.deadline {
                    counters.timeouts = counters.timeouts.saturating_add(1);
                    reactor.operations.send = None;
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
                            stream_readiness: 0,
                            stream_reserved: 0,
                            stream_accepted_bytes: 0,
                            stream_acknowledged_bytes: 0,
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
                    reactor.operations.send =
                        Some(SendOperation { request, awaiting_arp: false, submitted: true });
                    pending = next_id;
                    next_id = next_id.wrapping_add(1).max(1);
                    continue;
                }
            }
            if let Some(operation) = reactor.operations.echo
                && operation.awaiting_arp
                && state.arp_target().is_none()
            {
                let request = operation.request;
                if now >= request.deadline {
                    counters.timeouts = counters.timeouts.saturating_add(1);
                    reactor.operations.echo = None;
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
                    reactor.operations.echo = Some(EchoOperation { request, awaiting_arp: false });
                    pending = next_id;
                    next_id = next_id.wrapping_add(1).max(1);
                    continue;
                }
            }
            if action == DhcpAction::TcpReply {
                if pending != 0 {
                    continue;
                }
                if !submit_staged_tcp(
                    context,
                    &state,
                    &info,
                    offer,
                    server,
                    arp_reply,
                    icmp_reply,
                    &mut tcp_stage,
                    next_id,
                ) {
                    spin();
                }
                pending = next_id;
                next_id = next_id.wrapping_add(1).max(1);
                continue;
            }
            if action != DhcpAction::None && action != DhcpAction::Expired {
                if !submit_action(
                    context,
                    &state,
                    &info,
                    action,
                    offer,
                    server,
                    arp_reply,
                    icmp_reply,
                    tcp_stage.peek(),
                    next_id,
                ) {
                    spin();
                }
                pending = next_id;
                next_id = next_id.wrapping_add(1).max(1);
                continue;
            }
        }

        let deadline = if state.tcp().peek_tx().is_some() {
            now.saturating_add(1)
        } else {
            state.dhcp_deadline().max(now.saturating_add(1))
        };
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

fn issue_info(context: &mut ServiceContext, id: u32) -> bool {
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
    context: &mut ServiceContext,
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
    context: &mut ServiceContext,
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
    context: &mut ServiceContext,
    state: &NetworkState,
    info: &NetworkInfo,
    action: DhcpAction,
    offer: Ipv4,
    server: Ipv4,
    arp_reply: Option<Arp>,
    icmp_reply: Option<IcmpReply>,
    tcp_reply: Option<TcpTx>,
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
    if action == DhcpAction::TcpReply {
        let Some(reply) = tcp_reply else { return false };
        let Some(config) = state.dhcp_config() else { return false };
        let remote = state.resolve_arp(reply.destination, 0).unwrap_or(Mac::BROADCAST);
        let mut tcp = [0; logos_net::TCP_HEADER + logos_net::MAX_TCP_STREAM];
        let Ok(tcp_length) = encode_tcp(
            &mut tcp,
            config.address,
            reply.destination,
            reply.header,
            &reply.payload[..usize::from(reply.length)],
        ) else {
            return false;
        };
        let mut ipv4 =
            [0; logos_net::IPV4_HEADER + logos_net::TCP_HEADER + logos_net::MAX_TCP_STREAM];
        let Ok(ipv4_length) = encode_ipv4(
            &mut ipv4,
            config.address,
            reply.destination,
            id as u16,
            6,
            &tcp[..tcp_length],
        ) else {
            return false;
        };
        let tx = unsafe { core::slice::from_raw_parts_mut(pages.tx_address as *mut u8, 4096) };
        let Ok(frame_length) = encode_ethernet(tx, remote, mac, 0x0800, &ipv4[..ipv4_length])
        else {
            return false;
        };
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
        DhcpAction::None
        | DhcpAction::Expired
        | DhcpAction::ArpReply
        | DhcpAction::IcmpReply
        | DhcpAction::TcpReply => {
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
fn submit_staged_tcp(
    context: &mut ServiceContext,
    state: &NetworkState,
    info: &NetworkInfo,
    offer: Ipv4,
    server: Ipv4,
    arp_reply: Option<Arp>,
    icmp_reply: Option<IcmpReply>,
    tcp_stage: &mut TcpTxStage,
    id: u32,
) -> bool {
    tcp_stage.submit(|reply| {
        submit_action(
            context,
            state,
            info,
            DhcpAction::TcpReply,
            offer,
            server,
            arp_reply,
            icmp_reply,
            Some(reply),
            id,
        )
    })
}

#[allow(clippy::too_many_arguments)]
fn accept_dhcp(
    context: &ServiceContext,
    length: u16,
    now: u64,
    state: &mut NetworkState,
    info: NetworkInfo,
    offer: &mut Ipv4,
    server: &mut Ipv4,
    arp_reply: &mut Option<Arp>,
    icmp_reply: &mut Option<IcmpReply>,
    tcp_stage: &mut TcpTxStage,
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
        if arp.sender_ip.0 != [0; 4] {
            state.learn_arp(arp.sender_ip, arp.sender_mac, now, 60);
        }
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
    if ip.protocol == 6 {
        state.learn_arp(ip.source, ethernet.source, now, 60);
        let tcp = match parse_tcp(ip.payload, ip.source, ip.destination) {
            Ok(tcp) => tcp,
            Err(_) => {
                #[cfg(feature = "test-usernet")]
                if let Some(tcp) = parse_tcp_usernet(ip.payload) {
                    tcp
                } else {
                    counters.malformed = counters.malformed.saturating_add(1);
                    return DhcpAction::None;
                }
                #[cfg(not(feature = "test-usernet"))]
                {
                    counters.malformed = counters.malformed.saturating_add(1);
                    return DhcpAction::None;
                }
            }
        };
        let _ = state.tcp_mut().ingest(ip.source, tcp);
        tcp_stage.stage_from_state(state);
        return tcp_stage.peek().map_or(DhcpAction::None, |_| DhcpAction::TcpReply);
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

#[cfg(feature = "test-usernet")]
fn parse_tcp_usernet(bytes: &[u8]) -> Option<logos_net::Tcp<'_>> {
    if bytes.len() < logos_net::TCP_HEADER {
        return None;
    }
    let header = usize::from(bytes[12] >> 4) * 4;
    (header >= logos_net::TCP_HEADER && header <= bytes.len()).then_some(logos_net::Tcp {
        source_port: u16::from_be_bytes([bytes[0], bytes[1]]),
        destination_port: u16::from_be_bytes([bytes[2], bytes[3]]),
        sequence: u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
        acknowledgement: u32::from_be_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]),
        flags: bytes[13],
        window: u16::from_be_bytes([bytes[14], bytes[15]]),
        payload: &bytes[header..],
    })
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

#[cfg(feature = "test-usernet")]
fn configure_test_usernet(state: &mut NetworkState, now: u64, xid: u32) {
    state.dhcp_start(now, xid);
    if state.dhcp_offer(now, xid) {
        let _ = state.dhcp_acknowledge(
            now,
            xid,
            NetworkConfig {
                address: Ipv4([10, 0, 2, 15]),
                mask: Ipv4([255, 255, 255, 0]),
                router: Some(Ipv4([10, 0, 2, 2])),
                lease_until: now.saturating_add(600),
                renew_at: now.saturating_add(300),
                rebind_at: now.saturating_add(525),
            },
        );
    }
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

fn map_tcp_error(error: logos_net::TcpStateError) -> NetworkStatus {
    match error {
        logos_net::TcpStateError::Busy => NetworkStatus::Busy,
        logos_net::TcpStateError::NoData => NetworkStatus::Busy,
        logos_net::TcpStateError::Owner => NetworkStatus::Denied,
        logos_net::TcpStateError::Invalid | logos_net::TcpStateError::NotFound => {
            NetworkStatus::Invalid
        }
        logos_net::TcpStateError::MessageTooLarge => NetworkStatus::MessageTooLarge,
        logos_net::TcpStateError::Full | logos_net::TcpStateError::AddressInUse => {
            NetworkStatus::Full
        }
        logos_net::TcpStateError::Reset => NetworkStatus::Reset,
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
        stream_readiness: 0,
        stream_reserved: 0,
        stream_accepted_bytes: 0,
        stream_acknowledged_bytes: 0,
        info,
        counters,
    }
}

fn handle_request(
    state: &mut NetworkState,
    info: NetworkInfo,
    request: NetworkRequest,
    pages: Option<logos_service_rt::NetworkDmaResources>,
    counters: logos_abi::NetworkCounters,
) -> NetworkReply {
    #[cfg(feature = "test-usernet")]
    if state.dhcp_config().is_none() {
        let xid = state.dhcp_xid().max(1);
        configure_test_usernet(state, 1, xid);
    }
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
        NetworkOperation::SendTo
        | NetworkOperation::Echo
        | NetworkOperation::Listen
        | NetworkOperation::Accept
        | NetworkOperation::Read
        | NetworkOperation::Write
        | NetworkOperation::SubmitWrite
        | NetworkOperation::PollStream => (NetworkStatus::Offline, NetworkEndpoint(0)),
    };
    NetworkReply {
        id: request.id,
        status,
        endpoint,
        generation: info.generation,
        source_address,
        source_port,
        length,
        stream_readiness: 0,
        stream_reserved: 0,
        stream_accepted_bytes: 0,
        stream_acknowledged_bytes: 0,
        info: status_info,
        counters,
    }
}

fn spin() -> ! {
    loop {
        core::hint::spin_loop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use logos_abi::{NetworkScope, PageHandle};

    fn request(id: u32) -> NetworkRequest {
        NetworkRequest {
            id,
            operation: NetworkOperation::Read,
            endpoint: NetworkEndpoint(0),
            peer: NetworkScope(0),
            page: PageHandle(0),
            length: 0,
            generation: 1,
            deadline: 10,
        }
    }

    fn tcp_tx() -> TcpTx {
        TcpTx {
            source: Ipv4([10, 0, 2, 15]),
            destination: Ipv4([10, 0, 2, 2]),
            header: logos_net::TcpHeader {
                source_port: 40000,
                destination_port: 7443,
                sequence: 1,
                acknowledgement: 1,
                flags: 0x10,
                window: 1024,
            },
            length: 0,
            payload: [0; logos_net::MAX_TCP_STREAM],
        }
    }

    #[test]
    fn staged_tcp_tx_transfers_ownership_once() {
        let mut stage = TcpTxStage(Some(tcp_tx()));
        let mut attempts = 0;
        let mut accepted = 0;

        assert!(!stage.submit(|_| {
            attempts += 1;
            false
        }));
        assert!(stage.peek().is_some());
        assert!(stage.submit(|_| {
            attempts += 1;
            accepted += 1;
            true
        }));
        assert!(!stage.submit(|_| {
            attempts += 1;
            accepted += 1;
            true
        }));
        assert_eq!(attempts, 2);
        assert_eq!(accepted, 1);
        assert!(stage.peek().is_none());

        let mut state = NetworkState::new();
        stage.stage_from_state(&mut state);
        assert!(stage.peek().is_none());
    }

    #[test]
    fn pending_operations_reset_as_one_owned_state() {
        let mut reactor = NetworkReactor::new();
        reactor.operations = PendingOperations {
            receive: Some(ReceiveOperation { request: request(1), owner: 7 }),
            accept: Some(AcceptOperation { request: request(2), owner: 8 }),
            send: Some(SendOperation { request: request(3), awaiting_arp: true, submitted: false }),
            echo: Some(EchoOperation { request: request(4), awaiting_arp: false }),
        };

        reactor.operations.reset();

        assert!(reactor.operations.receive.is_none());
        assert!(reactor.operations.accept.is_none());
        assert!(reactor.operations.send.is_none());
        assert!(reactor.operations.echo.is_none());
    }

    #[test]
    fn pending_send_transition_keeps_request_identity() {
        let request = request(9);
        let mut reactor = NetworkReactor::new();
        reactor.operations = PendingOperations {
            send: Some(SendOperation { request, awaiting_arp: true, submitted: false }),
            ..PendingOperations::default()
        };

        let operation = reactor.operations.send.take().expect("pending send");
        reactor.operations.send = Some(SendOperation { awaiting_arp: false, ..operation });

        assert_eq!(reactor.operations.send.map(|operation| operation.request.id), Some(9));
        assert!(!reactor.operations.send.is_some_and(|operation| operation.awaiting_arp));
    }

    #[test]
    fn reactor_queues_events_fifo() {
        let mut reactor = NetworkReactor::new();
        let event = NetworkEvent {
            id: 1,
            kind: logos_abi::NetworkEventKind::Timer,
            generation: 1,
            device_generation: 1,
            page: PageHandle(0),
            length: 0,
            now: 1,
            metadata: [0; 16],
        };

        assert!(reactor.push_event(NetworkStepEvent::NetworkEvent(event)));
        assert!(reactor.pop_network_event().is_some());
        assert!(reactor.pop_network_event().is_none());
    }

    #[test]
    fn reactor_keeps_unexpected_event_at_fifo_head() {
        let mut reactor = NetworkReactor::new();
        let event = NetworkEvent {
            id: 1,
            kind: logos_abi::NetworkEventKind::Timer,
            generation: 1,
            device_generation: 1,
            page: PageHandle(0),
            length: 0,
            now: 1,
            metadata: [0; 16],
        };

        assert!(reactor.push_event(NetworkStepEvent::NetworkEvent(event)));
        assert!(reactor.pop_client_request().is_none());
        assert_eq!(reactor.events.len(), 1);
        assert!(reactor.pop_network_event().is_some());
    }
}
