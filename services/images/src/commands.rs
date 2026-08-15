#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", no_main)]
#![cfg_attr(not(target_os = "none"), allow(dead_code, unused_imports, unused_variables))]

mod common;

use logos_abi::{
    IPC_FLAG_MORE, IpcBytes, IpcStatus, MessageKind, STORAGE_API_FLAG_REPLACE, StorageApiOperation,
    StorageApiRequest, StorageApiResponse, StorageApiStatus,
};

const INPUT_CAPABILITY: usize = common::capability_slot(
    logos_abi::ServiceId::Commands,
    logos_abi::IpcEndpointId::SessionToCommands,
    logos_abi::IpcRights::Receive,
);
const OUTPUT_CAPABILITY: usize = common::capability_slot(
    logos_abi::ServiceId::Commands,
    logos_abi::IpcEndpointId::CommandsToSession,
    logos_abi::IpcRights::Send,
);
const STORAGE_SEND_CAPABILITY: usize = common::capability_slot(
    logos_abi::ServiceId::Commands,
    logos_abi::IpcEndpointId::CommandsToStorage,
    logos_abi::IpcRights::Send,
);
const STORAGE_RECEIVE_CAPABILITY: usize = common::capability_slot(
    logos_abi::ServiceId::Commands,
    logos_abi::IpcEndpointId::StorageToCommands,
    logos_abi::IpcRights::Receive,
);

struct PendingOutput {
    bytes: [u8; logos_commands::MAX_OUTPUT_BYTES],
    len: usize,
    offset: usize,
    pending: bool,
}

impl PendingOutput {
    const fn new() -> Self {
        Self { bytes: [0; logos_commands::MAX_OUTPUT_BYTES], len: 0, offset: 0, pending: false }
    }

    fn stage(&mut self, bytes: &[u8]) {
        let count = bytes.len().min(self.bytes.len());
        self.bytes[..count].copy_from_slice(&bytes[..count]);
        self.len = count;
        self.offset = 0;
        self.pending = true;
    }

    fn flush(&mut self, capability_slot: usize) -> bool {
        let mut progressed = false;
        while self.offset < self.len {
            let end = (self.offset + logos_abi::MAX_IPC_BYTES).min(self.len);
            let Some(mut message) =
                IpcBytes::from_bytes(MessageKind::SessionOutput, &self.bytes[self.offset..end])
            else {
                break;
            };
            if end < self.len {
                message.flags = IPC_FLAG_MORE;
            }
            if common::ipc_send(capability_slot, &message) != IpcStatus::Ok {
                break;
            }
            self.offset = end;
            progressed = true;
        }
        if self.pending && self.offset == self.len {
            let message = IpcBytes::empty(MessageKind::SessionOutput);
            if self.len == 0 && common::ipc_send(capability_slot, &message) == IpcStatus::Ok {
                self.pending = false;
                progressed = true;
            }
        }
        if self.pending && self.offset == self.len && self.len != 0 {
            self.len = 0;
            self.offset = 0;
            self.pending = false;
        }
        progressed
    }
}

#[derive(Clone, Copy)]
enum StorageWork {
    List,
    Touch,
    Cat,
    Write,
    Remove,
    Move,
    #[cfg(feature = "storage-proof")]
    AbortProof,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum StoragePhase {
    Begin,
    Operation,
    Commit,
    Abort,
    Read,
    List,
    Idle,
}

struct StorageClient {
    work: StorageWork,
    phase: StoragePhase,
    busy: bool,
    done: bool,
    sent: bool,
    request_id: u32,
    transaction_id: u64,
    cursor: u32,
    path: [u8; logos_commands::MAX_COMMAND_BYTES],
    path_len: usize,
    secondary_path: [u8; logos_commands::MAX_COMMAND_BYTES],
    secondary_len: usize,
    data: [u8; logos_commands::MAX_COMMAND_BYTES],
    data_len: usize,
    result: [u8; logos_commands::MAX_OUTPUT_BYTES],
    result_len: usize,
    failure: StorageApiStatus,
    last_status: StorageApiStatus,
}

impl StorageClient {
    const fn new() -> Self {
        Self {
            work: StorageWork::List,
            phase: StoragePhase::Idle,
            busy: false,
            done: false,
            sent: false,
            request_id: 1,
            transaction_id: 0,
            cursor: 0,
            path: [0; logos_commands::MAX_COMMAND_BYTES],
            path_len: 0,
            secondary_path: [0; logos_commands::MAX_COMMAND_BYTES],
            secondary_len: 0,
            data: [0; logos_commands::MAX_COMMAND_BYTES],
            data_len: 0,
            result: [0; logos_commands::MAX_OUTPUT_BYTES],
            result_len: 0,
            failure: StorageApiStatus::Invalid,
            last_status: StorageApiStatus::Invalid,
        }
    }

