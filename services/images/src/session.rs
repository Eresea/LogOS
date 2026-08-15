#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", no_main)]
#![cfg_attr(not(target_os = "none"), allow(dead_code, unused_imports, unused_variables))]

mod common;

use logos_abi::{IPC_FLAG_MORE, IpcBytes, IpcStatus, MAX_IPC_BYTES, MessageKind};
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
const COMMANDS_CAPABILITY: usize = common::capability_slot(
    logos_abi::ServiceId::Session,
    logos_abi::IpcEndpointId::SessionToCommands,
    logos_abi::IpcRights::Send,
);
const COMMAND_OUTPUT_CAPABILITY: usize = common::capability_slot(
    logos_abi::ServiceId::Session,
    logos_abi::IpcEndpointId::CommandsToSession,
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

struct PendingCommand {
    bytes: [u8; MAX_LINE_BYTES],
    len: usize,
    pending: bool,
}

impl PendingCommand {
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
static mut COMMAND: PendingCommand = PendingCommand::new();

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    let session = unsafe { &mut *core::ptr::addr_of_mut!(SESSION) };
    let pending = unsafe { &mut *core::ptr::addr_of_mut!(PENDING) };
    let command = unsafe { &mut *core::ptr::addr_of_mut!(COMMAND) };
    let mut command_bytes = [0; MAX_LINE_BYTES];
    let mut command_response = [0; logos_session::MAX_OUTPUT_BYTES];
    let mut command_response_len = 0;
    let mut waiting_for_command = false;
    let mut prompt = logos_session::ShellOutput::new();
    session.prompt(&mut prompt);
    pending.stage(prompt.as_bytes());
    let mut heartbeat_ticks = 0u16;
    loop {
        common::heartbeat_tick(&mut heartbeat_ticks, logos_abi::ServiceId::Session);
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
        if !command.is_empty() {
            if let Some(message) = command.take() {
                if common::ipc_send(COMMANDS_CAPABILITY, &message) == IpcStatus::Ok {
                    waiting_for_command = true;
                    progressed = true;
                } else {
                    command.stage(message.as_bytes().unwrap_or_default());
                }
            }
            if !progressed {
                common::wait(
                    common::ipc_write_event(logos_abi::IpcEndpointId::SessionToCommands),
                    logos_abi::ServiceId::Session,
                );
            }
            continue;
        }
        if waiting_for_command {
            let mut message = IpcBytes::empty(MessageKind::SessionOutput);
            if common::ipc_receive(COMMAND_OUTPUT_CAPABILITY, &mut message) == IpcStatus::Ok {
                if let Some(bytes) = message.as_bytes() {
                    let count = bytes.len().min(command_response.len() - command_response_len);
                    command_response[command_response_len..command_response_len + count]
                        .copy_from_slice(&bytes[..count]);
                    command_response_len += count;
                    if message.flags & IPC_FLAG_MORE == 0 {
                        let mut result = logos_session::ShellOutput::new();
                        session
                            .command_output(&command_response[..command_response_len], &mut result);
                        pending.stage(result.as_bytes());
                        command_response_len = 0;
                        waiting_for_command = false;
                    }
                    progressed = true;
                }
            }
            if !progressed {
                common::wait(
                    common::ipc_read_event(logos_abi::IpcEndpointId::CommandsToSession),
                    logos_abi::ServiceId::Session,
                );
            }
            continue;
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
                command.stage(&command_bytes[..length]);
            }
            if edit_output.len > 0 {
                pending.stage(edit_output.as_bytes());
                break;
            }
        }
        if !progressed {
            common::wait(
                common::ipc_read_event(logos_abi::IpcEndpointId::TerminalToSession),
                logos_abi::ServiceId::Session,
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
