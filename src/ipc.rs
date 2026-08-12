//! Bounded, typed mailbox mechanics used by service contracts.

use core::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SendError {
    Full,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReceiveError {
    Empty,
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
}