    fn start(&mut self, command: logos_commands::StorageCommand<'_>) -> bool {
        let (work, phase, path, secondary, data) = match command {
            logos_commands::StorageCommand::List { path } => {
                (StorageWork::List, StoragePhase::List, path, &[][..], &[][..])
            }
            logos_commands::StorageCommand::Touch { path } => {
                (StorageWork::Touch, StoragePhase::Begin, path, &[][..], &[][..])
            }
            logos_commands::StorageCommand::Cat { path } => {
                (StorageWork::Cat, StoragePhase::Read, path, &[][..], &[][..])
            }
            logos_commands::StorageCommand::Write { path, data } => {
                (StorageWork::Write, StoragePhase::Begin, path, &[][..], data)
            }
            logos_commands::StorageCommand::Remove { path } => {
                (StorageWork::Remove, StoragePhase::Begin, path, &[][..], &[][..])
            }
            logos_commands::StorageCommand::Move { from, to } => {
                (StorageWork::Move, StoragePhase::Begin, from, to, &[][..])
            }
        };
        self.start_work(work, phase, path, secondary, data)
    }

    #[cfg(feature = "storage-proof")]
    fn start_proof_abort(&mut self, path: &[u8]) -> bool {
        self.failure = StorageApiStatus::Ok;
        self.start_work(StorageWork::AbortProof, StoragePhase::Begin, path, &[], &[])
    }

    fn start_work(
        &mut self,
        work: StorageWork,
        phase: StoragePhase,
        path: &[u8],
        secondary: &[u8],
        data: &[u8],
    ) -> bool {
        if self.busy || self.done {
            return false;
        }
        self.path_len = 0;
        self.secondary_len = 0;
        self.data_len = 0;
        self.result_len = 0;
        self.cursor = 0;
        self.transaction_id = 0;
        self.last_status = StorageApiStatus::Invalid;
        if path.len() > self.path.len()
            || secondary.len() > self.secondary_path.len()
            || data.len() > self.data.len()
        {
            return false;
        }
        self.path[..path.len()].copy_from_slice(path);
        self.path_len = path.len();
        self.secondary_path[..secondary.len()].copy_from_slice(secondary);
        self.secondary_len = secondary.len();
        self.data[..data.len()].copy_from_slice(data);
        self.data_len = data.len();
        self.work = work;
        self.phase = phase;
        self.busy = true;
        self.done = false;
        self.sent = false;
        true
    }

    fn active(&self) -> bool {
        self.busy
    }

    fn next_request(&mut self) {
        self.request_id = self.request_id.wrapping_add(1).max(1);
        self.sent = false;
    }

