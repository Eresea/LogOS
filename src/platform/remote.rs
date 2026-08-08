use crate::platform::{block, secrets::RemoteState, storage};
use crate::sched::native_task;

pub struct LocalCommandReply {
    pub reply: logos_abi::SessionReply,
    pub enrolled: bool,
}

pub struct RemoteRuntime {
    state: Option<RemoteState>,
    gateway_started: bool,
}

impl RemoteRuntime {
    pub fn state(&self) -> Option<&RemoteState> {
        self.state.as_ref()
    }

    pub fn state_mut(&mut self) -> &mut Option<RemoteState> {
        &mut self.state
    }

    pub fn replace_state(&mut self, state: RemoteState) {
        self.state = Some(state);
    }

    pub fn new(bootstrap: Option<logos_remote::Bootstrap>) -> Self {
        Self { state: bootstrap.and_then(RemoteState::new), gateway_started: false }
    }

    pub fn start(
        &mut self,
        network_configured: bool,
        gateway: Option<crate::sched::native_task::Handle>,
        scheduler: &mut crate::sched::native_task::Scheduler<'_>,
    ) -> bool {
        if !self.gateway_started && network_configured {
            self.gateway_started =
                gateway.is_some_and(|handle| scheduler.run(handle) && !scheduler.failed(handle));
            return self.gateway_started;
        }
        false
    }

    pub const fn started(&self) -> bool {
        self.gateway_started
    }

    pub fn reset_transport(&mut self) {
        self.gateway_started = false;
        if let Some(state) = self.state.as_mut() {
            state.reset_transport();
        }
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
