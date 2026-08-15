#![no_std]

//! Bounded command-line editing for the Session service.
//!
//! Commands are executed by the Commands service. Session only edits a line,
//! forwards completed input, and prepends the prompt to command output.

#[cfg(test)]
extern crate std;

use logos_abi::{BUILTIN_COMMANDS, MAX_HISTORY_BYTES, MAX_HISTORY_ENTRIES};

pub const MAX_LINE_BYTES: usize = 256;
pub const MAX_OUTPUT_BYTES: usize = 512;
const PROMPT: &[u8] = b"\x1b[36mlogos\x1b[0m \x1b[33m>\x1b[0m ";

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

    fn push_decimal(&mut self, mut value: usize) {
        let mut digits = [0; 3];
        let mut count = 0;
        if value == 0 {
            self.push(b'0');
            return;
        }
        while value != 0 && count < digits.len() {
            digits[count] = b'0' + (value % 10) as u8;
            value /= 10;
            count += 1;
        }
        while count > 0 {
            count -= 1;
            self.push(digits[count]);
        }
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
    escape_param: u8,
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
            escape_param: 0,
        }
    }

    pub fn prompt(&self, output: &mut ShellOutput) {
        output.extend(PROMPT);
    }

    pub fn input_for_command(
        &mut self,
        bytes: &[u8],
        command: &mut [u8; MAX_LINE_BYTES],
        output: &mut ShellOutput,
    ) -> Option<usize> {
        let mut index = 0;
        while index < bytes.len() {
            let byte = bytes[index];
            if self.escape_state != 0 {
                self.feed_escape(byte, output);
                index += 1;
                continue;
            }
            let width = if byte >= 0x80 { utf8_sequence_width(&bytes[index..]) } else { 1 };
            if width > 1 {
                self.insert(&bytes[index..index + width], output);
                index += width;
                continue;
            }
            match byte {
                0x1b => self.escape_state = 1,
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
                0x01 => self.move_home(output),
                0x05 => self.move_end(output),
                0x0b => self.delete_to_end(output),
                0x0c => self.clear_screen(output),
                0x15 => self.clear_line(output),
                0x17 => self.delete_word(output),
                b'\t' => self.complete(output),
                0x08 | 0x7f => self.backspace(output),
                0x20..=0x7e | 0x80..=0xff => self.insert(&[byte], output),
                _ => {}
            }
            index += 1;
        }
        None
    }

    fn feed_escape(&mut self, byte: u8, output: &mut ShellOutput) {
        match self.escape_state {
            1 => {
                self.escape_state = if byte == b'[' { 2 } else { 0 };
            }
            2 => match byte {
                b'A' => {
                    self.recall_history(true, output);
                    self.escape_state = 0;
                }
                b'B' => {
                    self.recall_history(false, output);
                    self.escape_state = 0;
                }
                b'C' => {
                    self.move_right(output);
                    self.escape_state = 0;
                }
                b'D' => {
                    self.move_left(output);
                    self.escape_state = 0;
                }
                b'H' => {
                    self.move_home(output);
                    self.escape_state = 0;
                }
                b'F' => {
                    self.move_end(output);
                    self.escape_state = 0;
                }
                b'0'..=b'9' => {
                    self.escape_param = byte - b'0';
                    self.escape_state = 3;
                }
                _ => self.escape_state = 0,
            },
            3 => match byte {
                b'0'..=b'9' => {
                    self.escape_param =
                        self.escape_param.saturating_mul(10).saturating_add(byte - b'0');
                }
                b'~' => {
                    if self.escape_param == 3 {
                        self.delete_forward(output);
                    }
                    self.escape_state = 0;
                }
                _ => self.escape_state = 0,
            },
            _ => self.escape_state = 0,
        }
    }

    fn insert(&mut self, bytes: &[u8], output: &mut ShellOutput) {
        if bytes.len() > MAX_LINE_BYTES - self.line_len {
            return;
        }
        self.line.copy_within(self.cursor..self.line_len, self.cursor + bytes.len());
        self.line[self.cursor..self.cursor + bytes.len()].copy_from_slice(bytes);
        self.cursor += bytes.len();
        self.line_len += bytes.len();
        if self.cursor == self.line_len {
            output.extend(bytes);
        } else {
            self.redraw(output);
        }
    }

    fn complete(&mut self, output: &mut ShellOutput) {
        if self.cursor != self.line_len || self.line[..self.cursor].contains(&b' ') {
            return;
        }
        let prefix = &self.line[..self.cursor];
        let mut matches = [0; BUILTIN_COMMANDS.len()];
        let mut count = 0;
        for (index, candidate) in BUILTIN_COMMANDS.iter().enumerate() {
            if candidate.starts_with(prefix) {
                matches[count] = index;
                count += 1;
            }
        }
        match count {
            0 => {}
            1 => {
                let candidate = BUILTIN_COMMANDS[matches[0]];
                let suffix = &candidate[prefix.len()..];
                if self.line_len + suffix.len() <= MAX_LINE_BYTES {
                    self.line[self.line_len..self.line_len + suffix.len()].copy_from_slice(suffix);
                    self.line_len += suffix.len();
                    self.cursor = self.line_len;
                    output.extend(suffix);
                }
            }
            _ => {
                output.extend(b"\r\n");
                for index in 0..count {
                    if index > 0 {
                        output.extend(b"  ");
                    }
                    output.extend(BUILTIN_COMMANDS[matches[index]]);
                }
                output.extend(b"\r\n");
                self.redraw(output);
            }
        }
    }

    fn backspace(&mut self, output: &mut ShellOutput) {
        if self.cursor == 0 {
            return;
        }
        let start = previous_boundary(&self.line, self.cursor);
        self.line.copy_within(self.cursor..self.line_len, start);
        self.line_len -= self.cursor - start;
        self.cursor = start;
        self.redraw(output);
    }

    fn delete_forward(&mut self, output: &mut ShellOutput) {
        if self.cursor >= self.line_len {
            return;
        }
        let end = next_boundary(&self.line, self.cursor, self.line_len);
        self.line.copy_within(end..self.line_len, self.cursor);
        self.line_len -= end - self.cursor;
        self.redraw(output);
    }

    fn delete_to_end(&mut self, output: &mut ShellOutput) {
        self.line_len = self.cursor;
        self.redraw(output);
    }

    fn clear_line(&mut self, output: &mut ShellOutput) {
        self.line_len = 0;
        self.cursor = 0;
        self.redraw(output);
    }

    fn delete_word(&mut self, output: &mut ShellOutput) {
        let end = self.cursor;
        while self.cursor > 0 && self.line[self.cursor - 1] == b' ' {
            self.cursor -= 1;
        }
        while self.cursor > 0 && self.line[self.cursor - 1] != b' ' {
            self.cursor = previous_boundary(&self.line, self.cursor);
        }
        self.line.copy_within(end..self.line_len, self.cursor);
        self.line_len -= end - self.cursor;
        self.redraw(output);
    }

    fn move_left(&mut self, output: &mut ShellOutput) {
        if self.cursor > 0 {
            self.cursor = previous_boundary(&self.line, self.cursor);
            output.extend(b"\x1b[D");
        }
    }

    fn move_right(&mut self, output: &mut ShellOutput) {
        if self.cursor < self.line_len {
            self.cursor = next_boundary(&self.line, self.cursor, self.line_len);
            output.extend(b"\x1b[C");
        }
    }

    fn move_home(&mut self, output: &mut ShellOutput) {
        let distance = display_width(&self.line[..self.cursor]);
        self.cursor = 0;
        move_cursor(output, b'D', distance);
    }

    fn move_end(&mut self, output: &mut ShellOutput) {
        let distance = display_width(&self.line[self.cursor..self.line_len]);
        self.cursor = self.line_len;
        move_cursor(output, b'C', distance);
    }

    fn clear_screen(&mut self, output: &mut ShellOutput) {
        output.extend(b"\x1b[2J\x1b[H");
        self.redraw(output);
    }

    fn redraw(&self, output: &mut ShellOutput) {
        output.push(b'\r');
        self.prompt(output);
        output.extend(&self.line[..self.line_len]);
        output.extend(b"\x1b[K");
        move_cursor(output, b'D', display_width(&self.line[self.cursor..self.line_len]));
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
        self.redraw(output);
    }
}

