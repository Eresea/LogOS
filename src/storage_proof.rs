use core::sync::atomic::{AtomicU8, Ordering};

pub(crate) struct StorageProofObserver {
    mode: AtomicU8,
    pending: AtomicU8,
    missing: AtomicU8,
    reported: AtomicU8,
}

impl StorageProofObserver {
    pub const fn new() -> Self {
        Self {
            mode: AtomicU8::new(0),
            pending: AtomicU8::new(0),
            missing: AtomicU8::new(0),
            reported: AtomicU8::new(0),
        }
    }

    pub fn observe_request(&self, bytes: &[u8]) {
        if bytes.len() != core::mem::size_of::<logos_abi::IpcBytes>() {
            return;
        }
        if !logos_abi::IpcBytes::wire_enums_valid(bytes) {
            return;
        }
        let message =
            unsafe { core::ptr::read_unaligned(bytes.as_ptr().cast::<logos_abi::IpcBytes>()) };
        let Ok(request) = logos_abi::StorageApiRequest::decode(&message) else {
            return;
        };
        self.pending.store(request.operation as u8, Ordering::Release);
    }

    pub fn observe_response(&self, bytes: &[u8]) {
        if bytes.len() != core::mem::size_of::<logos_abi::IpcBytes>() {
            return;
        }
        if !logos_abi::IpcBytes::wire_enums_valid(bytes) {
            return;
        }
        let message =
            unsafe { core::ptr::read_unaligned(bytes.as_ptr().cast::<logos_abi::IpcBytes>()) };
        let Ok(response) = logos_abi::StorageApiResponse::decode(&message) else {
            return;
        };
        let operation = self.pending.load(Ordering::Acquire);
        if operation == logos_abi::StorageApiOperation::CreateFile as u8 {
            if self.mode.load(Ordering::Acquire) == 0 {
                if response.status == logos_abi::StorageApiStatus::Ok {
                    self.mode.store(1, Ordering::Release);
                } else if response.status == logos_abi::StorageApiStatus::AlreadyExists {
                    self.mode.store(2, Ordering::Release);
                }
            }
        }
        let mode = self.mode.load(Ordering::Acquire);
        let expected_data: &[u8] = if mode == 1 { b"replacement-api" } else { b"recovered-api" };
        if operation == logos_abi::StorageApiOperation::Read as u8
            && response.status == logos_abi::StorageApiStatus::Ok
            && !response.more
            && response.data == expected_data
            && mode != 0
        {
            let marker: &[u8] = if mode == 1 {
                b"LogOS vNext: storage command API PASS"
            } else {
                b"LogOS vNext: storage command API recovery PASS"
            };
            let expected = if mode == 1 { 1 } else { 2 };
            if self
                .reported
                .compare_exchange(0, expected, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                crate::arch_proof_line(marker);
            }
        }
        if response.status == logos_abi::StorageApiStatus::NotFound {
            let missing = self.missing.fetch_add(1, Ordering::AcqRel) + 1;
            if missing == 2 {
                crate::arch_proof_line(b"LogOS vNext: storage command API cleanup PASS");
            }
        }
    }
}
