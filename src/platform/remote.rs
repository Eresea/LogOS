use crate::platform::{block, secrets::RemoteState, storage};
use crate::sched::native_task;

pub struct LocalCommandReply {
    pub reply: logos_abi::SessionReply,
    pub enrolled: bool,
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RemotePhase {
    Received,
    Authenticating,
    PersistingPending,
    InvokingSession,
    PersistingCompletion,
    ReadyToReply,
    Complete,
    Failed,
    TimedOut,
}

#[allow(dead_code)]
#[derive(Clone, Copy)]
pub struct RemoteOperation {
    pub token: logos_abi::service::OperationToken,
    pub child: Option<logos_abi::service::OperationToken>,
    pub identity: logos_core::operation::OperationIdentity,
    pub page: logos_abi::PageHandle,
    pub sequence: u64,
    pub phase: RemotePhase,
    pub deadline: u64,
    pub input: [u8; logos_remote::MAX_FRAME],
    pub input_length: usize,
}

pub struct RemoteRuntime {
    state: Option<RemoteState>,
    gateway_started: bool,
    operation: Option<RemoteOperation>,
}

impl RemoteRuntime {
    pub fn state(&self) -> Option<&RemoteState> {
        self.state.as_ref()
    }

    pub fn replace_state(&mut self, state: RemoteState) {
        self.state = Some(state);
        self.operation = None;
    }

    pub fn load_control(
        &mut self,
        input: &mut [u8; logos_remote::REMOTE_CONTROL_BLOB_BYTES],
    ) -> bool {
        self.state.as_mut().is_some_and(|state| state.load_control(input))
    }

    pub fn disable(&mut self) {
        if let Some(state) = self.state.as_mut() {
            state.disable();
        }
    }

    pub fn handle_request<T>(&mut self, handler: impl FnOnce(&mut RemoteState) -> T) -> Option<T> {
        self.state.as_mut().map(handler)
    }

    pub fn new(bootstrap: Option<logos_remote::Bootstrap>) -> Self {
        Self {
            state: bootstrap.and_then(RemoteState::new),
            gateway_started: false,
            operation: None,
        }
    }

    pub fn start(
        &mut self,
        network_configured: bool,
        gateway: Option<crate::sched::native_task::Handle>,
    ) -> Option<crate::sched::native_task::Handle> {
        if !self.gateway_started && network_configured {
            return gateway;
        }
        None
    }

    pub fn mark_started(&mut self) {
        self.gateway_started = true;
    }

    pub const fn started(&self) -> bool {
        self.gateway_started
    }

    pub fn reset_transport(&mut self) {
        self.gateway_started = false;
        self.operation = None;
        if let Some(state) = self.state.as_mut() {
            state.reset_transport();
        }
    }

    pub fn begin_operation(
        &mut self,
        owner: u64,
        page: logos_abi::PageHandle,
        generation: u32,
        request_id: u32,
        deadline: u64,
        input: &[u8],
    ) -> bool {
        let Some(identity) =
            logos_core::operation::OperationIdentity::new(owner, generation, request_id)
        else {
            return false;
        };
        if input.len() > logos_remote::MAX_FRAME || self.operation.is_some() {
            return false;
        }
        let Some(token) =
            logos_abi::service::OperationToken::new(owner, generation, request_id, deadline, 1)
        else {
            return false;
        };
        let mut owned = [0; logos_remote::MAX_FRAME];
        owned[..input.len()].copy_from_slice(input);
        self.operation = Some(RemoteOperation {
            token,
            child: None,
            identity,
            page,
            sequence: 1,
            phase: RemotePhase::Received,
            deadline,
            input: owned,
            input_length: input.len(),
        });
        true
    }

    pub fn set_operation_phase(&mut self, phase: RemotePhase) {
        if let Some(operation) = self.operation.as_mut() {
            operation.phase = phase;
        }
    }

    pub fn begin_child_operation(&mut self) -> bool {
        let Some(operation) = self.operation.as_mut() else { return false };
        if operation.child.is_some() {
            return false;
        }
        let Some(child) = logos_abi::service::OperationToken::new(
            operation.token.owner,
            operation.token.generation,
            operation.token.request_id,
            operation.token.deadline,
            operation.token.sequence.saturating_add(1),
        ) else {
            return false;
        };
        operation.child = Some(child);
        true
    }

    pub fn finish_child_operation(&mut self, success: bool) -> bool {
        let Some(operation) = self.operation.as_mut() else { return false };
        let Some(child) = operation.child.take() else { return false };
        success
            && child.matches(
                operation.token.owner,
                operation.token.generation,
                operation.token.request_id,
            )
            && child.sequence > operation.token.sequence
    }

    #[allow(dead_code)]
    pub fn operation(&self) -> Option<RemoteOperation> {
        self.operation
    }

    pub fn finish_operation(&mut self) {
        if let Some(operation) = self.operation.as_mut() {
            operation.phase = RemotePhase::Complete;
        }
        self.operation = None;
    }

