#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", no_main)]
#![cfg_attr(not(target_os = "none"), allow(dead_code, unused_imports, unused_variables))]

mod common;

use logos_abi::{
    CompletionRequest, CompletionResponse, FetchPhase, FetchResponse, FetchStatus, FlowControl,
    IPC_FLAG_MORE, IpcBytes, IpcStatus, MAX_IPC_BYTES, MessageKind,
};
use logos_session::MAX_LINE_BYTES;

const INPUT_CAPABILITY: usize = common::capability_slot(
    logos_abi::ServiceId::Session,
    logos_abi::IpcEndpointId::TerminalToSession,
    logos_abi::IpcRights::Receive,
);
const OUTPUT_CAPABILITY: usize = common::capability_slot(
    logos_abi::ServiceId::Session,
    logos_abi::IpcEndpointId::SessionToTerminal,
    logos_abi::IpcRights::Send,
);
const FLOW_CAPABILITY: usize = common::capability_slot(
    logos_abi::ServiceId::Session,
    logos_abi::IpcEndpointId::SessionToFlow,
    logos_abi::IpcRights::Send,
);
const FLOW_OUTPUT_CAPABILITY: usize = common::capability_slot(
    logos_abi::ServiceId::Session,
    logos_abi::IpcEndpointId::FlowToSession,
    logos_abi::IpcRights::Receive,
);

struct PendingOutput {
    bytes: [u8; logos_session::MAX_OUTPUT_BYTES],
    len: usize,
    offset: usize,
}

impl PendingOutput {
    const fn new() -> Self {
        Self { bytes: [0; logos_session::MAX_OUTPUT_BYTES], len: 0, offset: 0 }
    }

    fn is_empty(&self) -> bool {
        self.offset >= self.len
    }

    fn stage(&mut self, bytes: &[u8]) {
        let count = bytes.len().min(self.bytes.len());
        self.bytes[..count].copy_from_slice(&bytes[..count]);
        self.len = count;
        self.offset = 0;
    }

    fn flush(&mut self, capability_slot: usize) -> bool {
        let mut progressed = false;
        while self.offset < self.len {
            let end = (self.offset + MAX_IPC_BYTES).min(self.len);
            let Some(message) =
                IpcBytes::from_bytes(MessageKind::SessionOutput, &self.bytes[self.offset..end])
            else {
                break;
            };
            if common::ipc_send(capability_slot, &message) != IpcStatus::Ok {
                break;
            }
            self.offset = end;
            progressed = true;
        }
        if self.is_empty() {
            self.len = 0;
            self.offset = 0;
        }
        progressed
    }
}

struct PendingFlowInput {
    bytes: [u8; MAX_LINE_BYTES],
    len: usize,
    pending: bool,
}

impl PendingFlowInput {
    const fn new() -> Self {
        Self { bytes: [0; MAX_LINE_BYTES], len: 0, pending: false }
    }

    fn is_empty(&self) -> bool {
        !self.pending
    }

    fn stage(&mut self, bytes: &[u8]) {
        self.bytes[..bytes.len()].copy_from_slice(bytes);
        self.len = bytes.len();
        self.pending = true;
    }

    fn take(&mut self) -> Option<IpcBytes> {
        let message = IpcBytes::from_bytes(MessageKind::SessionInput, &self.bytes[..self.len]);
        if message.is_some() {
            self.len = 0;
            self.pending = false;
        }
        message
    }
}

static mut SESSION: logos_session::SessionService = logos_session::SessionService::new();
static mut PENDING: PendingOutput = PendingOutput::new();
static mut FLOW_INPUT: PendingFlowInput = PendingFlowInput::new();

fn completion_request_message(request: CompletionRequest) -> IpcBytes {
    let bytes = unsafe {
        core::slice::from_raw_parts(
            (&request as *const CompletionRequest).cast::<u8>(),
            core::mem::size_of::<CompletionRequest>(),
        )
    };
    IpcBytes::from_bytes(MessageKind::CompletionRequest, bytes)
        .unwrap_or_else(|| IpcBytes::empty(MessageKind::CompletionRequest))
}

fn completion_response(message: &IpcBytes) -> Option<CompletionResponse> {
    (message.kind == MessageKind::CompletionResponse
        && message.len as usize == core::mem::size_of::<CompletionResponse>())
    .then(|| unsafe { core::ptr::read_unaligned(message.bytes.as_ptr().cast()) })
}

