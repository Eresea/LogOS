//! Property tests for the ABI message decoders: arbitrary bytes never panic,
//! valid encodings round-trip, and mutated encodings decode to something
//! consistent or fail cleanly. Skipped under Miri (proptest is too slow there;
//! `tests/wire_guards.rs` is the Miri-friendly counterpart).
#![cfg(not(miri))]

use logos_abi::*;
use proptest::prelude::*;

const OPERATIONS: [StorageApiOperation; 25] = [
    StorageApiOperation::Begin,
    StorageApiOperation::Commit,
    StorageApiOperation::Abort,
    StorageApiOperation::List,
    StorageApiOperation::CreateFile,
    StorageApiOperation::Read,
    StorageApiOperation::Write,
    StorageApiOperation::Remove,
    StorageApiOperation::Rename,
    StorageApiOperation::StageWriteBegin,
    StorageApiOperation::StageWriteChunk,
    StorageApiOperation::StageWriteCommit,
    StorageApiOperation::StageWriteAbort,
    StorageApiOperation::PackageList,
    StorageApiOperation::PackageInfo,
    StorageApiOperation::PackageInstall,
    StorageApiOperation::Open,
    StorageApiOperation::Close,
    StorageApiOperation::Stat,
    StorageApiOperation::Mkdir,
    StorageApiOperation::HandleRead,
    StorageApiOperation::HandleWrite,
    StorageApiOperation::Fsync,
    StorageApiOperation::MapRead,
    StorageApiOperation::UnmapRead,
];

fn ipc_bytes(kind: MessageKind) -> impl Strategy<Value = IpcBytes> {
    (
        any::<u8>(),
        0u16..=(MAX_IPC_BYTES as u16 + 8),
        proptest::collection::vec(any::<u8>(), MAX_IPC_BYTES),
    )
        .prop_map(move |(flags, len, bytes)| {
            let mut message = IpcBytes::empty(kind);
            message.flags = flags;
            message.len = len;
            message.bytes.copy_from_slice(&bytes);
            message
        })
}

