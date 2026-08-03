#![no_main]
#![no_std]

use logos_abi::{
    MAX_TCP_PAYLOAD, NetworkEndpoint, NetworkOperation, NetworkProtocol, NetworkRequest,
    NetworkScope, NetworkStatus, REMOTE_TCP_PORT,
};
use logos_service_rt::{Context, Header, ProtocolVersion};

const DEADLINE: u64 = u64::MAX / 2;

#[used]
#[unsafe(link_section = ".logos")]
static HEADER: Header =
    Header::new(*b"gateway\0\0\0\0\0\0\0\0\0", ProtocolVersion::V1, logos_service_entry);

#[unsafe(no_mangle)]
extern "C" fn logos_service_entry(context: logos_service_rt::EntryContext) -> ! {
    logos_service_rt::entry(context, run)
}

#[derive(Clone, Copy)]
enum State {
    Listen,
    WaitListen,
    Accept(NetworkEndpoint, u16),
    WaitAccept(NetworkEndpoint, u16),
    Read(NetworkEndpoint, u16),
    WaitRead(NetworkEndpoint, u16),
}

fn run(context: &mut Context) -> ! {
    if !context.ready() {
        spin();
    }
    let mut state = State::Listen;
    let mut id = 1u32;
    while context.acknowledged() {
        state = match state {
            State::Listen => {
                let request = NetworkRequest {
                    id,
                    operation: NetworkOperation::Listen,
                    endpoint: NetworkEndpoint(0),
                    peer: NetworkScope::new(NetworkProtocol::Tcp, 0, REMOTE_TCP_PORT),
                    page: logos_abi::PageHandle(0),
                    length: 0,
                    generation: 0,
                    deadline: DEADLINE,
                };
                id = id.wrapping_add(1).max(1);
                if context.request_network(request) { State::WaitListen } else { spin() }
            }
            State::WaitListen => {
                let expected = id.wrapping_sub(1).max(1);
                match context.network_response(expected) {
                    Some(reply)
                        if reply.status == NetworkStatus::Complete && reply.endpoint.valid() =>
                    {
                        State::Accept(reply.endpoint, reply.generation)
                    }
                    Some(_) => State::Listen,
                    None => {
                        context.network_wait(DEADLINE);
                        State::WaitListen
                    }
                }
            }
            State::Accept(listener, generation) => {
                let request = NetworkRequest {
                    id,
                    operation: NetworkOperation::Accept,
                    endpoint: listener,
                    peer: NetworkScope::new(NetworkProtocol::Tcp, 0, REMOTE_TCP_PORT),
                    page: logos_abi::PageHandle(0),
                    length: 0,
                    generation: 0,
                    deadline: DEADLINE,
                };
                id = id.wrapping_add(1).max(1);
                if context.request_network(request) {
                    State::WaitAccept(listener, generation)
                } else {
                    State::Accept(listener, generation)
                }
            }
            State::WaitAccept(listener, generation) => {
                let expected = id.wrapping_sub(1).max(1);
                match context.network_response(expected) {
                    Some(reply)
                        if reply.status == NetworkStatus::Complete && reply.endpoint.valid() =>
                    {
                        State::Read(reply.endpoint, reply.generation)
                    }
                    Some(_) => State::Accept(listener, generation),
                    None => {
                        context.network_wait(DEADLINE);
                        State::WaitAccept(listener, generation)
                    }
                }
            }
            State::Read(stream, generation) => {
                let Some(pages) = context.network_pages() else { spin() };
                let request = NetworkRequest {
                    id,
                    operation: NetworkOperation::Read,
                    endpoint: stream,
                    peer: NetworkScope::new(NetworkProtocol::Tcp, 0, 0),
                    page: pages.tx_handle,
                    length: MAX_TCP_PAYLOAD as u16,
                    generation,
                    deadline: DEADLINE,
                };
                id = id.wrapping_add(1).max(1);
                if context.request_network(request) {
                    State::WaitRead(stream, generation)
                } else {
                    State::Read(stream, generation)
                }
            }
            State::WaitRead(stream, generation) => {
                let expected = id.wrapping_sub(1).max(1);
                match context.network_response(expected) {
                    Some(reply) if reply.status == NetworkStatus::Complete => {
                        // Core owns the typed remote gate; Gateway only owns transport framing.
                        let _ = context.network_wait(DEADLINE);
                        State::Accept(stream, generation)
                    }
                    Some(_) => State::Accept(stream, generation),
                    None => {
                        context.network_wait(DEADLINE);
                        State::WaitRead(stream, generation)
                    }
                }
            }
        };
    }
    spin()
}

fn spin() -> ! {
    loop {
        core::hint::spin_loop();
    }
}
