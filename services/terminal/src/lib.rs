#![no_std]

//! Bounded fixed-size terminal emulator.

#[cfg(test)]
extern crate std;

use logos_abi::{
    Cell, DEFAULT_COLUMNS, DEFAULT_ROWS, InputMessage, IpcBytes, KeyCode, KeyState,
    MAX_RENDER_CELLS, MOD_ALT, MOD_CAPS_LOCK, MOD_CTRL, MOD_SHIFT, MessageKind, RenderMessage,
};

const MAX_PARAMS: usize = 16;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ParserState {
    Ground,
    Escape,
    Csi,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Parser {
    state: ParserState,
    params: [u16; MAX_PARAMS],
    param_count: usize,
    current: u16,
    has_current: bool,
}

impl Parser {
    const fn new() -> Self {
        Self {
            state: ParserState::Ground,
            params: [0; MAX_PARAMS],
            param_count: 0,
            current: 0,
            has_current: false,
        }
    }

    fn reset_csi(&mut self) {
        self.param_count = 0;
        self.current = 0;
        self.has_current = false;
    }

    fn push_param(&mut self) {
        if self.param_count < MAX_PARAMS {
            self.params[self.param_count] = if self.has_current { self.current } else { 0 };
            self.param_count += 1;
        }
        self.current = 0;
        self.has_current = false;
    }

    fn param(&self, index: usize, default: u16) -> u16 {
        match self.params.get(index).copied() {
            Some(0) | None => default,
            Some(value) => value,
        }
    }
}

pub struct TerminalState<const CELL_COUNT: usize> {
    cursor_column: usize,
    cursor_row: usize,
    screen: [Cell; CELL_COUNT],
    dirty: [bool; CELL_COUNT],
    parser: Parser,
}

pub struct TerminalService {
    terminal: TerminalState<{ DEFAULT_COLUMNS * DEFAULT_ROWS }>,
}

const _: () =
    assert!(core::mem::size_of::<TerminalService>() <= logos_abi::MAX_SERVICE_IMAGE_BYTES);

impl TerminalService {
    pub const fn new() -> Self {
        Self { terminal: TerminalState::new() }
    }

    pub fn input(&self, event: &InputMessage) -> Option<IpcBytes> {
        self.terminal.input(event)
    }

    pub fn session_output(&mut self, message: &IpcBytes) {
        if let Some(bytes) = message.as_bytes() {
            self.terminal.feed(bytes);
        }
    }

    pub fn session_output_bytes(&mut self, bytes: &[u8]) {
        self.terminal.feed(bytes);
    }

    pub fn next_render(&mut self) -> Option<RenderMessage> {
        self.terminal.next_render()
    }
}

impl Default for TerminalService {
    fn default() -> Self {
        Self::new()
    }
}

impl<const CELL_COUNT: usize> TerminalState<CELL_COUNT> {
    pub const fn new() -> Self {
        assert!(DEFAULT_COLUMNS * DEFAULT_ROWS <= CELL_COUNT);
        Self {
            cursor_column: 0,
            cursor_row: 0,
            screen: [Cell::EMPTY; CELL_COUNT],
            dirty: [true; CELL_COUNT],
            parser: Parser::new(),
        }
    }

    pub const fn cursor(&self) -> (usize, usize) {
        (self.cursor_column, self.cursor_row)
    }

    pub fn reset(&mut self) {
        self.cursor_column = 0;
        self.cursor_row = 0;
        self.parser = Parser::new();
        self.screen.fill(Cell::EMPTY);
        self.mark_all_dirty();
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.feed_byte(byte);
        }
    }

    fn feed_byte(&mut self, byte: u8) {
        match self.parser.state {
            ParserState::Ground => match byte {
                0x1b => self.parser.state = ParserState::Escape,
                0x08 | 0x7f => self.cursor_column = self.cursor_column.saturating_sub(1),
                0x09 => {
                    self.cursor_column =
                        ((self.cursor_column / 8) + 1).saturating_mul(8).min(DEFAULT_COLUMNS - 1)
                }
                0x0a..=0x0c => self.line_feed(),
                0x0d => self.cursor_column = 0,
                0x20..=0x7e => self.put(byte as u32),
                _ => {}
            },
            ParserState::Escape => match byte {
                b'[' => {
                    self.parser.reset_csi();
                    self.parser.state = ParserState::Csi;
                }
                b'c' => self.reset(),
                _ => self.parser.state = ParserState::Ground,
            },
            ParserState::Csi => self.feed_csi(byte),
        }
    }

    fn feed_csi(&mut self, byte: u8) {
        match byte {
            b'0'..=b'9' => {
                self.parser.current =
                    self.parser.current.saturating_mul(10).saturating_add(u16::from(byte - b'0'));
                self.parser.has_current = true;
            }
            b';' => self.parser.push_param(),
            0x40..=0x7e => {
                if self.parser.has_current || self.parser.param_count > 0 {
                    self.parser.push_param();
                }
                self.dispatch_csi(byte);
                self.parser.state = ParserState::Ground;
            }
            _ => self.parser.state = ParserState::Ground,
        }
    }

    fn dispatch_csi(&mut self, final_byte: u8) {
        match final_byte {
            b'H' | b'f' => {
                self.cursor_row = self.parser.param(0, 1).saturating_sub(1) as usize;
                self.cursor_column = self.parser.param(1, 1).saturating_sub(1) as usize;
                self.cursor_row = self.cursor_row.min(DEFAULT_ROWS - 1);
                self.cursor_column = self.cursor_column.min(DEFAULT_COLUMNS - 1);
            }
            b'J' => self.erase_display(self.parser.param(0, 0)),
            b'K' => self.erase_line(self.parser.param(0, 0)),
            _ => {}
        }
    }

    fn index(column: usize, row: usize) -> usize {
        row * DEFAULT_COLUMNS + column
    }

    fn put(&mut self, codepoint: u32) {
        if self.cursor_column >= DEFAULT_COLUMNS {
            self.line_feed();
            self.cursor_column = 0;
        }
        let index = Self::index(self.cursor_column, self.cursor_row);
        self.screen[index] = Cell { codepoint, ..Cell::EMPTY };
        self.dirty[index] = true;
        self.cursor_column += 1;
    }

    fn line_feed(&mut self) {
        if self.cursor_row + 1 >= DEFAULT_ROWS {
            self.scroll_up();
        } else {
            self.cursor_row += 1;
        }
    }

    fn scroll_up(&mut self) {
        for row in 0..DEFAULT_ROWS - 1 {
            for column in 0..DEFAULT_COLUMNS {
                let source = Self::index(column, row + 1);
                let target = Self::index(column, row);
                self.screen[target] = self.screen[source];
                self.dirty[target] = true;
            }
        }
        for column in 0..DEFAULT_COLUMNS {
            let index = Self::index(column, DEFAULT_ROWS - 1);
            self.screen[index] = Cell::EMPTY;
            self.dirty[index] = true;
        }
    }

    fn erase_display(&mut self, mode: u16) {
        match mode {
            0 => {
                self.erase_line(0);
                for row in self.cursor_row + 1..DEFAULT_ROWS {
                    self.erase_row(row);
                }
            }
            1 => {
                for row in 0..self.cursor_row {
                    self.erase_row(row);
                }
                self.erase_line(1);
            }
            2 | 3 => {
                for row in 0..DEFAULT_ROWS {
                    self.erase_row(row);
                }
            }
            _ => {}
        }
    }

    fn erase_row(&mut self, row: usize) {
        for column in 0..DEFAULT_COLUMNS {
            let index = Self::index(column, row);
            self.screen[index] = Cell::EMPTY;
            self.dirty[index] = true;
        }
    }

    fn erase_line(&mut self, mode: u16) {
        let (start, end) = match mode {
            0 => (self.cursor_column, DEFAULT_COLUMNS),
            1 => (0, self.cursor_column + 1),
            2 => (0, DEFAULT_COLUMNS),
            _ => return,
        };
        for column in start..end.min(DEFAULT_COLUMNS) {
            let index = Self::index(column, self.cursor_row);
            self.screen[index] = Cell::EMPTY;
            self.dirty[index] = true;
        }
    }

    fn mark_all_dirty(&mut self) {
        for row in 0..DEFAULT_ROWS {
            for column in 0..DEFAULT_COLUMNS {
                self.dirty[Self::index(column, row)] = true;
            }
        }
    }

    /// Return at most 128 dirty cells; repeated calls drain the dirty set.
    pub fn next_render(&mut self) -> Option<RenderMessage> {
        let mut message = RenderMessage::empty(MessageKind::RenderCells);
        message.columns = DEFAULT_COLUMNS as u16;
        message.rows = DEFAULT_ROWS as u16;
        message.cursor_column = self.cursor_column as u16;
        message.cursor_row = self.cursor_row as u16;
        let mut count = 0;
        for row in 0..DEFAULT_ROWS {
            for column in 0..DEFAULT_COLUMNS {
                let index = Self::index(column, row);
                if self.dirty[index] && count < MAX_RENDER_CELLS {
                    message.positions[count] = index as u16;
                    message.cells[count] = self.screen[index];
                    self.dirty[index] = false;
                    count += 1;
                }
            }
        }
        if count == 0 {
            None
        } else {
            message.count = count as u16;
            Some(message)
        }
    }

    pub fn input(&self, event: &InputMessage) -> Option<IpcBytes> {
        if matches!(event.kind, MessageKind::Text | MessageKind::Paste) {
            return IpcBytes::from_bytes(MessageKind::SessionInput, event.text_bytes()?);
        }
        if event.kind != MessageKind::Key || event.state == KeyState::Released {
            return None;
        }
        let code = KeyCode::from_raw(event.code);
        if let Some(byte) = code.character_byte() {
            let byte = modified_character(byte, event.modifiers);
            if event.modifiers & MOD_CTRL != 0 {
                return IpcBytes::from_bytes(MessageKind::SessionInput, &[control_byte(byte)?]);
            }
            if event.modifiers & MOD_ALT != 0 {
                return IpcBytes::from_bytes(MessageKind::SessionInput, &[b'\x1b', byte]);
            }
        }
        let bytes: &[u8] = match code {
            KeyCode::Escape => b"\x1b",
            KeyCode::Enter => b"\r",
            KeyCode::Backspace => b"\x7f",
            KeyCode::Tab => b"\t",
            KeyCode::Up => b"\x1b[A",
            KeyCode::Down => b"\x1b[B",
            KeyCode::Left => b"\x1b[D",
            KeyCode::Right => b"\x1b[C",
            KeyCode::Home => b"\x1b[H",
            KeyCode::End => b"\x1b[F",
            KeyCode::Delete => b"\x1b[3~",
            KeyCode::PageUp => b"\x1b[5~",
            KeyCode::PageDown => b"\x1b[6~",
            _ => return None,
        };
        IpcBytes::from_bytes(MessageKind::SessionInput, bytes)
    }
}

