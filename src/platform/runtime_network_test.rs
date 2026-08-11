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
    const ID: &str = "network/tcp-stream";
    const DEADLINE: u64 = u64::MAX / 2;
    const PORT: u16 = logos_abi::REMOTE_TCP_PORT;
    let scope = logos_abi::NetworkScope::new(logos_abi::NetworkProtocol::Tcp, 0, PORT);
    let request = |id, operation, endpoint, page, length, generation| logos_abi::NetworkRequest {
        id,
        operation,
        endpoint,
        peer: logos_abi::NetworkScope::new(logos_abi::NetworkProtocol::Tcp, 0, PORT),
        page,
        length,
        generation,
        deadline: DEADLINE,
    };
    test_hooks::event(ID, "starting");
    if !runtime.has_device() {
        test_hooks::event(ID, "network_device_unavailable");
        return false;
    }
    if runtime.resources().is_none() {
        test_hooks::event(ID, "network_resources_unavailable");
        return false;
    }
    if runtime.task().is_none() {
        test_hooks::event(ID, "network_unavailable");
        return false;
    }

    let listen = request(
        0x9000_0300,
        logos_abi::NetworkOperation::Listen,
        logos_abi::NetworkEndpoint(0),
        logos_abi::PageHandle(0),
        0,
        0,
    );
    let Some(listen_reply) = run_network_request(
        listen,
        client,
        runtime,
        scheduler,
        session,
        capabilities,
        shared_pages,
        owner,
    ) else {
        test_hooks::event(ID, "listener_failed");
        return false;
    };
    if listen_reply.status != logos_abi::NetworkStatus::Complete {
        test_hooks::event(ID, network_status_label(listen_reply.status));
        return false;
    }
    if !listen_reply.endpoint.valid() || listen_reply.generation == 0 {
        test_hooks::event(ID, "listener_shape_invalid");
        return false;
    }
    test_hooks::event(ID, "listener_waiting");

    let accept = request(
        0x9000_0301,
        logos_abi::NetworkOperation::Accept,
        listen_reply.endpoint,
        logos_abi::PageHandle(0),
        0,
        0,
    );
    let Some(accept_reply) = run_network_request(
        accept,
        client,
        runtime,
        scheduler,
        session,
        capabilities,
        shared_pages,
        owner,
    ) else {
        test_hooks::event(ID, "accept_failed");
        return false;
    };
    if accept_reply.status != logos_abi::NetworkStatus::Complete
        || !accept_reply.endpoint.valid()
        || accept_reply.generation != listen_reply.generation
        || accept_reply.source_address == 0
        || accept_reply.source_port == 0
    {
        test_hooks::event(ID, network_status_label(accept_reply.status));
        return false;
    }
    test_hooks::event(ID, "connection_established");

    let accept_second = request(
        0x9000_0306,
        logos_abi::NetworkOperation::Accept,
        listen_reply.endpoint,
        logos_abi::PageHandle(0),
        0,
        0,
    );
    let Some(second_reply) = run_network_request(
        accept_second,
        client,
        runtime,
        scheduler,
        session,
        capabilities,
        shared_pages,
        owner,
    ) else {
        test_hooks::event(ID, "second_accept_failed");
        return false;
    };
    if second_reply.status != logos_abi::NetworkStatus::Complete
        || !second_reply.endpoint.valid()
        || second_reply.endpoint == accept_reply.endpoint
        || second_reply.generation != listen_reply.generation
    {
        test_hooks::event(ID, network_status_label(second_reply.status));
        return false;
    }
    test_hooks::event(ID, "second_connection_established");

    let address = match shared_pages.address(owner, page) {
        Some(address) => address,
        None => return false,
    };
    let read = request(
        0x9000_0302,
        logos_abi::NetworkOperation::Read,
        accept_reply.endpoint,
        page,
        logos_abi::MAX_TCP_PAYLOAD as u16,
        accept_reply.generation,
    );
    let Some(read_reply) = run_network_request(
        read,
        client,
        runtime,
        scheduler,
        session,
        capabilities,
        shared_pages,
        owner,
    ) else {
        return false;
    };
    let hello =
        unsafe { core::slice::from_raw_parts(address as *const u8, read_reply.length as usize) };
    if read_reply.status != logos_abi::NetworkStatus::Complete || hello != b"hello" {
        return false;
    }
    test_hooks::event(ID, "connection_readable");

    unsafe {
        core::ptr::copy_nonoverlapping(b"wo".as_ptr(), address as *mut u8, 2);
    }
    test_hooks::event(ID, "write_pending");
    let write = request(
        0x9000_0303,
        logos_abi::NetworkOperation::SubmitWrite,
        accept_reply.endpoint,
        page,
        2,
        accept_reply.generation,
    );
    let Some(write_reply) = run_network_request(
        write,
        client,
        runtime,
        scheduler,
        session,
        capabilities,
        shared_pages,
        owner,
    ) else {
        return false;
    };
    if write_reply.status != logos_abi::NetworkStatus::Complete
        || write_reply.endpoint != accept_reply.endpoint
        || write_reply.stream_accepted_bytes != 2
        || write_reply.stream_acknowledged_bytes > write_reply.stream_accepted_bytes
    {
        return false;
    }
    test_hooks::event(ID, "write_accepted");
    unsafe {
        core::ptr::copy_nonoverlapping(b"rld".as_ptr(), address as *mut u8, 3);
    }
    let write_tail = request(
        0x9000_0307,
        logos_abi::NetworkOperation::SubmitWrite,
        accept_reply.endpoint,
        page,
        3,
        accept_reply.generation,
    );
    let Some(write_tail_reply) = run_network_request(
        write_tail,
        client,
        runtime,
        scheduler,
        session,
        capabilities,
        shared_pages,
        owner,
    ) else {
        return false;
    };
    if write_tail_reply.status != logos_abi::NetworkStatus::Complete
        || write_tail_reply.stream_accepted_bytes != 5
        || write_tail_reply.stream_acknowledged_bytes < write_reply.stream_acknowledged_bytes
        || write_tail_reply.stream_acknowledged_bytes > write_tail_reply.stream_accepted_bytes
    {
        return false;
    }
    test_hooks::event(ID, "write_tail_accepted");
    let poll = request(
        0x9000_0308,
        logos_abi::NetworkOperation::PollStream,
        accept_reply.endpoint,
        logos_abi::PageHandle(0),
        0,
        accept_reply.generation,
    );
    let Some(poll_reply) = run_network_request(
        poll,
        client,
        runtime,
        scheduler,
        session,
        capabilities,
        shared_pages,
        owner,
    ) else {
        return false;
    };
    if poll_reply.status != logos_abi::NetworkStatus::Complete
        || poll_reply.stream_accepted_bytes != 5
        || poll_reply.stream_acknowledged_bytes < write_tail_reply.stream_acknowledged_bytes
        || poll_reply.stream_acknowledged_bytes > poll_reply.stream_accepted_bytes
    {
        return false;
    }
    test_hooks::event(ID, "stream_polled");
    unsafe {
        core::ptr::copy_nonoverlapping(b"reply".as_ptr(), address as *mut u8, 5);
    }
    let second_write = request(
        0x9000_0309,
        logos_abi::NetworkOperation::SubmitWrite,
        second_reply.endpoint,
        page,
        5,
        second_reply.generation,
    );
    let Some(second_write_reply) = run_network_request(
        second_write,
        client,
        runtime,
        scheduler,
        session,
        capabilities,
        shared_pages,
        owner,
    ) else {
        return false;
    };
    if second_write_reply.status != logos_abi::NetworkStatus::Complete {
        return false;
    }
    let second_read = request(
        0x9000_0310,
        logos_abi::NetworkOperation::Read,
        second_reply.endpoint,
        page,
        logos_abi::MAX_TCP_PAYLOAD as u16,
        second_reply.generation,
    );
    let Some(second_read_reply) = run_network_request(
        second_read,
        client,
        runtime,
        scheduler,
        session,
        capabilities,
        shared_pages,
        owner,
    ) else {
        return false;
    };
    if second_read_reply.status != logos_abi::NetworkStatus::Complete
        || second_read_reply.length != 5
    {
        return false;
    }

    let mut expected = [0; logos_abi::MAX_TCP_PAYLOAD];
    for (index, byte) in expected.iter_mut().take(512).enumerate() {
        *byte = (index as u8).wrapping_mul(37).wrapping_add(11);
    }
    let large_read = request(
        0x9000_0304,
        logos_abi::NetworkOperation::Read,
        accept_reply.endpoint,
        page,
        logos_abi::MAX_TCP_PAYLOAD as u16,
        accept_reply.generation,
    );
    let Some(large_read_reply) = run_network_request(
        large_read,
        client,
        runtime,
        scheduler,
        session,
        capabilities,
        shared_pages,
        owner,
    ) else {
        return false;
    };
    let received = unsafe {
        core::slice::from_raw_parts(address as *const u8, large_read_reply.length as usize)
    };
    if large_read_reply.status != logos_abi::NetworkStatus::Complete || received != &expected[..512]
    {
        return false;
    }
    test_hooks::event(ID, "write_acknowledged");

    unsafe {
        for byte in core::slice::from_raw_parts_mut(address as *mut u8, 512) {
            *byte ^= 0xa5;
        }
    }
    let write_large = request(
        0x9000_0305,
        logos_abi::NetworkOperation::Write,
        accept_reply.endpoint,
        page,
        512,
        accept_reply.generation,
    );
    let Some(write_large_reply) = run_network_request(
        write_large,
        client,
        runtime,
        scheduler,
        session,
        capabilities,
        shared_pages,
        owner,
    ) else {
        test_hooks::event(ID, "large_write_failed");
        return false;
    };
    if write_large_reply.status != logos_abi::NetworkStatus::Complete {
        test_hooks::event(ID, network_status_label(write_large_reply.status));
        return false;
    }

    test_hooks::event(ID, "connection_closed");
    scope.valid()
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
