//! Fixed resources published by the UEFI handoff.
//!
//! UEFI protocol handles and borrowed memory-map entries must not cross
//! `ExitBootServices`. This module contains the copied, validated data that
//! later kernel and service code may retain.

use logos_abi::{MAX_FRAMEBUFFER_BYTES, MAX_MEMORY_DESCRIPTORS};

pub const PAGE_SIZE: u64 = 4096;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum PixelFormat {
    Bgr8 = 1,
    Rgb8 = 2,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FramebufferInfo {
    base: u64,
    bytes: u64,
    width: u32,
    height: u32,
    stride: u32,
    format: PixelFormat,
}

impl FramebufferInfo {
    pub fn new(
        base: u64,
        bytes: u64,
        width: u32,
        height: u32,
        stride: u32,
        format: PixelFormat,
    ) -> Option<Self> {
        if base == 0
            || base % PAGE_SIZE != 0
            || bytes == 0
            || bytes > MAX_FRAMEBUFFER_BYTES as u64
            || width == 0
            || height == 0
            || stride < width
        {
            return None;
        }
        let required = (height as u64).checked_mul(stride as u64)?.checked_mul(4)?;
        (required <= bytes).then_some(Self { base, bytes, width, height, stride, format })
    }

    pub const fn base(self) -> u64 {
        self.base
    }

    pub const fn bytes(self) -> u64 {
        self.bytes
    }

    pub const fn width(self) -> u32 {
        self.width
    }

    pub const fn height(self) -> u32 {
        self.height
    }

    pub const fn stride(self) -> u32 {
        self.stride
    }

    pub const fn format(self) -> PixelFormat {
        self.format
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct MemoryDescriptor {
    pub physical_start: u64,
    pub pages: u64,
    pub available: bool,
}

impl MemoryDescriptor {
    pub const fn new(physical_start: u64, pages: u64, available: bool) -> Option<Self> {
        if physical_start % PAGE_SIZE != 0 || pages == 0 {
            return None;
        }
        Some(Self { physical_start, pages, available })
    }

    pub fn end(self) -> Option<u64> {
        self.physical_start.checked_add(self.pages.checked_mul(PAGE_SIZE)?)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResourceError {
    Capacity,
    InvalidMemoryDescriptor,
    InvalidFramebuffer,
}

#[derive(Clone, Copy)]
pub struct MemoryMap {
    entries: [Option<MemoryDescriptor>; MAX_MEMORY_DESCRIPTORS],
    count: usize,
}

impl MemoryMap {
    pub const fn new() -> Self {
        Self { entries: [None; MAX_MEMORY_DESCRIPTORS], count: 0 }
    }

    pub fn push(&mut self, descriptor: MemoryDescriptor) -> Result<(), ResourceError> {
        if descriptor.end().is_none() {
            return Err(ResourceError::InvalidMemoryDescriptor);
        }
        let Some(entry) = self.entries.get_mut(self.count) else {
            return Err(ResourceError::Capacity);
        };
        *entry = Some(descriptor);
        self.count += 1;
        Ok(())
    }

    pub const fn len(&self) -> usize {
        self.count
    }

    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }

    pub fn get(&self, index: usize) -> Option<MemoryDescriptor> {
        self.entries.get(index).copied().flatten()
    }

    pub fn available_pages(&self) -> u64 {
        self.entries[..self.count]
            .iter()
            .flatten()
            .filter(|entry| entry.available)
            .map(|entry| entry.pages)
            .sum()
    }

    pub fn normalize(
        &self,
        exclusions: &[crate::memory::MemoryExclusion],
    ) -> Result<crate::memory::NormalizedMemoryMap, crate::memory::NormalizationError> {
        crate::memory::normalize_memory_map(self, exclusions)
    }
}

impl Default for MemoryMap {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KeyboardResource {
    pub irq: u8,
    pub data_port: u16,
}

impl KeyboardResource {
    pub const PS2: Self = Self { irq: 1, data_port: 0x60 };
}

#[derive(Clone, Copy)]
pub struct BootResources {
    memory_map: MemoryMap,
    framebuffer: Option<FramebufferInfo>,
    keyboard: KeyboardResource,
}

impl BootResources {
    pub const fn new(memory_map: MemoryMap, keyboard: KeyboardResource) -> Self {
        Self { memory_map, framebuffer: None, keyboard }
    }

    pub fn publish_framebuffer(&mut self, framebuffer: FramebufferInfo) {
        self.framebuffer = Some(framebuffer);
    }

    pub const fn memory_map(&self) -> &MemoryMap {
        &self.memory_map
    }

    pub const fn framebuffer(&self) -> Option<FramebufferInfo> {
        self.framebuffer
    }

    pub const fn keyboard(&self) -> KeyboardResource {
        self.keyboard
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn framebuffer_validation_is_bounded() {
        let valid = FramebufferInfo::new(0x1000, 640 * 400 * 4, 640, 400, 640, PixelFormat::Bgr8);
        assert!(valid.is_some());
        assert!(
            FramebufferInfo::new(0x1001, 640 * 400 * 4, 640, 400, 640, PixelFormat::Bgr8).is_none()
        );
        assert!(FramebufferInfo::new(0x1000, 1, 640, 400, 640, PixelFormat::Bgr8).is_none());
    }

    #[test]
    fn memory_map_capacity_and_available_pages_are_explicit() {
        let mut map = MemoryMap::new();
        map.push(MemoryDescriptor::new(0x1000, 4, true).unwrap()).unwrap();
        map.push(MemoryDescriptor::new(0x5000, 2, false).unwrap()).unwrap();
        assert_eq!(map.len(), 2);
        assert_eq!(map.available_pages(), 4);
        assert!(!map.get(1).unwrap().available);
    }

    #[test]
    fn invalid_memory_ranges_are_rejected() {
        assert!(MemoryDescriptor::new(1, 1, true).is_none());
        assert!(MemoryDescriptor::new(0x1000, 0, true).is_none());
        assert!(MemoryDescriptor::new(u64::MAX - 0xfff, 2, true).unwrap().end().is_none());
    }
}
