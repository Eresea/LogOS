//! Marker traits for wire values that cross a trust boundary as raw bytes.
//!
//! IPC frames are written by user-space services, so the Core reads them as
//! untrusted bytes. Materializing a Rust value from arbitrary bytes is only
//! sound when every byte pattern is a valid value of the type. Two traits
//! split the wire types accordingly:
//!
//! - [`WireBytes`]: every bit pattern is valid; the bytes may be reinterpreted
//!   as the type with no further checking.
//! - [`WireDecode`]: the type contains `repr(u8)` enums (or other fields with
//!   invalid bit patterns); the raw bytes must pass [`WireDecode::wire_valid`]
//!   before they may be reinterpreted.
//!
//! The Core's frame reader only accepts types through one of these traits, so
//! "forgot to validate the enum discriminants" cannot be written in safe code.

use crate::{
    AtriumSettingsRequest, AtriumSettingsResponse, CompletionResponse, DeviceRequest,
    DirectoryRequest, EventRequest, EventResponse, FetchControl, FetchResponse, FlowControl,
    IpcBytes, ManagerRequest, ManagerResponse, NetworkPacketDescriptor, NetworkRequest,
    NetworkResponse, PackageResponse, StorageRequest, UserRequest, UserResponse,
    UserStorageRequest, UserStorageResponse,
};

/// Marker for types for which **every** bit pattern of `size_of::<Self>()` bytes
/// is a valid value.
///
/// # Safety
///
/// Implementors must guarantee all of the following:
///
/// - The type has a defined layout (`repr(C)`, `repr(transparent)`, a primitive
///   integer or an array of such) and contains no padding-dependent invariant.
/// - Every field is itself `WireBytes`: no `bool`, `char`, references, pointers
///   with provenance, `NonZero*`, atomics, or enums (those must use
///   [`WireDecode`] instead).
/// - Semantic constraints (reserved fields being zero, lengths in range) are
///   *not* part of validity: they are checked by the consumer, never assumed by
///   the type.
pub unsafe trait WireBytes: Copy {}

/// Types that contain fields with invalid bit patterns (typically `repr(u8)`
/// enums) and so can only be materialized from bytes that pass a validator.
pub trait WireDecode: Copy {
    /// `true` if the leading `size_of::<Self>()` bytes of `bytes` hold a valid
    /// `Self`: every enum field, at its `offset_of!` position, holds a declared
    /// discriminant.
    ///
    /// A slice shorter than `size_of::<Self>()` may be rejected or may be
    /// checked only as far as it extends; callers must separately ensure the
    /// length before reading (the Core's `FrameBytes::decode` does so).
    /// Semantic validation (`is_valid`, reserved fields) is separate and runs
    /// on the decoded value.
    fn wire_valid(bytes: &[u8]) -> bool;
}

macro_rules! wire_bytes_primitives {
    ($($ty:ty),* $(,)?) => {
        // SAFETY: primitive integers accept every bit pattern.
        $(unsafe impl WireBytes for $ty {})*
    };
}

wire_bytes_primitives!(u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize);

// SAFETY: an array of values that accept every bit pattern accepts every bit
// pattern.
unsafe impl<T: WireBytes, const N: usize> WireBytes for [T; N] {}

macro_rules! wire_decode_via_enums {
    ($($ty:ty),* $(,)?) => {
        $(impl WireDecode for $ty {
            fn wire_valid(bytes: &[u8]) -> bool {
                <$ty>::wire_enums_valid(bytes)
            }
        })*
    };
}

