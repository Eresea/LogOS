#![no_std]
#![no_main]

mod common;

use logos_abi::{
    IPC_FLAG_MORE, IPC_PAGE_BYTES, IpcBytes, MessageKind, SERVICE_IPC_BASE, StreamIpc,
};

const SESSION_TO_COMMANDS: usize = SERVICE_IPC_BASE + IPC_PAGE_BYTES * 4;
const COMMANDS_TO_SESSION: usize = SERVICE_IPC_BASE + IPC_PAGE_BYTES * 5;

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

    fn flush(&mut self, ring: &StreamIpc, identity: logos_abi::MessageIdentity) -> bool {
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
            if ring.send(identity, message).is_err() {
                break;
            }
            self.offset = end;
            progressed = true;
        }
        if self.pending && self.offset == self.len {
            let message = IpcBytes::empty(MessageKind::SessionOutput);
            if self.len == 0 && ring.send(identity, message).is_ok() {
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

static mut COMMANDS: logos_commands::CommandService = logos_commands::CommandService::new();
static mut PENDING: PendingOutput = PendingOutput::new();

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    let commands = unsafe { &mut *core::ptr::addr_of_mut!(COMMANDS) };
    let pending = unsafe { &mut *core::ptr::addr_of_mut!(PENDING) };
    let input = unsafe { &*(SESSION_TO_COMMANDS as *const StreamIpc) };
    let output = unsafe { &*(COMMANDS_TO_SESSION as *const StreamIpc) };
    let mut heartbeat_ticks = 0u16;
    loop {
        common::heartbeat_tick(&mut heartbeat_ticks, logos_abi::ServiceId::Commands);
        let input_identity = input.endpoint().identity();
        let output_identity = output.endpoint().identity();
        let mut progressed = pending.flush(output, output_identity);
        if pending.pending {
            if !progressed {
                core::hint::spin_loop();
            }
            continue;
        }
        if let Ok(message) = input.receive(input_identity) {
            progressed = true;
            if message.kind == MessageKind::SessionInput {
                if let Some(bytes) = message.as_bytes() {
                    let result = commands.execute(bytes);
                    if result.clear_screen {
                        pending.stage(b"\x1b[2J\x1b[H");
                    } else {
                        pending.stage(result.as_bytes());
                    }
                }
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
