//! Checked byte view over a single fixed-size frame.
//!
//! IPC staging frames and storage/package data frames are identity-mapped
//! pages owned by the Core frame pool. Every access to their contents goes
//! through [`FrameBytes`], which validates the requested length (or the size of
//! the requested wire type) against the frame capacity before any raw-pointer
//! access. The `unsafe` for those accesses lives only in this module, and the
//! base pointer is a plain constructor argument so the type is host-testable
//! over a stack buffer.

use core::mem::MaybeUninit;

use logos_abi::{NoPadding, WireBytes, WireDecode};

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
    /// `T: WireBytes` guarantees every bit pattern is a valid `T`, so no
    /// validation is needed. Types with enum fields use [`Self::decode`].
    pub(crate) fn read<T: WireBytes>(&self) -> Option<T> {
        if core::mem::size_of::<T>() > self.capacity {
            return None;
        }
        // SAFETY: the size check above keeps the read inside the region,
        // `read_unaligned` places no alignment requirement on `base`, and
        // `T: WireBytes` makes any bytes a valid `T`.
        Some(unsafe { core::ptr::read_unaligned(self.base.cast::<T>()) })
    }

    /// Decodes a `T` from offset zero, or `None` if it does not fit or its
    /// bytes are not a valid `T` (for example an undeclared enum discriminant).
    ///
    /// The frame may be mutated concurrently by a user task, so the bytes are
    /// first copied (volatile, byte-wise) into a local, and validation and
    /// materialization both use that snapshot; the validated bytes are exactly
    /// the bytes that become the value.
    pub(crate) fn decode<T: WireDecode>(&self) -> Option<T> {
        let size = core::mem::size_of::<T>();
        if size > self.capacity {
            return None;
        }
        let mut value = MaybeUninit::<T>::uninit();
        let dst = value.as_mut_ptr().cast::<u8>();
        for index in 0..size {
            // SAFETY: `index < size <= capacity`, so the source byte is inside
            // the region; `dst` is valid for `size` bytes of writes.
            unsafe { dst.add(index).write(core::ptr::read_volatile(self.base.add(index))) };
        }
        finish_decode(value)
    }

    /// Writes `value` at offset zero (unaligned); `false` if it does not fit.
    ///
    /// `T: NoPadding` guarantees every byte of the value is initialized, so the
    /// write cannot copy uninitialized kernel bytes into a user-visible frame
    /// (see `docs/architecture.md`, "Wire frame reads").
    pub(crate) fn write<T: NoPadding>(&mut self, value: T) -> bool {
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

    /// Volatile variant of [`Self::write`]; `false` if it does not fit or the
    /// base is not aligned for `T`.
    pub(crate) fn write_volatile<T: NoPadding>(&mut self, value: T) -> bool {
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
/// message or page. `T: NoPadding` makes every byte initialized, so the slice
/// over the value is sound and leaks nothing.
pub(crate) fn value_bytes<T: NoPadding>(value: &T) -> &[u8] {
    // SAFETY: `value` is a live reference, so it is valid for `size_of::<T>()`
    // bytes; `u8` has no alignment requirement and the borrow keeps the value
    // alive and unmutated for the lifetime of the slice.
    unsafe {
        core::slice::from_raw_parts((value as *const T).cast::<u8>(), core::mem::size_of::<T>())
    }
}

/// Reads a `T` (unaligned) from the start of `bytes`, or `None` if `bytes` is
/// shorter than `T`. `T: WireBytes` makes every byte pattern valid; see
/// [`decode_from_slice`] for types with enum fields.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn read_from_slice<T: WireBytes>(bytes: &[u8]) -> Option<T> {
    if core::mem::size_of::<T>() > bytes.len() {
        return None;
    }
    // SAFETY: the length check keeps the read inside `bytes`, and
    // `read_unaligned` places no alignment requirement on the pointer.
    Some(unsafe { core::ptr::read_unaligned(bytes.as_ptr().cast::<T>()) })
}

/// Decodes a `T` (unaligned) from the start of `bytes`, or `None` if `bytes` is
/// shorter than `T` or does not hold a valid `T`. Same contract as
/// [`FrameBytes::decode`], for byte arrays already copied out of a frame.
pub(crate) fn decode_from_slice<T: WireDecode>(bytes: &[u8]) -> Option<T> {
    let size = core::mem::size_of::<T>();
    let bytes = bytes.get(..size)?;
    let mut value = MaybeUninit::<T>::uninit();
    // SAFETY: `value` is valid for `size` bytes and `bytes` is exactly `size`
    // bytes; the regions cannot overlap (`value` is a fresh local).
    unsafe {
        core::ptr::copy_nonoverlapping(bytes.as_ptr(), value.as_mut_ptr().cast::<u8>(), size)
    };
    finish_decode(value)
}

/// Validates the fully initialized bytes of `value`, then materializes it.
fn finish_decode<T: WireDecode>(value: MaybeUninit<T>) -> Option<T> {
    // SAFETY: both callers have written all `size_of::<T>()` bytes of `value`.
    let snapshot = unsafe {
        core::slice::from_raw_parts(value.as_ptr().cast::<u8>(), core::mem::size_of::<T>())
    };
    if !T::wire_valid(snapshot) {
        return None;
    }
    // SAFETY: the bytes are initialized and `wire_valid` accepted them, which
    // is the `WireDecode` contract for them being a valid `T`.
    Some(unsafe { value.assume_init() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[repr(C)]
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    struct Wire {
        a: u32,
        a_hi: u32,
        b: u64,
    }

    // SAFETY: `repr(C)`, integer fields only, no padding.
    unsafe impl WireBytes for Wire {}
    // SAFETY: 4 + 4 + 8 bytes of fields in a 16-byte struct.
    unsafe impl NoPadding for Wire {}

    #[repr(u8)]
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    enum Mode {
        Off = 0,
        Fast = 1,
        Slow = 2,
    }

    #[repr(C)]
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    struct Command {
        mode: Mode,
        other: Mode,
        len: u16,
    }

    // SAFETY: 1 + 1 + 2 bytes of fields in a 4-byte struct.
    unsafe impl NoPadding for Command {}

    impl WireDecode for Command {
        fn wire_valid(bytes: &[u8]) -> bool {
            let ok = |offset: usize| bytes.get(offset).is_some_and(|raw| *raw <= Mode::Slow as u8);
            ok(core::mem::offset_of!(Command, mode)) && ok(core::mem::offset_of!(Command, other))
        }
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
        let value = Wire { a: 0xdead_beef, a_hi: 1, b: 0x0123_4567_89ab_cdef };

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
            let value = Wire { a: offset as u32, a_hi: 2, b: !(offset as u64) };
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
    fn volatile_write_round_trip_and_bounds() {
        #[repr(align(8))]
        struct Aligned([u8; 16]);
        let mut backing = Aligned([0; 16]);
        let mut frame = unsafe { FrameBytes::from_raw(backing.0.as_mut_ptr(), 16) };
        assert!(frame.write_volatile(0x1122_3344_5566_7788u64));
        assert_eq!(frame.read::<u64>(), Some(0x1122_3344_5566_7788));
        assert!(!frame.write_volatile([0u8; 17]));
        assert!(frame.read::<[u8; 17]>().is_none());
        let mut frame = unsafe { FrameBytes::from_raw(backing.0.as_mut_ptr().add(1), 15) };
        assert!(!frame.write_volatile(1u64));
    }

    #[test]
    fn decode_rejects_every_invalid_discriminant() {
        let mut buffer = [0u8; 4];
        for mode in 0..=255u8 {
            for other in [0u8, 2, mode] {
                buffer[0] = mode;
                buffer[1] = other;
                buffer[2] = 9;
                let frame = view(&mut buffer);
                let expected = mode <= 2 && other <= 2;
                assert_eq!(frame.decode::<Command>().is_some(), expected, "{mode} {other}");
                assert_eq!(decode_from_slice::<Command>(&buffer).is_some(), expected);
            }
        }
    }

    #[test]
    fn decode_round_trips_and_checks_bounds() {
        let value = Command { mode: Mode::Slow, other: Mode::Fast, len: 0x0a0b };
        let mut buffer = [0u8; 4];
        let mut frame = view(&mut buffer);
        assert!(frame.write(value));
        assert_eq!(frame.decode::<Command>(), Some(value));
        assert_eq!(decode_from_slice::<Command>(&buffer), Some(value));
        assert!(decode_from_slice::<Command>(&buffer[..3]).is_none());
        let mut short = [0u8; 3];
        assert!(view(&mut short).decode::<Command>().is_none());
        let _ = (Mode::Off, Mode::Fast);
    }

    #[test]
    fn decode_does_not_materialize_on_rejection() {
        let mut buffer = [3u8, 0, 0, 0];
        let frame = view(&mut buffer);
        assert!(frame.decode::<Command>().is_none());
    }

    #[test]
    fn value_bytes_round_trips_through_read_from_slice() {
        let wire = Wire { a: 7, a_hi: 0, b: 9 };
        let bytes = value_bytes(&wire);
        assert_eq!(bytes.len(), core::mem::size_of::<Wire>());
        assert_eq!(read_from_slice::<Wire>(bytes), Some(wire));
    }

    /// Writes `value` over a 0xAA-filled buffer and checks that no 0xAA byte
    /// survives inside the value and none is touched past it. Every field of the
    /// values used here is zero or avoids the 0xAA byte, so a surviving 0xAA
    /// can only be a padding byte the write failed to initialize.
    fn assert_no_stale_bytes<T: NoPadding>(value: T) {
        let size = core::mem::size_of::<T>();
        let mut buffer = std::vec![0xAAu8; size + 16];
        let mut frame = view(&mut buffer);
        assert!(frame.write(value));
        assert!(buffer[..size].iter().all(|byte| *byte != 0xAA), "padding leaked");
        assert!(buffer[size..].iter().all(|byte| *byte == 0xAA), "write overran");
        assert!(value_bytes(&value).iter().all(|byte| *byte != 0xAA));
    }

    #[test]
    fn padded_wire_types_write_no_uninitialized_bytes() {
        use logos_abi::*;
        let mut network = NetworkResponse::new(
            NetworkOperation::Status,
            NetworkResult::Ok,
            NetworkState::Ready,
            7,
        );
        network.payload[0] = 0x55;
        assert_no_stale_bytes(network);
        let mut descriptor = NetworkPacketDescriptor::new(NetworkPacketOperation::SubmitTx, 1, 9);
        descriptor.mac = [1, 2, 3, 4, 5, 6];
        assert_no_stale_bytes(descriptor);
        assert_no_stale_bytes(DirectoryResponse::empty(
            DirectoryOperation::Capabilities,
            DirectoryStatus::Ok,
            3,
        ));
        assert_no_stale_bytes(EventResponse::empty(EventStatus::Ok, 5));
        assert_no_stale_bytes(StorageResponse::new(1, StorageStatus::Ok, 2, 3, 4, 5));
        assert_no_stale_bytes(ManagerResponse::new(ManagerOperation::List, ManagerStatus::Ok, 6));
        assert_no_stale_bytes(ProgramBootstrapPage::empty());
        assert_no_stale_bytes(FramebufferConfig::new(4096, 64, 32, 64, FramebufferFormat::Bgr8));
    }

    #[test]
    fn volatile_write_of_aligned_wire_type_initializes_every_byte() {
        use logos_abi::*;
        #[repr(align(16))]
        struct Aligned([u8; 2112]);
        let mut backing = Aligned([0xAA; 2112]);
        let mut frame = unsafe { FrameBytes::from_raw(backing.0.as_mut_ptr(), 2112) };
        let response =
            DirectoryResponse::empty(DirectoryOperation::Capabilities, DirectoryStatus::Ok, 3);
        assert!(frame.write_volatile(response));
        let size = core::mem::size_of::<DirectoryResponse>();
        assert_eq!(size, 2080);
        assert!(backing.0[..size].iter().all(|byte| *byte != 0xAA));
        assert!(backing.0[size..].iter().all(|byte| *byte == 0xAA));
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
