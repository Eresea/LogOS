//! Bounded, typed mailbox mechanics used by service contracts.

use core::cell::UnsafeCell;
use core::mem::MaybeUninit;
use core::sync::atomic::{AtomicBool, AtomicU16, Ordering};

use crate::terminal_abi::{EndpointHeader, MessageIdentity};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SendError {
    Full,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReceiveError {
    Empty,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SharedSendError {
    Full,
    Stale,
    Disconnected,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SharedReceiveError {
    Empty,
    Stale,
    Disconnected,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Notify {
    Notified,
    AlreadyNotified,
}

#[derive(Debug)]
pub struct Doorbell {
    notified: AtomicBool,
}

impl Doorbell {
    pub const fn new() -> Self {
        Self { notified: AtomicBool::new(false) }
    }

    pub fn ring(&self) -> bool {
        !self.notified.swap(true, Ordering::AcqRel)
    }

    pub fn take(&self) -> bool {
        self.notified.swap(false, Ordering::AcqRel)
    }
}

impl Default for Doorbell {
    fn default() -> Self {
        Self::new()
    }
}

/// A fixed SPSC ring intended to live in a shared IPC page.
///
/// The producer owns `head`, the consumer owns `tail`, and each side only
/// reads the other cursor. No references cross the boundary; entries are
/// copied into and out of `MaybeUninit` slots. The ring is generation-stamped
/// so a restarted service cannot consume messages from its predecessor.
#[repr(C)]
pub struct SharedIpc<T: Copy, const N: usize> {
    endpoint: EndpointHeader,
    connected: AtomicBool,
    head: AtomicU16,
    tail: AtomicU16,
    doorbell: Doorbell,
    entries: [UnsafeCell<MaybeUninit<T>>; N],
}

unsafe impl<T: Copy + Send, const N: usize> Send for SharedIpc<T, N> {}
unsafe impl<T: Copy + Send, const N: usize> Sync for SharedIpc<T, N> {}

impl<T: Copy, const N: usize> SharedIpc<T, N> {
    pub const fn new(endpoint: EndpointHeader) -> Self {
        assert!(N > 0 && N <= u16::MAX as usize);
        Self {
            endpoint,
            connected: AtomicBool::new(true),
            head: AtomicU16::new(0),
            tail: AtomicU16::new(0),
            doorbell: Doorbell::new(),
            entries: [const { UnsafeCell::new(MaybeUninit::uninit()) }; N],
        }
    }

    pub const fn endpoint(&self) -> EndpointHeader {
        self.endpoint
    }

    pub fn disconnect(&self) {
        self.connected.store(false, Ordering::Release);
        self.doorbell.ring();
    }

    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Acquire)
    }

    pub fn send(&self, identity: MessageIdentity, entry: T) -> Result<Notify, SharedSendError> {
        if !identity.accepts(self.endpoint) {
            return Err(SharedSendError::Stale);
        }
        if !self.is_connected() {
            return Err(SharedSendError::Disconnected);
        }
        let head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Acquire);
        if head.wrapping_sub(tail) >= N as u16 {
            return Err(SharedSendError::Full);
        }
        let was_empty = head == tail;
        let slot = (head as usize) % N;
        unsafe { (*self.entries[slot].get()).write(entry) };
        self.head.store(head.wrapping_add(1), Ordering::Release);
        Ok(if was_empty && self.doorbell.ring() {
            Notify::Notified
        } else {
            Notify::AlreadyNotified
        })
    }

    pub fn receive(&self, identity: MessageIdentity) -> Result<T, SharedReceiveError> {
        if !identity.accepts(self.endpoint) {
            return Err(SharedReceiveError::Stale);
        }
        if !self.is_connected() {
            return Err(SharedReceiveError::Disconnected);
        }
        let tail = self.tail.load(Ordering::Relaxed);
        let head = self.head.load(Ordering::Acquire);
        if tail == head {
            return Err(SharedReceiveError::Empty);
        }
        let slot = (tail as usize) % N;
        let entry = unsafe { (*self.entries[slot].get()).assume_init_read() };
        self.tail.store(tail.wrapping_add(1), Ordering::Release);
        if tail.wrapping_add(1) == head {
            self.doorbell.take();
        }
        Ok(entry)
    }

    pub fn pending(&self) -> usize {
        self.head.load(Ordering::Acquire).wrapping_sub(self.tail.load(Ordering::Acquire)) as usize
    }
}

pub struct BoundedQueue<T: Copy, const N: usize> {
    entries: [Option<T>; N],
    head: usize,
    tail: usize,
    len: usize,
    pub doorbell: Doorbell,
}

impl<T: Copy, const N: usize> BoundedQueue<T, N> {
    pub const fn new() -> Self {
        assert!(N > 0);
        Self { entries: [None; N], head: 0, tail: 0, len: 0, doorbell: Doorbell::new() }
    }

    pub const fn capacity(&self) -> usize {
        N
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub const fn is_full(&self) -> bool {
        self.len == N
    }

    pub fn send(&mut self, entry: T) -> Result<(), SendError> {
        if self.is_full() {
            return Err(SendError::Full);
        }
        let was_empty = self.is_empty();
        self.entries[self.tail] = Some(entry);
        self.tail = (self.tail + 1) % N;
        self.len += 1;
        if was_empty {
            self.doorbell.ring();
        }
        Ok(())
    }

    pub fn receive(&mut self) -> Result<T, ReceiveError> {
        let Some(entry) = self.entries[self.head].take() else {
            return Err(ReceiveError::Empty);
        };
        self.head = (self.head + 1) % N;
        self.len -= 1;
        if self.is_empty() {
            self.doorbell.take();
        }
        Ok(entry)
    }

    pub fn clear(&mut self) {
        while self.receive().is_ok() {}
    }
}

impl<T: Copy, const N: usize> Default for BoundedQueue<T, N> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal_abi::{InputMessage, KeyCode, KeyState};

    #[test]
    fn queue_backpressure_and_wakeup_are_explicit() {
        let mut queue = BoundedQueue::<u8, 2>::new();
        assert!(queue.doorbell.ring());
        assert!(queue.send(1).is_ok());
        assert!(!queue.doorbell.ring());
        assert!(queue.send(2).is_ok());
        assert_eq!(queue.send(3), Err(SendError::Full));
        assert_eq!(queue.receive(), Ok(1));
        assert_eq!(queue.receive(), Ok(2));
        assert_eq!(queue.receive(), Err(ReceiveError::Empty));
        assert!(!queue.doorbell.take());
    }

    #[test]
    fn clear_releases_all_entries() {
        let mut queue = BoundedQueue::<u8, 2>::new();
        queue.send(1).unwrap();
        queue.send(2).unwrap();
        queue.clear();
        assert!(queue.is_empty());
        assert!(queue.send(3).is_ok());
    }

    #[test]
    fn shared_ring_is_bounded_and_generation_safe() {
        let endpoint = EndpointHeader::new(2, 9);
        let ring = SharedIpc::<InputMessage, 2>::new(endpoint);
        let identity = endpoint.identity();
        let stale = MessageIdentity::new(1, 9);
        let message = InputMessage::key(KeyCode::character(b'a'), KeyState::Pressed, 0);

        assert_eq!(ring.receive(identity), Err(SharedReceiveError::Empty));
        assert_eq!(ring.send(stale, message), Err(SharedSendError::Stale));
        assert_eq!(ring.send(identity, message), Ok(Notify::Notified));
        assert_eq!(ring.send(identity, message), Ok(Notify::AlreadyNotified));
        assert_eq!(ring.send(identity, message), Err(SharedSendError::Full));
        assert_eq!(ring.pending(), 2);
        assert_eq!(ring.receive(identity), Ok(message));
        assert_eq!(ring.receive(identity), Ok(message));
        assert_eq!(ring.receive(identity), Err(SharedReceiveError::Empty));
    }

    #[test]
    fn shared_ring_disconnect_is_explicit() {
        let endpoint = EndpointHeader::new(1, 1);
        let ring = SharedIpc::<u8, 1>::new(endpoint);
        ring.disconnect();
        assert_eq!(ring.send(endpoint.identity(), 1), Err(SharedSendError::Disconnected));
        assert_eq!(ring.receive(endpoint.identity()), Err(SharedReceiveError::Disconnected));
    }
}