    fn request(&self) -> Option<IpcBytes> {
        let (operation, transaction_id, flags, offset, path, secondary, data) = match self.phase {
            StoragePhase::Begin => (StorageApiOperation::Begin, 0, 0, 0, &[][..], &[][..], &[][..]),
            StoragePhase::Operation => {
                let operation = match self.work {
                    StorageWork::Touch => StorageApiOperation::CreateFile,
                    StorageWork::Write => StorageApiOperation::Write,
                    StorageWork::Remove => StorageApiOperation::Remove,
                    StorageWork::Move => StorageApiOperation::Rename,
                    #[cfg(feature = "storage-proof")]
                    StorageWork::AbortProof => StorageApiOperation::CreateFile,
                    StorageWork::List | StorageWork::Cat => StorageApiOperation::Read,
                };
                (
                    operation,
                    self.transaction_id,
                    if matches!(self.work, StorageWork::Write) {
                        STORAGE_API_FLAG_REPLACE
                    } else {
                        0
                    },
                    0,
                    &self.path[..self.path_len],
                    &self.secondary_path[..self.secondary_len],
                    &self.data[..self.data_len],
                )
            }
            StoragePhase::Commit => {
                (StorageApiOperation::Commit, self.transaction_id, 0, 0, &[][..], &[][..], &[][..])
            }
            StoragePhase::Abort => {
                (StorageApiOperation::Abort, self.transaction_id, 0, 0, &[][..], &[][..], &[][..])
            }
            StoragePhase::Read => (
                StorageApiOperation::Read,
                0,
                0,
                self.cursor,
                &self.path[..self.path_len],
                &[][..],
                &[][..],
            ),
            StoragePhase::List => (
                StorageApiOperation::List,
                0,
                0,
                self.cursor,
                &self.path[..self.path_len],
                &[][..],
                &[][..],
            ),
            StoragePhase::Idle => return None,
        };
        StorageApiRequest::encode(
            operation,
            flags,
            self.request_id,
            transaction_id,
            offset,
            path,
            secondary,
            data,
        )
    }

    fn drive(&mut self) -> bool {
        if !self.busy {
            return false;
        }
        if !self.sent {
            let Some(request) = self.request() else {
                self.fail(StorageApiStatus::Invalid);
                return true;
            };
            if common::ipc_send(STORAGE_SEND_CAPABILITY, &request) != IpcStatus::Ok {
                return false;
            }
            self.sent = true;
            return true;
        }
        let mut message = IpcBytes::empty(MessageKind::StorageResponse);
        if common::ipc_receive(STORAGE_RECEIVE_CAPABILITY, &mut message) != IpcStatus::Ok {
            return false;
        }
        self.sent = false;
        let Ok(response) = StorageApiResponse::decode(&message) else {
            self.fail(StorageApiStatus::Invalid);
            return true;
        };
        if response.request_id != self.request_id {
            self.fail(StorageApiStatus::Stale);
            return true;
        }
        self.handle_response(response);
        true
    }

    fn handle_response(&mut self, response: StorageApiResponse<'_>) {
        if response.status != StorageApiStatus::Ok {
            if self.phase == StoragePhase::Operation && self.transaction_id != 0 {
                self.failure = response.status;
                self.phase = StoragePhase::Abort;
                self.next_request();
            } else {
                self.fail(response.status);
            }
            return;
        }
        match self.phase {
            StoragePhase::Begin => {
                if response.transaction_id == 0 {
                    self.fail(StorageApiStatus::Invalid);
                } else {
                    self.transaction_id = response.transaction_id;
                    self.phase = StoragePhase::Operation;
                    self.next_request();
                }
            }
            StoragePhase::Operation => {
                self.phase = if self.operation_aborts() {
                    StoragePhase::Abort
                } else {
                    StoragePhase::Commit
                };
                self.next_request();
            }
            StoragePhase::Commit => self.succeed(),
            StoragePhase::Abort => self.fail(self.failure),
            StoragePhase::Read => {
                if response.data.is_empty() && response.more {
                    self.fail(StorageApiStatus::Invalid);
                } else {
                    self.append(response.data);
                    self.cursor = self.cursor.saturating_add(response.data.len() as u32);
                    if response.more && self.result_len < self.result.len() {
                        self.next_request();
                    } else {
                        self.succeed();
                    }
                }
            }
            StoragePhase::List => {
                if response.data.len() + 2 <= self.result.len() - self.result_len {
                    self.append(response.data);
                    self.append(b"\r\n");
                }
                self.cursor = self.cursor.saturating_add(1);
                if response.more && self.result_len < self.result.len() {
                    self.next_request();
                } else {
                    self.succeed();
                }
            }
            StoragePhase::Idle => self.fail(StorageApiStatus::Invalid),
        }
    }

