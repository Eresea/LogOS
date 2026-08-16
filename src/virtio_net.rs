//! Host-testable bounded VirtIO-net queue ownership model.

use logos_abi::{NETWORK_DMA_BUFFER_BYTES, NETWORK_MAX_FRAME_BYTES, NETWORK_QUEUE_DESCRIPTORS};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Descriptor {
    pub index: u16,
    pub generation: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QueueError {
    Full,
    Empty,
    Stale,
    InvalidLength,
    Timeout,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Entry {
    generation: u16,
    length: u16,
    owned: bool,
}

impl Entry {
    const EMPTY: Self = Self { generation: 1, length: 0, owned: false };
}

pub struct VirtioNetQueue {
    entries: [Entry; NETWORK_QUEUE_DESCRIPTORS],
    next: usize,
    generation: u16,
}

impl Default for VirtioNetQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl VirtioNetQueue {
    pub const fn new() -> Self {
        Self { entries: [Entry::EMPTY; NETWORK_QUEUE_DESCRIPTORS], next: 0, generation: 1 }
    }

    pub fn submit(&mut self, length: usize) -> Result<Descriptor, QueueError> {
        if length > NETWORK_MAX_FRAME_BYTES || length > NETWORK_DMA_BUFFER_BYTES {
            return Err(QueueError::InvalidLength);
        }
        for offset in 0..NETWORK_QUEUE_DESCRIPTORS {
            let index = (self.next + offset) % NETWORK_QUEUE_DESCRIPTORS;
            if !self.entries[index].owned {
                self.entries[index].owned = true;
                self.entries[index].length = length as u16;
                self.next = (index + 1) % NETWORK_QUEUE_DESCRIPTORS;
                return Ok(Descriptor {
                    index: index as u16,
                    generation: self.entries[index].generation,
                });
            }
        }
        Err(QueueError::Full)
    }

    pub fn complete(&mut self, descriptor: Descriptor, length: usize) -> Result<(), QueueError> {
        if length > NETWORK_MAX_FRAME_BYTES || length > NETWORK_DMA_BUFFER_BYTES {
            return Err(QueueError::InvalidLength);
        }
        let Some(entry) = self.entries.get_mut(descriptor.index as usize) else {
            return Err(QueueError::Stale);
        };
        if !entry.owned || entry.generation != descriptor.generation {
            return Err(QueueError::Stale);
        }
        entry.length = length as u16;
        entry.owned = false;
        Ok(())
    }

    pub fn reset(&mut self) {
        self.generation = self.generation.wrapping_add(1).max(1);
        for entry in &mut self.entries {
            entry.generation = self.generation;
            entry.length = 0;
            entry.owned = false;
        }
        self.next = 0;
    }

    pub const fn generation(&self) -> u16 {
        self.generation
    }

    pub fn pending(&self) -> usize {
        self.entries.iter().filter(|entry| entry.owned).count()
    }
}

pub struct VirtioNetDevice {
    present: bool,
    initialized: bool,
    generation: u16,
    rx: VirtioNetQueue,
    tx: VirtioNetQueue,
}

impl VirtioNetDevice {
    pub const fn new() -> Self {
        Self {
            present: false,
            initialized: false,
            generation: 1,
            rx: VirtioNetQueue::new(),
            tx: VirtioNetQueue::new(),
        }
    }

    pub fn discover(&mut self, present: bool) {
        self.present = present;
        self.initialized = false;
    }

    pub fn initialize(&mut self) -> bool {
        if !self.present {
            return false;
        }
        self.initialized = true;
        true
    }

    pub fn submit_tx(&mut self, length: usize) -> Result<Descriptor, QueueError> {
        if !self.initialized {
            return Err(QueueError::Stale);
        }
        self.tx.submit(length)
    }

    pub fn complete_tx(&mut self, descriptor: Descriptor, length: usize) -> Result<(), QueueError> {
        self.tx.complete(descriptor, length)
    }

    pub fn reset(&mut self) {
        self.generation = self.generation.wrapping_add(1).max(1);
        self.initialized = false;
        self.rx.reset();
        self.tx.reset();
    }

    pub fn timeout_reset(&mut self, elapsed: u64, deadline: u64) -> Result<(), QueueError> {
        if elapsed > deadline {
            self.reset();
            Err(QueueError::Timeout)
        } else {
            Ok(())
        }
    }

    pub const fn generation(&self) -> u16 {
        self.generation
    }

    pub const fn rx(&self) -> &VirtioNetQueue {
        &self.rx
    }

    pub const fn tx(&self) -> &VirtioNetQueue {
        &self.tx
    }
}

impl Default for VirtioNetDevice {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_ownership_is_bounded_and_generation_safe() {
        let mut queue = VirtioNetQueue::new();
        let mut descriptors = [Descriptor { index: 0, generation: 0 }; NETWORK_QUEUE_DESCRIPTORS];
        for descriptor in &mut descriptors {
            *descriptor = queue.submit(64).unwrap();
        }
        assert_eq!(queue.submit(64), Err(QueueError::Full));
        queue.complete(descriptors[0], 64).unwrap();
        assert_eq!(queue.pending(), NETWORK_QUEUE_DESCRIPTORS - 1);
        queue.reset();
        assert_eq!(queue.complete(descriptors[1], 64), Err(QueueError::Stale));
    }

    #[test]
    fn device_does_not_initialize_when_absent() {
        let mut device = VirtioNetDevice::new();
        assert!(!device.initialize());
        device.discover(true);
        assert!(device.initialize());
        let descriptor = device.submit_tx(128).unwrap();
        assert!(device.complete_tx(descriptor, 128).is_ok());
    }

    #[test]
    fn invalid_frames_and_timeouts_reset_only_the_device() {
        let mut device = VirtioNetDevice::new();
        device.discover(true);
        assert!(device.initialize());
        assert_eq!(device.submit_tx(NETWORK_DMA_BUFFER_BYTES + 1), Err(QueueError::InvalidLength));
        let generation = device.generation();
        assert_eq!(device.timeout_reset(11, 10), Err(QueueError::Timeout));
        assert_ne!(device.generation(), generation);
        assert_eq!(device.tx().pending(), 0);
    }
}
