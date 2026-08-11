use crate::arch::interrupts;
use crate::platform::{network, session};
use crate::sched::native_task;
use crate::test_hooks;

use logos_core::capabilities;

#[allow(clippy::too_many_arguments)]
pub(super) fn run_network_request(
    request: logos_abi::NetworkRequest,
    terminal: native_task::NetworkClientEndpoint,
    runtime: &mut network::NetworkRuntime,
    scheduler: &mut native_task::Scheduler<'_>,
    session: &session::Context,
    capabilities: &capabilities::CapabilityManager,
    shared_pages: &logos_core::shared_pages::SharedPages,
    terminal_owner: u64,
) -> Option<logos_abi::NetworkReply> {
    if !terminal.issue(request) {
        return None;
    }
    runtime.task()?;
    for _ in 0..1_000_000 {
        let tick = interrupts::ticks();
        if !runtime.poll(tick) {
            return None;
        }
        if !super::drain_network_wakes(runtime, scheduler) {
            return None;
        }
        if !runtime.relay_probe(terminal, session, capabilities, shared_pages, terminal_owner, tick)
        {
            return None;
        }
        if !super::drain_network_wakes(runtime, scheduler) || !runtime.poll(tick) {
            return None;
        }
        if !runtime.relay_probe(terminal, session, capabilities, shared_pages, terminal_owner, tick)
            || !super::drain_network_wakes(runtime, scheduler)
        {
            return None;
        }
        if let Some(reply) = terminal.response(request.id) {
            return Some(reply);
        }
    }
    None
}

#[allow(clippy::too_many_arguments)]
pub(super) fn run_network_tcp_stream(
    client: native_task::NetworkClientEndpoint,
    runtime: &mut network::NetworkRuntime,
    scheduler: &mut native_task::Scheduler<'_>,
    session: &session::Context,
    capabilities: &capabilities::CapabilityManager,
    shared_pages: &logos_core::shared_pages::SharedPages,
    owner: u64,
    page: logos_abi::PageHandle,
) -> bool {
    let scope = logos_abi::NetworkScope::new(logos_abi::NetworkProtocol::Tcp, 0, TCP_STREAM_PORT);
    test_hooks::event(TCP_STREAM_ID, "starting");
    if !network_ready(runtime) {
        return false;
    }
    let mut proof =
        TcpProof { client, runtime, scheduler, session, capabilities, shared_pages, owner, page };
    let Some(connections) = establish_connections(&mut proof) else {
        return false;
    };
    let Some(address) = proof.page_address() else {
        return false;
    };
    if !read_hello(&mut proof, &connections, address)
        || !write_primary_stream(&mut proof, &connections, address)
        || !verify_secondary_connection(&mut proof, &connections, address)
        || !verify_bulk_stream(&mut proof, &connections, address)
    {
        return false;
    }
    test_hooks::event(TCP_STREAM_ID, "connection_closed");
    scope.valid()
}

const TCP_STREAM_ID: &str = "network/tcp-stream";
const TCP_STREAM_DEADLINE: u64 = u64::MAX / 2;
const TCP_STREAM_PORT: u16 = logos_abi::REMOTE_TCP_PORT;

struct TcpProof<'a, 'task> {
    client: native_task::NetworkClientEndpoint,
    runtime: &'a mut network::NetworkRuntime,
    scheduler: &'a mut native_task::Scheduler<'task>,
    session: &'a session::Context,
    capabilities: &'a capabilities::CapabilityManager,
    shared_pages: &'a logos_core::shared_pages::SharedPages,
    owner: u64,
    page: logos_abi::PageHandle,
}

struct TcpConnections {
    primary: logos_abi::NetworkEndpoint,
    secondary: logos_abi::NetworkEndpoint,
    generation: u16,
}

impl<'a, 'task> TcpProof<'a, 'task> {
    fn request(
        &self,
        id: u32,
        operation: logos_abi::NetworkOperation,
        endpoint: logos_abi::NetworkEndpoint,
        page: logos_abi::PageHandle,
        length: u16,
        generation: u16,
    ) -> logos_abi::NetworkRequest {
        logos_abi::NetworkRequest {
            id,
            operation,
            endpoint,
            peer: logos_abi::NetworkScope::new(logos_abi::NetworkProtocol::Tcp, 0, TCP_STREAM_PORT),
            page,
            length,
            generation,
            deadline: TCP_STREAM_DEADLINE,
        }
    }

    fn issue(&mut self, request: logos_abi::NetworkRequest) -> Option<logos_abi::NetworkReply> {
        run_network_request(
            request,
            self.client,
            self.runtime,
            self.scheduler,
            self.session,
            self.capabilities,
            self.shared_pages,
            self.owner,
        )
    }

    fn page_address(&self) -> Option<u64> {
        self.shared_pages.address(self.owner, self.page)
    }

