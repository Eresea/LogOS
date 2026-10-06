use core::{mem, ptr};

use logos_abi::{
    FetchBodyChunk, FetchControl, FetchPhase, FetchRequest, FetchResponse, FetchStatus,
    FlowControl, IpcBytes, IpcStatus, MessageKind,
};

use crate::{
    FlowService, IdSpace, MAX_FLOW_BYTES, PendingOutput, Port, Transport,
    interpreter::{MAX_VALUE_BYTES, MAX_VARIABLE_NAME_BYTES},
};

/// Single-flight client for the Fetch service. Progress frames are forwarded to
/// the Session output tagged with the owning session; the body is collected
/// only in "response" mode (no destination file).
pub struct FetchClient {
    active: bool,
    request_id: u32,
    initial_progress: Option<FetchResponse>,
    cancel_pending: bool,
    response_mode: bool,
    foreground: bool,
    response_status: u16,
    response_ok: bool,
    body: [u8; MAX_VALUE_BYTES],
    body_len: usize,
    promise_name: [u8; MAX_VARIABLE_NAME_BYTES],
    promise_name_len: usize,
    callback_destination: [u8; MAX_FLOW_BYTES],
    callback_destination_len: usize,
}

impl Default for FetchClient {
    fn default() -> Self {
        Self::new()
    }
}

impl FetchClient {
    pub const fn new() -> Self {
        Self {
            active: false,
            request_id: 0,
            initial_progress: None,
            cancel_pending: false,
            response_mode: false,
            foreground: true,
            response_status: 0,
            response_ok: false,
            body: [0; MAX_VALUE_BYTES],
            body_len: 0,
            promise_name: [0; MAX_VARIABLE_NAME_BYTES],
            promise_name_len: 0,
            callback_destination: [0; MAX_FLOW_BYTES],
            callback_destination_len: 0,
        }
    }

    pub fn active(&self) -> bool {
        self.active
    }

    fn start_with_mode<T: Transport>(
        &mut self,
        transport: &mut T,
        url: &[u8],
        destination: &[u8],
        foreground: bool,
    ) -> bool {
        if self.active {
            return false;
        }
        let request_id = transport.next_request_id(IdSpace::Network);
        let Some(request) = FetchRequest::new(request_id, url, destination) else {
            return false;
        };
        let bytes = unsafe {
            core::slice::from_raw_parts(
                (&request as *const FetchRequest).cast::<u8>(),
                mem::size_of::<FetchRequest>(),
            )
        };
        let Some(message) = IpcBytes::from_bytes(MessageKind::FetchRequest, bytes) else {
            return false;
        };
        if transport.send(Port::Fetch, &message) != IpcStatus::Ok {
            return false;
        }
        self.active = true;
        self.request_id = request_id;
        self.cancel_pending = false;
        self.response_mode = destination.is_empty();
        self.foreground = foreground;
        self.response_status = 0;
        self.response_ok = false;
        self.body_len = 0;
        self.callback_destination_len = 0;
        self.initial_progress = Some(FetchResponse::new(
            request_id,
            FetchPhase::Connect,
            FetchStatus::InProgress,
            0,
            None,
        ));
        transport.proof_line(b"LogOS vNext: Flow fetch started");
        true
    }

    pub fn start<T: Transport>(
        &mut self,
        transport: &mut T,
        url: &[u8],
        destination: &[u8],
    ) -> bool {
        self.start_with_mode(transport, url, destination, true)
    }

    pub fn start_response<T: Transport>(&mut self, transport: &mut T, url: &[u8]) -> bool {
        self.start_with_mode(transport, url, &[], true)
    }

    pub fn start_response_background<T: Transport>(
        &mut self,
        transport: &mut T,
        url: &[u8],
    ) -> bool {
        self.start_with_mode(transport, url, &[], false)
    }

    pub fn start_named_response<T: Transport>(
        &mut self,
        transport: &mut T,
        url: &[u8],
        name: &[u8],
        foreground: bool,
    ) -> bool {
        if name.len() > self.promise_name.len()
            || !self.start_with_mode(transport, url, &[], foreground)
        {
            return false;
        }
        self.promise_name[..name.len()].copy_from_slice(name);
        self.promise_name_len = name.len();
        true
    }

    pub fn start_to_file_mode<T: Transport>(
        &mut self,
        transport: &mut T,
        url: &[u8],
        destination: &[u8],
        foreground: bool,
    ) -> bool {
        self.start_with_mode(transport, url, destination, foreground)
    }