wire_decode_via_enums!(
    AtriumSettingsRequest,
    AtriumSettingsResponse,
    CompletionResponse,
    DeviceRequest,
    DirectoryRequest,
    EventRequest,
    EventResponse,
    FetchControl,
    FetchResponse,
    FlowControl,
    IpcBytes,
    ManagerRequest,
    ManagerResponse,
    NetworkPacketDescriptor,
    NetworkRequest,
    NetworkResponse,
    PackageResponse,
    StorageRequest,
    UserRequest,
    UserResponse,
    UserStorageRequest,
    UserStorageResponse,
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        DirectoryOperation, EventOperation, EventStatus, ManagerOperation, ManagerState,
        ManagerStatus, MessageKind, NetworkOperation, NetworkResult, NetworkState,
        PackageOperation, PackageStatus, PackageTargetKind, StorageOperation,
    };
    use core::mem::{offset_of, size_of};

    type Valid = fn(u8) -> bool;

    /// Sweeps all 256 values of each enum byte (one at a time, with the others
    /// held valid) and checks `wire_valid` against the enum's own decoder.
    fn sweep<T: WireDecode>(fields: &[(usize, Valid)]) {
        let mut buffer = [0u8; 4096];
        let bytes = &mut buffer[..size_of::<T>()];
        for (offset, valid) in fields {
            let first = (0..=255u8).find(|v| valid(*v)).expect("enum has a valid value");
            bytes[*offset] = first;
        }
        assert!(T::wire_valid(bytes), "baseline must be valid");
        for (offset, valid) in fields {
            let saved = bytes[*offset];
            for raw in 0..=255u8 {
                bytes[*offset] = raw;
                assert_eq!(
                    T::wire_valid(bytes),
                    valid(raw),
                    "{} offset {offset} raw {raw}",
                    core::any::type_name::<T>()
                );
            }
            bytes[*offset] = saved;
        }
        // A truncated buffer must never validate.
        let cut = fields.iter().map(|(offset, _)| *offset).max().unwrap();
        assert!(!T::wire_valid(&bytes[..cut]));
    }

    #[test]
    fn package_response_rejects_every_invalid_discriminant() {
        sweep::<PackageResponse>(&[
            (offset_of!(PackageResponse, operation), |v| PackageOperation::from_raw(v).is_some()),
            (offset_of!(PackageResponse, status), |v| PackageStatus::from_raw(v).is_some()),
            (offset_of!(PackageResponse, target) + offset_of!(crate::PackageTarget, kind), |v| {
                PackageTargetKind::from_raw(v).is_some()
            }),
        ]);
    }

    #[test]
    fn network_response_rejects_every_invalid_discriminant() {
        sweep::<NetworkResponse>(&[
            (offset_of!(NetworkResponse, operation), |v| NetworkOperation::from_raw(v).is_some()),
            (offset_of!(NetworkResponse, result), |v| v <= NetworkResult::Cancelled as u8),
            (offset_of!(NetworkResponse, state), |v| v <= NetworkState::Faulted as u8),
        ]);
    }

    #[test]
    fn storage_request_rejects_every_invalid_discriminant() {
        sweep::<StorageRequest>(&[(offset_of!(StorageRequest, operation), |v| {
            (StorageOperation::Read as u8..=StorageOperation::AbortTransaction as u8).contains(&v)
        })]);
    }

    #[test]
    fn manager_response_rejects_every_invalid_discriminant() {
        sweep::<ManagerResponse>(&[
            (offset_of!(ManagerResponse, operation), |v| ManagerOperation::from_raw(v).is_some()),
            (offset_of!(ManagerResponse, status), |v| v <= ManagerStatus::Unsupported as u8),
            (offset_of!(ManagerResponse, record.state), |v| v <= ManagerState::Faulted as u8),
        ]);
    }

    #[test]
    fn event_and_directory_wire_types_reject_invalid_discriminants() {
        sweep::<EventResponse>(&[(offset_of!(EventResponse, status), |v| {
            EventStatus::from_raw(usize::from(v)).is_some()
        })]);
        sweep::<EventRequest>(&[(offset_of!(EventRequest, operation), |v| {
            EventOperation::from_raw(v).is_some()
        })]);
        sweep::<DirectoryRequest>(&[(offset_of!(DirectoryRequest, operation), |v| {
            DirectoryOperation::from_raw(v).is_some()
        })]);
        sweep::<IpcBytes>(&[(offset_of!(IpcBytes, kind), MessageKind::raw_is_valid)]);
    }

    #[test]
    fn wire_bytes_round_trip_through_raw_bytes() {
        fn round_trip<T: WireBytes + PartialEq + core::fmt::Debug>(value: T) {
            let mut buffer = [0u8; 512];
            let raw = &mut buffer[..size_of::<T>()];
            // SAFETY: `raw` has exactly `size_of::<T>()` bytes; `write_unaligned`
            // has no alignment requirement.
            unsafe { core::ptr::write_unaligned(raw.as_mut_ptr().cast::<T>(), value) };
            // SAFETY: `T: WireBytes` accepts any bit pattern and `raw` is long enough.
            let back = unsafe { core::ptr::read_unaligned(raw.as_ptr().cast::<T>()) };
            assert_eq!(back, value);
        }
        round_trip(0xa5u8);
        round_trip(0xbeefu16);
        round_trip(0xdead_beefu32);
        round_trip(u64::MAX - 3);
        round_trip(-5i32);
        round_trip([1u16, 2, 3, 4]);
        round_trip([[7u8; 3]; 2]);
        round_trip(crate::EventHandle::new(3, 9).unwrap());
        round_trip(crate::StorageMapRequest {
            operation: 1,
            flags: 2,
            reserved: 0,
            request_id: 7,
            generation: 8,
            client: 1,
            pages: 2,
            source_page: 3,
            target_page: 4,
            window_generation: 5,
            reserved_tail: 0,
        });
    }
}
