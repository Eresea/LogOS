use core::sync::atomic::{AtomicU8, AtomicU32, Ordering};

pub(crate) struct StorageProofObserver {
    mode: AtomicU8,
    pending_operation: AtomicU8,
    pending_path: AtomicU8,
    pending_request: AtomicU32,
    missing_paths: AtomicU8,
    reported: AtomicU8,
}

const PATH_SURVIVOR: u8 = 1;
const PATH_ABORTED: u8 = 2;
const PATH_REMOVED: u8 = 4;

fn proof_path(path: &[u8]) -> u8 {
    match path {
        b"/api-survivor" => PATH_SURVIVOR,
        b"/api-aborted" => PATH_ABORTED,
        b"/api-removed" => PATH_REMOVED,
        _ => 0,
    }
}

impl StorageProofObserver {
    pub const fn new() -> Self {
        Self {
            mode: AtomicU8::new(0),
            pending_operation: AtomicU8::new(0),
            pending_path: AtomicU8::new(0),
            pending_request: AtomicU32::new(0),
            missing_paths: AtomicU8::new(0),
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
        let path = proof_path(request.path);
        if request.operation == logos_abi::StorageApiOperation::CreateFile
            && path == PATH_SURVIVOR
            && self.mode.load(Ordering::Acquire) == 0
        {
            crate::arch_proof_line(b"LogOS vNext: storage command API START");
        }
        self.pending_operation.store(request.operation as u8, Ordering::Release);
        self.pending_path.store(path, Ordering::Release);
        self.pending_request.store(request.request_id, Ordering::Release);
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
        let request_id = self.pending_request.load(Ordering::Acquire);
        if request_id == 0 || response.request_id != request_id {
            return;
        }
        let operation = self.pending_operation.load(Ordering::Acquire);
        let path = self.pending_path.load(Ordering::Acquire);
        self.pending_request.store(0, Ordering::Release);
        if operation == logos_abi::StorageApiOperation::CreateFile as u8 {
            if path == PATH_SURVIVOR && self.mode.load(Ordering::Acquire) == 0 {
                if response.status == logos_abi::StorageApiStatus::Ok {
                    self.mode.store(1, Ordering::Release);
                } else if response.status == logos_abi::StorageApiStatus::AlreadyExists {
                    self.mode.store(2, Ordering::Release);
                } else {
                    crate::arch_proof_line(b"LogOS vNext: storage command API FAIL");
                }
            }
        }
        let mode = self.mode.load(Ordering::Acquire);
        let expected_data: &[u8] = if mode == 1 { b"replacement-api" } else { b"recovered-api" };
        if operation == logos_abi::StorageApiOperation::Read as u8
            && response.status == logos_abi::StorageApiStatus::Ok
            && !response.more
            && response.data == expected_data
            && path == PATH_SURVIVOR
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
        if response.status == logos_abi::StorageApiStatus::NotFound
            && (path == PATH_ABORTED || path == PATH_REMOVED)
        {
            let previous = self.missing_paths.fetch_or(path, Ordering::AcqRel);
            let required = PATH_ABORTED | PATH_REMOVED;
            if previous & required != required && (previous | path) & required == required {
                crate::arch_proof_line(b"LogOS vNext: storage command API cleanup PASS");
            }
        }
    }
}