fn small_bytes(max: usize) -> impl Strategy<Value = Vec<u8>> {
    proptest::collection::vec(any::<u8>(), 0..=max)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1024))]

    #[test]
    fn storage_request_decode_never_panics(message in ipc_bytes(MessageKind::StorageRequest)) {
        let _ = StorageApiRequest::decode(&message);
    }

    #[test]
    fn storage_response_decode_never_panics(message in ipc_bytes(MessageKind::StorageResponse)) {
        let _ = StorageApiResponse::decode(&message);
    }

    /// Bytes shaped like a header (valid version, operation, request id) so the
    /// length and shape checks behind them are actually reached.
    #[test]
    fn storage_request_decode_with_plausible_header(
        version in prop_oneof![Just(STORAGE_API_VERSION), Just(STORAGE_API_EXTENSION_VERSION)],
        operation in 1u8..=25,
        header_tail in proptest::collection::vec(any::<u8>(), 24),
        payload in small_bytes(64),
        request_id in 1u32..,
    ) {
        let mut bytes = vec![version, operation];
        bytes.extend_from_slice(&header_tail);
        bytes[4..8].copy_from_slice(&request_id.to_le_bytes());
        bytes.extend_from_slice(&payload);
        let message = IpcBytes::from_bytes(MessageKind::StorageRequest, &bytes).unwrap();
        if let Ok(request) = StorageApiRequest::decode(&message) {
            prop_assert!(
                request.path.len() + request.secondary_path.len() + request.data.len()
                    <= payload.len()
            );
        }
    }

    #[test]
    fn storage_request_round_trips(
        version in prop_oneof![Just(STORAGE_API_VERSION), Just(STORAGE_API_EXTENSION_VERSION)],
        operation in 0usize..OPERATIONS.len(),
        flags in any::<u8>(),
        request_id in any::<u32>(),
        transaction_id in any::<u64>(),
        offset in any::<u32>(),
        path in small_bytes(80),
        secondary in small_bytes(80),
        data in small_bytes(80),
    ) {
        let operation = OPERATIONS[operation];
        let encoded = StorageApiRequest::encode_versioned(
            version, operation, flags, request_id, transaction_id, offset, &path, &secondary, &data,
        );
        if let Some(message) = encoded {
            let decoded = StorageApiRequest::decode(&message).expect("encoded request must decode");
            prop_assert_eq!(decoded.version, version);
            prop_assert_eq!(decoded.operation, operation);
            prop_assert_eq!(decoded.flags, flags);
            prop_assert_eq!(decoded.request_id, request_id);
            prop_assert_eq!(decoded.transaction_id, transaction_id);
            prop_assert_eq!(decoded.offset, offset);
            prop_assert_eq!(decoded.path, &path[..]);
            prop_assert_eq!(decoded.secondary_path, &secondary[..]);
            prop_assert_eq!(decoded.data, &data[..]);
        }
    }

    #[test]
    fn storage_request_mutation_is_canonical_or_rejected(
        operation in 0usize..OPERATIONS.len(),
        path in small_bytes(40),
        secondary in small_bytes(40),
        data in small_bytes(40),
        flips in proptest::collection::vec((0usize..120, any::<u8>()), 1..4),
    ) {
        let operation = OPERATIONS[operation];
        let Some(mut message) =
            StorageApiRequest::encode(operation, 0, 1, 2, 3, &path, &secondary, &data)
        else {
            return Ok(());
        };
        for (index, value) in flips {
            message.bytes[index % MAX_IPC_BYTES] = value;
        }
        if let Ok(decoded) = StorageApiRequest::decode(&message) {
            // Whatever still decodes must re-encode to exactly the same message.
            let again = StorageApiRequest::encode_versioned(
                decoded.version,
                decoded.operation,
                decoded.flags,
                decoded.request_id,
                decoded.transaction_id,
                decoded.offset,
                decoded.path,
                decoded.secondary_path,
                decoded.data,
            )
            .expect("decoded request must re-encode");
            prop_assert_eq!(again.as_bytes(), message.as_bytes());
        }
    }

    #[test]
    fn storage_response_round_trips(
        version in prop_oneof![Just(STORAGE_API_VERSION), Just(STORAGE_API_EXTENSION_VERSION)],
        status in 0u8..20,
        request_id in any::<u32>(),
        transaction_id in any::<u64>(),
        data in small_bytes(STORAGE_API_RESPONSE_DATA_BYTES + 8),
        more in any::<bool>(),
    ) {
        let status = StorageApiStatus::from_raw(status).unwrap();
        let encoded = StorageApiResponse::encode_versioned(
            version, status, request_id, transaction_id, &data, more,
        );
        if let Some(message) = encoded {
            let decoded =
                StorageApiResponse::decode(&message).expect("encoded response must decode");
            prop_assert_eq!(decoded.version, version);
            prop_assert_eq!(decoded.status, status);
            prop_assert_eq!(decoded.request_id, request_id);
            prop_assert_eq!(decoded.transaction_id, transaction_id);
            prop_assert_eq!(decoded.data, &data[..]);
            prop_assert_eq!(decoded.more, more);
        } else {
            prop_assert!(request_id == 0 || data.len() > STORAGE_API_RESPONSE_DATA_BYTES);
        }
    }

    #[test]
    fn storage_response_mutation_is_consistent_or_rejected(
        data in small_bytes(40),
        flips in proptest::collection::vec((0usize..60, any::<u8>()), 1..4),
        flags in any::<u8>(),
    ) {
        let mut message =
            StorageApiResponse::encode(StorageApiStatus::Ok, 5, 6, &data, false).unwrap();
        for (index, value) in flips {
            message.bytes[index % MAX_IPC_BYTES] = value;
        }
        message.flags = flags;
        if let Ok(decoded) = StorageApiResponse::decode(&message) {
            let again = StorageApiResponse::encode_versioned(
                decoded.version,
                decoded.status,
                decoded.request_id,
                decoded.transaction_id,
                decoded.data,
                decoded.more,
            )
            .expect("decoded response must re-encode");
            let redecoded = StorageApiResponse::decode(&again).unwrap();
            prop_assert_eq!(redecoded.data, decoded.data);
            prop_assert_eq!(redecoded.status, decoded.status);
            prop_assert_eq!(redecoded.request_id, decoded.request_id);
        }
    }

    #[test]
    fn ipc_bytes_from_bytes_round_trips(bytes in small_bytes(MAX_IPC_BYTES + 8)) {
        match IpcBytes::from_bytes(MessageKind::StorageRequest, &bytes) {
            Some(message) => prop_assert_eq!(message.as_bytes(), Some(&bytes[..])),
            None => prop_assert!(bytes.len() > MAX_IPC_BYTES),
        }
    }

    #[test]
    fn fetch_request_round_trips_and_arbitrary_is_clean(
        id in any::<u32>(),
        url in small_bytes(260),
        dest in small_bytes(260),
        raw in proptest::collection::vec(any::<u8>(), FETCH_REQUEST_DATA_BYTES),
        url_len in any::<u16>(),
        dest_len in any::<u16>(),
        reserved in 0u16..2,
    ) {
        if let Some(request) = FetchRequest::new(id, &url, &dest) {
            prop_assert!(request.is_valid());
            prop_assert_eq!(request.url(), Some(&url[..]));
            prop_assert_eq!(request.destination(), Some(&dest[..]));
        }
        let mut arbitrary = FetchRequest::new(1, b"u", b"").unwrap();
        arbitrary.data.copy_from_slice(&raw);
        arbitrary.url_len = url_len;
        arbitrary.destination_len = dest_len;
        arbitrary.reserved = reserved;
        let _ = arbitrary.url();
        let _ = arbitrary.destination();
    }

    #[test]
    fn package_target_validate_agrees_with_constructors(
        kind in 1u8..=3,
        service in prop_oneof![0u8..16, any::<u8>()],
        name_len in prop_oneof![Just(0u8), 0u8..=40],
        reserved in 0u8..2,
        name in proptest::collection::vec(any::<u8>(), MAX_PACKAGE_NAME_BYTES),
        printable in prop_oneof![Just(String::new()), "[a-z0-9-]{0,32}"],
        use_printable in any::<bool>(),
    ) {
        let kind = PackageTargetKind::from_raw(kind).unwrap();
        let mut target = PackageTarget {
            kind,
            service,
            name_len,
            reserved,
            name: [0; MAX_PACKAGE_NAME_BYTES],
        };
        if use_printable {
            let bytes = printable.as_bytes();
            target.name[..bytes.len()].copy_from_slice(bytes);
            if name_len as usize > bytes.len() {
                target.name_len = bytes.len() as u8;
            }
        } else {
            target.name.copy_from_slice(&name);
        }
        if target.validate().is_ok() {
            let used = &target.name[..target.name_len as usize];
            match target.kind {
                PackageTargetKind::Service => {
                    // Wire service ids are 1-based (`ServiceId as u8`); 0 is not a service.
                    prop_assert!(target.service != 0);
                    let id = ServiceId::from_index(target.service as usize - 1)
                        .expect("validated service id");
                    prop_assert_eq!(PackageTarget::service(id), target);
                }
                PackageTargetKind::Program => {
                    prop_assert_eq!(PackageTarget::program(used), Some(target));
                }
                PackageTargetKind::NamedService => {
                    prop_assert_eq!(PackageTarget::named_service(used), Some(target));
                }
            }
        }
    }
}
