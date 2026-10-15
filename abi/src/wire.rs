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

/// Marker for types whose every byte is initialized by construction: `repr(C)`
/// (or primitive) layout with no implicit padding.
///
/// The Core copies values into user-visible frames and messages; a type with
/// padding would copy uninitialized kernel bytes (an information leak, and
/// undefined behaviour when viewed as `&[u8]`). `FrameBytes::write` and friends
/// therefore accept only `NoPadding` types.
///
/// # Safety
///
/// The type must have a defined layout in which `size_of::<Self>()` equals the
/// sum of its field sizes, and every field must itself be `NoPadding` (or an
/// integer/`repr(u8)` enum). Implement it only through `assert_no_padding!`,
/// which proves the size equation at compile time.
pub unsafe trait NoPadding: Copy {}

macro_rules! wire_bytes_primitives {
    ($($ty:ty),* $(,)?) => {
        // SAFETY: primitive integers accept every bit pattern and have no padding.
        $(unsafe impl WireBytes for $ty {} unsafe impl NoPadding for $ty {})*
    };
}

wire_bytes_primitives!(u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize);

// SAFETY: an array of values that accept every bit pattern accepts every bit
// pattern.
unsafe impl<T: WireBytes, const N: usize> WireBytes for [T; N] {}
// SAFETY: an array of padding-free elements has no padding between elements.
unsafe impl<T: NoPadding, const N: usize> NoPadding for [T; N] {}

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

/// Compile-time proof that a `repr(C)` wire type has no padding: its size must
/// equal the sum of its field sizes.
///
/// The Core copies written values into user-visible frames; padding bytes are
/// not guaranteed initialized, so a padded type would leak (or expose
/// uninitialized) bytes. The macro also implements [`NoPadding`] for the type.
macro_rules! assert_no_padding {
    ($ty:ty; $($field:ty),+ $(,)?) => {
        const _: () = assert!(
            core::mem::size_of::<$ty>() == 0 $(+ core::mem::size_of::<$field>())+,
            "wire type has padding"
        );
        // SAFETY: the assertion above proves the size equals the sum of the
        // field sizes, so there are no padding bytes.
        unsafe impl NoPadding for $ty {}
    };
}

assert_no_padding!(IpcBytes; crate::MessageKind, u8, u16, [u8; crate::MAX_IPC_BYTES]);
assert_no_padding!(
    ManagerRequest;
    u16, crate::ManagerOperation, crate::ManagerTargetKind, u32, crate::ServiceHandle, u64, u8, u8,
    [u8; 2], u32, [u8; crate::MAX_PACKAGE_NAME_BYTES]
);
assert_no_padding!(
    EventRequest;
    u16, crate::EventOperation, u8, u32, crate::EventSetHandle, crate::EventHandle, u64
);
assert_no_padding!(
    DirectoryRequest;
    u16, crate::DirectoryOperation, u8, u32, u64, crate::ServiceHandle
);

/// Padding proofs for every wire type the Core copies into a frame or message.
///
/// Each type has explicit `reserved*` fields where the `repr(C)` layout would
/// otherwise insert implicit padding, so the byte layout is unchanged while no
/// byte is uninitialized. Types with nested wire structs additionally require
/// the nested type to be `NoPadding` (the size sum alone cannot see padding
/// inside a nested struct).
mod padding_proofs {
    #![allow(clippy::wildcard_imports)]
    use super::NoPadding;
    use crate::*;

    const fn require<T: NoPadding>() {}

