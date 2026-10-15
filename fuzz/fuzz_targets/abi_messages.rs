#![no_main]

use libfuzzer_sys::fuzz_target;
use logos_abi::*;

fuzz_target!(|data: &[u8]| {
    let Some((&selector, rest)) = data.split_first() else { return };
    let kind = if selector & 1 == 0 {
        MessageKind::StorageRequest
    } else {
        MessageKind::StorageResponse
    };
    if let Some(mut message) = IpcBytes::from_bytes(kind, rest) {
        // Let the input also drive the (untrusted) flags and length fields.
        if selector & 2 != 0 {
            message.flags = selector >> 2;
        }
        if selector & 0x80 != 0 && rest.len() >= 2 {
            message.len = u16::from_le_bytes([rest[0], rest[1]]);
        }
        match kind {
            MessageKind::StorageRequest => {
                if let Ok(request) = StorageApiRequest::decode(&message) {
                    let again = StorageApiRequest::encode_versioned(
                        request.version,
                        request.operation,
                        request.flags,
                        request.request_id,
                        request.transaction_id,
                        request.offset,
                        request.path,
                        request.secondary_path,
                        request.data,
                    )
                    .expect("decoded request must re-encode");
                    assert_eq!(again.as_bytes(), message.as_bytes());
                }
            }
            _ => {
                let _ = StorageApiResponse::decode(&message);
            }
        }
    }

    // Raw wire guards: whenever one accepts the bytes, a typed read must be sound.
    macro_rules! guard {
        ($ty:ty) => {
            let size = core::mem::size_of::<$ty>();
            if rest.len() >= size && <$ty>::wire_enums_valid(&rest[..size]) {
                // SAFETY: `rest[..size]` is initialised; the guard under test
                // is what must make every enum field valid.
                let value: $ty = unsafe { core::ptr::read_unaligned(rest.as_ptr().cast()) };
                core::hint::black_box(value);
            }
        };
    }
    guard!(NetworkRequest);
    guard!(NetworkResponse);
    guard!(NetworkPacketDescriptor);
    guard!(FetchControl);
    guard!(FetchResponse);
    guard!(FlowControl);
    guard!(CompletionResponse);
    guard!(PackageResponse);
    guard!(EventRequest);
    guard!(DirectoryRequest);
    guard!(ManagerRequest);
    guard!(DeviceRequest);
    guard!(UserRequest);
    guard!(UserResponse);
    guard!(UserStorageRequest);
    guard!(UserStorageResponse);
    guard!(AtriumSettingsRequest);
    guard!(AtriumSettingsResponse);
});