fn modified_character(byte: u8, modifiers: u16) -> u8 {
    if byte.is_ascii_alphabetic() {
        let upper = (modifiers & MOD_SHIFT != 0) ^ (modifiers & MOD_CAPS_LOCK != 0);
        if upper { byte.to_ascii_uppercase() } else { byte.to_ascii_lowercase() }
    } else if modifiers & MOD_SHIFT != 0 {
        shifted_ascii(byte)
    } else {
        byte
    }
}

fn control_byte(byte: u8) -> Option<u8> {
    match byte {
        b'?' => Some(0x7f),
        b'a'..=b'z' | b'A'..=b'Z' => Some(byte.to_ascii_uppercase() & 0x1f),
        b' '..=b'_' => Some(byte & 0x1f),
        _ => None,
    }
}

const fn shifted_ascii(byte: u8) -> u8 {
    match byte {
        b'1' => b'!',
        b'2' => b'@',
        b'3' => b'#',
        b'4' => b'$',
        b'5' => b'%',
        b'6' => b'^',
        b'7' => b'&',
        b'8' => b'*',
        b'9' => b'(',
        b'0' => b')',
        b'-' => b'_',
        b'=' => b'+',
        b'[' => b'{',
        b']' => b'}',
        b';' => b':',
        b'\'' => b'"',
        b',' => b'<',
        b'.' => b'>',
        b'/' => b'?',
        b'`' => b'~',
        b'\\' => b'|',
        _ => byte,
    }
}