    #[allow(clippy::too_many_arguments)]
    pub fn local_command(
        &mut self,
        request: logos_abi::SessionRequest,
        bootstrap: Option<logos_remote::Bootstrap>,
        storage_runtime: &mut storage::StorageRuntime,
        block_context: &mut block::DispatchContext<'_>,
        scheduler: &mut native_task::Scheduler<'_>,
        page: logos_abi::PageHandle,
        owner: u64,
        tick: u64,
    ) -> LocalCommandReply {
        let mut reply = logos_abi::SessionReply::from_bytes(b"remote unavailable").unwrap();
        let mut enrolled = false;
        let Some(state) = self.state.as_mut() else {
            return LocalCommandReply { reply, enrolled };
        };
        match request.syscall {
            logos_abi::Syscall::RemoteKey if state.available() => {
                let mut key = [0; 64];
                hex_key(&state.machine_public(), &mut key);
                reply = logos_abi::SessionReply::from_bytes(&key).unwrap();
            }
            logos_abi::Syscall::Enroll => {
                let mut client_key = [0; 32];
                if request.length == 64
                    && hex_decode_key(&request.argument[..request.length], &mut client_key)
                    && state.enroll(client_key).is_some()
                    && storage_runtime.persist_remote_enrollment(
                        state,
                        bootstrap,
                        block_context,
                        scheduler,
                        page,
                        owner,
                        tick,
                    )
                {
                    let mut key = [0; 64];
                    hex_key(&state.machine_public(), &mut key);
                    let mut output = [0; 96];
                    output[..64].copy_from_slice(&key);
                    output[64] = b':';
                    let mut digits = [0; 20];
                    let length = decimal_u64(state.enrollment().generation, &mut digits);
                    output[65..65 + length].copy_from_slice(&digits[..length]);
                    reply = logos_abi::SessionReply::from_bytes(&output[..65 + length]).unwrap();
                    enrolled = true;
                } else {
                    reply = logos_abi::SessionReply::from_bytes(b"invalid enrollment key").unwrap();
                }
            }
            logos_abi::Syscall::Unenroll
                if state.unenroll().is_some()
                    && storage_runtime.persist_remote_enrollment(
                        state,
                        bootstrap,
                        block_context,
                        scheduler,
                        page,
                        owner,
                        tick,
                    ) =>
            {
                reply = logos_abi::SessionReply::from_bytes(b"remote unenrolled").unwrap();
            }
            _ => {}
        }
        LocalCommandReply { reply, enrolled }
    }
}

fn hex_key(key: &[u8; 32], output: &mut [u8; 64]) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for (index, byte) in key.iter().copied().enumerate() {
        output[index * 2] = HEX[usize::from(byte >> 4)];
        output[index * 2 + 1] = HEX[usize::from(byte & 0xf)];
    }
}

fn hex_decode_key(input: &[u8], output: &mut [u8; 32]) -> bool {
    if input.len() != 64 {
        return false;
    }
    input.chunks_exact(2).zip(output.iter_mut()).all(|(pair, byte)| {
        let high = hex_value(pair[0]);
        let low = hex_value(pair[1]);
        match (high, low) {
            (Some(high), Some(low)) => {
                *byte = high << 4 | low;
                true
            }
            _ => false,
        }
    })
}

fn hex_value(byte: u8) -> Option<u8> {
    let byte = byte.to_ascii_lowercase();
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

fn decimal_u64(mut value: u64, output: &mut [u8; 20]) -> usize {
    let mut length = 0;
    loop {
        output[length] = b'0' + (value % 10) as u8;
        length += 1;
        value /= 10;
        if value == 0 {
            break;
        }
    }
    output[..length].reverse();
    length
}

/// Remote owns the cross-service gate that decides whether the replaceable
/// gateway may start. The gateway receives no policy state; Core only supplies
/// the resulting typed endpoint.
pub fn gateway_allowed(
    network_ready: bool,
    sessions_ready: bool,
    storage_ready: bool,
    state: Option<&RemoteState>,
    _test_hooks: bool,
) -> bool {
    network_ready
        && sessions_ready
        && storage_ready
        && state.is_some_and(|state| state.available() && state.enrollment().active)
}

pub fn self_check() -> bool {
    !gateway_allowed(false, true, true, None, false)
        && !gateway_allowed(true, false, true, None, false)
        && !gateway_allowed(true, true, true, None, true)
}

#[cfg(test)]
mod tests {
    use super::RemoteRuntime;

    #[test]
    fn gateway_start_requires_composition_confirmation() {
        let mut runtime = RemoteRuntime::new(None);
        assert!(runtime.start(true, None).is_none());
        assert!(!runtime.started());
        runtime.mark_started();
        assert!(runtime.started());
        runtime.reset_transport();
        assert!(!runtime.started());
    }

    #[test]
    fn operation_slot_owns_one_bounded_request() {
        let mut runtime = RemoteRuntime::new(None);
        assert!(runtime.begin_operation(7, logos_abi::PageHandle(3), 2, 9, 100, b"payload",));
        assert!(!runtime.begin_operation(7, logos_abi::PageHandle(3), 2, 10, 100, b"other"));
        let operation = runtime.operation().unwrap();
        assert_eq!(operation.phase, RemotePhase::Received);
        assert_eq!(&operation.input[..operation.input_length], b"payload");
        runtime.set_operation_phase(RemotePhase::PersistingPending);
        assert_eq!(runtime.operation().unwrap().phase, RemotePhase::PersistingPending);
        runtime.finish_operation();
        assert!(runtime.operation().is_none());
    }
}
