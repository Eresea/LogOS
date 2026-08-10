#![cfg_attr(not(test), no_main)]
#![cfg_attr(not(test), no_std)]

use logos_abi::{
    MAX_TCP_PAYLOAD, NetworkEndpoint, NetworkOperation, NetworkProtocol, NetworkRequest,
    NetworkScope, NetworkStatus, NetworkStreamReadiness, REMOTE_TCP_PORT,
    service::{RemoteGateOperation, RemoteGateStatus, RemotePageRequest},
};
use logos_remote::{
    FrameDecoder, MAX_FRAME, MAX_FRAME_BUFFER, RemoteMessage, RemoteMessageKind, frame_encode,
};
use logos_service_rt::{Header, ProtocolVersion, ServiceContext};

const DEADLINE: u64 = u64::MAX / 2;
const STREAM_CLOSED: u16 = NetworkStreamReadiness::Closed.bits();
const STREAM_WRITABLE: u16 = NetworkStreamReadiness::Writable.bits();
const MAX_CONNECTIONS: usize = 4;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ConnectionPhase {
    Read,
    Decode,
    Remote,
    Write,
    Close,
}

#[derive(Clone, Copy)]
struct ConnectionEntry {
    stream: logos_abi::NetworkReply,
    phase: ConnectionPhase,
    authenticated: bool,
}

struct ConnectionTable {
    entries: [Option<ConnectionEntry>; MAX_CONNECTIONS],
}

impl ConnectionTable {
    const fn new() -> Self {
        Self { entries: [None; MAX_CONNECTIONS] }
    }

    fn insert(&mut self, stream: logos_abi::NetworkReply) -> Option<usize> {
        let index = self.entries.iter().position(Option::is_none)?;
        self.entries[index] =
            Some(ConnectionEntry { stream, phase: ConnectionPhase::Read, authenticated: false });
        Some(index)
    }

    fn phase(&mut self, index: usize, phase: ConnectionPhase) -> bool {
        let Some(entry) = self.entries.get_mut(index).and_then(Option::as_mut) else {
            return false;
        };
        if entry.stream.endpoint.0 == 0 {
            return false;
        }
        entry.phase = phase;
        true
    }

    fn authenticate(&mut self, index: usize) -> bool {
        let Some(entry) = self.entries.get_mut(index).and_then(Option::as_mut) else {
            return false;
        };
        entry.authenticated = true;
        true
    }

    fn remove(&mut self, index: usize) -> Option<ConnectionEntry> {
        self.entries.get_mut(index)?.take()
    }
}

#[derive(Clone, Copy)]
struct StreamTxState {
    endpoint: NetworkEndpoint,
    generation: u16,
    accepted: u64,
}

impl StreamTxState {
    fn new(stream: logos_abi::NetworkReply) -> Self {
        Self {
            endpoint: stream.endpoint,
            generation: stream.generation,
            accepted: stream.stream_accepted_bytes,
        }
    }

    fn observe_poll(&mut self, reply: logos_abi::NetworkReply) -> bool {
        if reply.status != NetworkStatus::Complete
            || reply.endpoint != self.endpoint
            || reply.generation != self.generation
            || reply.stream_readiness & STREAM_CLOSED != 0
            || reply.stream_acknowledged_bytes > reply.stream_accepted_bytes
            || reply.stream_accepted_bytes < self.accepted
        {
            return false;
        }
        self.accepted = reply.stream_accepted_bytes;
        true
    }

    fn accept_write(&mut self, reply: logos_abi::NetworkReply, length: usize) -> bool {
        let expected = self.accepted.saturating_add(length as u64);
        if !self.observe_poll(reply) || self.accepted != expected {
            return false;
        }
        true
    }
}

#[used]
#[unsafe(link_section = ".logos")]
static HEADER: Header =
    Header::new(*b"gateway\0\0\0\0\0\0\0\0\0", ProtocolVersion::V2, logos_service_entry);

#[unsafe(no_mangle)]
extern "C" fn logos_service_entry(context: logos_service_rt::EntryControlPage) -> ! {
    logos_service_rt::entry(context, run)
}

