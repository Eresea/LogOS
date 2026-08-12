#![no_std]

//! Bounded interactive session and POSIX-lite shell policy.

#[cfg(test)]
extern crate std;

use logos_abi::{
    MAX_CHILD_PROCESSES, MAX_HISTORY_BYTES, MAX_HISTORY_ENTRIES, MAX_PIPELINE_STAGES,
    MAX_VOLATILE_FILE_BYTES, MAX_VOLATILE_FILES,
};

pub const MAX_LINE_BYTES: usize = 256;
pub const MAX_TOKENS: usize = 32;
pub const MAX_TOKEN_BYTES: usize = 64;
pub const MAX_ENV_ENTRIES: usize = 16;
pub const MAX_ENV_BYTES: usize = 64;
/// Shell output is deliberately chunked below the IPC maximum so the
/// session's worst-case command path fits the existing fixed task stack.
pub const MAX_OUTPUT_BYTES: usize = 512;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ShellOutput {
    pub bytes: [u8; MAX_OUTPUT_BYTES],
    pub len: usize,
    pub status: u8,
    pub exit_requested: bool,
    pub clear_screen: bool,
}

impl ShellOutput {
    pub const fn new() -> Self {
        Self {
            bytes: [0; MAX_OUTPUT_BYTES],
            len: 0,
            status: 0,
            exit_requested: false,
            clear_screen: false,
        }
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

    pub fn line(&mut self, bytes: &[u8]) {
        self.extend(bytes);
        self.push(b'\r');
        self.push(b'\n');
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

#[derive(Clone, Copy)]
struct EnvEntry {
    name: [u8; MAX_ENV_BYTES],
    name_len: usize,
    value: [u8; MAX_ENV_BYTES],
    value_len: usize,
    valid: bool,
}

impl EnvEntry {
    const EMPTY: Self = Self {
        name: [0; MAX_ENV_BYTES],
        name_len: 0,
        value: [0; MAX_ENV_BYTES],
        value_len: 0,
        valid: false,
    };
}

#[derive(Clone, Copy)]
struct VolatileFile {
    name: [u8; MAX_ENV_BYTES],
    name_len: usize,
    bytes: [u8; MAX_VOLATILE_FILE_BYTES],
    len: usize,
    valid: bool,
}

impl VolatileFile {
    const EMPTY: Self = Self {
        name: [0; MAX_ENV_BYTES],
        name_len: 0,
        bytes: [0; MAX_VOLATILE_FILE_BYTES],
        len: 0,
        valid: false,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Token {
    bytes: [u8; MAX_TOKEN_BYTES],
    len: usize,
}

impl Token {
    const EMPTY: Self = Self { bytes: [0; MAX_TOKEN_BYTES], len: 0 };

    fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ChildState {
    Vacant,
    Running,
}

#[derive(Clone, Copy)]
struct Child {
    state: ChildState,
    pid: u16,
}

impl Child {
    const EMPTY: Self = Self { state: ChildState::Vacant, pid: 0 };
}

pub struct Session {
    line: [u8; MAX_LINE_BYTES],
    line_len: usize,
    cursor: usize,
    history: [HistoryEntry; MAX_HISTORY_ENTRIES],
    history_len: usize,
    history_cursor: usize,
    escape_state: u8,
    env: [EnvEntry; MAX_ENV_ENTRIES],
    files: [VolatileFile; MAX_VOLATILE_FILES],
    total_file_bytes: usize,
    children: [Child; MAX_CHILD_PROCESSES],
    next_pid: u16,
}

/// Entry-ready Session service façade over one bounded stream operation.
pub struct SessionService {
    session: Session,
}

impl SessionService {
    pub const fn new() -> Self {
        Self { session: Session::new() }
    }

    pub fn prompt(&self, output: &mut ShellOutput) {
        self.session.prompt(output);
    }

    pub fn input(&mut self, message: &logos_abi::StreamMessage) -> Option<ShellOutput> {
        self.session.input(message.as_bytes()?)
    }

    pub const fn session(&self) -> &Session {
        &self.session
    }
}

impl Default for SessionService {
    fn default() -> Self {
        Self::new()
    }
}

impl Session {
    pub const fn new() -> Self {
        Self {
            line: [0; MAX_LINE_BYTES],
            line_len: 0,
            cursor: 0,
            history: [HistoryEntry::EMPTY; MAX_HISTORY_ENTRIES],
            history_len: 0,
            history_cursor: 0,
            escape_state: 0,
            env: [EnvEntry::EMPTY; MAX_ENV_ENTRIES],
            files: [VolatileFile::EMPTY; MAX_VOLATILE_FILES],
            total_file_bytes: 0,
            children: [Child::EMPTY; MAX_CHILD_PROCESSES],
            next_pid: 1,
        }
    }

    pub const fn line_len(&self) -> usize {
        self.line_len
    }
    pub const fn child_capacity(&self) -> usize {
        MAX_CHILD_PROCESSES
    }
    pub const fn volatile_bytes(&self) -> usize {
        self.total_file_bytes
    }

    pub fn prompt(&self, output: &mut ShellOutput) {
        output.extend(b"logos> ");
    }

    /// Consume terminal input.  Printable bytes edit the line; Enter runs it.
    pub fn input(&mut self, bytes: &[u8]) -> Option<ShellOutput> {
        let mut output = ShellOutput::new();
        for &byte in bytes {
            if self.escape_state != 0 {
                match (self.escape_state, byte) {
                    (1, b'[') => self.escape_state = 2,
                    (2, b'A') => {
                        self.recall_history(true, &mut output);
                        self.escape_state = 0;
                    }
                    (2, b'B') => {
                        self.recall_history(false, &mut output);
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
                    self.line_len = 0;
                    self.cursor = 0;
                    output = self.execute(&line[..length]);
                    self.prompt(&mut output);
                    return Some(output);
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
        (output.len > 0).then_some(output)
    }

    pub fn execute(&mut self, line: &[u8]) -> ShellOutput {
        let mut output = ShellOutput::new();
        let mut tokens = [Token::EMPTY; MAX_TOKENS];
        let token_count = tokenize(line, &mut tokens);
        if token_count == 0 {
            return output;
        }
        let mut stages = [0usize; MAX_PIPELINE_STAGES];
        let mut stage_count = 1;
        for (index, token) in tokens.iter().take(token_count).enumerate() {
            if token.as_bytes() == b"|" {
                if stage_count == MAX_PIPELINE_STAGES {
                    output.status = 2;
                    output.line(b"pipeline too long");
                    return output;
                }
                stages[stage_count] = index + 1;
                stage_count += 1;
            }
        }
        let mut input = [0u8; MAX_OUTPUT_BYTES];
        let mut input_len = 0;
        for stage_index in 0..stage_count {
            let start = stages[stage_index];
            let stage_end = if stage_index + 1 < stage_count {
                stages[stage_index + 1] - 1
            } else {
                token_count
            };
            let mut stage_output = ShellOutput::new();
            self.run_command(&tokens[start..stage_end], &input[..input_len], &mut stage_output);
            input[..stage_output.len].copy_from_slice(stage_output.as_bytes());
            input_len = stage_output.len;
            output.status = stage_output.status;
            output.exit_requested |= stage_output.exit_requested;
            output.clear_screen |= stage_output.clear_screen;
        }
        output.extend(&input[..input_len]);
        output
    }

    fn run_command(&mut self, tokens: &[Token], input: &[u8], output: &mut ShellOutput) {
        if tokens.is_empty() {
            return;
        }
        let mut command_end = tokens.len();
        let mut redirect: Option<(&[u8], bool)> = None;
        for index in 1..tokens.len() {
            if tokens[index].as_bytes() == b">" || tokens[index].as_bytes() == b">>" {
                if index + 1 >= tokens.len() {
                    output.status = 2;
                    output.line(b"missing redirect target");
                    return;
                }
                command_end = index;
                redirect = Some((tokens[index + 1].as_bytes(), tokens[index].as_bytes() == b">>"));
                break;
            }
        }
        let command = tokens[0].as_bytes();
        match command {
            b"help" => output.line(b"help echo cat clear ps uptime env set exit"),
            b"echo" => {
                for (index, token) in tokens[1..command_end].iter().enumerate() {
                    if index > 0 {
                        output.push(b' ');
                    }
                    self.append_expanded(token.as_bytes(), output);
                }
                output.push(b'\r');
                output.push(b'\n');
            }
            b"cat" => {
                if command_end == 1 {
                    output.extend(input);
                } else {
                    for token in &tokens[1..command_end] {
                        if let Some(file) = self.find_file(token.as_bytes()) {
                            output.extend(&file.bytes[..file.len]);
                        } else {
                            output.status = 1;
                            output.line(b"cat: file not found");
                        }
                    }
                }
            }
            b"clear" => {
                output.clear_screen = true;
                output.extend(b"\x1b[2J\x1b[H");
            }
            b"ps" => {
                output.line(b"PID STATE COMMAND");
                for child in self.children.iter().filter(|child| child.state == ChildState::Running)
                {
                    output.line(&format_pid(child.pid));
                }
            }
            b"uptime" => output.line(b"uptime: bounded session clock"),
            b"env" => {
                for entry in self.env.iter().filter(|entry| entry.valid) {
                    output.extend(&entry.name[..entry.name_len]);
                    output.push(b'=');
                    output.line(&entry.value[..entry.value_len]);
                }
            }
            b"set" => {
                if command_end != 2 || !self.set_env(tokens[1].as_bytes()) {
                    output.status = 2;
                    output.line(b"set: expected NAME=VALUE");
                }
            }
            b"exit" => output.exit_requested = true,
            _ => {
                if command_end > 0 && self.allocate_child(output) {
                    output.status = 127;
                    output.extend(command);
                    output.line(b": command not found");
                    self.finish_child();
                }
            }
        }
        if let Some((name, append)) = redirect {
            let bytes = output.as_bytes();
            if !self.write_file(name, bytes, append) {
                *output = ShellOutput::new();
                output.status = 1;
                output.line(b"redirect: volatile storage full");
            } else {
                output.len = 0;
            }
        }
    }

    fn allocate_child(&mut self, output: &mut ShellOutput) -> bool {
        let Some(child) = self.children.iter_mut().find(|child| child.state == ChildState::Vacant)
        else {
            output.status = 1;
            output.line(b"process capacity exhausted");
            return false;
        };
        child.state = ChildState::Running;
        child.pid = self.next_pid;
        self.next_pid = self.next_pid.wrapping_add(1).max(1);
        true
    }

    fn finish_child(&mut self) {
        if let Some(child) =
            self.children.iter_mut().find(|child| child.state == ChildState::Running)
        {
            child.state = ChildState::Vacant;
        }
    }

    fn append_expanded(&self, bytes: &[u8], output: &mut ShellOutput) {
        if bytes.first() != Some(&b'$') {
            output.extend(bytes);
            return;
        }
        let name = &bytes[1..];
        if let Some(entry) =
            self.env.iter().find(|entry| entry.valid && entry.name[..entry.name_len] == *name)
        {
            output.extend(&entry.value[..entry.value_len]);
        }
    }

    fn set_env(&mut self, assignment: &[u8]) -> bool {
        let Some(separator) = assignment.iter().position(|&byte| byte == b'=') else {
            return false;
        };
        let name = &assignment[..separator];
        let value = &assignment[separator + 1..];
        if name.is_empty() || name.len() > MAX_ENV_BYTES || value.len() > MAX_ENV_BYTES {
            return false;
        }
        let mut slot = None;
        for (index, entry) in self.env.iter().enumerate() {
            if entry.valid && entry.name[..entry.name_len] == *name {
                slot = Some(index);
                break;
            }
        }
        if slot.is_none() {
            slot = self.env.iter().position(|entry| !entry.valid);
        }
        let Some(slot) = slot else {
            return false;
        };
        let entry = &mut self.env[slot];
        entry.name[..name.len()].copy_from_slice(name);
        entry.name_len = name.len();
        entry.value[..value.len()].copy_from_slice(value);
        entry.value_len = value.len();
        entry.valid = true;
        true
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

    fn find_file(&self, name: &[u8]) -> Option<&VolatileFile> {
        self.files.iter().find(|file| file.valid && file.name[..file.name_len] == *name)
    }

    fn write_file(&mut self, name: &[u8], bytes: &[u8], append: bool) -> bool {
        if name.is_empty() || name.len() > MAX_ENV_BYTES || bytes.len() > MAX_VOLATILE_FILE_BYTES {
            return false;
        }
        let index = if let Some((index, _)) = self
            .files
            .iter()
            .enumerate()
            .find(|(_, file)| file.valid && file.name[..file.name_len] == *name)
        {
            index
        } else if let Some((index, file)) =
            self.files.iter_mut().enumerate().find(|(_, file)| !file.valid)
        {
            file.valid = true;
            file.name_len = name.len();
            file.name[..name.len()].copy_from_slice(name);
            index
        } else {
            return false;
        };
        let old_file_len = self.files[index].len;
        let old_len = if append { old_file_len } else { 0 };
        let new_len = old_len.saturating_add(bytes.len());
        if new_len > MAX_VOLATILE_FILE_BYTES {
            return false;
        }
        let new_total = self.total_file_bytes.saturating_sub(old_file_len).saturating_add(new_len);
        if new_total > 512 * 1024 {
            return false;
        }
        let file = &mut self.files[index];
        file.bytes[old_len..new_len].copy_from_slice(bytes);
        file.len = new_len;
        self.total_file_bytes = new_total;
        true
    }
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

fn tokenize(line: &[u8], tokens: &mut [Token; MAX_TOKENS]) -> usize {
    let mut count = 0;
    let mut index = 0;
    while index < line.len() && count < MAX_TOKENS {
        while index < line.len() && line[index].is_ascii_whitespace() {
            index += 1;
        }
        if index == line.len() {
            break;
        }
        let mut token = Token::EMPTY;
        let mut quote = 0;
        while index < line.len() {
            let byte = line[index];
            if quote == 0 && byte.is_ascii_whitespace() {
                break;
            }
            if quote == 0 && (byte == b'|' || byte == b'>' || byte == b'<') {
                if token.len > 0 {
                    break;
                }
                token.bytes[0] = byte;
                token.len = 1;
                index += 1;
                if byte == b'>' && index < line.len() && line[index] == b'>' {
                    token.bytes[1] = b'>';
                    token.len = 2;
                    index += 1;
                }
                break;
            }
            if byte == b'\'' || byte == b'"' {
                if quote == 0 {
                    quote = byte;
                    index += 1;
                    continue;
                }
                if quote == byte {
                    quote = 0;
                    index += 1;
                    continue;
                }
            }
            if byte == b'\\' && index + 1 < line.len() {
                index += 1;
            }
            if token.len < MAX_TOKEN_BYTES {
                token.bytes[token.len] = line[index];
                token.len += 1;
            }
            index += 1;
        }
        tokens[count] = token;
        count += 1;
    }
    count
}

fn format_pid(pid: u16) -> [u8; 32] {
    let mut output = [b' '; 32];
    output[0] = b'0';
    output[1] = b'x';
    let mut value = pid as u32;
    for index in (2..10).rev() {
        let digit = (value & 0xf) as u8;
        output[index] = if digit < 10 { b'0' + digit } else { b'a' + digit - 10 };
        value >>= 4;
    }
    output[10..25].copy_from_slice(b" RUNNING command");
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_runs_builtins_and_tracks_history() {
        let mut session = Session::new();
        let output = session.execute(b"echo hello");
        assert_eq!(output.status, 0);
        assert_eq!(&output.as_bytes()[..7], b"hello\r\n");
        let output = session.execute(b"help");
        assert!(output.as_bytes().windows(4).any(|window| window == b"echo"));
        assert_eq!(session.execute(b"set NAME=value").status, 0);
        assert_eq!(session.execute(b"echo $NAME").as_bytes(), b"value\r\n");
    }

    #[test]
    fn pipelines_and_volatile_redirection_are_bounded() {
        let mut session = Session::new();
        let output = session.execute(b"echo hello > note");
        assert_eq!(output.len, 0);
        assert!(session.volatile_bytes() > 0);
        let output = session.execute(b"cat note");
        assert_eq!(output.as_bytes(), b"hello\r\n");
        let output = session.execute(b"echo hello | cat");
        assert_eq!(output.as_bytes(), b"hello\r\n");
    }

    #[test]
    fn process_capacity_and_exit_are_explicit() {
        let mut session = Session::new();
        let output = session.execute(b"not-a-command");
        assert_eq!(output.status, 127);
        let output = session.execute(b"exit");
        assert!(output.exit_requested);
    }

    #[test]
    fn line_input_commits_on_enter() {
        let mut session = Session::new();
        assert!(session.input(b"echo hi").is_some());
        let output = session.input(b"\r").unwrap();
        assert!(output.as_bytes().windows(2).any(|window| window == b"hi"));
    }

    #[test]
    fn history_navigation_is_bounded_and_redraws_the_line() {
        let mut session = Session::new();
        session.input(b"echo first\r");
        session.input(b"echo second\r");
        let output = session.input(b"\x1b[A").unwrap();
        assert!(output.as_bytes().windows(6).any(|window| window == b"second"));
        let output = session.input(b"\x1b[B").unwrap();
        assert!(output.as_bytes().windows(7).any(|window| window == b"logos> "));
    }
}