impl<const CELL_COUNT: usize> Default for TerminalState<CELL_COUNT> {
    fn default() -> Self {
        Self::new()
    }
}

pub type Terminal = TerminalState<{ DEFAULT_COLUMNS * DEFAULT_ROWS }>;

#[cfg(test)]
mod tests {
    use super::*;

    fn drain(terminal: &mut Terminal) -> usize {
        let mut count = 0;
        while terminal.next_render().is_some() {
            count += 1;
        }
        count
    }

    #[test]
    fn text_and_scroll_are_bounded() {
        let mut terminal = Terminal::new();
        drain(&mut terminal);
        terminal.feed(b"hello\nworld");
        assert!(drain(&mut terminal) > 0);
        terminal.feed(b"\x1b[2J");
        assert!(drain(&mut terminal) > 0);
    }

    #[test]
    fn cursor_and_erase_are_bounded() {
        let mut terminal = Terminal::new();
        terminal.feed(b"\x1b[2;4Hx\x1b[2K");
        assert_eq!(terminal.cursor(), (4, 1));
        assert!(drain(&mut terminal) > 0);
    }

    #[test]
    fn semantic_input_is_small_and_stable() {
        let terminal = Terminal::new();
        let event = InputMessage::key(KeyCode::Up, KeyState::Pressed, 0);
        assert_eq!(terminal.input(&event).unwrap().as_bytes(), Some(&b"\x1b[A"[..]));
    }

    #[test]
    fn render_is_chunked() {
        let mut terminal = Terminal::new();
        assert!(drain(&mut terminal) > 1);
        assert!(terminal.next_render().is_none());
    }
}