fn run(context: &mut ServiceContext) -> ! {
    if !context.ready() {
        spin();
    }
    let Some(page) = context.shared_page() else { spin() };
    let mut id = 1;
    let mut connections = ConnectionTable::new();
    while context.acknowledged() {
        let Some(listener) = network(
            context,
            &mut id,
            NetworkOperation::Listen,
            NetworkEndpoint(0),
            0,
            logos_abi::PageHandle(0),
            0,
        ) else {
            continue;
        };
        loop {
            let Some(stream) = network(
                context,
                &mut id,
                NetworkOperation::Accept,
                listener.endpoint,
                0,
                logos_abi::PageHandle(0),
                0,
            ) else {
                break;
            };
            let Some(connection) = connections.insert(stream) else {
                let _ = network(
                    context,
                    &mut id,
                    NetworkOperation::Close,
                    stream.endpoint,
                    stream.generation,
                    logos_abi::PageHandle(0),
                    0,
                );
                continue;
            };
            serve(
                context,
                page.handle,
                page.address,
                listener.endpoint,
                stream,
                &mut id,
                &mut connections,
                connection,
            );
            let _ = connections.phase(connection, ConnectionPhase::Close);
            let _ = connections.remove(connection);
        }
    }
    spin()
}

#[allow(clippy::too_many_arguments)]
fn serve(
    context: &mut ServiceContext,
    page: logos_abi::PageHandle,
    address: u64,
    _listener: NetworkEndpoint,
    stream: logos_abi::NetworkReply,
    id: &mut u32,
    connections: &mut ConnectionTable,
    connection: usize,
) {
    let mut decoder = FrameDecoder::new();
    let mut authenticated = false;
    'connection: loop {
        let _ = connections.phase(connection, ConnectionPhase::Read);
        let Some(reply) = network(
            context,
            id,
            NetworkOperation::Read,
            stream.endpoint,
            stream.generation,
            page,
            MAX_TCP_PAYLOAD as u16,
        ) else {
            break;
        };
        if reply.length == 0 {
            break;
        }
        let input =
            unsafe { core::slice::from_raw_parts(address as *const u8, reply.length as usize) };
        for &byte in input {
            if decoder.push(core::slice::from_ref(&byte)).is_err() {
                break 'connection;
            }
            let Some(frame) = decoder.ready().ok().flatten() else { continue };
            let _ = connections.phase(connection, ConnectionPhase::Decode);
            let mut request = [0; MAX_FRAME];
            request[..frame.len()].copy_from_slice(frame);
            let operation = if authenticated {
                RemoteGateOperation::Open
            } else {
                RemoteGateOperation::Handshake
            };
            let _ = connections.phase(connection, ConnectionPhase::Remote);
            let Some(opened) = gate(context, id, page, address, operation, &request[..frame.len()])
            else {
                break 'connection;
            };
            let mut stream_events = false;
            let response_length = if authenticated {
                let plaintext =
                    unsafe { core::slice::from_raw_parts(address as *const u8, opened) };
                let Ok(message) = RemoteMessage::decode(plaintext) else {
                    break 'connection;
                };
                let operation = match message.kind {
                    RemoteMessageKind::Invoke => RemoteGateOperation::Invoke,
                    RemoteMessageKind::Subscribe => RemoteGateOperation::Subscribe,
                    RemoteMessageKind::Credit => RemoteGateOperation::Credit,
                    RemoteMessageKind::Cancel => RemoteGateOperation::Acknowledge,
                    _ => break 'connection,
                };
                stream_events = matches!(
                    message.kind,
                    RemoteMessageKind::Subscribe | RemoteMessageKind::Credit
                );
                let Some(length) = gate(context, id, page, address, operation, plaintext) else {
                    break 'connection;
                };
                if length == 0 {
                    decoder.consume().ok();
                    continue;
                }
                let plaintext =
                    unsafe { core::slice::from_raw_parts(address as *const u8, length) };
                let mut plain = [0; MAX_FRAME];
                plain[..length].copy_from_slice(plaintext);
                let Some(sealed) =
                    gate(context, id, page, address, RemoteGateOperation::Seal, &plain[..length])
                else {
                    break 'connection;
                };
                sealed
            } else {
                authenticated = true;
                let _ = connections.authenticate(connection);
                opened
            };
            let response =
                unsafe { core::slice::from_raw_parts(address as *const u8, response_length) };
            let mut framed = [0; MAX_FRAME_BUFFER];
            let Ok(framed_length) = frame_encode(&mut framed, response) else {
                break 'connection;
            };
            let _ = connections.phase(connection, ConnectionPhase::Write);
            if !write_all(context, id, page, address, stream, &framed[..framed_length]) {
                break 'connection;
            }
            if decoder.consume().is_err() {
                break 'connection;
            }
            if authenticated && stream_events {
                loop {
                    let Some(length) =
                        gate(context, id, page, address, RemoteGateOperation::Acknowledge, &[])
                    else {
                        break 'connection;
                    };
                    if length == 0 {
                        break;
                    }
                    let plaintext =
                        unsafe { core::slice::from_raw_parts(address as *const u8, length) };
                    let mut plain = [0; MAX_FRAME];
                    plain[..length].copy_from_slice(plaintext);
                    let Some(sealed) = gate(
                        context,
                        id,
                        page,
                        address,
                        RemoteGateOperation::Seal,
                        &plain[..length],
                    ) else {
                        break 'connection;
                    };
                    let response =
                        unsafe { core::slice::from_raw_parts(address as *const u8, sealed) };
                    let mut event_frame = [0; MAX_FRAME_BUFFER];
                    let Ok(event_length) = frame_encode(&mut event_frame, response) else {
                        break 'connection;
                    };
                    if !write_all(context, id, page, address, stream, &event_frame[..event_length])
                    {
                        break 'connection;
                    }
                }
            }
        }
    }
    let _ = gate(context, id, page, address, RemoteGateOperation::Reset, &[]);
    let _ = network(
        context,
        id,
        NetworkOperation::Close,
        stream.endpoint,
        0,
        logos_abi::PageHandle(0),
        0,
    );
}