    assert_no_padding!(FramebufferConfig; u64, u32, u32, u32, FramebufferFormat, [u8; 3]);
    assert_no_padding!(StorageResponse; u32, StorageStatus, u8, u16, u16, u16, [u8; 4], u64, u64);
    assert_no_padding!(StorageMapRequest; u8, u8, u16, u32, u64, u16, u16, [u8; 4], u64, u64, u32, u32);
    assert_no_padding!(StorageMapResponse; u32, StorageStatus, [u8; 3], u64, u64, u8, [u8; 7], u32, [u8; 4]);
    assert_no_padding!(NetworkConfig; u16, NetworkProfile, u8, [u8; 4], [u8; 4], [u8; 4], u32, u32, u64);
    assert_no_padding!(NetworkPacketDescriptor; u16, NetworkPacketOperation, NetworkResult, u16, u16, u16, u16, [u8; 4], u64, u32, [u8; 6], [u8; 8], [u8; 6]);
    assert_no_padding!(NetworkResponse; u16, NetworkOperation, NetworkResult, NetworkState, [u8; 3], u32, u32, u16, u16, [u8; 4], u64, u16, [u8; 16], [u8; NETWORK_INLINE_PAYLOAD_BYTES], [u8; 6]);
    assert_no_padding!(PackageTarget; PackageTargetKind, u8, u8, u8, [u8; MAX_PACKAGE_NAME_BYTES]);
    assert_no_padding!(PackageRequest; PackageOperation, u8, PackageTarget, [u8; 2], u32, u16, [u8; 2], CapabilityHandle, u64, u32, u32, u16, u16, [u8; 4]);
    const _: () = require::<PackageTarget>();
    assert_no_padding!(PackageResponse; PackageOperation, PackageStatus, PackageTarget, [u8; 2], u32, u16, u16, u64, u32, u32, u16, u16, u32, u32, u32);
    const _: () = require::<PackageTarget>();
    assert_no_padding!(ServiceBootstrapPage; u16, u16, u16, u16, u64, ServiceHandle, CapabilityHandle, CapabilityHandle, CapabilityHandle, u64, u32, u32);
    assert_no_padding!(ProgramBootstrapPage; u16, u16, u16, u16, u32, [u8; 4], ServiceHandle, CapabilityHandle, CapabilityHandle, CapabilityHandle, CapabilityHandle, CapabilityHandle);
    assert_no_padding!(EventResponse; u16, EventStatus, u8, u32, EventHandle, EventSetHandle, [u8; 8]);
    assert_no_padding!(DirectoryRecord; DirectoryRecordKind, u8, u16, [u8; 4], u64, ServiceHandle, u16, u16, u16, [u8; 2], EventHandle, u8, [u8; 1], [u8; MAX_SERVICE_NAME_BYTES], [u8; 6]);
    assert_no_padding!(DirectoryResponse; u16, DirectoryOperation, DirectoryStatus, u32, u64, u8, u8, [u8; 2], [u8; 4], [DirectoryRecord; DIRECTORY_RECORDS_PER_PAGE], [u8; 8]);
    const _: () = require::<DirectoryRecord>();
    assert_no_padding!(DeviceRecord; u8, DeviceKind, DeviceState, u8, u32, u64, [u8; MAX_DEVICE_NAME_BYTES]);
    assert_no_padding!(DeviceResponse; u16, DeviceOperation, DeviceStatus, u32, u16, u16, [u8; 4], u64, u8, [u8; 7], [DeviceRecord; MAX_DEVICES]);
    const _: () = require::<DeviceRecord>();
    assert_no_padding!(ServiceManagerRecord; ServiceHandle, ManagerState, u8, u8, u8, u16, [u8; 2], u8, [u8; 3], u32, [u8; MAX_PACKAGE_NAME_BYTES]);
    assert_no_padding!(ManagerResponse; u16, ManagerOperation, ManagerStatus, u32, u64, ServiceManagerRecord);
    const _: () = require::<ServiceManagerRecord>();
}

/// Making the padding explicit must not move any field: size and offsets are the
/// layout shipped before the padding became explicit.
mod layout_proofs {
    #![allow(clippy::wildcard_imports)]
    use crate::*;
    use core::mem::{offset_of, size_of};

