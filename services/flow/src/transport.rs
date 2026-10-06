//! The one seam between the Flow clients and the rest of the system.
//!
//! Every client (Storage, Package, User, Device, Network, Fetch) and
//! `PendingOutput` talk to their peers only through [`Transport`]. Two adapters
//! exist: the service image's IPC-syscall adapter (`services/images/src/flow.rs`)
//! and the in-memory fake used by this crate's host tests.

use logos_abi::IpcStatus;

/// A peer endpoint the Flow image holds a send and/or receive capability for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Port {
    Storage,
    Network,
    Fetch,
    Device,
    User,
    /// Session input: `SessionInput`, `CompletionRequest` and `FlowControl`.
    Input,
    /// Session output: `SessionOutput`, `FlowProgress` and completion replies.
    Output,
}

/// Independent request-id counters; ids start at 1 and never yield 0.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdSpace {
    /// Shared by the Network and Fetch clients.
    Network,
    Device,
    User,
}

pub trait Transport {
    /// Send one fixed-size message on `port`. `Full` means retry later.
    fn send<T: Copy>(&mut self, port: Port, message: &T) -> IpcStatus;
    /// Receive one fixed-size message from `port`; `Empty` means none queued.
    fn receive<T: Copy>(&mut self, port: Port, message: &mut T) -> IpcStatus;
    /// Block until any port may have progressed.
    fn wait(&mut self);
    fn next_request_id(&mut self, space: IdSpace) -> u32;
    /// Emit a QEMU proof marker. Adapters without proof builds ignore it.
    fn proof_line(&mut self, _line: &[u8]) {}
}

#[cfg(test)]
pub(crate) mod fake {
    use super::*;
    use core::mem;
    use std::{collections::VecDeque, vec::Vec};

    const PORTS: usize = 7;

    /// In-memory [`Transport`]: tests queue replies per port and inspect what
    /// the client sent.
    pub(crate) struct FakeTransport {
        inbound: [VecDeque<Result<Vec<u8>, IpcStatus>>; PORTS],
        sent: [Vec<Vec<u8>>; PORTS],
        send_failure: [Option<IpcStatus>; PORTS],
        ids: [u32; 3],
        pub waits: u32,
        pub proof_lines: Vec<Vec<u8>>,
    }

    impl FakeTransport {
        pub fn new() -> Self {
            Self {
                inbound: Default::default(),
                sent: Default::default(),
                send_failure: [None; PORTS],
                ids: [1; 3],
                waits: 0,
                proof_lines: Vec::new(),
            }
        }

        /// Queue a reply the client will receive from `port`.
        pub fn reply<T: Copy>(&mut self, port: Port, message: &T) {
            let bytes = unsafe {
                core::slice::from_raw_parts((message as *const T).cast::<u8>(), mem::size_of::<T>())
            };
            self.inbound[port as usize].push_back(Ok(bytes.to_vec()));
        }

        /// Queue a receive failure (for example `Disconnected`).
        pub fn reply_status(&mut self, port: Port, status: IpcStatus) {
            self.inbound[port as usize].push_back(Err(status));
        }

        /// Make every following send on `port` fail with `status`.
        pub fn fail_sends(&mut self, port: Port, status: IpcStatus) {
            self.send_failure[port as usize] = Some(status);
        }

        pub fn sent_count(&self, port: Port) -> usize {
            self.sent[port as usize].len()
        }

        /// The `index`th message sent on `port`, decoded as `T`.
        pub fn sent<T: Copy>(&self, port: Port, index: usize) -> T {
            let bytes = &self.sent[port as usize][index];
            assert_eq!(bytes.len(), mem::size_of::<T>());
            unsafe { core::ptr::read_unaligned(bytes.as_ptr().cast()) }
        }

        pub fn last_sent<T: Copy>(&self, port: Port) -> T {
            self.sent(port, self.sent_count(port) - 1)
        }
    }

    impl Transport for FakeTransport {
        fn send<T: Copy>(&mut self, port: Port, message: &T) -> IpcStatus {
            if let Some(status) = self.send_failure[port as usize] {
                return status;
            }
            let bytes = unsafe {
                core::slice::from_raw_parts((message as *const T).cast::<u8>(), mem::size_of::<T>())
            };
            self.sent[port as usize].push(bytes.to_vec());
            IpcStatus::Ok
        }

        fn receive<T: Copy>(&mut self, port: Port, message: &mut T) -> IpcStatus {
            match self.inbound[port as usize].pop_front() {
                None => IpcStatus::Empty,
                Some(Err(status)) => status,
                Some(Ok(bytes)) if bytes.len() == mem::size_of::<T>() => {
                    *message = unsafe { core::ptr::read_unaligned(bytes.as_ptr().cast()) };
                    IpcStatus::Ok
                }
                Some(Ok(_)) => IpcStatus::Malformed,
            }
        }

        fn wait(&mut self) {
            self.waits += 1;
        }

        fn next_request_id(&mut self, space: IdSpace) -> u32 {
            let id = &mut self.ids[space as usize];
            let current = *id;
            *id = id.wrapping_add(1).max(1);
            current
        }

        fn proof_line(&mut self, line: &[u8]) {
            self.proof_lines.push(line.to_vec());
        }
    }
}