fn gate(
    context: &mut ServiceContext,
    id: &mut u32,
    page: logos_abi::PageHandle,
    address: u64,
    operation: RemoteGateOperation,
    input: &[u8],
) -> Option<usize> {
    if input.len() > logos_abi::PAGE_SIZE {
        return None;
    }
    unsafe { core::ptr::copy_nonoverlapping(input.as_ptr(), address as *mut u8, input.len()) };
    let request_id = *id;
    *id = id.wrapping_add(1).max(1);
    if !context.request_remote_gate(RemotePageRequest {
        id: request_id,
        operation,
        page,
        length: input.len() as u16,
        deadline: DEADLINE,
    }) {
        return None;
    }
    let reply = context.remote_gate_reply(request_id)?;
    (reply.status == RemoteGateStatus::Complete).then_some(reply.length as usize)
}

fn write_all(
    context: &mut ServiceContext,
    id: &mut u32,
    page: logos_abi::PageHandle,
    address: u64,
    stream: logos_abi::NetworkReply,
    bytes: &[u8],
) -> bool {
    let mut state = StreamTxState::new(stream);
    let mut offset = 0;
    while offset < bytes.len() {
        let Some(poll) = network_reply(
            context,
            id,
            NetworkOperation::PollStream,
            stream.endpoint,
            stream.generation,
            logos_abi::PageHandle(0),
            0,
        ) else {
            return false;
        };
        if !state.observe_poll(poll) {
            return false;
        }
        if poll.stream_readiness & STREAM_WRITABLE == 0 {
            if !await_writable(context, id, stream, &mut state) {
                return false;
            }
            continue;
        }
        let length = core::cmp::min(MAX_TCP_PAYLOAD, bytes.len() - offset);
        unsafe {
            core::ptr::copy_nonoverlapping(
                bytes[offset..offset + length].as_ptr(),
                address as *mut u8,
                length,
            )
        };
        let Some(reply) = network_reply(
            context,
            id,
            NetworkOperation::SubmitWrite,
            stream.endpoint,
            stream.generation,
            page,
            length as u16,
        ) else {
            return false;
        };
        match reply.status {
            NetworkStatus::Complete if state.accept_write(reply, length) => offset += length,
            NetworkStatus::Busy => {
                if !await_writable(context, id, stream, &mut state) {
                    return false;
                }
            }
            _ => return false,
        }
    }
    true
}

fn await_writable(
    context: &mut ServiceContext,
    id: &mut u32,
    stream: logos_abi::NetworkReply,
    state: &mut StreamTxState,
) -> bool {
    network_reply(
        context,
        id,
        NetworkOperation::AwaitWritable,
        stream.endpoint,
        stream.generation,
        logos_abi::PageHandle(0),
        0,
    )
    .is_some_and(|reply| state.observe_poll(reply) && reply.stream_readiness & STREAM_WRITABLE != 0)
}