    fn append(&mut self, bytes: &[u8]) {
        let count = bytes.len().min(self.result.len() - self.result_len);
        self.result[self.result_len..self.result_len + count].copy_from_slice(&bytes[..count]);
        self.result_len += count;
    }

    #[cfg(feature = "storage-proof")]
    fn operation_aborts(&self) -> bool {
        matches!(self.work, StorageWork::AbortProof)
    }

    #[cfg(not(feature = "storage-proof"))]
    const fn operation_aborts(&self) -> bool {
        false
    }

    fn fail(&mut self, status: StorageApiStatus) {
        self.last_status = status;
        self.result_len = 0;
        self.append(status_text(status));
        self.phase = StoragePhase::Idle;
        self.busy = false;
        self.done = true;
    }

    fn succeed(&mut self) {
        self.last_status = StorageApiStatus::Ok;
        if matches!(
            self.work,
            StorageWork::Touch | StorageWork::Write | StorageWork::Remove | StorageWork::Move
        ) {
            self.append(b"ok\r\n");
        } else if matches!(self.work, StorageWork::Cat) {
            self.append(b"\r\n");
        }
        self.phase = StoragePhase::Idle;
        self.busy = false;
        self.done = true;
    }

    fn take_result(&mut self, pending: &mut PendingOutput) {
        if self.done {
            pending.stage(&self.result[..self.result_len]);
            self.done = false;
        }
    }

    #[cfg(feature = "storage-proof")]
    fn discard_result(&mut self) -> StorageApiStatus {
        self.done = false;
        self.last_status
    }
}

fn status_text(status: StorageApiStatus) -> &'static [u8] {
    match status {
        StorageApiStatus::NotFound => b"not found\r\n",
        StorageApiStatus::AlreadyExists => b"already exists\r\n",
        StorageApiStatus::Busy => b"storage busy\r\n",
        StorageApiStatus::Stale => b"stale transaction\r\n",
        StorageApiStatus::TooLarge => b"data too large\r\n",
        StorageApiStatus::NoTransaction => b"no transaction\r\n",
        _ => b"storage error\r\n",
    }
}

