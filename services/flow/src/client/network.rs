use core::{mem, ptr};

use logos_abi::{
    FlowControl, IpcBytes, IpcStatus, MessageKind, NetworkOperation, NetworkRequest,
    NetworkResponse, NetworkResult, NetworkState,
};

use super::fetch::FetchClient;
use crate::{IdSpace, NetworkCommand, PendingOutput, Port, Transport};

/// Synchronous request/response client for the Network service. Requests are
/// matched by id; a Flow cancel control sends a `Cancel` request mid-wait.
pub struct NetworkClient {
    cancelled: bool,
}

impl Default for NetworkClient {
    fn default() -> Self {
        Self::new()
    }
}

impl NetworkClient {
    pub const fn new() -> Self {
        Self { cancelled: false }
    }

    pub fn take_cancelled(&mut self) -> bool {
        let cancelled = self.cancelled;
        self.cancelled = false;
        cancelled
    }

    pub fn request<T: Transport>(
        &mut self,
        transport: &mut T,
        operation: NetworkOperation,
        address: [u8; 4],
        port: u16,
    ) -> Result<NetworkResponse, IpcStatus> {
        let mut request =
            NetworkRequest::new(operation, transport.next_request_id(IdSpace::Network));
        request.address = address;
        request.port = port;
        if operation == NetworkOperation::IcmpPing {
            request.timeout_ticks = logos_abi::NETWORK_PING_TIMEOUT_TICKS;
        } else if operation == NetworkOperation::TcpConnect {
            request.timeout_ticks = logos_abi::NETWORK_TCP_CONNECT_TIMEOUT_TICKS;
        }
        self.request_message(transport, request)
    }

    pub fn request_message<T: Transport>(
        &mut self,
        transport: &mut T,
        request: NetworkRequest,
    ) -> Result<NetworkResponse, IpcStatus> {
        self.cancelled = false;
        let request_bytes = unsafe {
            core::slice::from_raw_parts(
                (&request as *const NetworkRequest).cast::<u8>(),
                mem::size_of::<NetworkRequest>(),
            )
        };
        let message = IpcBytes::from_bytes(MessageKind::NetworkRequest, request_bytes)
            .ok_or(IpcStatus::Malformed)?;
        match transport.send(Port::Network, &message) {
            IpcStatus::Ok => {}
            status => return Err(status),
        }
        let mut cancel_requested = false;
        let mut cancel_sent = false;
        for _ in 0..256 {
            if !cancel_requested {
                let mut control = IpcBytes::empty(MessageKind::FlowControl);
                if transport.receive(Port::Input, &mut control) == IpcStatus::Ok
                    && control.len as usize == mem::size_of::<FlowControl>()
                {
                    if !FlowControl::wire_enums_valid(
                        &control.bytes[..mem::size_of::<FlowControl>()],
                    ) {
                        continue;
                    }
                    let value: FlowControl =
                        unsafe { ptr::read_unaligned(control.bytes.as_ptr().cast()) };
                    cancel_requested = value.is_valid()
                        && (value.request_id == 0 || value.request_id == request.request_id);
                }
            }
            if cancel_requested && !cancel_sent {
                let cancel = NetworkRequest::new(NetworkOperation::Cancel, request.request_id);
                let cancel_bytes = unsafe {
                    core::slice::from_raw_parts(
                        (&cancel as *const NetworkRequest).cast::<u8>(),
                        mem::size_of::<NetworkRequest>(),
                    )
                };
                let cancel_message =
                    IpcBytes::from_bytes(MessageKind::NetworkRequest, cancel_bytes)
                        .ok_or(IpcStatus::Malformed)?;
                match transport.send(Port::Network, &cancel_message) {
                    IpcStatus::Ok => cancel_sent = true,
                    IpcStatus::Full => {
                        transport.wait();
                        continue;
                    }
                    status => return Err(status),
                }
            }
            let mut response = IpcBytes::empty(MessageKind::NetworkResponse);
            match transport.receive(Port::Network, &mut response) {
                IpcStatus::Ok => {
                    if response.kind != MessageKind::NetworkResponse
                        || response.len as usize != mem::size_of::<NetworkResponse>()
                    {
                        return Err(IpcStatus::Malformed);
                    }
                    if !NetworkResponse::wire_enums_valid(
                        &response.bytes[..mem::size_of::<NetworkResponse>()],
                    ) {
                        return Err(IpcStatus::Malformed);
                    }
                    let value: NetworkResponse =
                        unsafe { ptr::read_unaligned(response.bytes.as_ptr().cast()) };
                    if cancel_sent {
                        if value.operation == NetworkOperation::Cancel
                            && value.request_id == request.request_id
                        {
                            self.cancelled = true;
                            return Err(IpcStatus::Empty);
                        }
                        continue;
                    }
                    return value.is_valid_for(request).then_some(value).ok_or(IpcStatus::Stale);
                }
                IpcStatus::Empty => transport.wait(),
                status => return Err(status),
            }
        }
        if cancel_sent {
            self.cancelled = true;
        }
        Err(IpcStatus::Empty)
    }