fn network_reply(
    context: &mut ServiceContext,
    id: &mut u32,
    operation: NetworkOperation,
    endpoint: NetworkEndpoint,
    generation: u16,
    page: logos_abi::PageHandle,
    length: u16,
) -> Option<logos_abi::NetworkReply> {
    let request_id = *id;
    *id = id.wrapping_add(1).max(1);
    let request = NetworkRequest {
        id: request_id,
        operation,
        endpoint,
        peer: if matches!(operation, NetworkOperation::Close | NetworkOperation::Cancel) {
            NetworkScope(0)
        } else {
            NetworkScope::new(NetworkProtocol::Tcp, 0, REMOTE_TCP_PORT)
        },
        page,
        length,
        generation,
        deadline: DEADLINE,
    };
    if !context.request_network(request) {
        return None;
    }
    context.network_response(request_id)
}

fn network(
    context: &mut ServiceContext,
    id: &mut u32,
    operation: NetworkOperation,
    endpoint: NetworkEndpoint,
    generation: u16,
    page: logos_abi::PageHandle,
    length: u16,
) -> Option<logos_abi::NetworkReply> {
    network_reply(context, id, operation, endpoint, generation, page, length)
        .filter(|reply| reply.status == NetworkStatus::Complete)
}

fn spin() -> ! {
    loop {
        core::hint::spin_loop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(
        status: NetworkStatus,
        endpoint: NetworkEndpoint,
        generation: u16,
        readiness: u16,
        accepted: u64,
        acknowledged: u64,
    ) -> logos_abi::NetworkReply {
        logos_abi::NetworkReply {
            id: 1,
            status,
            endpoint,
            generation,
            source_address: 0,
            source_port: 0,
            length: 0,
            stream_readiness: readiness,
            stream_reserved: 0,
            stream_accepted_bytes: accepted,
            stream_acknowledged_bytes: acknowledged,
            info: logos_abi::NetworkInfo::default(),
            counters: logos_abi::NetworkCounters::default(),
        }
    }

    #[test]
    fn stream_writer_tracks_cumulative_acceptance_without_duplicate_chunks() {
        let endpoint = NetworkEndpoint(7);
        let mut state =
            StreamTxState::new(reply(NetworkStatus::Complete, endpoint, 3, STREAM_WRITABLE, 0, 0));

        assert!(state.observe_poll(reply(
            NetworkStatus::Complete,
            endpoint,
            3,
            STREAM_WRITABLE,
            0,
            0,
        )));
        assert!(!state.accept_write(reply(NetworkStatus::Busy, endpoint, 3, 0, 0, 0), 4,));
        assert_eq!(state.accepted, 0);
        assert!(
            state.accept_write(
                reply(NetworkStatus::Complete, endpoint, 3, STREAM_WRITABLE, 4, 0),
                4,
            )
        );
        assert!(
            !state.accept_write(
                reply(NetworkStatus::Complete, endpoint, 3, STREAM_WRITABLE, 4, 4),
                4,
            )
        );
        assert!(
            state.accept_write(
                reply(NetworkStatus::Complete, endpoint, 3, STREAM_WRITABLE, 7, 4),
                3,
            )
        );
        assert_eq!(state.accepted, 7);
    }

    #[test]
    fn stream_writer_rejects_reset_stale_and_closed_progress() {
        let endpoint = NetworkEndpoint(7);
        let mut state =
            StreamTxState::new(reply(NetworkStatus::Complete, endpoint, 3, STREAM_WRITABLE, 0, 0));
        assert!(!state.observe_poll(reply(NetworkStatus::Reset, endpoint, 3, 0, 0, 0,)));
        assert!(!state.observe_poll(reply(
            NetworkStatus::Complete,
            endpoint,
            4,
            STREAM_WRITABLE,
            0,
            0,
        )));
        assert!(!state.observe_poll(reply(
            NetworkStatus::Complete,
            endpoint,
            3,
            STREAM_CLOSED,
            0,
            0,
        )));
    }

    #[test]
    fn connection_table_is_bounded_and_phase_owned() {
        let stream = reply(NetworkStatus::Complete, NetworkEndpoint(7), 3, STREAM_WRITABLE, 0, 0);
        let mut table = ConnectionTable::new();
        let first = table.insert(stream).expect("first connection");
        assert!(table.phase(first, ConnectionPhase::Decode));
        assert!(table.phase(first, ConnectionPhase::Remote));
        assert!(table.authenticate(first));
        assert!(table.remove(first).is_some());
        assert!(table.insert(stream).is_some());
        assert!(table.insert(stream).is_some());
        assert!(table.insert(stream).is_some());
        assert!(table.insert(stream).is_some());
        assert!(table.insert(stream).is_none());
    }
}
