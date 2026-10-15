//! Deterministic fuzz of the `wire_enums_valid` guards.
//!
//! Ring-3 peers hand the kernel and services raw bytes that are then read as
//! `#[repr(C)]` structs containing Rust enums. `wire_enums_valid` is the only
//! thing standing between an arbitrary byte and an invalid enum value (UB).
//! For every guarded type this feeds xorshift-generated buffers to the guard
//! and, when it accepts, performs the typed read. Under Miri an enum field
//! the guard forgot to check is reported as UB; natively the loop still
//! checks that the guard never panics on short or odd-sized slices.

use logos_abi::*;

struct XorShift(u64);

impl XorShift {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    /// Mostly small bytes so enum discriminants are plausible, sometimes any byte.
    fn byte(&mut self) -> u8 {
        let r = self.next();
        if r & 7 == 0 { (r >> 8) as u8 } else { ((r >> 8) % 9) as u8 }
    }
}

fn iterations() -> usize {
    if cfg!(miri) { 300 } else { 200_000 }
}

macro_rules! fuzz_guard {
    ($name:ident, $ty:ty) => {
        #[test]
        fn $name() {
            let size = core::mem::size_of::<$ty>();
            let mut rng = XorShift(0x9e37_79b9_7f4a_7c15 ^ size as u64);
            let mut buffer = vec![0u8; size];
            let mut accepted = 0usize;
            for _ in 0..iterations() {
                for byte in buffer.iter_mut() {
                    *byte = rng.byte();
                }
                // Short or truncated slices must be rejected or accepted, never panic.
                let _ = <$ty>::wire_enums_valid(&buffer[..size / 2]);
                let _ = <$ty>::wire_enums_valid(&[]);
                if <$ty>::wire_enums_valid(&buffer) {
                    accepted += 1;
                    // SAFETY: `buffer` holds `size_of::<$ty>()` initialised bytes; the
                    // guard under test is what must make every enum field valid.
                    let value: $ty =
                        unsafe { core::ptr::read_unaligned(buffer.as_ptr().cast::<$ty>()) };
                    core::hint::black_box(value);
                }
            }
            assert!(accepted > 0, "guard never accepted a buffer; test is vacuous");
        }
    };
}

fuzz_guard!(network_request, NetworkRequest);
fuzz_guard!(network_response, NetworkResponse);
fuzz_guard!(network_packet_descriptor, NetworkPacketDescriptor);
fuzz_guard!(ipc_bytes, IpcBytes);
fuzz_guard!(fetch_control, FetchControl);
fuzz_guard!(fetch_response, FetchResponse);
fuzz_guard!(flow_control, FlowControl);
fuzz_guard!(completion_response, CompletionResponse);
fuzz_guard!(package_response, PackageResponse);
fuzz_guard!(event_request, EventRequest);
fuzz_guard!(directory_request, DirectoryRequest);
fuzz_guard!(manager_request, ManagerRequest);
fuzz_guard!(device_request, DeviceRequest);
fuzz_guard!(user_request, UserRequest);
fuzz_guard!(user_response, UserResponse);
fuzz_guard!(user_storage_request, UserStorageRequest);
fuzz_guard!(user_storage_response, UserStorageResponse);
fuzz_guard!(atrium_settings_request, AtriumSettingsRequest);
fuzz_guard!(atrium_settings_response, AtriumSettingsResponse);
