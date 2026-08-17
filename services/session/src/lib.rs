#![no_std]

//! Bounded command-line editing for the Session service.
//!
//! Commands are executed by the Commands service. Session only edits a line,
//! forwards completed input, and prepends the prompt to command output.

#[cfg(test)]
extern crate std;

use logos_abi::{
    CompletionRequest, CompletionResponse, CompletionStatus, MAX_COMPLETION_CANDIDATES,
    MAX_COMPLETION_ITEM_BYTES, MAX_HISTORY_BYTES, MAX_HISTORY_ENTRIES,
};

pub const MAX_LINE_BYTES: usize = 256;
pub const MAX_OUTPUT_BYTES: usize = 512;
const PROMPT: &[u8] = b"\x1b[36mlogos\x1b[0m \x1b[33m>\x1b[0m ";
const COMMAND_COLOR: &[u8] = b"\x1b[36m";
const STRING_COLOR: &[u8] = b"\x1b[32m";
const RESET_COLOR: &[u8] = b"\x1b[0m";
const DIM_COLOR: &[u8] = b"\x1b[2m";
const COMPLETION_ERROR: &[u8] = b"\r\ncompletion unavailable\r\n";
const COMPLETION_TIMEOUT_TICKS: u8 = 32;

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

    pub fn push_decimal(&mut self, mut value: usize) {
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
    escape_modifier: u8,
    completion_enabled: bool,
    line_revision: u8,
    completion_requested: bool,
    completion_next_id: u16,
    completion_pending_id: Option<u16>,
    completion_pending_request: Option<CompletionRequest>,
    completion_wait: u8,
    completion_active: bool,
    completion_replace_start: usize,
    completion_replace_end: usize,
    completion_count: usize,
    completion_lengths: [u8; MAX_COMPLETION_CANDIDATES],
    completion_cursor_offsets: [u8; MAX_COMPLETION_CANDIDATES],
    completion_candidates: [[u8; MAX_COMPLETION_ITEM_BYTES]; MAX_COMPLETION_CANDIDATES],
    completion_selected: usize,
}

/// Entry-ready Session facade over one bounded line-editing operation.
pub struct SessionService {
    session: LineEditor,
}

impl SessionService {
    pub const fn new() -> Self {
        Self { session: LineEditor::new() }
    }

    pub const fn new_with_completion(enabled: bool) -> Self {
        Self { session: LineEditor::new_with_completion(enabled) }
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
        let reserve = PROMPT.len() + 2;
        let available = output.bytes.len().saturating_sub(output.len + reserve);
        output.extend(&bytes[..bytes.len().min(available)]);
        match output.as_bytes().last() {
            Some(b'\n') => {}
            Some(b'\r') => output.push(b'\n'),
            _ => output.extend(b"\r\n"),
        }
        self.prompt(output);
    }

    pub fn take_completion_request(&mut self) -> Option<CompletionRequest> {
        self.session.take_completion_request()
    }

    pub fn apply_completion_response(
        &mut self,
        response: CompletionResponse,
        output: &mut ShellOutput,
    ) {
        self.session.apply_completion_response(response, output);
    }

    pub fn completion_pending(&self) -> bool {
        self.session.completion_pending()
    }

    pub fn completion_tick(&mut self, output: &mut ShellOutput) {
        self.session.completion_tick(output);
    }

    pub fn completion_failed(&mut self, output: &mut ShellOutput) {
        self.session.completion_failed(output);
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
        Self::new_with_completion(true)
    }

    pub const fn new_with_completion(completion_enabled: bool) -> Self {
        Self {
            line: [0; MAX_LINE_BYTES],
            line_len: 0,
            cursor: 0,
            history: [HistoryEntry::EMPTY; MAX_HISTORY_ENTRIES],
            history_len: 0,
            history_cursor: 0,
            escape_state: 0,
            escape_param: 0,
            escape_modifier: 0,
            completion_enabled,
            line_revision: 0,
            completion_requested: false,
            completion_next_id: 1,
            completion_pending_id: None,
            completion_pending_request: None,
            completion_wait: 0,
            completion_active: false,
            completion_replace_start: 0,
            completion_replace_end: 0,
            completion_count: 0,
            completion_lengths: [0; MAX_COMPLETION_CANDIDATES],
            completion_cursor_offsets: [0; MAX_COMPLETION_CANDIDATES],
            completion_candidates: [[0; MAX_COMPLETION_ITEM_BYTES]; MAX_COMPLETION_CANDIDATES],
            completion_selected: 0,
        }
    }