    const _: () = {
        assert!(size_of::<FramebufferConfig>() == 24);
        assert!(offset_of!(FramebufferConfig, bytes) == 0);
        assert!(offset_of!(FramebufferConfig, width) == 8);
        assert!(offset_of!(FramebufferConfig, height) == 12);
        assert!(offset_of!(FramebufferConfig, stride) == 16);
        assert!(offset_of!(FramebufferConfig, format) == 20);
    };
    const _: () = {
        assert!(size_of::<StorageResponse>() == 32);
        assert!(offset_of!(StorageResponse, request_id) == 0);
        assert!(offset_of!(StorageResponse, status) == 4);
        assert!(offset_of!(StorageResponse, reserved) == 5);
        assert!(offset_of!(StorageResponse, generation) == 6);
        assert!(offset_of!(StorageResponse, blocks_completed) == 8);
        assert!(offset_of!(StorageResponse, payload_bytes) == 10);
        assert!(offset_of!(StorageResponse, transaction_id) == 16);
        assert!(offset_of!(StorageResponse, block_count) == 24);
    };
    const _: () = {
        assert!(size_of::<StorageMapRequest>() == 48);
        assert!(offset_of!(StorageMapRequest, operation) == 0);
        assert!(offset_of!(StorageMapRequest, flags) == 1);
        assert!(offset_of!(StorageMapRequest, reserved) == 2);
        assert!(offset_of!(StorageMapRequest, request_id) == 4);
        assert!(offset_of!(StorageMapRequest, generation) == 8);
        assert!(offset_of!(StorageMapRequest, client) == 16);
        assert!(offset_of!(StorageMapRequest, pages) == 18);
        assert!(offset_of!(StorageMapRequest, source_page) == 24);
        assert!(offset_of!(StorageMapRequest, target_page) == 32);
        assert!(offset_of!(StorageMapRequest, window_generation) == 40);
        assert!(offset_of!(StorageMapRequest, reserved_tail) == 44);
    };
    const _: () = {
        assert!(size_of::<StorageMapResponse>() == 40);
        assert!(offset_of!(StorageMapResponse, request_id) == 0);
        assert!(offset_of!(StorageMapResponse, status) == 4);
        assert!(offset_of!(StorageMapResponse, reserved) == 5);
        assert!(offset_of!(StorageMapResponse, generation) == 8);
        assert!(offset_of!(StorageMapResponse, target_page) == 16);
        assert!(offset_of!(StorageMapResponse, pages) == 24);
        assert!(offset_of!(StorageMapResponse, reserved_tail) == 25);
        assert!(offset_of!(StorageMapResponse, window_generation) == 32);
        assert!(offset_of!(StorageMapResponse, reserved_end) == 36);
    };
    const _: () = {
        assert!(size_of::<NetworkConfig>() == 32);
        assert!(offset_of!(NetworkConfig, abi_version) == 0);
        assert!(offset_of!(NetworkConfig, profile) == 2);
        assert!(offset_of!(NetworkConfig, reserved) == 3);
        assert!(offset_of!(NetworkConfig, address) == 4);
        assert!(offset_of!(NetworkConfig, netmask) == 8);
        assert!(offset_of!(NetworkConfig, gateway) == 12);
        assert!(offset_of!(NetworkConfig, gateway_deadline_ticks) == 16);
        assert!(offset_of!(NetworkConfig, dhcp_deadline_ticks) == 20);
        assert!(offset_of!(NetworkConfig, service_epoch) == 24);
    };
    const _: () = {
        assert!(size_of::<NetworkPacketDescriptor>() == 48);
        assert!(offset_of!(NetworkPacketDescriptor, abi_version) == 0);
        assert!(offset_of!(NetworkPacketDescriptor, operation) == 2);
        assert!(offset_of!(NetworkPacketDescriptor, result) == 3);
        assert!(offset_of!(NetworkPacketDescriptor, page) == 4);
        assert!(offset_of!(NetworkPacketDescriptor, length) == 6);
        assert!(offset_of!(NetworkPacketDescriptor, generation) == 8);
        assert!(offset_of!(NetworkPacketDescriptor, reserved) == 10);
        assert!(offset_of!(NetworkPacketDescriptor, service_epoch) == 16);
        assert!(offset_of!(NetworkPacketDescriptor, sequence) == 24);
        assert!(offset_of!(NetworkPacketDescriptor, mac) == 28);
        assert!(offset_of!(NetworkPacketDescriptor, reserved_tail) == 34);
    };
    const _: () = {
        assert!(size_of::<NetworkResponse>() == 248);
        assert!(offset_of!(NetworkResponse, abi_version) == 0);
        assert!(offset_of!(NetworkResponse, operation) == 2);
        assert!(offset_of!(NetworkResponse, result) == 3);
        assert!(offset_of!(NetworkResponse, state) == 4);
        assert!(offset_of!(NetworkResponse, request_id) == 8);
        assert!(offset_of!(NetworkResponse, handle) == 12);
        assert!(offset_of!(NetworkResponse, generation) == 16);
        assert!(offset_of!(NetworkResponse, reserved) == 18);
        assert!(offset_of!(NetworkResponse, service_epoch) == 24);
        assert!(offset_of!(NetworkResponse, payload_len) == 32);
        assert!(offset_of!(NetworkResponse, detail) == 34);
        assert!(offset_of!(NetworkResponse, payload) == 50);
    };
    const _: () = {
        assert!(size_of::<PackageTarget>() == 36);
        assert!(offset_of!(PackageTarget, kind) == 0);
        assert!(offset_of!(PackageTarget, service) == 1);
        assert!(offset_of!(PackageTarget, name_len) == 2);
        assert!(offset_of!(PackageTarget, reserved) == 3);
        assert!(offset_of!(PackageTarget, name) == 4);
    };
    const _: () = {
        assert!(size_of::<PackageRequest>() == 80);
        assert!(offset_of!(PackageRequest, operation) == 0);
        assert!(offset_of!(PackageRequest, flags) == 1);
        assert!(offset_of!(PackageRequest, target) == 2);
        assert!(offset_of!(PackageRequest, request_id) == 40);
        assert!(offset_of!(PackageRequest, generation) == 44);
        assert!(offset_of!(PackageRequest, capability) == 48);
        assert!(offset_of!(PackageRequest, service_epoch) == 56);
        assert!(offset_of!(PackageRequest, package_generation) == 64);
        assert!(offset_of!(PackageRequest, offset) == 68);
        assert!(offset_of!(PackageRequest, length) == 72);
        assert!(offset_of!(PackageRequest, reserved2) == 74);
    };
    const _: () = {
        assert!(size_of::<PackageResponse>() == 80);
        assert!(offset_of!(PackageResponse, operation) == 0);
        assert!(offset_of!(PackageResponse, status) == 1);
        assert!(offset_of!(PackageResponse, target) == 2);
        assert!(offset_of!(PackageResponse, request_id) == 40);
        assert!(offset_of!(PackageResponse, generation) == 44);
        assert!(offset_of!(PackageResponse, reserved2) == 46);
        assert!(offset_of!(PackageResponse, service_epoch) == 48);
        assert!(offset_of!(PackageResponse, package_generation) == 56);
        assert!(offset_of!(PackageResponse, offset) == 60);
        assert!(offset_of!(PackageResponse, bytes) == 64);
        assert!(offset_of!(PackageResponse, reserved3) == 66);
        assert!(offset_of!(PackageResponse, package_bytes) == 68);
        assert!(offset_of!(PackageResponse, package_version) == 72);
        assert!(offset_of!(PackageResponse, payload_crc32c) == 76);
    };
    const _: () = {
        assert!(size_of::<ServiceBootstrapPage>() == 64);
        assert!(offset_of!(ServiceBootstrapPage, abi_version) == 0);
        assert!(offset_of!(ServiceBootstrapPage, flags) == 2);
        assert!(offset_of!(ServiceBootstrapPage, ipc_generation) == 4);
        assert!(offset_of!(ServiceBootstrapPage, reserved) == 6);
        assert!(offset_of!(ServiceBootstrapPage, service_epoch) == 8);
        assert!(offset_of!(ServiceBootstrapPage, service) == 16);
        assert!(offset_of!(ServiceBootstrapPage, control) == 24);
        assert!(offset_of!(ServiceBootstrapPage, directory) == 32);
        assert!(offset_of!(ServiceBootstrapPage, heap) == 40);
        assert!(offset_of!(ServiceBootstrapPage, heap_base) == 48);
        assert!(offset_of!(ServiceBootstrapPage, heap_pages) == 56);
        assert!(offset_of!(ServiceBootstrapPage, heap_quota_pages) == 60);
    };
    const _: () = {
        assert!(size_of::<ProgramBootstrapPage>() == 64);
        assert!(offset_of!(ProgramBootstrapPage, abi_version) == 0);
        assert!(offset_of!(ProgramBootstrapPage, flags) == 2);
        assert!(offset_of!(ProgramBootstrapPage, ipc_generation) == 4);
        assert!(offset_of!(ProgramBootstrapPage, reserved) == 6);
        assert!(offset_of!(ProgramBootstrapPage, program_generation) == 8);
        assert!(offset_of!(ProgramBootstrapPage, client) == 16);
        assert!(offset_of!(ProgramBootstrapPage, surface_request) == 24);
        assert!(offset_of!(ProgramBootstrapPage, surface_response) == 32);
        assert!(offset_of!(ProgramBootstrapPage, surface_input) == 40);
        assert!(offset_of!(ProgramBootstrapPage, surface_render) == 48);
        assert!(offset_of!(ProgramBootstrapPage, surface_draw) == 56);
    };
    const _: () = {
        assert!(size_of::<EventResponse>() == 32);
        assert!(offset_of!(EventResponse, abi_version) == 0);
        assert!(offset_of!(EventResponse, status) == 2);
        assert!(offset_of!(EventResponse, reserved) == 3);
        assert!(offset_of!(EventResponse, request_id) == 4);
        assert!(offset_of!(EventResponse, event) == 8);
        assert!(offset_of!(EventResponse, event_set) == 16);
    };
    const _: () = {
        assert!(size_of::<DirectoryRecord>() == 64);
        assert!(offset_of!(DirectoryRecord, kind) == 0);
        assert!(offset_of!(DirectoryRecord, rights) == 1);
        assert!(offset_of!(DirectoryRecord, flags) == 2);
        assert!(offset_of!(DirectoryRecord, handle) == 8);
        assert!(offset_of!(DirectoryRecord, peer) == 16);
        assert!(offset_of!(DirectoryRecord, contract_id) == 24);
        assert!(offset_of!(DirectoryRecord, message_bytes) == 26);
        assert!(offset_of!(DirectoryRecord, queue_capacity) == 28);
        assert!(offset_of!(DirectoryRecord, event) == 32);
        assert!(offset_of!(DirectoryRecord, name_len) == 40);
        assert!(offset_of!(DirectoryRecord, reserved) == 41);
        assert!(offset_of!(DirectoryRecord, name) == 42);
    };
    const _: () = {
        assert!(size_of::<DirectoryResponse>() == 2080);
        assert!(offset_of!(DirectoryResponse, abi_version) == 0);
        assert!(offset_of!(DirectoryResponse, operation) == 2);
        assert!(offset_of!(DirectoryResponse, status) == 3);
        assert!(offset_of!(DirectoryResponse, request_id) == 4);
        assert!(offset_of!(DirectoryResponse, cursor) == 8);
        assert!(offset_of!(DirectoryResponse, count) == 16);
        assert!(offset_of!(DirectoryResponse, flags) == 17);
        assert!(offset_of!(DirectoryResponse, reserved) == 18);
        assert!(offset_of!(DirectoryResponse, records) == 24);
    };
    const _: () = {
        assert!(size_of::<DeviceRecord>() == 32);
        assert!(offset_of!(DeviceRecord, id) == 0);
        assert!(offset_of!(DeviceRecord, kind) == 1);
        assert!(offset_of!(DeviceRecord, state) == 2);
        assert!(offset_of!(DeviceRecord, flags) == 3);
        assert!(offset_of!(DeviceRecord, block_size) == 4);
        assert!(offset_of!(DeviceRecord, block_count) == 8);
        assert!(offset_of!(DeviceRecord, name) == 16);
    };
    const _: () = {
        assert!(size_of::<DeviceResponse>() == 288);
        assert!(offset_of!(DeviceResponse, abi_version) == 0);
        assert!(offset_of!(DeviceResponse, operation) == 2);
        assert!(offset_of!(DeviceResponse, status) == 3);
        assert!(offset_of!(DeviceResponse, request_id) == 4);
        assert!(offset_of!(DeviceResponse, generation) == 8);
        assert!(offset_of!(DeviceResponse, reserved) == 10);
        assert!(offset_of!(DeviceResponse, service_epoch) == 16);
        assert!(offset_of!(DeviceResponse, count) == 24);
        assert!(offset_of!(DeviceResponse, reserved_tail) == 25);
        assert!(offset_of!(DeviceResponse, records) == 32);
    };
    const _: () = {
        assert!(size_of::<ServiceManagerRecord>() == 56);
        assert!(offset_of!(ServiceManagerRecord, service) == 0);
        assert!(offset_of!(ServiceManagerRecord, state) == 8);
        assert!(offset_of!(ServiceManagerRecord, restarts) == 9);
        assert!(offset_of!(ServiceManagerRecord, name_len) == 10);
        assert!(offset_of!(ServiceManagerRecord, dependencies) == 12);
        assert!(offset_of!(ServiceManagerRecord, reserved) == 14);
        assert!(offset_of!(ServiceManagerRecord, program_slot) == 16);
        assert!(offset_of!(ServiceManagerRecord, reserved_program) == 17);
        assert!(offset_of!(ServiceManagerRecord, program_generation) == 20);
        assert!(offset_of!(ServiceManagerRecord, name) == 24);
    };
    const _: () = {
        assert!(size_of::<ManagerResponse>() == 72);
        assert!(offset_of!(ManagerResponse, abi_version) == 0);
        assert!(offset_of!(ManagerResponse, operation) == 2);
        assert!(offset_of!(ManagerResponse, status) == 3);
        assert!(offset_of!(ManagerResponse, request_id) == 4);
        assert!(offset_of!(ManagerResponse, cursor) == 8);
        assert!(offset_of!(ManagerResponse, record) == 16);
    };
}

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
            reserved_align: [0; 4],
            source_page: 3,
            target_page: 4,
            window_generation: 5,
            reserved_tail: 0,
        });
    }
}
