#![no_std]

//! Bounded command-line editing for the Session service.
//!
//! Commands are executed by the Commands service. Session only edits a line,
//! forwards completed input, and prepends the prompt to command output.

#[cfg(test)]
extern crate std;

use logos_abi::{MAX_HISTORY_BYTES, MAX_HISTORY_ENTRIES};

pub const MAX_LINE_BYTES: usize = 256;
pub const MAX_OUTPUT_BYTES: usize = 512;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ShellOutput {
    pub bytes: [u8; MAX_OUTPUT_BYTES],
    pub len: usize,
}

impl ShellOutput {
    pub const fn new() -> Self {
        Self { bytes: [0; MAX_OUTPUT_BYTES], len: 0 }
    }

    pub fn push(&mut self, byte: u8) {
        if self.len < self.bytes.len() {
            self.bytes[self.len] = byte;
            self.len += 1;
        }
    }

    pub fn extend(&mut self, bytes: &[u8]) {
        let count = bytes.len().min(self.bytes.len() - self.len);
        self.bytes[self.len..self.len + count].copy_from_slice(&bytes[..count]);
        self.len += count;
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

impl Default for ShellOutput {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy)]
struct HistoryEntry {
    bytes: [u8; MAX_HISTORY_BYTES],
    len: usize,
}

impl HistoryEntry {
    const EMPTY: Self = Self { bytes: [0; MAX_HISTORY_BYTES], len: 0 };
}

pub struct LineEditor {
    line: [u8; MAX_LINE_BYTES],
    line_len: usize,
    cursor: usize,
    history: [HistoryEntry; MAX_HISTORY_ENTRIES],
    history_len: usize,
    history_cursor: usize,
    escape_state: u8,
}

/// Entry-ready Session facade over one bounded line-editing operation.
pub struct SessionService {
    session: LineEditor,
}

impl SessionService {
    pub const fn new() -> Self {
        Self { session: LineEditor::new() }
    }

    pub fn prompt(&self, output: &mut ShellOutput) {
        self.session.prompt(output);
    }

    pub fn input_for_command(
        &mut self,
        bytes: &[u8],
        command: &mut [u8; MAX_LINE_BYTES],
        output: &mut ShellOutput,
    ) -> Option<usize> {
        self.session.input_for_command(bytes, command, output)
    }

    pub fn command_output(&self, bytes: &[u8], output: &mut ShellOutput) {
        output.extend(bytes);
        self.prompt(output);
    }
}

impl Default for SessionService {
    fn default() -> Self {
        Self::new()
    }
}

const _: () = assert!(core::mem::size_of::<SessionService>() <= logos_abi::MAX_SERVICE_IMAGE_BYTES);

impl LineEditor {
    pub const fn new() -> Self {
        Self {
            line: [0; MAX_LINE_BYTES],
            line_len: 0,
            cursor: 0,
            history: [HistoryEntry::EMPTY; MAX_HISTORY_ENTRIES],
            history_len: 0,
            history_cursor: 0,
            escape_state: 0,
        }
    }

    pub fn prompt(&self, output: &mut ShellOutput) {
        output.extend(b"logos> ");
    }

    pub fn input_for_command(
        &mut self,
        bytes: &[u8],
        command: &mut [u8; MAX_LINE_BYTES],
        output: &mut ShellOutput,
    ) -> Option<usize> {
        for &byte in bytes {
            if self.escape_state != 0 {
                match (self.escape_state, byte) {
                    (1, b'[') => self.escape_state = 2,
                    (2, b'A') => {
                        self.recall_history(true, output);
                        self.escape_state = 0;
                    }
                    (2, b'B') => {
                        self.recall_history(false, output);
                        self.escape_state = 0;
                    }
                    _ => self.escape_state = 0,
                }
                continue;
            }
            if byte == 0x1b {
                self.escape_state = 1;
                continue;
            }
            match byte {
                b'\r' | b'\n' => {
                    let line = self.line;
                    let length = self.line_len;
                    self.record_history(&line[..length]);
                    command[..length].copy_from_slice(&line[..length]);
                    self.line_len = 0;
                    self.cursor = 0;
                    output.extend(b"\r\n");
                    return Some(length);
                }
                0x7f | 0x08 => {
                    if self.cursor > 0 {
                        self.cursor -= 1;
                        self.line_len -= 1;
                        self.line.copy_within(self.cursor + 1..self.line_len + 1, self.cursor);
                        output.extend(b"\x08 \x08");
                    }
                }
                0x20..=0x7e => {
                    if self.line_len < MAX_LINE_BYTES {
                        self.line.copy_within(self.cursor..self.line_len, self.cursor + 1);
                        self.line[self.cursor] = byte;
                        self.cursor += 1;
                        self.line_len += 1;
                        output.push(byte);
                    }
                }
                _ => {}
            }
        }
        None
    }

    fn record_history(&mut self, line: &[u8]) {
        if line.is_empty() {
            return;
        }
        let index = self.history_len.min(MAX_HISTORY_ENTRIES - 1);
        if self.history_len == MAX_HISTORY_ENTRIES {
            self.history.copy_within(1.., 0);
        }
        let entry = &mut self.history[index];
        entry.len = line.len().min(MAX_HISTORY_BYTES);
        entry.bytes[..entry.len].copy_from_slice(&line[..entry.len]);
        self.history_len = (self.history_len + 1).min(MAX_HISTORY_ENTRIES);
        self.history_cursor = self.history_len;
    }

    fn recall_history(&mut self, previous: bool, output: &mut ShellOutput) {
        if self.history_len == 0 {
            return;
        }
        if previous {
            self.history_cursor = self.history_cursor.saturating_sub(1);
        } else if self.history_cursor < self.history_len {
            self.history_cursor += 1;
        }
        self.line_len = 0;
        self.cursor = 0;
        if self.history_cursor < self.history_len {
            let entry = self.history[self.history_cursor];
            self.line[..entry.len].copy_from_slice(&entry.bytes[..entry.len]);
            self.line_len = entry.len;
            self.cursor = entry.len;
        }
        output.extend(b"\r\x1b[2Klogos> ");
        output.extend(&self.line[..self.line_len]);
    }
}

impl Default for LineEditor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_input_commits_on_enter() {
        let mut editor = LineEditor::new();
        let mut command = [0; MAX_LINE_BYTES];
        let mut output = ShellOutput::new();
        assert_eq!(editor.input_for_command(b"echo hi", &mut command, &mut output), None);
        assert_eq!(editor.input_for_command(b"\r", &mut command, &mut output), Some(7));
        assert_eq!(&command[..7], b"echo hi");
    }

    #[test]
    fn history_navigation_is_bounded_and_redraws_the_line() {
        let mut editor = LineEditor::new();
        let mut command = [0; MAX_LINE_BYTES];
        let mut output = ShellOutput::new();
        editor.input_for_command(b"first\r", &mut command, &mut output);
        editor.input_for_command(b"second\r", &mut command, &mut output);
        output = ShellOutput::new();
        editor.input_for_command(b"\x1b[A", &mut command, &mut output);
        assert!(output.as_bytes().windows(6).any(|window| window == b"second"));
        output = ShellOutput::new();
        editor.input_for_command(b"\x1b[B", &mut command, &mut output);
        assert!(output.as_bytes().windows(7).any(|window| window == b"logos> "));
    }

    #[test]
    fn command_output_is_followed_by_a_prompt() {
        let service = SessionService::new();
        let mut output = ShellOutput::new();
        service.command_output(b"ok\r\n", &mut output);
        assert_eq!(output.as_bytes(), b"ok\r\nlogos> ");
    }
}