    pub fn start_response_to_file<T: Transport>(
        &mut self,
        transport: &mut T,
        url: &[u8],
        destination: &[u8],
        foreground: bool,
    ) -> bool {
        if destination.is_empty() || destination.len() > self.callback_destination.len() {
            return false;
        }
        if !self.start_with_mode(transport, url, &[], foreground) {
            return false;
        }
        self.callback_destination[..destination.len()].copy_from_slice(destination);
        self.callback_destination_len = destination.len();
        true
    }

    pub fn foreground(&self) -> bool {
        self.foreground
    }

    /// Promote a background fetch to the foreground (`await`).
    pub fn set_foreground(&mut self) {
        self.foreground = true;
    }

    /// The finished response body and its destination file, when a
    /// `write response` callback is waiting to be published.
    pub fn take_callback(&mut self) -> Option<(&[u8], &[u8])> {
        if self.callback_destination_len == 0 || self.active || !self.response_ok {
            return None;
        }
        Some((
            &self.callback_destination[..self.callback_destination_len],
            &self.body[..self.body_len],
        ))
    }

    pub fn clear_callback(&mut self) {
        self.callback_destination_len = 0;
        self.body_len = 0;
        self.response_ok = false;
    }

    pub fn active_promise_is(&self, name: &[u8]) -> bool {
        self.promise_name_len == name.len() && self.promise_name[..self.promise_name_len] == *name
    }

    pub fn resolve_promise(&mut self, flow: &mut FlowService) {
        if self.active || self.promise_name_len == 0 {
            return;
        }
        let name = &self.promise_name[..self.promise_name_len];
        if self.response_ok {
            let _ = flow.resolve_response_promise(
                name,
                self.response_status,
                &self.body[..self.body_len],
            );
        } else {
            let _ = flow.cancel_promise(name);
        }
        self.promise_name_len = 0;
    }

    pub fn cancel(&mut self) {
        if !self.active {
            return;
        }
        self.cancel_pending = true;
    }

    /// Whether a cancel is queued but not yet sent to the Fetch service.
    pub fn cancel_pending(&self) -> bool {
        self.cancel_pending
    }

    /// Try to send a queued cancel. `Full` means retry after waiting; every
    /// other outcome clears the queued cancel.
    pub fn send_cancel<T: Transport>(&mut self, transport: &mut T) -> IpcStatus {
        let control = FetchControl::cancel(self.request_id);
        let bytes = unsafe {
            core::slice::from_raw_parts(
                (&control as *const FetchControl).cast::<u8>(),
                mem::size_of::<FetchControl>(),
            )
        };
        let status = match IpcBytes::from_bytes(MessageKind::FetchControl, bytes) {
            Some(message) => transport.send(Port::Fetch, &message),
            None => IpcStatus::Malformed,
        };
        if status != IpcStatus::Full {
            self.cancel_pending = false;
        }
        status
    }

    /// Apply a Session `FlowControl` cancel addressed to this fetch.
    pub fn handle_control(&mut self, message: &IpcBytes, session: u8) -> bool {
        if message.kind != MessageKind::FlowControl
            || message.len as usize != mem::size_of::<FlowControl>()
        {
            return false;
        }
        if !FlowControl::wire_enums_valid(&message.bytes[..mem::size_of::<FlowControl>()]) {
            return false;
        }
        let control: FlowControl = unsafe { ptr::read_unaligned(message.bytes.as_ptr().cast()) };
        if control.is_valid()
            && control.session == session
            && (control.request_id == 0 || control.request_id == self.request_id)
        {
            self.cancel();
            true
        } else {
            false
        }
    }

    fn forward_progress<T: Transport>(
        transport: &mut T,
        session: u8,
        response: FetchResponse,
    ) -> IpcStatus {
        let bytes = unsafe {
            core::slice::from_raw_parts(
                (&response as *const FetchResponse).cast::<u8>(),
                mem::size_of::<FetchResponse>(),
            )
        };
        let Some(message) = IpcBytes::from_bytes(MessageKind::FlowProgress, bytes) else {
            return IpcStatus::Malformed;
        };
        transport.send(Port::Output, &message.with_session(session))
    }