fn move_cursor(output: &mut ShellOutput, direction: u8, distance: usize) {
    if distance == 0 {
        return;
    }
    output.extend(b"\x1b[");
    output.push_decimal(distance);
    output.push(direction);
}

fn is_utf8_continuation(byte: u8) -> bool {
    byte & 0xc0 == 0x80
}

fn utf8_sequence_width(bytes: &[u8]) -> usize {
    let Some(&first) = bytes.first() else { return 0 };
    let width = match first {
        0xc2..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf4 => 4,
        _ => return 1,
    };
    if bytes.len() < width || core::str::from_utf8(&bytes[..width]).is_err() { 1 } else { width }
}

fn previous_boundary(bytes: &[u8], index: usize) -> usize {
    let mut index = index.saturating_sub(1);
    while index > 0 && is_utf8_continuation(bytes[index]) {
        index -= 1;
    }
    index
}

fn next_boundary(bytes: &[u8], index: usize, len: usize) -> usize {
    let mut index = (index + 1).min(len);
    while index < len && is_utf8_continuation(bytes[index]) {
        index += 1;
    }
    index
}

fn display_width(bytes: &[u8]) -> usize {
    bytes.iter().filter(|byte| !is_utf8_continuation(**byte)).count()
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
        assert!(output.as_bytes().windows(5).any(|window| window == b"logos"));
    }

    #[test]
    fn command_output_is_followed_by_a_prompt() {
        let service = SessionService::new();
        let mut output = ShellOutput::new();
        service.command_output(b"ok\r\n", &mut output);
        assert_eq!(output.as_bytes(), b"ok\r\n\x1b[36mlogos\x1b[0m \x1b[33m>\x1b[0m ");
    }

    #[test]
    fn line_editor_supports_cursor_editing_and_controls() {
        let mut editor = LineEditor::new();
        let mut command = [0; MAX_LINE_BYTES];
        let mut output = ShellOutput::new();
        editor.input_for_command(b"ac\x1b[Db\r", &mut command, &mut output);
        assert_eq!(&command[..3], b"abc");

        output = ShellOutput::new();
        let length =
            editor.input_for_command(b"hello world\x17\r", &mut command, &mut output).unwrap();
        assert_eq!(&command[..length], b"hello ");
    }

    #[test]
    fn line_editor_preserves_utf8_and_edits_by_character() {
        let mut editor = LineEditor::new();
        let mut command = [0; MAX_LINE_BYTES];
        let mut output = ShellOutput::new();
        let length =
            editor.input_for_command(b"echo \xc3\xa9\r", &mut command, &mut output).unwrap();
        assert_eq!(&command[..length], b"echo \xc3\xa9");

        editor.input_for_command(b"\xc3\xa9", &mut command, &mut output);
        let length = editor.input_for_command(b"\x7f\r", &mut command, &mut output).unwrap();
        assert_eq!(length, 0);
    }

    #[test]
    fn line_editor_rejects_utf8_that_would_exceed_byte_limit() {
        let mut editor = LineEditor::new();
        let mut command = [0; MAX_LINE_BYTES];
        let mut output = ShellOutput::new();
        let prefix = [b'a'; MAX_LINE_BYTES - 1];
        editor.input_for_command(&prefix, &mut command, &mut output);
        let length = editor.input_for_command(b"\xc3\xa9\r", &mut command, &mut output).unwrap();
        assert_eq!(length, MAX_LINE_BYTES - 1);
        assert_eq!(&command[..length], &prefix);
    }

    #[test]
    fn tab_completes_builtins_and_lists_all_at_empty_prompt() {
        let mut editor = LineEditor::new();
        let mut command = [0; MAX_LINE_BYTES];
        let mut output = ShellOutput::new();
        let length = editor.input_for_command(b"he\t\r", &mut command, &mut output).unwrap();
        assert_eq!(&command[..length], b"help");

        let mut editor = LineEditor::new();
        output = ShellOutput::new();
        editor.input_for_command(b"\t", &mut command, &mut output);
        assert!(output.as_bytes().windows(4).any(|window| window == b"help"));
        assert!(output.as_bytes().windows(7).any(|window| window == b"version"));
    }
}