    pub fn close_tcp_response<T: Transport>(
        &mut self,
        transport: &mut T,
        response: NetworkResponse,
    ) {
        if response.generation == 0 || response.service_epoch == 0 {
            return;
        }
        let mut close = NetworkRequest::new(
            NetworkOperation::Close,
            transport.next_request_id(IdSpace::Network),
        );
        close.handle = response.handle;
        close.generation = response.generation;
        close.service_epoch = response.service_epoch;
        let _ = self.request_message(transport, close);
    }
}

pub fn network_result_text(result: NetworkResult) -> &'static [u8] {
    match result {
        NetworkResult::Full => b"network queue full\r\n",
        NetworkResult::WouldBlock => b"network configuring\r\n",
        NetworkResult::Disabled => b"network disabled\r\n",
        NetworkResult::Unavailable => b"network unavailable\r\n",
        NetworkResult::Timeout => b"network timeout\r\n",
        NetworkResult::Stale => b"network restarting\r\n",
        NetworkResult::Refused => b"network refused\r\n",
        NetworkResult::Checksum => b"network checksum failure\r\n",
        NetworkResult::NotFound => b"network socket not found\r\n",
        NetworkResult::Invalid | NetworkResult::Unsupported => b"network request invalid\r\n",
        NetworkResult::Cancelled => b"network cancelled\r\n",
        NetworkResult::Ok => b"ok\r\n",
    }
}

pub fn network_state_text(state: NetworkState) -> &'static [u8] {
    match state {
        NetworkState::Disabled => b"network disabled\r\n",
        NetworkState::Unavailable => b"network unavailable\r\n",
        NetworkState::Configuring => b"network configuring\r\n",
        NetworkState::Ready => b"network ready\r\n",
        NetworkState::Restarting => b"network restarting\r\n",
        NetworkState::Faulted => b"network unavailable\r\n",
    }
}

