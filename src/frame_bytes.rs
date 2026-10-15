//! Checked byte view over a single fixed-size frame.
//!
//! IPC staging frames and storage/package data frames are identity-mapped
//! pages owned by the Core frame pool. Every access to their contents goes
//! through [`FrameBytes`], which validates the requested length (or the size of
//! the requested wire type) against the frame capacity before any raw-pointer
//! access. The `unsafe` for those accesses lives only in this module, and the
//! base pointer is a plain constructor argument so the type is host-testable
//! over a stack buffer.

use crate::memory::FrameAddress;

/// A bounds-checked view over `capacity` bytes starting at `base`.
pub(crate) struct FrameBytes {
    base: *mut u8,
    capacity: usize,
}

impl FrameBytes {
    /// View over a pool frame. The capacity is one page.
    ///
    /// Core frames are identity-mapped, page-sized and exclusively owned by the
    /// Core while it holds the `FrameAddress`; a null address yields an empty
    /// view so that no access can dereference it.
    #[cfg_attr(not(target_os = "uefi"), allow(dead_code))]
    pub(crate) fn from_frame(frame: FrameAddress) -> Self {
        let base = frame.raw() as usize as *mut u8;
        let capacity = if base.is_null() { 0 } else { crate::loader::PAGE_SIZE };
        Self { base, capacity }
    }

    /// View over an arbitrary region.
    ///
    /// # Safety
    /// `base` must be valid for reads and writes of `capacity` bytes for as long
    /// as the returned value (and any slice borrowed from it) is used, and no
    /// other live reference may alias the region during that time.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) unsafe fn from_raw(base: *mut u8, capacity: usize) -> Self {
        Self { base, capacity }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn capacity(&self) -> usize {
        self.capacity
    }

    /// The first `len` bytes, or `None` if `len` exceeds the capacity.
    pub(crate) fn bytes(&self, len: usize) -> Option<&[u8]> {
        if len > self.capacity {
            return None;
        }
        if len == 0 {
            return Some(&[]);
        }
        // SAFETY: `len <= capacity` and `from_raw`/`from_frame` guarantee `base`
        // is valid for `capacity` bytes; the returned borrow ties the slice to
        // `self`, so it cannot outlive the view or coexist with `bytes_mut`.
        Some(unsafe { core::slice::from_raw_parts(self.base, len) })
    }

    /// The first `len` bytes mutably, or `None` if `len` exceeds the capacity.
    pub(crate) fn bytes_mut(&mut self, len: usize) -> Option<&mut [u8]> {
        if len > self.capacity {
            return None;
        }
        if len == 0 {
            return Some(&mut []);
        }
        // SAFETY: as in `bytes`; `&mut self` makes the borrow exclusive.
        Some(unsafe { core::slice::from_raw_parts_mut(self.base, len) })
    }

    /// Reads a `T` from offset zero (unaligned), or `None` if it does not fit.
    ///
    /// Callers use this only for plain `repr(C)` wire types, and validate enum
    /// discriminants on the raw bytes first where the type has any.
    pub(crate) fn read<T: Copy>(&self) -> Option<T> {
        if core::mem::size_of::<T>() > self.capacity {
            return None;
        }
        // SAFETY: the size check above keeps the read inside the region, and
        // `read_unaligned` places no alignment requirement on `base`.
        Some(unsafe { core::ptr::read_unaligned(self.base.cast::<T>()) })
    }

    /// Writes `value` at offset zero (unaligned); `false` if it does not fit.
    pub(crate) fn write<T: Copy>(&mut self, value: T) -> bool {
        self.write_init(value)
    }

    /// Like [`Self::write`] for values that are not `Copy` (for example types
    /// holding atomics) and are moved into a freshly cleared frame.
    pub(crate) fn write_init<T>(&mut self, value: T) -> bool {
        if core::mem::size_of::<T>() > self.capacity {
            return false;
        }
        // SAFETY: the size check above keeps the write inside the region, and
        // `write_unaligned` places no alignment requirement on `base`.
        unsafe { core::ptr::write_unaligned(self.base.cast::<T>(), value) };
        true
    }

    /// Volatile variant of [`Self::read`] for frames a user task can mutate
    /// concurrently; the access is a single unaligned-tolerant copy.
    pub(crate) fn read_volatile<T: Copy>(&self) -> Option<T> {
        if core::mem::size_of::<T>() > self.capacity {
            return None;
        }
        if self.base.align_offset(core::mem::align_of::<T>()) != 0 {
            return None;
        }
        // SAFETY: the size check above keeps the read inside the region and the
        // alignment check makes the volatile access well defined.
        Some(unsafe { core::ptr::read_volatile(self.base.cast::<T>()) })
    }

    /// Volatile variant of [`Self::write`]; `false` if it does not fit or the
    /// base is not aligned for `T`.
    pub(crate) fn write_volatile<T: Copy>(&mut self, value: T) -> bool {
        if core::mem::size_of::<T>() > self.capacity
            || self.base.align_offset(core::mem::align_of::<T>()) != 0
        {
            return false;
        }
        // SAFETY: size and alignment were checked above.
        unsafe { core::ptr::write_volatile(self.base.cast::<T>(), value) };
        true
    }
}

/// The in-memory bytes of a plain `repr(C)` wire value, for copying into a
/// message or page.
pub(crate) fn value_bytes<T: Copy>(value: &T) -> &[u8] {
    // SAFETY: `value` is a live reference, so it is valid for `size_of::<T>()`
    // bytes; `u8` has no alignment requirement and the borrow keeps the value
    // alive and unmutated for the lifetime of the slice.
    unsafe {
        core::slice::from_raw_parts((value as *const T).cast::<u8>(), core::mem::size_of::<T>())
    }
}