static mut COMMANDS: logos_commands::CommandService = logos_commands::CommandService::new();
static mut PENDING: PendingOutput = PendingOutput::new();
static mut STORAGE: StorageClient = StorageClient::new();

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    let commands = unsafe { &mut *core::ptr::addr_of_mut!(COMMANDS) };
    let pending = unsafe { &mut *core::ptr::addr_of_mut!(PENDING) };
    let storage = unsafe { &mut *core::ptr::addr_of_mut!(STORAGE) };
    #[cfg(feature = "storage-proof")]
    let mut proof_step = 0u8;
    #[cfg(feature = "storage-proof")]
    let mut proof_active = false;
    #[cfg(feature = "storage-proof")]
    let mut proof_recovery = false;
    let mut heartbeat_ticks = 0u16;
    loop {
        common::heartbeat_tick(&mut heartbeat_ticks, logos_abi::ServiceId::Commands);
        let mut progressed = pending.flush(OUTPUT_CAPABILITY);
        if pending.pending {
            if !progressed {
                common::wait(
                    common::ipc_write_event(logos_abi::IpcEndpointId::CommandsToSession),
                    logos_abi::ServiceId::Commands,
                );
            }
            continue;
        }
        if storage.active() {
            progressed |= storage.drive();
            if storage.done {
                #[cfg(feature = "storage-proof")]
                if !proof_active {
                    storage.take_result(pending);
                }
                #[cfg(not(feature = "storage-proof"))]
                storage.take_result(pending);
                progressed = true;
            }
            if storage.active() {
                if !progressed {
                    common::wait(
                        common::ipc_read_event(logos_abi::IpcEndpointId::StorageToCommands)
                            | common::ipc_write_event(logos_abi::IpcEndpointId::CommandsToStorage)
                            | common::ipc_read_event(logos_abi::IpcEndpointId::SessionToCommands),
                        logos_abi::ServiceId::Commands,
                    );
                }
                continue;
            }
        }
        #[cfg(feature = "storage-proof")]
        if storage.done && proof_active {
            let status = storage.discard_result();
            if proof_step == 0 && status == StorageApiStatus::AlreadyExists {
                proof_recovery = true;
            }
            let accepted = if proof_recovery {
                match proof_step {
                    0 => status == StorageApiStatus::AlreadyExists,
                    1 | 2 | 3 | 5 => status == StorageApiStatus::Ok,
                    4 | 6 | 7 => {
                        matches!(status, StorageApiStatus::Ok | StorageApiStatus::NotFound)
                    }
                    _ => false,
                }
            } else {
                match proof_step {
                    0 | 1 | 2 | 3 => status == StorageApiStatus::Ok,
                    4 | 5 => status == StorageApiStatus::NotFound,
                    _ => false,
                }
            };
            if accepted {
                proof_step = proof_step.saturating_add(1);
                proof_active = false;
                if (!proof_recovery && proof_step > 5) || (proof_recovery && proof_step > 7) {
                    proof_step = u8::MAX;
                    proof_active = false;
                }
            } else {
                proof_step = u8::MAX;
                proof_active = false;
            }
            progressed = true;
        }
        #[cfg(feature = "storage-proof")]
        if !proof_active
            && proof_step != u8::MAX
            && !storage.active()
            && !storage.done
            && !pending.pending
        {
            let started = match proof_step {
                0 => {
                    storage.start(logos_commands::StorageCommand::Touch { path: b"/api-survivor" })
                }
                1 => storage.start(logos_commands::StorageCommand::Write {
                    path: b"/api-survivor",
                    data: b"durable-api",
                }),
                2 => storage.start_proof_abort(b"/api-aborted"),
                3 if proof_recovery => {
                    storage.start(logos_commands::StorageCommand::Touch { path: b"/api-removed" })
                }
                3 => storage.start(logos_commands::StorageCommand::Cat { path: b"/api-survivor" }),
                4 if proof_recovery => {
                    storage.start(logos_commands::StorageCommand::Remove { path: b"/api-removed" })
                }
                4 => storage.start(logos_commands::StorageCommand::Cat { path: b"/api-aborted" }),
                5 if proof_recovery => {
                    storage.start(logos_commands::StorageCommand::Cat { path: b"/api-survivor" })
                }
                5 => storage.start(logos_commands::StorageCommand::Cat { path: b"/api-removed" }),
                6 => storage.start(logos_commands::StorageCommand::Cat { path: b"/api-aborted" }),
                7 => storage.start(logos_commands::StorageCommand::Cat { path: b"/api-removed" }),
                _ => false,
            };
            if started {
                proof_active = true;
                progressed = true;
            }
        }
        let mut message = IpcBytes::empty(MessageKind::SessionInput);
        if common::ipc_receive(INPUT_CAPABILITY, &mut message) == IpcStatus::Ok {
            progressed = true;
            if message.kind == MessageKind::SessionInput {
                if let Some(bytes) = message.as_bytes() {
                    match logos_commands::parse_storage_command(bytes) {
                        Ok(Some(command)) => {
                            if !storage.start(command) {
                                pending.stage(b"storage request too large\r\n");
                            }
                        }
                        Err(logos_commands::StorageCommandError::Usage) => {
                            pending.stage(b"usage error\r\n");
                        }
                        Ok(None) => {
                            let result = commands.execute(bytes);
                            if result.action != logos_commands::CommandAction::None {
                                let status = common::power(result.action as usize);
                                if status != 0 {
                                    pending.stage(b"power action denied\r\n");
                                }
                            } else if result.clear_screen {
                                pending.stage(b"\x1b[2J\x1b[H");
                            } else {
                                pending.stage(result.as_bytes());
                            }
                        }
                    }
                }
            }
        }
        if !progressed {
            common::wait(
                common::ipc_read_event(logos_abi::IpcEndpointId::SessionToCommands)
                    | common::ipc_read_event(logos_abi::IpcEndpointId::StorageToCommands),
                logos_abi::ServiceId::Commands,
            );
        }
    }
}

#[cfg(target_os = "none")]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo<'_>) -> ! {
    common::idle()
}

#[cfg(not(target_os = "none"))]
fn main() {}
