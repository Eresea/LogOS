#![no_std]
#![no_main]

mod common;

use logos_abi::{
    IPC_PAGE_BYTES, IpcBytes, MAX_IPC_BYTES, MessageKind, SERVICE_IPC_BASE, StreamIpc,
};

const TERMINAL_TO_SESSION: usize = SERVICE_IPC_BASE + IPC_PAGE_BYTES * 2;
const SESSION_TO_TERMINAL: usize = SERVICE_IPC_BASE + IPC_PAGE_BYTES * 3;

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

static mut SESSION: logos_session::SessionService = logos_session::SessionService::new();
static mut PENDING: PendingOutput = PendingOutput::new();

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    let session = unsafe { &mut *core::ptr::addr_of_mut!(SESSION) };
    let pending = unsafe { &mut *core::ptr::addr_of_mut!(PENDING) };
    let input = unsafe { &*(TERMINAL_TO_SESSION as *const StreamIpc) };
    let output = unsafe { &*(SESSION_TO_TERMINAL as *const StreamIpc) };
    let input_identity = input.endpoint().identity();
    let output_identity = output.endpoint().identity();
    let mut prompt = logos_session::ShellOutput::new();
    session.prompt(&mut prompt);
    pending.stage(prompt.as_bytes());
    loop {
        let mut progressed = pending.flush(output, output_identity);
        if !pending.is_empty() {
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
            if let Some(result) = session.input_bytes(bytes) {
                pending.stage(result.as_bytes());
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