/// Reads a `T` (unaligned) from the start of `bytes`, or `None` if `bytes` is
/// shorter than `T`. Same contract as [`FrameBytes::read`], for byte arrays that
/// are already inside a validated message.
pub(crate) fn read_from_slice<T: Copy>(bytes: &[u8]) -> Option<T> {
    if core::mem::size_of::<T>() > bytes.len() {
        return None;
    }
    // SAFETY: the length check keeps the read inside `bytes`, and
    // `read_unaligned` places no alignment requirement on the pointer.
    Some(unsafe { core::ptr::read_unaligned(bytes.as_ptr().cast::<T>()) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[repr(C)]
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    struct Wire {
        a: u32,
        b: u64,
    }

    fn view(buffer: &mut [u8]) -> FrameBytes {
        // SAFETY: the buffer outlives the view within each test and is not
        // otherwise accessed while the view is used.
        unsafe { FrameBytes::from_raw(buffer.as_mut_ptr(), buffer.len()) }
    }

    #[test]
    fn bytes_respect_capacity() {
        let mut buffer = [7u8; 16];
        let frame = view(&mut buffer);
        assert_eq!(frame.capacity(), 16);
        assert_eq!(frame.bytes(0).map(<[u8]>::len), Some(0));
        assert_eq!(frame.bytes(1).map(<[u8]>::len), Some(1));
        assert_eq!(frame.bytes(16).map(<[u8]>::len), Some(16));
        assert!(frame.bytes(17).is_none());
        assert!(frame.bytes(usize::MAX).is_none());
    }

    #[test]
    fn bytes_mut_respect_capacity_and_write_through() {
        let mut buffer = [0u8; 8];
        {
            let mut frame = view(&mut buffer);
            assert!(frame.bytes_mut(9).is_none());
            assert_eq!(frame.bytes_mut(0).map(|b| b.len()), Some(0));
            frame.bytes_mut(8).unwrap().copy_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
        }
        assert_eq!(buffer, [1, 2, 3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn zero_capacity_view_never_dereferences() {
        let mut frame = unsafe { FrameBytes::from_raw(core::ptr::null_mut(), 0) };
        assert_eq!(frame.bytes(0).map(<[u8]>::len), Some(0));
        assert!(frame.bytes(1).is_none());
        assert!(frame.bytes_mut(1).is_none());
        assert!(frame.read::<u8>().is_none());
        assert!(!frame.write(1u8));
    }

    #[test]
    fn read_write_exact_capacity_and_one_over() {
        let size = core::mem::size_of::<Wire>();
        let value = Wire { a: 0xdead_beef, b: 0x0123_4567_89ab_cdef };

        let mut exact = std::vec![0u8; size];
        let mut frame = view(&mut exact);
        assert!(frame.write(value));
        assert_eq!(frame.read::<Wire>(), Some(value));

        let mut short = std::vec![0u8; size - 1];
        let mut frame = view(&mut short);
        assert!(!frame.write(value));
        assert!(frame.read::<Wire>().is_none());
        assert!(short.iter().all(|byte| *byte == 0), "failed write must not store");
    }

    #[test]
    fn unaligned_backing_store() {
        let mut backing = [0u8; 32];
        for offset in 0..8 {
            let region = &mut backing[offset..offset + core::mem::size_of::<Wire>()];
            let mut frame = view(region);
            let value = Wire { a: offset as u32, b: !(offset as u64) };
            assert!(frame.write(value));
            assert_eq!(frame.read::<Wire>(), Some(value));
        }
    }

    #[test]
    fn write_then_bytes_observes_little_endian_layout() {
        let mut buffer = [0u8; 8];
        let mut frame = view(&mut buffer);
        assert!(frame.write(0x0102_0304u32));
        assert_eq!(frame.bytes(4), Some(&[4, 3, 2, 1][..]));
        assert_eq!(frame.read::<u32>(), Some(0x0102_0304));
    }

    #[test]
    fn volatile_round_trip_and_bounds() {
        #[repr(align(8))]
        struct Aligned([u8; 16]);
        let mut backing = Aligned([0; 16]);
        let mut frame = unsafe { FrameBytes::from_raw(backing.0.as_mut_ptr(), 16) };
        assert!(frame.write_volatile(0x1122_3344_5566_7788u64));
        assert_eq!(frame.read_volatile::<u64>(), Some(0x1122_3344_5566_7788));
        assert!(!frame.write_volatile([0u8; 17]));
        assert!(frame.read_volatile::<[u8; 17]>().is_none());
        let mut frame = unsafe { FrameBytes::from_raw(backing.0.as_mut_ptr().add(1), 15) };
        assert!(!frame.write_volatile(1u64));
        assert!(frame.read_volatile::<u64>().is_none());
    }

    #[test]
    fn value_bytes_round_trips_through_read_from_slice() {
        let wire = Wire { a: 7, b: 9 };
        let bytes = value_bytes(&wire);
        assert_eq!(bytes.len(), core::mem::size_of::<Wire>());
        assert_eq!(read_from_slice::<Wire>(bytes), Some(wire));
    }

    #[test]
    fn read_from_slice_bounds() {
        let bytes = [1u8, 0, 0, 0, 9];
        assert_eq!(read_from_slice::<u32>(&bytes), Some(1));
        assert_eq!(read_from_slice::<u32>(&bytes[1..]), Some(0x0900_0000));
        assert!(read_from_slice::<u32>(&bytes[..3]).is_none());
        assert!(read_from_slice::<u8>(&[]).is_none());
    }
}