    pub fn prompt(&self, output: &mut ShellOutput) {
        output.extend(PROMPT);
    }

    pub fn take_completion_request(&mut self) -> Option<CompletionRequest> {
        if !self.completion_requested || !self.completion_enabled {
            self.completion_requested = false;
            return None;
        }
        self.completion_requested = false;
        let request_id = self.completion_next_id;
        self.completion_next_id = self.completion_next_id.wrapping_add(1).max(1);
        let mut request =
            CompletionRequest::new(request_id, &self.line[..self.line_len], self.cursor)?;
        request.line_revision = self.line_revision;
        self.completion_pending_id = Some(request_id);
        self.completion_pending_request = Some(request);
        self.completion_wait = 0;
        Some(request)
    }

    pub fn completion_pending(&self) -> bool {
        self.completion_pending_id.is_some()
    }

    pub fn completion_tick(&mut self, output: &mut ShellOutput) {
        if self.completion_pending_id.is_none() {
            return;
        }
        self.completion_wait = self.completion_wait.saturating_add(1);
        if self.completion_wait >= COMPLETION_TIMEOUT_TICKS {
            self.completion_failed(output);
        }
    }

    pub fn completion_failed(&mut self, output: &mut ShellOutput) {
        self.completion_enabled = false;
        let had_menu = self.completion_active;
        self.clear_completion_state();
        if had_menu {
            self.redraw(output);
        }
        output.extend(COMPLETION_ERROR);
        self.redraw(output);
    }

    pub fn apply_completion_response(
        &mut self,
        response: CompletionResponse,
        output: &mut ShellOutput,
    ) {
        let Some(request) = self.completion_pending_request else { return };
        if self.completion_pending_id != Some(response.request_id) {
            return;
        }
        if !response.is_valid_for(request) {
            self.completion_failed(output);
            return;
        }
        self.completion_pending_id = None;
        self.completion_pending_request = None;
        self.completion_wait = 0;
        match response.status {
            CompletionStatus::Ok if response.candidate_count != 0 => {
                self.completion_replace_start = usize::from(response.replace_start);
                self.completion_replace_end = usize::from(response.replace_end);
                self.completion_count = usize::from(response.candidate_count);
                self.completion_lengths = response.lengths;
                self.completion_cursor_offsets = response.cursor_offsets;
                self.completion_candidates = response.candidates;
                self.completion_selected = 0;
                self.completion_active = true;
                self.redraw(output);
            }
            CompletionStatus::NoMatch => {
                self.dismiss_completion(output);
            }
            CompletionStatus::Unavailable | CompletionStatus::Malformed => {
                self.completion_enabled = false;
                self.completion_failed(output);
            }
            CompletionStatus::Ok => {}
        }
    }

    fn request_completion(&mut self) {
        if self.completion_enabled && self.completion_pending_id.is_none() {
            self.completion_requested = true;
        }
    }

    fn clear_completion_state(&mut self) {
        self.completion_requested = false;
        self.completion_pending_id = None;
        self.completion_pending_request = None;
        self.completion_wait = 0;
        self.completion_active = false;
        self.completion_count = 0;
        self.completion_selected = 0;
    }

    fn dismiss_completion(&mut self, output: &mut ShellOutput) {
        let had_menu = self.completion_active;
        self.clear_completion_state();
        if had_menu {
            self.redraw(output);
        }
    }

    fn select_completion(&mut self, direction: isize, output: &mut ShellOutput) {
        if self.completion_count == 0 {
            return;
        }
        self.completion_selected = if direction < 0 {
            if self.completion_selected == 0 {
                self.completion_count - 1
            } else {
                self.completion_selected - 1
            }
        } else {
            (self.completion_selected + 1) % self.completion_count
        };
        self.redraw(output);
    }

    fn accept_completion(&mut self, output: &mut ShellOutput) {
        let Some(candidate) = self.completion_candidate(self.completion_selected) else {
            return;
        };
        let mut replacement = [0; MAX_COMPLETION_ITEM_BYTES];
        replacement[..candidate.len()].copy_from_slice(candidate);
        let replacement_len = candidate.len();
        let start = self.completion_replace_start.min(self.line_len);
        let end = self.completion_replace_end.min(self.line_len).max(start);
        let Some(new_len) = start
            .checked_add(replacement_len)
            .and_then(|length| length.checked_add(self.line_len - end))
        else {
            return;
        };
        if new_len > MAX_LINE_BYTES {
            return;
        }
        self.line.copy_within(end..self.line_len, start + replacement_len);
        self.line[start..start + replacement_len].copy_from_slice(&replacement[..replacement_len]);
        self.line_len = new_len;
        self.cursor = start
            + usize::from(self.completion_cursor_offsets[self.completion_selected])
                .min(replacement_len);
        self.bump_line_revision();
        self.clear_completion_state();
        self.redraw(output);
    }