    pub fn drive<T: Transport>(
        &mut self,
        transport: &mut T,
        pending: &mut PendingOutput,
        session: u8,
    ) -> bool {
        if let Some(response) = self.initial_progress {
            match Self::forward_progress(transport, session, response) {
                IpcStatus::Ok => self.initial_progress = None,
                IpcStatus::Full => return false,
                _ => self.initial_progress = None,
            }
        }
        if self.cancel_pending {
            match self.send_cancel(transport) {
                IpcStatus::Full => return false,
                _ => self.cancel_pending = false,
            }
        }
        let mut message = IpcBytes::empty(MessageKind::FetchResponse);
        match transport.receive(Port::Fetch, &mut message) {
            IpcStatus::Empty => return false,
            IpcStatus::Ok => {}
            IpcStatus::Stale
            | IpcStatus::Disconnected
            | IpcStatus::Unauthorized
            | IpcStatus::Full
            | IpcStatus::Malformed => {
                self.active = false;
                pending.stage(b"fetch failed\r\n");
                return true;
            }
        }
        if message.kind == MessageKind::FetchBodyChunk {
            if message.len as usize != mem::size_of::<FetchBodyChunk>() {
                self.active = false;
                pending.stage(b"fetch body malformed\r\n");
                return true;
            }
            let chunk: FetchBodyChunk =
                unsafe { ptr::read_unaligned(message.bytes.as_ptr().cast()) };
            let end = chunk.offset as usize + usize::from(chunk.len);
            if !self.response_mode
                || !chunk.is_valid()
                || chunk.request_id != self.request_id
                || chunk.offset as usize != self.body_len
                || end > self.body.len()
            {
                self.active = false;
                pending.stage(b"fetch body stale\r\n");
                return true;
            }
            self.body[self.body_len..end].copy_from_slice(&chunk.bytes[..usize::from(chunk.len)]);
            self.body_len = end;
            return true;
        }
        if message.kind != MessageKind::FetchResponse
            || message.len as usize != mem::size_of::<FetchResponse>()
        {
            self.active = false;
            pending.stage(b"fetch failed\r\n");
            return true;
        }
        if !FetchResponse::wire_enums_valid(&message.bytes[..mem::size_of::<FetchResponse>()]) {
            self.active = false;
            pending.stage(b"fetch failed\r\n");
            return true;
        }
        let response: FetchResponse = unsafe { ptr::read_unaligned(message.bytes.as_ptr().cast()) };
        if !response.is_valid() || response.request_id != self.request_id {
            self.active = false;
            pending.stage(b"fetch failed\r\n");
            return true;
        }
        if matches!(
            response.phase,
            FetchPhase::Complete | FetchPhase::Failed | FetchPhase::Cancelled
        ) {
            self.response_status = response.response_status;
            self.response_ok = response.status == FetchStatus::Ok;
            self.active = false;
            self.cancel_pending = false;
            let message = match response.status {
                FetchStatus::Ok => {
                    transport.proof_line(b"LogOS vNext: Flow fetch complete");
                    b"fetch complete\r\n" as &[u8]
                }
                FetchStatus::Cancelled => {
                    transport.proof_line(b"LogOS vNext: Flow fetch cancelled");
                    b"fetch cancelled\r\n"
                }
                _ => {
                    transport.proof_line(b"LogOS vNext: Flow fetch failed");
                    b"fetch failed\r\n"
                }
            };
            if !(response.status == FetchStatus::Ok
                && !self.foreground
                && self.callback_destination_len == 0)
            {
                pending.stage(message);
            }
        } else {
            let _ = Self::forward_progress(transport, session, response);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::fake::FakeTransport;
    use std::vec::Vec;

    const SESSION: u8 = 1;
    const URL: &[u8] = b"http://10.0.2.2/x";

    fn wrap<T: Copy>(kind: MessageKind, value: &T) -> IpcBytes {
        let bytes = unsafe {
            core::slice::from_raw_parts((value as *const T).cast::<u8>(), mem::size_of::<T>())
        };
        IpcBytes::from_bytes(kind, bytes).unwrap()
    }

    fn request_id(transport: &FakeTransport) -> u32 {
        let message: IpcBytes = transport.last_sent(Port::Fetch);
        let request: FetchRequest = unsafe { ptr::read_unaligned(message.bytes.as_ptr().cast()) };
        request.request_id
    }

    fn respond(
        transport: &mut FakeTransport,
        id: u32,
        phase: FetchPhase,
        status: FetchStatus,
        http: u16,
    ) {
        let response = FetchResponse::new(id, phase, status, 0, None).with_response_status(http);
        transport.reply(Port::Fetch, &wrap(MessageKind::FetchResponse, &response));
    }

    fn chunk(transport: &mut FakeTransport, id: u32, offset: u32, bytes: &[u8]) {
        let chunk = FetchBodyChunk::new(id, offset, bytes).unwrap();
        transport.reply(Port::Fetch, &wrap(MessageKind::FetchBodyChunk, &chunk));
    }

    fn output(pending: &PendingOutput) -> Vec<u8> {
        pending.staged().to_vec()
    }

    #[test]
    fn file_fetch_forwards_progress_then_reports_completion() {
        let mut transport = FakeTransport::new();
        let mut pending = PendingOutput::new();
        let mut client = FetchClient::new();
        assert!(client.start(&mut transport, URL, b"/out"));
        assert!(client.active());
        let id = request_id(&transport);

        // The Connect progress frame goes to Session tagged with the session.
        assert!(!client.drive(&mut transport, &mut pending, SESSION), "nothing to receive yet");
        let progress: IpcBytes = transport.last_sent(Port::Output);
        assert_eq!((progress.kind, progress.session()), (MessageKind::FlowProgress, SESSION));

        respond(&mut transport, id, FetchPhase::ReadResponse, FetchStatus::InProgress, 0);
        assert!(client.drive(&mut transport, &mut pending, SESSION));
        assert!(client.active(), "progress does not finish the fetch");
        assert_eq!(transport.sent_count(Port::Output), 2);

        respond(&mut transport, id, FetchPhase::Complete, FetchStatus::Ok, 200);
        assert!(client.drive(&mut transport, &mut pending, SESSION));
        assert!(!client.active());
        assert_eq!(output(&pending), b"fetch complete\r\n");
        assert_eq!(
            transport.proof_lines,
            [
                b"LogOS vNext: Flow fetch started".to_vec(),
                b"LogOS vNext: Flow fetch complete".to_vec()
            ]
        );
    }

    #[test]
    fn mismatched_request_id_fails_the_fetch() {
        let mut transport = FakeTransport::new();
        let mut pending = PendingOutput::new();
        let mut client = FetchClient::new();
        assert!(client.start(&mut transport, URL, b"/out"));
        let id = request_id(&transport);
        respond(&mut transport, id + 1, FetchPhase::Complete, FetchStatus::Ok, 200);
        assert!(client.drive(&mut transport, &mut pending, SESSION));
        assert!(!client.active());
        assert_eq!(output(&pending), b"fetch failed\r\n");
    }

    #[test]
    fn error_and_cancel_statuses_map_to_their_text() {
        for (status, text, proof) in [
            (
                FetchStatus::Network,
                &b"fetch failed\r\n"[..],
                &b"LogOS vNext: Flow fetch failed"[..],
            ),
            (
                FetchStatus::Cancelled,
                &b"fetch cancelled\r\n"[..],
                &b"LogOS vNext: Flow fetch cancelled"[..],
            ),
        ] {
            let mut transport = FakeTransport::new();
            let mut pending = PendingOutput::new();
            let mut client = FetchClient::new();
            assert!(client.start(&mut transport, URL, b"/out"));
            let id = request_id(&transport);
            let phase = if status == FetchStatus::Cancelled {
                FetchPhase::Cancelled
            } else {
                FetchPhase::Failed
            };
            respond(&mut transport, id, phase, status, 0);
            assert!(client.drive(&mut transport, &mut pending, SESSION));
            assert_eq!(output(&pending), text);
            assert_eq!(transport.proof_lines.last().unwrap(), proof);
        }
    }

    #[test]
    fn transport_failures_fail_the_fetch() {
        let mut transport = FakeTransport::new();
        let mut pending = PendingOutput::new();
        let mut client = FetchClient::new();
        assert!(client.start(&mut transport, URL, b"/out"));
        transport.reply_status(Port::Fetch, IpcStatus::Disconnected);
        assert!(client.drive(&mut transport, &mut pending, SESSION));
        assert!(!client.active());
        assert_eq!(output(&pending), b"fetch failed\r\n");
    }

    #[test]
    fn response_mode_collects_chunks_for_a_file_callback() {
        let mut transport = FakeTransport::new();
        let mut pending = PendingOutput::new();
        let mut client = FetchClient::new();
        assert!(client.start_response_to_file(&mut transport, URL, b"/saved", true));
        let id = request_id(&transport);
        chunk(&mut transport, id, 0, b"hel");
        chunk(&mut transport, id, 3, b"lo");
        respond(&mut transport, id, FetchPhase::Complete, FetchStatus::Ok, 200);
        for _ in 0..3 {
            assert!(client.drive(&mut transport, &mut pending, SESSION));
        }
        assert!(!client.active());
        let (destination, body) = client.take_callback().unwrap();
        assert_eq!((destination, body), (&b"/saved"[..], &b"hello"[..]));
        client.clear_callback();
        assert!(client.take_callback().is_none());
    }

    #[test]
    fn out_of_order_chunk_is_rejected_as_stale() {
        let mut transport = FakeTransport::new();
        let mut pending = PendingOutput::new();
        let mut client = FetchClient::new();
        assert!(client.start_response(&mut transport, URL));
        let id = request_id(&transport);
        chunk(&mut transport, id, 4, b"late");
        assert!(client.drive(&mut transport, &mut pending, SESSION));
        assert!(!client.active());
        assert_eq!(output(&pending), b"fetch body stale\r\n");
    }

    #[test]
    fn chunks_are_refused_when_writing_straight_to_a_file() {
        let mut transport = FakeTransport::new();
        let mut pending = PendingOutput::new();
        let mut client = FetchClient::new();
        assert!(client.start(&mut transport, URL, b"/out"));
        let id = request_id(&transport);
        chunk(&mut transport, id, 0, b"x");
        assert!(client.drive(&mut transport, &mut pending, SESSION));
        assert_eq!(output(&pending), b"fetch body stale\r\n");
    }

    #[test]
    fn background_success_without_callback_is_silent() {
        let mut transport = FakeTransport::new();
        let mut pending = PendingOutput::new();
        let mut client = FetchClient::new();
        assert!(client.start_response_background(&mut transport, URL));
        assert!(!client.foreground());
        let id = request_id(&transport);
        respond(&mut transport, id, FetchPhase::Complete, FetchStatus::Ok, 200);
        assert!(client.drive(&mut transport, &mut pending, SESSION));
        assert!(!pending.is_pending());
        client.set_foreground();
        assert!(client.foreground());
    }

    #[test]
    fn flow_cancel_for_this_session_sends_a_fetch_cancel() {
        let mut transport = FakeTransport::new();
        let mut pending = PendingOutput::new();
        let mut client = FetchClient::new();
        assert!(client.start(&mut transport, URL, b"/out"));
        let id = request_id(&transport);

        let other = wrap(MessageKind::FlowControl, &FlowControl::cancel(0, SESSION + 1));
        assert!(!client.handle_control(&other, SESSION), "another session's cancel is ignored");
        let wrong_id = wrap(MessageKind::FlowControl, &FlowControl::cancel(id + 5, SESSION));
        assert!(
            !client.handle_control(&wrong_id, SESSION),
            "a cancel for another request is ignored"
        );
        assert!(!client.cancel_pending());

        let mine = wrap(MessageKind::FlowControl, &FlowControl::cancel(0, SESSION));
        assert!(client.handle_control(&mine, SESSION));
        assert!(client.cancel_pending());
        client.drive(&mut transport, &mut pending, SESSION);
        assert!(!client.cancel_pending());
        let sent: IpcBytes = transport.last_sent(Port::Fetch);
        assert_eq!(sent.kind, MessageKind::FetchControl);
        let control: FetchControl = unsafe { ptr::read_unaligned(sent.bytes.as_ptr().cast()) };
        assert_eq!(control.request_id, id);
    }

    #[test]
    fn full_queue_keeps_the_cancel_pending() {
        let mut transport = FakeTransport::new();
        let mut client = FetchClient::new();
        assert!(client.start(&mut transport, URL, b"/out"));
        client.cancel();
        transport.fail_sends(Port::Fetch, IpcStatus::Full);
        assert_eq!(client.send_cancel(&mut transport), IpcStatus::Full);
        assert!(client.cancel_pending());
        transport.fail_sends(Port::Fetch, IpcStatus::Disconnected);
        assert_eq!(client.send_cancel(&mut transport), IpcStatus::Disconnected);
        assert!(!client.cancel_pending(), "a dead peer drops the queued cancel");
    }

    #[test]
    fn second_fetch_and_unsendable_requests_are_refused() {
        let mut transport = FakeTransport::new();
        let mut client = FetchClient::new();
        assert!(client.start(&mut transport, URL, b"/out"));
        assert!(!client.start(&mut transport, URL, b"/out"));

        let mut transport = FakeTransport::new();
        transport.fail_sends(Port::Fetch, IpcStatus::Full);
        let mut client = FetchClient::new();
        assert!(!client.start(&mut transport, URL, b"/out"));
        assert!(!client.active());
        assert!(!client.start(&mut FakeTransport::new(), b"", b"/out"), "empty URL is invalid");
    }

    #[test]
    fn named_responses_remember_their_promise() {
        let mut transport = FakeTransport::new();
        let mut client = FetchClient::new();
        assert!(client.start_named_response(&mut transport, URL, b"p", true));
        assert!(client.active_promise_is(b"p"));
        assert!(!client.active_promise_is(b"q"));
    }
}
