use logos_abi::{IPC_FLAG_MORE, IpcBytes, IpcStatus, MessageKind};

use crate::{MAX_OUTPUT_BYTES, Port, Transport};

pub struct PendingOutput {
    bytes: [u8; MAX_OUTPUT_BYTES],
    len: usize,
    offset: usize,
    pending: bool,
}

impl Default for PendingOutput {
    fn default() -> Self {
        Self::new()
    }
}

impl PendingOutput {
    pub const fn new() -> Self {
        Self { bytes: [0; MAX_OUTPUT_BYTES], len: 0, offset: 0, pending: false }
    }

    pub fn is_pending(&self) -> bool {
        self.pending
    }

    /// The bytes currently staged (for proof probes).
    pub fn staged(&self) -> &[u8] {
        &self.bytes[..self.len]
    }

    /// Drop the staged output without sending it (for proof probes).
    pub fn discard(&mut self) {
        self.len = 0;
        self.offset = 0;
        self.pending = false;
    }

    pub fn stage(&mut self, bytes: &[u8]) {
        let count = bytes.len().min(self.bytes.len());
        self.bytes[..count].copy_from_slice(&bytes[..count]);
        self.len = count;
        self.offset = 0;
        self.pending = true;
    }

    /// Tags every chunk with `session` (T3c, #98): Flow only ever has one
    /// session's command in flight at a time, but Session and Terminal still
    /// need the tag to route the reply back to the right tab.
    pub fn flush<T: Transport>(&mut self, transport: &mut T, session: u8) -> bool {
        let mut progressed = false;
        while self.offset < self.len {
            let end = (self.offset + logos_abi::MAX_IPC_BYTES).min(self.len);
            let Some(mut message) =
                IpcBytes::from_bytes(MessageKind::SessionOutput, &self.bytes[self.offset..end])
            else {
                break;
            };
            if end < self.len {
                message.flags = IPC_FLAG_MORE;
            }
            message = message.with_session(session);
            if transport.send(Port::Output, &message) != IpcStatus::Ok {
                break;
            }
            self.offset = end;
            progressed = true;
        }
        if self.pending && self.offset == self.len {
            let message = IpcBytes::empty(MessageKind::SessionOutput).with_session(session);
            if self.len == 0 && transport.send(Port::Output, &message) == IpcStatus::Ok {
                self.pending = false;
                progressed = true;
            }
        }
        if self.pending && self.offset == self.len && self.len != 0 {
            self.len = 0;
            self.offset = 0;
            self.pending = false;
        }
        progressed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::fake::FakeTransport;

    #[test]
    fn flush_tags_every_chunk_with_its_session() {
        let mut transport = FakeTransport::new();
        let mut pending = PendingOutput::new();
        let text = [b'x'; MAX_OUTPUT_BYTES];
        pending.stage(&text);
        assert!(pending.flush(&mut transport, 2));
        assert!(!pending.is_pending());
        let mut total = 0;
        for index in 0..transport.sent_count(Port::Output) {
            let message: IpcBytes = transport.sent(Port::Output, index);
            assert_eq!(message.session(), 2);
            total += usize::from(message.len);
        }
        assert_eq!(total, MAX_OUTPUT_BYTES);
        let first: IpcBytes = transport.sent(Port::Output, 0);
        assert_ne!(first.flags & IPC_FLAG_MORE, 0);
    }

    #[test]
    fn undeliverable_output_stays_queued() {
        let mut transport = FakeTransport::new();
        transport.fail_sends(Port::Output, IpcStatus::Full);
        let mut pending = PendingOutput::new();
        pending.stage(b"ok\r\n");
        assert!(!pending.flush(&mut transport, 2));
        assert!(
            pending.is_pending(),
            "an undeliverable message stays queued, not silently dropped"
        );
    }

    #[test]
    fn empty_output_sends_a_bare_terminator() {
        let mut transport = FakeTransport::new();
        let mut pending = PendingOutput::new();
        pending.stage(&[]);
        assert!(pending.flush(&mut transport, 1));
        assert!(!pending.is_pending());
        let message: IpcBytes = transport.last_sent(Port::Output);
        assert_eq!((message.len, message.session()), (0, 1));
    }
}