    fn write_page(&self, address: u64, bytes: &[u8]) {
        unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), address as *mut u8, bytes.len()) };
    }

    fn xor_page(&self, address: u64, length: usize) {
        unsafe {
            for byte in core::slice::from_raw_parts_mut(address as *mut u8, length) {
                *byte ^= 0xa5;
            }
        }
    }
}

fn network_ready(runtime: &network::NetworkRuntime) -> bool {
    if !runtime.has_device() {
        test_hooks::event(TCP_STREAM_ID, "network_device_unavailable");
        return false;
    }
    if runtime.resources().is_none() {
        test_hooks::event(TCP_STREAM_ID, "network_resources_unavailable");
        return false;
    }
    if runtime.task().is_none() {
        test_hooks::event(TCP_STREAM_ID, "network_unavailable");
        return false;
    }
    true
}

fn establish_connections(proof: &mut TcpProof<'_, '_>) -> Option<TcpConnections> {
    let listen = proof.request(
        0x9000_0300,
        logos_abi::NetworkOperation::Listen,
        logos_abi::NetworkEndpoint(0),
        logos_abi::PageHandle(0),
        0,
        0,
    );
    let Some(listen_reply) = proof.issue(listen) else {
        test_hooks::event(TCP_STREAM_ID, "listener_failed");
        return None;
    };
    if listen_reply.status != logos_abi::NetworkStatus::Complete {
        test_hooks::event(TCP_STREAM_ID, network_status_label(listen_reply.status));
        return None;
    }
    if !listen_reply.endpoint.valid() || listen_reply.generation == 0 {
        test_hooks::event(TCP_STREAM_ID, "listener_shape_invalid");
        return None;
    }
    test_hooks::event(TCP_STREAM_ID, "listener_waiting");

    let accept = proof.request(
        0x9000_0301,
        logos_abi::NetworkOperation::Accept,
        listen_reply.endpoint,
        logos_abi::PageHandle(0),
        0,
        0,
    );
    let Some(accept_reply) = proof.issue(accept) else {
        test_hooks::event(TCP_STREAM_ID, "accept_failed");
        return None;
    };
    if accept_reply.status != logos_abi::NetworkStatus::Complete
        || !accept_reply.endpoint.valid()
        || accept_reply.generation != listen_reply.generation
        || accept_reply.source_address == 0
        || accept_reply.source_port == 0
    {
        test_hooks::event(TCP_STREAM_ID, network_status_label(accept_reply.status));
        return None;
    }
    test_hooks::event(TCP_STREAM_ID, "connection_established");

    let accept_second = proof.request(
        0x9000_0306,
        logos_abi::NetworkOperation::Accept,
        listen_reply.endpoint,
        logos_abi::PageHandle(0),
        0,
        0,
    );
    let Some(second_reply) = proof.issue(accept_second) else {
        test_hooks::event(TCP_STREAM_ID, "second_accept_failed");
        return None;
    };
    if second_reply.status != logos_abi::NetworkStatus::Complete
        || !second_reply.endpoint.valid()
        || second_reply.endpoint == accept_reply.endpoint
        || second_reply.generation != listen_reply.generation
    {
        test_hooks::event(TCP_STREAM_ID, network_status_label(second_reply.status));
        return None;
    }
    test_hooks::event(TCP_STREAM_ID, "second_connection_established");
    Some(TcpConnections {
        primary: accept_reply.endpoint,
        secondary: second_reply.endpoint,
        generation: listen_reply.generation,
    })
}

fn read_hello(proof: &mut TcpProof<'_, '_>, connections: &TcpConnections, address: u64) -> bool {
    let read = proof.request(
        0x9000_0302,
        logos_abi::NetworkOperation::Read,
        connections.primary,
        proof.page,
        logos_abi::MAX_TCP_PAYLOAD as u16,
        connections.generation,
    );
    let Some(read_reply) = proof.issue(read) else {
        return false;
    };
    let hello =
        unsafe { core::slice::from_raw_parts(address as *const u8, read_reply.length as usize) };
    if read_reply.status != logos_abi::NetworkStatus::Complete || hello != b"hello" {
        return false;
    }
    test_hooks::event(TCP_STREAM_ID, "connection_readable");
    true
}