fn fetch_progress(message: &IpcBytes) -> Option<FetchResponse> {
    (message.kind == MessageKind::FlowProgress
        && message.len as usize == core::mem::size_of::<FetchResponse>())
    .then(|| unsafe { core::ptr::read_unaligned(message.bytes.as_ptr().cast()) })
    .filter(|response: &FetchResponse| response.is_valid())
}

fn flow_control_message(request_id: u32) -> IpcBytes {
    let control = FlowControl::cancel(request_id);
    let bytes = unsafe {
        core::slice::from_raw_parts(
            (&control as *const FlowControl).cast::<u8>(),
            core::mem::size_of::<FlowControl>(),
        )
    };
    IpcBytes::from_bytes(MessageKind::FlowControl, bytes)
        .unwrap_or_else(|| IpcBytes::empty(MessageKind::FlowControl))
}

fn render_fetch_progress(response: FetchResponse, output: &mut logos_session::ShellOutput) {
    output.extend(b"\r\x1b[Kfetch: ");
    match response.phase {
        FetchPhase::Connect => output.extend(b"connecting"),
        FetchPhase::SendRequest => output.extend(b"request sent"),
        FetchPhase::ReadResponse => {
            output.push_decimal(response.downloaded_bytes as usize);
            output.extend(b" bytes");
        }
        FetchPhase::StageStorage => output.extend(b"staging"),
        FetchPhase::Commit => output.extend(b"committing"),
        _ => output.extend(match response.status {
            FetchStatus::Cancelled => &b"cancelled"[..],
            _ => &b"working"[..],
        }),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    common::init_service_allocator();
    let session = unsafe { &mut *core::ptr::addr_of_mut!(SESSION) };
    let pending = unsafe { &mut *core::ptr::addr_of_mut!(PENDING) };
    let flow_input = unsafe { &mut *core::ptr::addr_of_mut!(FLOW_INPUT) };
    let mut command_bytes = [0; MAX_LINE_BYTES];
    let mut command_response = [0; logos_session::MAX_OUTPUT_BYTES];
    let mut command_response_len = 0;
    let mut waiting_for_command = false;
    let mut pending_completion = None;
    let mut pending_control = None;
    let mut prompt = logos_session::ShellOutput::new();
    session.prompt(&mut prompt);
    pending.stage(prompt.as_bytes());
    let mut heartbeat_ticks = 0u16;
    loop {
        common::heartbeat_tick(&mut heartbeat_ticks, logos_abi::ServiceId::Session);
        if waiting_for_command && pending_control.is_none() {
            let mut terminal_input = IpcBytes::empty(MessageKind::SessionInput);
            if common::ipc_receive(INPUT_CAPABILITY, &mut terminal_input) == IpcStatus::Ok
                && terminal_input.kind == MessageKind::SessionInput
                && terminal_input.as_bytes().is_some_and(|bytes| bytes == [0x03])
            {
                pending_control = Some(flow_control_message(0));
            }
        }
        if let Some(control) = pending_control {
            match common::ipc_send(FLOW_CAPABILITY, &control) {
                IpcStatus::Ok => pending_control = None,
                IpcStatus::Full => {
                    common::wait(
                        common::ipc_write_event(logos_abi::IpcEndpointId::SessionToFlow),
                        logos_abi::ServiceId::Session,
                    );
                    continue;
                }
                IpcStatus::Stale
                | IpcStatus::Disconnected
                | IpcStatus::Unauthorized
                | IpcStatus::Malformed
                | IpcStatus::Empty => pending_control = None,
            }
        }
        let mut progressed = pending.flush(OUTPUT_CAPABILITY);
        if !pending.is_empty() {
            if !progressed {
                common::wait(
                    common::ipc_write_event(logos_abi::IpcEndpointId::SessionToTerminal),
                    logos_abi::ServiceId::Session,
                );
            }
            continue;
        }
        if let Some(message) = pending_completion {
            match common::ipc_send(FLOW_CAPABILITY, &message) {
                IpcStatus::Ok => {
                    pending_completion = None;
                    progressed = true;
                }
                IpcStatus::Full => {
                    common::wait(
                        common::ipc_write_event(logos_abi::IpcEndpointId::SessionToFlow),
                        logos_abi::ServiceId::Session,
                    );
                    continue;
                }
                IpcStatus::Stale
                | IpcStatus::Disconnected
                | IpcStatus::Unauthorized
                | IpcStatus::Malformed
                | IpcStatus::Empty => {
                    pending_completion = None;
                    let mut failure = logos_session::ShellOutput::new();
                    session.completion_failed(&mut failure);
                    pending.stage(failure.as_bytes());
                    continue;
                }
            }
        }
        if !flow_input.is_empty() {
            if let Some(message) = flow_input.take() {
                if common::ipc_send(FLOW_CAPABILITY, &message) == IpcStatus::Ok {
                    waiting_for_command = true;
                    progressed = true;
                } else {
                    flow_input.stage(message.as_bytes().unwrap_or_default());
                }
            }
            if !progressed {
                common::wait(
                    common::ipc_write_event(logos_abi::IpcEndpointId::SessionToFlow),
                    logos_abi::ServiceId::Session,
                );
            }
            continue;
        }
        if waiting_for_command {
            let mut message = IpcBytes::empty(MessageKind::SessionOutput);
            if common::ipc_receive(FLOW_OUTPUT_CAPABILITY, &mut message) == IpcStatus::Ok {
                if let Some(response) = completion_response(&message) {
                    let mut edit_output = logos_session::ShellOutput::new();
                    session.apply_completion_response(response, &mut edit_output);
                    if edit_output.len > 0 {
                        pending.stage(edit_output.as_bytes());
                    }
                    progressed = true;
                } else if let Some(response) = fetch_progress(&message) {
                    let mut progress = logos_session::ShellOutput::new();
                    render_fetch_progress(response, &mut progress);
                    pending.stage(progress.as_bytes());
                    progressed = true;
                } else if message.kind == MessageKind::SessionOutput {
                    if let Some(bytes) = message.as_bytes() {
                        let count = bytes.len().min(command_response.len() - command_response_len);
                        command_response[command_response_len..command_response_len + count]
                            .copy_from_slice(&bytes[..count]);
                        command_response_len += count;
                        if message.flags & IPC_FLAG_MORE == 0 {
                            let mut result = logos_session::ShellOutput::new();
                            session.command_output(
                                &command_response[..command_response_len],
                                &mut result,
                            );
                            pending.stage(result.as_bytes());
                            command_response_len = 0;
                            waiting_for_command = false;
                        }
                        progressed = true;
                    }
                }
            }
            if !progressed {
                common::wait(
                    common::ipc_read_event(logos_abi::IpcEndpointId::FlowToSession),
                    logos_abi::ServiceId::Session,
                );
            }
            continue;
        }
        let mut completion_message = IpcBytes::empty(MessageKind::CompletionResponse);
        if common::ipc_receive(FLOW_OUTPUT_CAPABILITY, &mut completion_message) == IpcStatus::Ok {
            if let Some(response) = completion_response(&completion_message) {
                let mut edit_output = logos_session::ShellOutput::new();
                session.apply_completion_response(response, &mut edit_output);
                if edit_output.len > 0 {
                    pending.stage(edit_output.as_bytes());
                }
                progressed = true;
            }
        }
        let mut message = IpcBytes::empty(MessageKind::SessionInput);
        while common::ipc_receive(INPUT_CAPABILITY, &mut message) == IpcStatus::Ok {
            progressed = true;
            if message.kind != MessageKind::SessionInput {
                continue;
            }
            let Some(bytes) = message.as_bytes() else {
                continue;
            };
            let mut edit_output = logos_session::ShellOutput::new();
            if let Some(length) =
                session.input_for_command(bytes, &mut command_bytes, &mut edit_output)
            {
                flow_input.stage(&command_bytes[..length]);
            }
            if let Some(request) = session.take_completion_request() {
                pending_completion = Some(completion_request_message(request));
            }
            if edit_output.len > 0 {
                pending.stage(edit_output.as_bytes());
                break;
            }
        }
        let mut completion_output = logos_session::ShellOutput::new();
        session.completion_tick(&mut completion_output);
        if completion_output.len > 0 {
            pending.stage(completion_output.as_bytes());
        }
        if !progressed {
            let mut wait_mask = common::ipc_read_event(logos_abi::IpcEndpointId::TerminalToSession);
            if session.completion_pending() {
                wait_mask |= common::ipc_read_event(logos_abi::IpcEndpointId::FlowToSession);
            }
            common::wait(wait_mask, logos_abi::ServiceId::Session);
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
