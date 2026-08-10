#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EventError {
    Full,
}

pub struct EventQueue<T: Copy, const N: usize> {
    entries: [Option<T>; N],
    head: usize,
    len: usize,
    dropped: u32,
}

impl<T: Copy, const N: usize> EventQueue<T, N> {
    pub const fn new() -> Self {
        Self { entries: [const { None }; N], head: 0, len: 0, dropped: 0 }
    }

    pub fn push(&mut self, event: T) -> Result<(), EventError> {
        if self.len == N {
            self.dropped = self.dropped.saturating_add(1);
            return Err(EventError::Full);
        }
        let index = (self.head + self.len) % N;
        self.entries[index] = Some(event);
        self.len += 1;
        Ok(())
    }

    pub fn pop(&mut self) -> Option<T> {
        if self.len == 0 {
            return None;
        }
        let event = self.entries[self.head].take();
        self.head = (self.head + 1) % N;
        self.len -= 1;
        event
    }

    pub fn peek(&self) -> Option<&T> {
        (self.len != 0).then(|| self.entries[self.head].as_ref()).flatten()
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub const fn dropped(&self) -> u32 {
        self.dropped
    }
}

impl<T: Copy, const N: usize> Default for EventQueue<T, N> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_is_bounded_and_fifo() {
        let mut queue = EventQueue::<u8, 2>::new();
        assert_eq!(queue.push(1), Ok(()));
        assert_eq!(queue.push(2), Ok(()));
        assert_eq!(queue.push(3), Err(EventError::Full));
        assert_eq!(queue.pop(), Some(1));
        assert_eq!(queue.pop(), Some(2));
        assert_eq!(queue.dropped(), 1);
    }

    #[test]
    fn peek_preserves_fifo_head() {
        let mut queue = EventQueue::<u8, 2>::new();
        assert!(queue.push(1).is_ok());
        assert_eq!(queue.peek(), Some(&1));
        assert_eq!(queue.peek(), Some(&1));
        assert_eq!(queue.pop(), Some(1));
    }
}
