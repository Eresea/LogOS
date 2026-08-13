#![no_std]
#![no_main]

mod common;

use logos_abi::{
    IPC_FLAG_MORE, IPC_PAGE_BYTES, IpcBytes, MAX_IPC_BYTES, MessageKind, SERVICE_IPC_BASE,
    StreamIpc,
};
use logos_session::MAX_LINE_BYTES;

const TERMINAL_TO_SESSION: usize = SERVICE_IPC_BASE + IPC_PAGE_BYTES * 2;
const SESSION_TO_TERMINAL: usize = SERVICE_IPC_BASE + IPC_PAGE_BYTES * 3;
const SESSION_TO_COMMANDS: usize = SERVICE_IPC_BASE + IPC_PAGE_BYTES * 4;
const COMMANDS_TO_SESSION: usize = SERVICE_IPC_BASE + IPC_PAGE_BYTES * 5;

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

    fn flush(&mut self, ring: &StreamIpc, identity: logos_abi::MessageIdentity) -> bool {
        let mut progressed = false;
        while self.offset < self.len {
            let end = (self.offset + MAX_IPC_BYTES).min(self.len);
            let Some(message) =
                IpcBytes::from_bytes(MessageKind::SessionOutput, &self.bytes[self.offset..end])
            else {
                break;
            };
            if ring.send(identity, message).is_err() {
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
    let input = unsafe { &*(TERMINAL_TO_SESSION as *const StreamIpc) };
    let output = unsafe { &*(SESSION_TO_TERMINAL as *const StreamIpc) };
    let commands = unsafe { &*(SESSION_TO_COMMANDS as *const StreamIpc) };
    let command_output = unsafe { &*(COMMANDS_TO_SESSION as *const StreamIpc) };
    let mut command_bytes = [0; MAX_LINE_BYTES];
    let mut command_response = [0; logos_session::MAX_OUTPUT_BYTES];
    let mut command_response_len = 0;
    let mut waiting_for_command = false;
    let mut prompt = logos_session::ShellOutput::new();
    session.prompt(&mut prompt);
    pending.stage(prompt.as_bytes());
    loop {
        let input_identity = input.endpoint().identity();
        let output_identity = output.endpoint().identity();
        let commands_identity = commands.endpoint().identity();
        let command_output_identity = command_output.endpoint().identity();
        let mut progressed = pending.flush(output, output_identity);
        if !pending.is_empty() {
            if !progressed {
                core::hint::spin_loop();
            }
            continue;
        }
        if !command.is_empty() {
            if let Some(message) = command.take() {
                if commands.send(commands_identity, message).is_ok() {
                    waiting_for_command = true;
                    progressed = true;
                } else {
                    command.stage(message.as_bytes().unwrap_or_default());
                }
            }
            if !progressed {
                core::hint::spin_loop();
            }
            continue;
        }
        if waiting_for_command {
            if let Ok(message) = command_output.receive(command_output_identity) {
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
                core::hint::spin_loop();
            }
            continue;
        }
        while let Ok(message) = input.receive(input_identity) {
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
            core::hint::spin_loop();
        }
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo<'_>) -> ! {
    common::idle()
}
