//! Fixed physical-frame supply for user address spaces.

use logos_abi::MAX_MANAGED_FRAMES;

use crate::boot_resources::{MemoryMap, PAGE_SIZE};

const FRAME_WORDS: usize = MAX_MANAGED_FRAMES / 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameAddress(u64);

impl FrameAddress {
    pub(crate) const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    pub const fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FramePoolError {
    InvalidMap,
    Exhausted,
}

pub struct FramePool {
    frames: [u64; MAX_MANAGED_FRAMES],
    used: [u64; FRAME_WORDS],
    count: usize,
    cursor: usize,
}

impl FramePool {
    pub const fn empty() -> Self {
        Self { frames: [0; MAX_MANAGED_FRAMES], used: [0; FRAME_WORDS], count: 0, cursor: 0 }
    }

    pub fn initialize(&mut self, memory_map: &MemoryMap) -> Result<(), FramePoolError> {
        self.frames.fill(0);
        self.used.fill(0);
        self.count = 0;
        self.cursor = 0;
        for index in 0..memory_map.len() {
            let Some(descriptor) = memory_map.get(index) else {
                return Err(FramePoolError::InvalidMap);
            };
            if !descriptor.available {
                continue;
            }
            for page in 0..descriptor.pages {
                if self.count == MAX_MANAGED_FRAMES {
                    return Ok(());
                }
                let Some(offset) = page.checked_mul(PAGE_SIZE) else {
                    return Err(FramePoolError::InvalidMap);
                };
                let Some(address) = descriptor.physical_start.checked_add(offset) else {
                    return Err(FramePoolError::InvalidMap);
                };
                self.frames[self.count] = address;
                self.count += 1;
            }
        }
        Ok(())
    }

    pub const fn capacity(&self) -> usize {
        self.count
    }

    pub fn allocate(&mut self) -> Result<FrameAddress, FramePoolError> {
        if self.count == 0 {
            return Err(FramePoolError::Exhausted);
        }
        for offset in 0..self.count {
            let index = (self.cursor + offset) % self.count;
            let word = index / 64;
            let bit = 1u64 << (index % 64);
            if self.used[word] & bit == 0 {
                self.used[word] |= bit;
                self.cursor = (index + 1) % self.count;
                return Ok(FrameAddress(self.frames[index]));
            }
        }
        Err(FramePoolError::Exhausted)
    }

    pub fn release(&mut self, frame: FrameAddress) -> Result<(), FramePoolError> {
        let Some(index) = self.frames[..self.count].iter().position(|address| *address == frame.0)
        else {
            return Err(FramePoolError::InvalidMap);
        };
        let word = index / 64;
        let bit = 1u64 << (index % 64);
        if self.used[word] & bit == 0 {
            return Err(FramePoolError::InvalidMap);
        }
        self.used[word] &= !bit;
        self.cursor = index;
        Ok(())
    }
}

impl Default for FramePool {
    fn default() -> Self {
        Self::empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boot_resources::MemoryDescriptor;

    #[test]
    fn pool_allocates_and_reuses_fixed_frames() {
        let mut map = MemoryMap::new();
        map.push(MemoryDescriptor::new(0x1000, 2, true).unwrap()).unwrap();
        let mut pool = FramePool::empty();
        pool.initialize(&map).unwrap();
        assert_eq!(pool.capacity(), 2);
        let first = pool.allocate().unwrap();
        let second = pool.allocate().unwrap();
        assert_ne!(first, second);
        assert_eq!(pool.allocate(), Err(FramePoolError::Exhausted));
        pool.release(first).unwrap();
        assert_eq!(pool.allocate(), Ok(first));
    }

    #[test]
    fn reserved_memory_is_not_allocated() {
        let mut map = MemoryMap::new();
        map.push(MemoryDescriptor::new(0x1000, 2, false).unwrap()).unwrap();
        let mut pool = FramePool::empty();
        pool.initialize(&map).unwrap();
        assert_eq!(pool.capacity(), 0);
        assert_eq!(pool.allocate(), Err(FramePoolError::Exhausted));
    }
}