/// Run one `net.*` command to completion (or start the Fetch client) and stage
/// its user-visible result.
pub fn network_command<T: Transport>(
    command: NetworkCommand<'_>,
    client: &mut NetworkClient,
    transport: &mut T,
    fetch: &mut FetchClient,
    pending: &mut PendingOutput,
) {
    if let NetworkCommand::Fetch { url, destination } = command {
        if !fetch.start(transport, url, destination) {
            pending.stage(if fetch.active() {
                b"fetch already active\r\n"
            } else {
                b"fetch request too large\r\n"
            });
        }
        return;
    }
    if let NetworkCommand::InterfaceStatus { name } = command {
        if name != b"eth0" {
            pending.stage(b"network interface not found\r\n");
            return;
        }
    }
    let (operation, address, port, success): (NetworkOperation, [u8; 4], u16, &[u8]) = match command
    {
        NetworkCommand::Status => (NetworkOperation::Status, [0; 4], 0, b""),
        NetworkCommand::InterfaceStatus { .. } => (NetworkOperation::Status, [0; 4], 0, b""),
        NetworkCommand::Ping { address } => {
            (NetworkOperation::IcmpPing, address, 0, b"ping ok\r\n")
        }
        NetworkCommand::TcpProbe { address, port } => {
            (NetworkOperation::TcpConnect, address, port, b"tcp probe ok\r\n")
        }
        NetworkCommand::Fetch { .. } => unreachable!(),
    };
    match client.request(transport, operation, address, port) {
        Ok(response) if operation == NetworkOperation::Status => {
            pending.stage(network_state_text(response.state))
        }
        Ok(response) if operation == NetworkOperation::TcpConnect => {
            client.close_tcp_response(transport, response);
            if response.result == NetworkResult::Ok {
                pending.stage(success);
            } else {
                pending.stage(network_result_text(response.result));
            }
        }
        Ok(response) if response.result == NetworkResult::Ok => pending.stage(success),
        Ok(response) => pending.stage(network_result_text(response.result)),
        Err(IpcStatus::Stale | IpcStatus::Disconnected) => pending.stage(b"network restarting\r\n"),
        Err(IpcStatus::Empty) if client.take_cancelled() => pending.stage(b"network cancelled\r\n"),
        Err(IpcStatus::Unauthorized | IpcStatus::Empty) => {
            pending.stage(b"network unavailable\r\n")
        }
        Err(IpcStatus::Full) => pending.stage(b"network queue full\r\n"),
        Err(IpcStatus::Malformed | IpcStatus::Ok) => pending.stage(b"network request invalid\r\n"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::fake::FakeTransport;

    fn wrap<T: Copy>(kind: MessageKind, value: &T) -> IpcBytes {
        let bytes = unsafe {
            core::slice::from_raw_parts((value as *const T).cast::<u8>(), mem::size_of::<T>())
        };
        IpcBytes::from_bytes(kind, bytes).unwrap()
    }

    fn sent(transport: &FakeTransport, index: usize) -> NetworkRequest {
        let message: IpcBytes = transport.sent(Port::Network, index);
        assert_eq!(message.kind, MessageKind::NetworkRequest);
        unsafe { ptr::read_unaligned(message.bytes.as_ptr().cast()) }
    }

    /// The id the client will use for its first request on a fresh transport.
    const FIRST_ID: u32 = 1;

    fn reply(
        transport: &mut FakeTransport,
        operation: NetworkOperation,
        result: NetworkResult,
        state: NetworkState,
        request_id: u32,
    ) {
        let response = NetworkResponse::new(operation, result, state, request_id);
        transport.reply(Port::Network, &wrap(MessageKind::NetworkResponse, &response));
    }

    #[test]
    fn status_request_returns_the_matching_response() {
        let mut transport = FakeTransport::new();
        reply(
            &mut transport,
            NetworkOperation::Status,
            NetworkResult::Ok,
            NetworkState::Ready,
            FIRST_ID,
        );
        let mut client = NetworkClient::new();
        let response = client.request(&mut transport, NetworkOperation::Status, [0; 4], 0).unwrap();
        assert_eq!(response.state, NetworkState::Ready);
        assert_eq!(sent(&transport, 0).operation, NetworkOperation::Status);
    }

    #[test]
    fn ping_and_connect_carry_their_timeouts() {
        let mut transport = FakeTransport::new();
        let mut client = NetworkClient::new();
        reply(
            &mut transport,
            NetworkOperation::IcmpPing,
            NetworkResult::Ok,
            NetworkState::Ready,
            1,
        );
        client.request(&mut transport, NetworkOperation::IcmpPing, [10, 0, 2, 2], 0).unwrap();
        reply(
            &mut transport,
            NetworkOperation::TcpConnect,
            NetworkResult::Ok,
            NetworkState::Ready,
            2,
        );
        client.request(&mut transport, NetworkOperation::TcpConnect, [10, 0, 2, 2], 80).unwrap();
        let ping = sent(&transport, 0);
        assert_eq!(
            (ping.address, ping.timeout_ticks),
            ([10, 0, 2, 2], logos_abi::NETWORK_PING_TIMEOUT_TICKS)
        );
        let connect = sent(&transport, 1);
        assert_eq!(
            (connect.port, connect.timeout_ticks),
            (80, logos_abi::NETWORK_TCP_CONNECT_TIMEOUT_TICKS)
        );
    }

    #[test]
    fn mismatched_request_id_is_rejected_as_stale() {
        let mut transport = FakeTransport::new();
        reply(
            &mut transport,
            NetworkOperation::Status,
            NetworkResult::Ok,
            NetworkState::Ready,
            FIRST_ID + 1,
        );
        let mut client = NetworkClient::new();
        let result = client.request(&mut transport, NetworkOperation::Status, [0; 4], 0);
        assert_eq!(result, Err(IpcStatus::Stale));
    }

    #[test]
    fn wrong_message_kind_and_dead_peer_map_to_ipc_errors() {
        let mut transport = FakeTransport::new();
        let response = NetworkResponse::new(
            NetworkOperation::Status,
            NetworkResult::Ok,
            NetworkState::Ready,
            FIRST_ID,
        );
        transport.reply(Port::Network, &wrap(MessageKind::FetchResponse, &response));
        let mut client = NetworkClient::new();
        assert_eq!(
            client.request(&mut transport, NetworkOperation::Status, [0; 4], 0),
            Err(IpcStatus::Malformed)
        );

        let mut transport = FakeTransport::new();
        transport.reply_status(Port::Network, IpcStatus::Disconnected);
        assert_eq!(
            client.request(&mut transport, NetworkOperation::Status, [0; 4], 0),
            Err(IpcStatus::Disconnected)
        );

        let mut transport = FakeTransport::new();
        transport.fail_sends(Port::Network, IpcStatus::Full);
        assert_eq!(
            client.request(&mut transport, NetworkOperation::Status, [0; 4], 0),
            Err(IpcStatus::Full)
        );
    }

    #[test]
    fn no_reply_waits_a_bounded_number_of_times() {
        let mut transport = FakeTransport::new();
        let mut client = NetworkClient::new();
        assert_eq!(
            client.request(&mut transport, NetworkOperation::Status, [0; 4], 0),
            Err(IpcStatus::Empty)
        );
        assert_eq!(transport.waits, 256);
        assert!(!client.take_cancelled());
    }

    #[test]
    fn flow_cancel_sends_a_cancel_request_and_reports_cancelled() {
        let mut transport = FakeTransport::new();
        let control = FlowControl::cancel(0, 0);
        transport.reply(Port::Input, &wrap(MessageKind::FlowControl, &control));
        reply(
            &mut transport,
            NetworkOperation::Cancel,
            NetworkResult::Cancelled,
            NetworkState::Ready,
            FIRST_ID,
        );
        let mut client = NetworkClient::new();
        let result = client.request(&mut transport, NetworkOperation::IcmpPing, [10, 0, 2, 2], 0);
        assert_eq!(result, Err(IpcStatus::Empty));
        assert!(client.take_cancelled());
        assert!(!client.take_cancelled(), "the flag is consumed once");
        assert_eq!(sent(&transport, 1).operation, NetworkOperation::Cancel);
        assert_eq!(sent(&transport, 1).request_id, FIRST_ID);
    }

    #[test]
    fn closing_a_tcp_response_needs_a_live_generation() {
        let mut transport = FakeTransport::new();
        let mut client = NetworkClient::new();
        let mut response = NetworkResponse::new(
            NetworkOperation::TcpConnect,
            NetworkResult::Ok,
            NetworkState::Ready,
            1,
        );
        client.close_tcp_response(&mut transport, response);
        assert_eq!(transport.sent_count(Port::Network), 0);

        response.handle = 4;
        response.generation = 2;
        response.service_epoch = 9;
        reply(&mut transport, NetworkOperation::Close, NetworkResult::Ok, NetworkState::Ready, 1);
        client.close_tcp_response(&mut transport, response);
        let close = sent(&transport, 0);
        assert_eq!(close.operation, NetworkOperation::Close);
        assert_eq!((close.handle, close.generation, close.service_epoch), (4, 2, 9));
    }

    fn run(
        transport: &mut FakeTransport,
        command: NetworkCommand<'_>,
    ) -> (std::vec::Vec<u8>, FetchClient) {
        let mut client = NetworkClient::new();
        let mut fetch = FetchClient::new();
        let mut pending = PendingOutput::new();
        network_command(command, &mut client, transport, &mut fetch, &mut pending);
        (pending.staged().to_vec(), fetch)
    }

    #[test]
    fn network_command_maps_responses_to_user_text() {
        let ping = || NetworkCommand::Ping { address: [10, 0, 2, 2] };
        let mut transport = FakeTransport::new();
        reply(
            &mut transport,
            NetworkOperation::IcmpPing,
            NetworkResult::Ok,
            NetworkState::Ready,
            1,
        );
        assert_eq!(run(&mut transport, ping()).0, b"ping ok\r\n");

        let mut transport = FakeTransport::new();
        reply(
            &mut transport,
            NetworkOperation::IcmpPing,
            NetworkResult::Timeout,
            NetworkState::Ready,
            1,
        );
        assert_eq!(run(&mut transport, ping()).0, b"network timeout\r\n");

        let mut transport = FakeTransport::new();
        reply(&mut transport, NetworkOperation::Status, NetworkResult::Ok, NetworkState::Ready, 1);
        assert_eq!(run(&mut transport, NetworkCommand::Status).0, b"network ready\r\n");

        let mut transport = FakeTransport::new();
        reply(
            &mut transport,
            NetworkOperation::TcpConnect,
            NetworkResult::Refused,
            NetworkState::Ready,
            1,
        );
        let probe = NetworkCommand::TcpProbe { address: [10, 0, 2, 2], port: 80 };
        assert_eq!(run(&mut transport, probe).0, b"network refused\r\n");
    }

    #[test]
    fn network_command_maps_transport_errors_to_user_text() {
        for (status, text) in [
            (IpcStatus::Stale, &b"network restarting\r\n"[..]),
            (IpcStatus::Disconnected, b"network restarting\r\n"),
            (IpcStatus::Unauthorized, b"network unavailable\r\n"),
            (IpcStatus::Malformed, b"network request invalid\r\n"),
        ] {
            let mut transport = FakeTransport::new();
            transport.reply_status(Port::Network, status);
            assert_eq!(run(&mut transport, NetworkCommand::Status).0, text);
        }
        let mut transport = FakeTransport::new();
        transport.fail_sends(Port::Network, IpcStatus::Full);
        assert_eq!(run(&mut transport, NetworkCommand::Status).0, b"network queue full\r\n");

        let mut transport = FakeTransport::new();
        assert_eq!(
            run(&mut transport, NetworkCommand::Status).0,
            b"network unavailable\r\n",
            "no reply within the bound"
        );
    }

    #[test]
    fn network_command_reports_cancel_and_unknown_interfaces() {
        let mut transport = FakeTransport::new();
        let control = FlowControl::cancel(0, 0);
        transport.reply(Port::Input, &wrap(MessageKind::FlowControl, &control));
        reply(
            &mut transport,
            NetworkOperation::Cancel,
            NetworkResult::Cancelled,
            NetworkState::Ready,
            1,
        );
        let ping = NetworkCommand::Ping { address: [10, 0, 2, 2] };
        assert_eq!(run(&mut transport, ping).0, b"network cancelled\r\n");

        let mut transport = FakeTransport::new();
        let unknown = NetworkCommand::InterfaceStatus { name: b"wlan0" };
        assert_eq!(run(&mut transport, unknown).0, b"network interface not found\r\n");
        assert_eq!(transport.sent_count(Port::Network), 0);
    }

    #[test]
    fn network_fetch_command_starts_the_fetch_client() {
        let mut transport = FakeTransport::new();
        let fetch = NetworkCommand::Fetch { url: b"http://10.0.2.2/", destination: b"/f" };
        let (output, fetch_client) = run(&mut transport, fetch);
        assert!(output.is_empty());
        assert!(fetch_client.active());
        assert_eq!(transport.sent_count(Port::Fetch), 1);

        let mut client = NetworkClient::new();
        let mut busy = fetch_client;
        let mut pending = PendingOutput::new();
        network_command(
            NetworkCommand::Fetch { url: b"http://10.0.2.2/", destination: b"/f" },
            &mut client,
            &mut transport,
            &mut busy,
            &mut pending,
        );
        assert_eq!(pending.staged(), b"fetch already active\r\n");

        let mut transport = FakeTransport::new();
        let empty = NetworkCommand::Fetch { url: b"", destination: b"/f" };
        assert_eq!(run(&mut transport, empty).0, b"fetch request too large\r\n");
    }

    #[test]
    fn result_and_state_text_is_stable() {
        assert_eq!(network_result_text(NetworkResult::Timeout), b"network timeout\r\n");
        assert_eq!(network_result_text(NetworkResult::Unsupported), b"network request invalid\r\n");
        assert_eq!(network_state_text(NetworkState::Faulted), b"network unavailable\r\n");
        assert_eq!(network_state_text(NetworkState::Ready), b"network ready\r\n");
    }
}