    fn completion_candidate(&self, index: usize) -> Option<&[u8]> {
        if index >= self.completion_count || index >= MAX_COMPLETION_CANDIDATES {
            return None;
        }
        let length = usize::from(self.completion_lengths[index]);
        (length <= MAX_COMPLETION_ITEM_BYTES).then(|| &self.completion_candidates[index][..length])
    }

    pub fn input_for_command(
        &mut self,
        bytes: &[u8],
        command: &mut [u8; MAX_LINE_BYTES],
        output: &mut ShellOutput,
    ) -> Option<usize> {
        let mut index = 0;
        while index < bytes.len() {
            let line_revision = self.line_revision;
            let byte = bytes[index];
            if self.completion_active {
                if bytes[index..].starts_with(b"\x1b[A") {
                    self.select_completion(-1, output);
                    index += 3;
                    continue;
                }
                if bytes[index..].starts_with(b"\x1b[B") {
                    self.select_completion(1, output);
                    index += 3;
                    continue;
                }
                if byte == b'\t' {
                    self.accept_completion(output);
                    self.request_completion();
                    index += 1;
                    continue;
                }
                if byte == 0x1b {
                    self.dismiss_completion(output);
                    index += 1;
                    continue;
                }
                self.dismiss_completion(output);
            } else if self.completion_pending_id.is_some() && byte != b'\t' {
                self.clear_completion_state();
            }
            if self.escape_state != 0 {
                self.feed_escape(byte, output);
                if self.line_revision != line_revision {
                    self.request_completion();
                }
                index += 1;
                continue;
            }
            let width = if byte >= 0x80 { utf8_sequence_width(&bytes[index..]) } else { 1 };
            if width > 1 {
                self.insert(&bytes[index..index + width], output);
                self.request_completion();
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
                    self.clear_completion_state();
                    self.bump_line_revision();
                    output.extend(b"\r\n");
                    return Some(length);
                }
                0x01 => self.move_home(output),
                0x05 => self.move_end(output),
                0x0b => self.delete_to_end(output),
                0x0c => self.clear_screen(output),
                0x15 => self.clear_line(output),
                0x17 => self.delete_word(output),
                b'\t' => self.request_completion(),
                0x08 | 0x7f => self.backspace(output),
                0x20..=0x7e | 0x80..=0xff => self.insert(&[byte], output),
                _ => {}
            }
            if self.line_revision != line_revision {
                self.request_completion();
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
                b';' => {
                    self.escape_modifier = 0;
                    self.escape_state = 4;
                }
                b'~' => {
                    if self.escape_param == 3 {
                        self.delete_forward(output);
                    }
                    self.escape_state = 0;
                }
                _ => self.escape_state = 0,
            },
            4 => match byte {
                b'0'..=b'9' => {
                    self.escape_modifier =
                        self.escape_modifier.saturating_mul(10).saturating_add(byte - b'0');
                }
                b'C' if self.escape_param == 1 && self.escape_modifier == 5 => {
                    self.move_word_right(output);
                    self.escape_state = 0;
                }
                b'D' if self.escape_param == 1 && self.escape_modifier == 5 => {
                    self.move_word_left(output);
                    self.escape_state = 0;
                }
                b'~' if self.escape_param == 3 && self.escape_modifier == 5 => {
                    self.delete_word_forward(output);
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
        self.bump_line_revision();
        self.redraw(output);
    }

    fn backspace(&mut self, output: &mut ShellOutput) {
        if self.cursor == 0 {
            return;
        }
        let start = previous_boundary(&self.line, self.cursor);
        self.line.copy_within(self.cursor..self.line_len, start);
        self.line_len -= self.cursor - start;
        self.cursor = start;
        self.bump_line_revision();
        self.redraw(output);
    }

    fn delete_forward(&mut self, output: &mut ShellOutput) {
        if self.cursor >= self.line_len {
            return;
        }
        let end = next_boundary(&self.line, self.cursor, self.line_len);
        self.line.copy_within(end..self.line_len, self.cursor);
        self.line_len -= end - self.cursor;
        self.bump_line_revision();
        self.redraw(output);
    }

    fn delete_to_end(&mut self, output: &mut ShellOutput) {
        self.line_len = self.cursor;
        self.bump_line_revision();
        self.redraw(output);
    }

    fn clear_line(&mut self, output: &mut ShellOutput) {
        self.line_len = 0;
        self.cursor = 0;
        self.bump_line_revision();
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
        self.bump_line_revision();
        self.redraw(output);
    }

    fn delete_word_forward(&mut self, output: &mut ShellOutput) {
        let start = self.cursor;
        while self.cursor < self.line_len && self.line[self.cursor] == b' ' {
            self.cursor += 1;
        }
        while self.cursor < self.line_len && self.line[self.cursor] != b' ' {
            self.cursor = next_boundary(&self.line, self.cursor, self.line_len);
        }
        self.line.copy_within(self.cursor..self.line_len, start);
        self.line_len -= self.cursor - start;
        self.cursor = start;
        self.bump_line_revision();
        self.redraw(output);
    }

    fn bump_line_revision(&mut self) {
        self.line_revision = self.line_revision.wrapping_add(1);
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

    fn move_word_left(&mut self, output: &mut ShellOutput) {
        let old_cursor = self.cursor;
        while self.cursor > 0 && self.line[self.cursor - 1] == b' ' {
            self.cursor -= 1;
        }
        while self.cursor > 0 && self.line[self.cursor - 1] != b' ' {
            self.cursor = previous_boundary(&self.line, self.cursor);
        }
        move_cursor(output, b'D', display_width(&self.line[self.cursor..old_cursor]));
    }

    fn move_word_right(&mut self, output: &mut ShellOutput) {
        let old_cursor = self.cursor;
        while self.cursor < self.line_len && self.line[self.cursor] == b' ' {
            self.cursor += 1;
        }
        while self.cursor < self.line_len && self.line[self.cursor] != b' ' {
            self.cursor = next_boundary(&self.line, self.cursor, self.line_len);
        }
        move_cursor(output, b'C', display_width(&self.line[old_cursor..self.cursor]));
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

    fn redraw(&mut self, output: &mut ShellOutput) {
        output.push(b'\r');
        self.prompt(output);
        self.highlight_line(output);
        let ghost_width = self.append_completion_ghost(output);
        output.extend(RESET_COLOR);
        output.extend(b"\x1b[K");
        move_cursor(
            output,
            b'D',
            display_width(&self.line[self.cursor..self.line_len]) + ghost_width,
        );
    }

    fn append_completion_ghost(&self, output: &mut ShellOutput) -> usize {
        let Some(candidate) = self.completion_candidate(self.completion_selected) else {
            return 0;
        };
        let start = self.completion_replace_start.min(self.line_len);
        let end = self.completion_replace_end.min(self.line_len).max(start);
        let prefix = &self.line[start..end];
        if !candidate.starts_with(prefix) {
            return 0;
        }
        let ghost = &candidate[prefix.len()..];
        output.extend(DIM_COLOR);
        output.extend(ghost);
        output.extend(RESET_COLOR);
        display_width(ghost)
    }

    fn highlight_line(&self, output: &mut ShellOutput) {
        let command_len = command_name_len(&self.line[..self.line_len]);
        let extra = syntax_extra_bytes(&self.line[..self.line_len], command_len);
        let reserve = RESET_COLOR.len() + 3 + 8;
        let available = output.bytes.len().saturating_sub(output.len + reserve);
        if self.line_len + extra > available {
            output.extend(&self.line[..self.line_len]);
            return;
        }

        let mut index = 0;
        if command_len != 0 {
            output.extend(COMMAND_COLOR);
            output.extend(&self.line[..command_len]);
            output.extend(RESET_COLOR);
            index = command_len;
        }
        while index < self.line_len {
            if self.line[index] == b'"' {
                let start = index;
                index += 1;
                while index < self.line_len {
                    let end = self.line[index] == b'"';
                    index += 1;
                    if end {
                        break;
                    }
                }
                output.extend(STRING_COLOR);
                output.extend(&self.line[start..index]);
                output.extend(RESET_COLOR);
            } else {
                output.push(self.line[index]);
                index += 1;
            }
        }
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
        self.bump_line_revision();
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

fn command_name_len(bytes: &[u8]) -> usize {
    if !bytes.first().is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'_') {
        return 0;
    }
    bytes
        .iter()
        .position(|byte| !(byte.is_ascii_alphanumeric() || *byte == b'_'))
        .unwrap_or(bytes.len())
}

fn syntax_extra_bytes(bytes: &[u8], command_len: usize) -> usize {
    let mut extra = if command_len == 0 { 0 } else { COMMAND_COLOR.len() + RESET_COLOR.len() };
    let mut index = command_len;
    while index < bytes.len() {
        if bytes[index] == b'"' {
            extra += STRING_COLOR.len() + RESET_COLOR.len();
            index += 1;
            while index < bytes.len() {
                let end = bytes[index] == b'"';
                index += 1;
                if end {
                    break;
                }
            }
        } else {
            index += 1;
        }
    }
    extra
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
    fn command_output_reserves_newline_and_prompt() {
        let service = SessionService::new();
        let mut output = ShellOutput::new();
        service.command_output(&[b'x'; MAX_OUTPUT_BYTES], &mut output);
        assert!(output.as_bytes().ends_with(PROMPT));
        assert!(output.as_bytes().windows(2).any(|window| window == b"\r\n"));
    }

    #[test]
    fn line_editor_highlights_command_names_and_strings() {
        let mut editor = LineEditor::new();
        let mut command = [0; MAX_LINE_BYTES];
        let mut output = ShellOutput::new();
        editor.input_for_command(b"echo(\"hi\")", &mut command, &mut output);
        assert!(
            output.as_bytes().windows(COMMAND_COLOR.len()).any(|window| window == COMMAND_COLOR)
        );
        assert!(output.as_bytes().windows(STRING_COLOR.len()).any(|window| window == STRING_COLOR));
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
    fn word_navigation_and_home_end_are_bounded() {
        let mut editor = LineEditor::new();
        let mut command = [0; MAX_LINE_BYTES];
        let mut output = ShellOutput::new();
        editor.input_for_command(b"one two three", &mut command, &mut output);
        assert_eq!(editor.cursor, 13);

        editor.input_for_command(b"\x1b[1;5D", &mut command, &mut output);
        assert_eq!(editor.cursor, 8);
        editor.input_for_command(b"\x1b[1;5D", &mut command, &mut output);
        assert_eq!(editor.cursor, 4);
        editor.input_for_command(b"\x1b[1;5C", &mut command, &mut output);
        assert_eq!(editor.cursor, 7);

        editor.input_for_command(b"\x1b[H", &mut command, &mut output);
        assert_eq!(editor.cursor, 0);
        editor.input_for_command(b"\x1b[F", &mut command, &mut output);
        assert_eq!(editor.cursor, 13);
    }

    #[test]
    fn ctrl_delete_removes_the_forward_word() {
        let mut editor = LineEditor::new();
        let mut command = [0; MAX_LINE_BYTES];
        let mut output = ShellOutput::new();
        editor.input_for_command(b"one two three\x1b[H\x1b[3;5~\r", &mut command, &mut output);
        assert_eq!(&command[..10], b" two three");
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
    fn proactive_completion_requests_follow_line_edits() {
        let mut editor = LineEditor::new();
        let mut command = [0; MAX_LINE_BYTES];
        let mut output = ShellOutput::new();

        editor.input_for_command(b"he", &mut command, &mut output);
        let first = editor.take_completion_request().unwrap();
        assert_eq!(first.line(), Some(&b"he"[..]));

        editor.input_for_command(b"l", &mut command, &mut output);
        let second = editor.take_completion_request().unwrap();
        assert_ne!(second.request_id, first.request_id);
        assert_ne!(second.line_revision, first.line_revision);
        assert_eq!(second.line(), Some(&b"hel"[..]));

        editor.input_for_command(b"p\r", &mut command, &mut output);
        assert!(editor.take_completion_request().is_none());
    }

    #[test]
    fn targeted_completion_renders_ghost_and_accepts_with_tab() {
        let mut editor = LineEditor::new();
        let mut command = [0; MAX_LINE_BYTES];
        let mut output = ShellOutput::new();
        editor.input_for_command(b"he\t", &mut command, &mut output);
        let request = editor.take_completion_request().unwrap();
        let mut response = CompletionResponse::empty(request.request_id, CompletionStatus::Ok);
        response.line_revision = request.line_revision;
        response.replace_end = 2;
        assert!(response.push_candidate(b"help()"));
        editor.apply_completion_response(response, &mut output);
        assert!(output.as_bytes().windows(DIM_COLOR.len()).any(|window| window == DIM_COLOR));
        assert!(!output.as_bytes().windows(4).any(|window| window == b"\x1b[1B"));

        output = ShellOutput::new();
        editor.input_for_command(b"\t\r", &mut command, &mut output);
        assert_eq!(&command[..6], b"help()");
    }

    #[test]
    fn completion_accepts_string_argument_with_inner_cursor() {
        let mut editor = LineEditor::new();
        let mut command = [0; MAX_LINE_BYTES];
        let mut output = ShellOutput::new();
        editor.input_for_command(b"echo", &mut command, &mut output);
        let request = editor.take_completion_request().unwrap();
        let mut response = CompletionResponse::empty(request.request_id, CompletionStatus::Ok);
        response.line_revision = request.line_revision;
        response.replace_end = 4;
        assert!(response.push_candidate_with_cursor(b"echo(\"\")", 6));
        editor.apply_completion_response(response, &mut output);
        editor.input_for_command(b"\t", &mut command, &mut output);
        assert_eq!(&editor.line[..editor.line_len], b"echo(\"\")");
        assert_eq!(editor.cursor, 6);
    }

    #[test]
    fn completion_accepts_multi_argument_helper_in_first_slot() {
        let mut editor = LineEditor::new();
        let mut command = [0; MAX_LINE_BYTES];
        let mut output = ShellOutput::new();
        editor.input_for_command(b"write", &mut command, &mut output);
        let request = editor.take_completion_request().unwrap();
        let mut response = CompletionResponse::empty(request.request_id, CompletionStatus::Ok);
        response.line_revision = request.line_revision;
        response.replace_end = 5;
        assert!(response.push_candidate_with_cursor(b"write(\"\", \"\")", 7));
        editor.apply_completion_response(response, &mut output);
        editor.input_for_command(b"\t", &mut command, &mut output);
        assert_eq!(&editor.line[..editor.line_len], b"write(\"\", \"\")");
        assert_eq!(editor.cursor, 7);
    }

    #[test]
    fn targeted_completion_navigates_and_dismisses() {
        let mut editor = LineEditor::new();
        let mut command = [0; MAX_LINE_BYTES];
        let mut output = ShellOutput::new();
        editor.input_for_command(b"net.", &mut command, &mut output);
        editor.input_for_command(b"\t", &mut command, &mut output);
        let request = editor.take_completion_request().unwrap();
        let mut response = CompletionResponse::empty(request.request_id, CompletionStatus::Ok);
        response.line_revision = request.line_revision;
        response.replace_start = 4;
        response.replace_end = 4;
        assert!(response.push_candidate(b"status"));
        assert!(response.push_candidate(b"ping()"));
        editor.apply_completion_response(response, &mut output);
        editor.input_for_command(b"\x1b[B\t", &mut command, &mut output);
        assert_eq!(&editor.line[..editor.line_len], b"net.ping()");

        editor.input_for_command(b"\t", &mut command, &mut output);
        let request = editor.take_completion_request().unwrap();
        let mut response = CompletionResponse::empty(request.request_id, CompletionStatus::Ok);
        response.line_revision = request.line_revision;
        response.replace_start = 10;
        response.replace_end = 10;
        assert!(response.push_candidate(b"status"));
        editor.apply_completion_response(response, &mut output);
        editor.input_for_command(b"\x1b", &mut command, &mut output);
        assert_eq!(&editor.line[..editor.line_len], b"net.ping()");
    }

    #[test]
    fn completion_failure_disables_provider_after_one_diagnostic() {
        let mut editor = LineEditor::new();
        let mut command = [0; MAX_LINE_BYTES];
        let mut output = ShellOutput::new();
        editor.input_for_command(b"he\t", &mut command, &mut output);
        let request = editor.take_completion_request().unwrap();
        let mut response =
            CompletionResponse::empty(request.request_id, CompletionStatus::Unavailable);
        response.line_revision = request.line_revision;
        editor.apply_completion_response(response, &mut output);
        assert!(
            output
                .as_bytes()
                .windows(b"completion unavailable".len())
                .any(|window| { window == b"completion unavailable" })
        );
        output = ShellOutput::new();
        editor.input_for_command(b"\t", &mut command, &mut output);
        assert!(editor.take_completion_request().is_none());
    }
}