fn write_primary_stream(
    proof: &mut TcpProof<'_, '_>,
    connections: &TcpConnections,
    address: u64,
) -> bool {
    proof.write_page(address, b"wo");
    test_hooks::event(TCP_STREAM_ID, "write_pending");
    let write = proof.request(
        0x9000_0303,
        logos_abi::NetworkOperation::SubmitWrite,
        connections.primary,
        proof.page,
        2,
        connections.generation,
    );
    let Some(write_reply) = proof.issue(write) else {
        return false;
    };
    if write_reply.status != logos_abi::NetworkStatus::Complete
        || write_reply.endpoint != connections.primary
        || write_reply.stream_accepted_bytes != 2
        || write_reply.stream_acknowledged_bytes > write_reply.stream_accepted_bytes
    {
        return false;
    }
    test_hooks::event(TCP_STREAM_ID, "write_accepted");

    proof.write_page(address, b"rld");
    let write_tail = proof.request(
        0x9000_0307,
        logos_abi::NetworkOperation::SubmitWrite,
        connections.primary,
        proof.page,
        3,
        connections.generation,
    );
    let Some(write_tail_reply) = proof.issue(write_tail) else {
        return false;
    };
    if write_tail_reply.status != logos_abi::NetworkStatus::Complete
        || write_tail_reply.stream_accepted_bytes != 5
        || write_tail_reply.stream_acknowledged_bytes < write_reply.stream_acknowledged_bytes
        || write_tail_reply.stream_acknowledged_bytes > write_tail_reply.stream_accepted_bytes
    {
        return false;
    }
    test_hooks::event(TCP_STREAM_ID, "write_tail_accepted");

    let poll = proof.request(
        0x9000_0308,
        logos_abi::NetworkOperation::PollStream,
        connections.primary,
        logos_abi::PageHandle(0),
        0,
        connections.generation,
    );
    let Some(poll_reply) = proof.issue(poll) else {
        return false;
    };
    if poll_reply.status != logos_abi::NetworkStatus::Complete
        || poll_reply.stream_accepted_bytes != 5
        || poll_reply.stream_acknowledged_bytes < write_tail_reply.stream_acknowledged_bytes
        || poll_reply.stream_acknowledged_bytes > poll_reply.stream_accepted_bytes
    {
        return false;
    }
    test_hooks::event(TCP_STREAM_ID, "stream_polled");
    true
}

fn verify_secondary_connection(
    proof: &mut TcpProof<'_, '_>,
    connections: &TcpConnections,
    address: u64,
) -> bool {
    proof.write_page(address, b"reply");
    let second_write = proof.request(
        0x9000_0309,
        logos_abi::NetworkOperation::SubmitWrite,
        connections.secondary,
        proof.page,
        5,
        connections.generation,
    );
    let Some(second_write_reply) = proof.issue(second_write) else {
        return false;
    };
    if second_write_reply.status != logos_abi::NetworkStatus::Complete {
        return false;
    }
    let second_read = proof.request(
        0x9000_0310,
        logos_abi::NetworkOperation::Read,
        connections.secondary,
        proof.page,
        logos_abi::MAX_TCP_PAYLOAD as u16,
        connections.generation,
    );
    let Some(second_read_reply) = proof.issue(second_read) else {
        return false;
    };
    second_read_reply.status == logos_abi::NetworkStatus::Complete && second_read_reply.length == 5
}

fn verify_bulk_stream(
    proof: &mut TcpProof<'_, '_>,
    connections: &TcpConnections,
    address: u64,
) -> bool {
    let mut expected = [0; logos_abi::MAX_TCP_PAYLOAD];
    for (index, byte) in expected.iter_mut().take(512).enumerate() {
        *byte = (index as u8).wrapping_mul(37).wrapping_add(11);
    }
    let large_read = proof.request(
        0x9000_0304,
        logos_abi::NetworkOperation::Read,
        connections.primary,
        proof.page,
        logos_abi::MAX_TCP_PAYLOAD as u16,
        connections.generation,
    );
    let Some(large_read_reply) = proof.issue(large_read) else {
        return false;
    };
    let received = unsafe {
        core::slice::from_raw_parts(address as *const u8, large_read_reply.length as usize)
    };
    if large_read_reply.status != logos_abi::NetworkStatus::Complete || received != &expected[..512]
    {
        return false;
    }
    test_hooks::event(TCP_STREAM_ID, "write_acknowledged");

    proof.xor_page(address, 512);
    let write_large = proof.request(
        0x9000_0305,
        logos_abi::NetworkOperation::Write,
        connections.primary,
        proof.page,
        512,
        connections.generation,
    );
    let Some(write_large_reply) = proof.issue(write_large) else {
        test_hooks::event(TCP_STREAM_ID, "large_write_failed");
        return false;
    };
    if write_large_reply.status != logos_abi::NetworkStatus::Complete {
        test_hooks::event(TCP_STREAM_ID, network_status_label(write_large_reply.status));
        return false;
    }
    true
}

pub(super) const fn network_status_label(status: logos_abi::NetworkStatus) -> &'static str {
    match status {
        logos_abi::NetworkStatus::Complete => "complete",
        logos_abi::NetworkStatus::Denied => "denied",
        logos_abi::NetworkStatus::Invalid => "invalid",
        logos_abi::NetworkStatus::Busy => "busy",
        logos_abi::NetworkStatus::Full => "full",
        logos_abi::NetworkStatus::Offline => "offline",
        logos_abi::NetworkStatus::NoRoute => "no_route",
        logos_abi::NetworkStatus::AddressInUse => "address_in_use",
        logos_abi::NetworkStatus::MessageTooLarge => "message_too_large",
        logos_abi::NetworkStatus::TimedOut => "timed_out",
        logos_abi::NetworkStatus::Cancelled => "cancelled",
        logos_abi::NetworkStatus::Reset => "reset",
        logos_abi::NetworkStatus::Io => "io",
    }
}
