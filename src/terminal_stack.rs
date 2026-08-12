//! Deterministic end-to-end service graph used by host and QEMU proofs.

use crate::{
    display::{Display, DisplayError},
    input::{DecodedInput, InputDecoder},
    ipc::BoundedQueue,
    session::{Session, ShellOutput},
    terminal::Terminal,
    terminal_abi::{
        EndpointHeader, IPC_RING_SLOTS, InputMessage, MessageKind, RenderMessage, StreamMessage,
    },
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StackError {
    InputBackpressure,
    TerminalBackpressure,
    SessionBackpressure,
    Display(DisplayError),
}

impl From<DisplayError> for StackError {
    fn from(error: DisplayError) -> Self {
        Self::Display(error)
    }
}

pub struct TerminalStack {
    pub input: InputDecoder,
    pub terminal: Terminal,
    pub session: Session,
    pub display: Display,
    input_to_terminal: BoundedQueue<InputMessage, IPC_RING_SLOTS>,
    terminal_to_session: BoundedQueue<StreamMessage, IPC_RING_SLOTS>,
    session_to_terminal: BoundedQueue<StreamMessage, IPC_RING_SLOTS>,
    terminal_to_display: BoundedQueue<RenderMessage, IPC_RING_SLOTS>,
    pub input_endpoint: EndpointHeader,
    pub terminal_endpoint: EndpointHeader,
    pub session_endpoint: EndpointHeader,
    pub display_endpoint: EndpointHeader,
}

impl TerminalStack {
    pub const fn new() -> Self {
        Self {
            input: InputDecoder::new(),
            terminal: Terminal::new(),
            session: Session::new(),
            display: Display::new(1),
            input_to_terminal: BoundedQueue::new(),
            terminal_to_session: BoundedQueue::new(),
            session_to_terminal: BoundedQueue::new(),
            terminal_to_display: BoundedQueue::new(),
            input_endpoint: EndpointHeader::new(1, 1),
            terminal_endpoint: EndpointHeader::new(1, 1),
            session_endpoint: EndpointHeader::new(1, 1),
            display_endpoint: EndpointHeader::new(1, 1),
        }
    }

    pub fn boot(&mut self) -> Result<(), StackError> {
        self.session_prompt()?;
        self.pump()
    }

    pub fn feed_keyboard(&mut self, bytes: &[u8]) -> Result<(), StackError> {
        for &byte in bytes {
            if let Some(DecodedInput { key, text }) = self.input.feed(byte) {
                self.input_to_terminal.send(key).map_err(|_| StackError::InputBackpressure)?;
                if let Some(text) = text {
                    self.input_to_terminal.send(text).map_err(|_| StackError::InputBackpressure)?;
                }
            }
            self.pump()?;
        }
        Ok(())
    }

    pub fn feed_session_output(&mut self, bytes: &[u8]) -> Result<(), StackError> {
        let message = StreamMessage::from_bytes(MessageKind::SessionOutput, bytes)
            .ok_or(StackError::SessionBackpressure)?;
        self.session_to_terminal.send(message).map_err(|_| StackError::SessionBackpressure)?;
        self.pump()
    }

    pub fn restart_terminal(&mut self) -> Result<(), StackError> {
        self.terminal_endpoint.generation =
            self.terminal_endpoint.generation.wrapping_add(1).max(1);
        self.terminal_endpoint.service_epoch =
            self.terminal_endpoint.service_epoch.wrapping_add(1).max(1);
        self.terminal.reset();
        self.display.replace_generation(self.terminal_endpoint.generation);
        self.display_endpoint.generation = self.terminal_endpoint.generation;
        self.display_endpoint.service_epoch = self.terminal_endpoint.service_epoch;
        self.session_prompt()?;
        self.pump()
    }

    pub fn pump(&mut self) -> Result<(), StackError> {
        while let Ok(input) = self.input_to_terminal.receive() {
            if !self
                .input_endpoint
                .accepts(self.input_endpoint.generation, self.input_endpoint.service_epoch)
            {
                continue;
            }
            if let Some(message) = self.terminal.input(&input) {
                self.terminal_to_session
                    .send(message)
                    .map_err(|_| StackError::TerminalBackpressure)?;
            }
        }
        while let Ok(message) = self.terminal_to_session.receive() {
            let Some(bytes) = message.as_bytes() else { continue };
            if let Some(output) = self.session.input(bytes) {
                self.queue_session_output(output)?;
            }
        }
        while let Ok(message) = self.session_to_terminal.receive() {
            let Some(bytes) = message.as_bytes() else { continue };
            self.terminal.feed(bytes);
        }
        while let Some(render) = self.terminal.next_render() {
            self.terminal_to_display.send(render).map_err(|_| StackError::TerminalBackpressure)?;
            let render =
                self.terminal_to_display.receive().map_err(|_| StackError::TerminalBackpressure)?;
            self.display.apply(self.display_endpoint.generation, &render)?;
        }
        Ok(())
    }

    fn session_prompt(&mut self) -> Result<(), StackError> {
        let mut output = ShellOutput::new();
        self.session.prompt(&mut output);
        self.queue_session_output(output)
    }

    fn queue_session_output(&mut self, output: ShellOutput) -> Result<(), StackError> {
        if output.clear_screen {
            self.terminal.reset();
        }
        if output.len == 0 {
            return Ok(());
        }
        let message = StreamMessage::from_bytes(MessageKind::SessionOutput, output.as_bytes())
            .ok_or(StackError::SessionBackpressure)?;
        self.session_to_terminal.send(message).map_err(|_| StackError::SessionBackpressure)
    }
}

impl Default for TerminalStack {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::ReceiveError;
    use std::sync::{Mutex, MutexGuard};

    static STACKS: [Mutex<TerminalStack>; 4] = [
        const { Mutex::new(TerminalStack::new()) },
        const { Mutex::new(TerminalStack::new()) },
        const { Mutex::new(TerminalStack::new()) },
        const { Mutex::new(TerminalStack::new()) },
    ];

    fn stack(index: usize) -> MutexGuard<'static, TerminalStack> {
        STACKS[index].lock().unwrap()
    }

    #[test]
    fn keyboard_session_terminal_display_path_is_end_to_end() {
        let mut stack = stack(0);
        stack.boot().unwrap();
        stack.feed_keyboard(&[0x24]).unwrap(); // e
        assert_eq!(stack.display.cell(7, 0).unwrap().codepoint, b'e' as u32);
        stack.feed_session_output(b"\x1b[31mOK").unwrap();
        assert_eq!(stack.display.cell(8, 0).unwrap().codepoint, b'O' as u32);
    }

    #[test]
    fn terminal_restart_rebinds_display_and_keeps_session_alive() {
        let mut stack = stack(1);
        stack.boot().unwrap();
        let old = stack.terminal_endpoint;
        stack.restart_terminal().unwrap();
        assert_ne!(old, stack.terminal_endpoint);
        assert_eq!(stack.display.generation(), stack.terminal_endpoint.generation);
        assert_eq!(stack.display.cell(0, 0).unwrap().codepoint, b'l' as u32);
    }

    #[test]
    fn stale_display_generation_is_not_accepted() {
        let mut stack = stack(2);
        stack.boot().unwrap();
        let old = stack.display_endpoint.generation;
        stack.restart_terminal().unwrap();
        assert_ne!(old, stack.display_endpoint.generation);
        assert_eq!(
            stack.display.apply(old, &RenderMessage::empty(MessageKind::RenderCells)),
            Err(DisplayError::StaleGeneration)
        );
    }

    #[test]
    fn empty_queues_report_empty_without_spin() {
        let mut stack = stack(3);
        assert_eq!(stack.input_to_terminal.receive(), Err(ReceiveError::Empty));
        assert!(stack.terminal_to_session.is_empty());
    }
}
