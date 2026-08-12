#![no_std]

//! Bounded VT-style terminal emulator.

#[cfg(test)]
extern crate std;

use logos_abi::{
    Cell, InputMessage, KeyCode, KeyState, MAX_COLUMNS, MAX_RENDER_CELLS, MAX_ROWS,
    MAX_SCROLLBACK_LINES, MOD_ALT, MOD_CAPS_LOCK, MOD_CTRL, MOD_SHIFT, MessageKind, RenderMessage,
    StreamMessage,
};

const ATTR_BOLD: u16 = 1 << 0;
const ATTR_UNDERLINE: u16 = 1 << 1;
const ATTR_REVERSE: u16 = 1 << 2;
const ATTR_DIM: u16 = 1 << 3;
const DEFAULT_FG: u32 = 0x00ff_ffff;
const DEFAULT_BG: u32 = 0;
const MAX_PARAMS: usize = 16;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ParserState {
    Ground,
    Escape,
    Csi,
    Osc,
    Utf8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalError {
    InvalidSize,
    RenderFull,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Style {
    foreground: u32,
    background: u32,
    attributes: u16,
}

impl Style {
    const DEFAULT: Self = Self { foreground: DEFAULT_FG, background: DEFAULT_BG, attributes: 0 };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerminalModes {
    pub alternate_screen: bool,
    pub application_cursor: bool,
    pub bracketed_paste: bool,
    pub insert: bool,
    pub origin: bool,
    pub wrap: bool,
}

impl TerminalModes {
    const DEFAULT: Self = Self {
        alternate_screen: false,
        application_cursor: false,
        bracketed_paste: false,
        insert: false,
        origin: false,
        wrap: true,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Parser {
    state: ParserState,
    params: [u16; MAX_PARAMS],
    param_count: usize,
    current: u16,
    has_current: bool,
    private: bool,
    utf8_codepoint: u32,
    utf8_min: u32,
    utf8_remaining: u8,
    osc_len: usize,
    osc: [u8; 128],
}

impl Parser {
    const fn new() -> Self {
        Self {
            state: ParserState::Ground,
            params: [0; MAX_PARAMS],
            param_count: 0,
            current: 0,
            has_current: false,
            private: false,
            utf8_codepoint: 0,
            utf8_min: 0,
            utf8_remaining: 0,
            osc_len: 0,
            osc: [0; 128],
        }
    }

    fn reset_csi(&mut self) {
        self.param_count = 0;
        self.current = 0;
        self.has_current = false;
        self.private = false;
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

pub struct Terminal {
    columns: usize,
    rows: usize,
    cursor_column: usize,
    cursor_row: usize,
    saved_column: usize,
    saved_row: usize,
    style: Style,
    saved_style: Style,
    modes: TerminalModes,
    screen: [Cell; MAX_COLUMNS * MAX_ROWS],
    alternate: [Cell; MAX_COLUMNS * MAX_ROWS],
    dirty: [bool; MAX_COLUMNS * MAX_ROWS],
    parser: Parser,
    scrollback_lines: usize,
    bell: bool,
    title: [u8; 128],
    title_len: usize,
}

impl Terminal {
    pub const fn new() -> Self {
        Self {
            columns: logos_abi::DEFAULT_COLUMNS,
            rows: logos_abi::DEFAULT_ROWS,
            cursor_column: 0,
            cursor_row: 0,
            saved_column: 0,
            saved_row: 0,
            style: Style::DEFAULT,
            saved_style: Style::DEFAULT,
            modes: TerminalModes::DEFAULT,
            screen: [Cell::EMPTY; MAX_COLUMNS * MAX_ROWS],
            alternate: [Cell::EMPTY; MAX_COLUMNS * MAX_ROWS],
            dirty: [true; MAX_COLUMNS * MAX_ROWS],
            parser: Parser::new(),
            scrollback_lines: 0,
            bell: false,
            title: [0; 128],
            title_len: 0,
        }
    }

    pub const fn columns(&self) -> usize {
        self.columns
    }
    pub const fn rows(&self) -> usize {
        self.rows
    }
    pub const fn cursor(&self) -> (usize, usize) {
        (self.cursor_column, self.cursor_row)
    }
    pub const fn modes(&self) -> TerminalModes {
        self.modes
    }
    pub const fn scrollback_lines(&self) -> usize {
        self.scrollback_lines
    }
    pub fn title(&self) -> &[u8] {
        &self.title[..self.title_len]
    }

    pub fn resize(&mut self, columns: usize, rows: usize) -> Result<(), TerminalError> {
        if !(1..=MAX_COLUMNS).contains(&columns) || !(1..=MAX_ROWS).contains(&rows) {
            return Err(TerminalError::InvalidSize);
        }
        self.columns = columns;
        self.rows = rows;
        self.cursor_column = self.cursor_column.min(columns - 1);
        self.cursor_row = self.cursor_row.min(rows - 1);
        self.mark_all_dirty();
        Ok(())
    }

    pub fn reset(&mut self) {
        self.columns = logos_abi::DEFAULT_COLUMNS;
        self.rows = logos_abi::DEFAULT_ROWS;
        self.cursor_column = 0;
        self.cursor_row = 0;
        self.saved_column = 0;
        self.saved_row = 0;
        self.style = Style::DEFAULT;
        self.saved_style = Style::DEFAULT;
        self.modes = TerminalModes::DEFAULT;
        self.parser = Parser::new();
        self.scrollback_lines = 0;
        self.bell = false;
        self.title_len = 0;
        self.screen.fill(Cell::EMPTY);
        self.alternate.fill(Cell::EMPTY);
        self.mark_all_dirty();
    }

    pub fn take_bell(&mut self) -> bool {
        let bell = self.bell;
        self.bell = false;
        bell
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.feed_byte(byte);
        }
    }

    fn feed_byte(&mut self, byte: u8) {
        if self.parser.state == ParserState::Utf8 {
            self.feed_utf8(byte);
            return;
        }
        match self.parser.state {
            ParserState::Ground => match byte {
                0x1b => self.parser.state = ParserState::Escape,
                0x07 => self.bell = true,
                0x08 => self.cursor_column = self.cursor_column.saturating_sub(1),
                0x09 => {
                    self.cursor_column = ((self.cursor_column / 8) + 1).min(self.columns - 1) * 8
                }
                0x0a..=0x0c => self.line_feed(),
                0x0d => self.cursor_column = 0,
                0x20..=0x7e => self.put(byte as u32),
                0xc2..=0xdf => self.start_utf8(byte, 1, 0x80),
                0xe0..=0xef => self.start_utf8(byte, 2, 0x800),
                0xf0..=0xf4 => self.start_utf8(byte, 3, 0x10000),
                _ => {}
            },
            ParserState::Escape => match byte {
                b'[' => {
                    self.parser.reset_csi();
                    self.parser.state = ParserState::Csi;
                }
                b']' => {
                    self.parser.osc_len = 0;
                    self.parser.state = ParserState::Osc;
                }
                b'7' => {
                    self.saved_column = self.cursor_column;
                    self.saved_row = self.cursor_row;
                    self.saved_style = self.style;
                    self.parser.state = ParserState::Ground;
                }
                b'8' => {
                    self.cursor_column = self.saved_column.min(self.columns - 1);
                    self.cursor_row = self.saved_row.min(self.rows - 1);
                    self.style = self.saved_style;
                    self.parser.state = ParserState::Ground;
                }
                b'c' => {
                    self.reset();
                }
                _ => self.parser.state = ParserState::Ground,
            },
            ParserState::Csi => self.feed_csi(byte),
            ParserState::Osc => {
                if byte == 0x07 || byte == 0x1b {
                    self.finish_osc();
                    if byte == 0x1b {
                        self.parser.state = ParserState::Escape;
                    }
                } else if self.parser.osc_len < self.parser.osc.len() {
                    self.parser.osc[self.parser.osc_len] = byte;
                    self.parser.osc_len += 1;
                }
            }
            ParserState::Utf8 => unreachable!(),
        }
    }

    fn start_utf8(&mut self, byte: u8, remaining: u8, min: u32) {
        self.parser.state = ParserState::Utf8;
        self.parser.utf8_remaining = remaining;
        self.parser.utf8_min = min;
        self.parser.utf8_codepoint = (byte & (0x7f >> remaining)) as u32;
    }

    fn feed_utf8(&mut self, byte: u8) {
        if byte & 0xc0 != 0x80 {
            self.parser.state = ParserState::Ground;
            self.feed_byte(byte);
            return;
        }
        self.parser.utf8_codepoint = (self.parser.utf8_codepoint << 6) | u32::from(byte & 0x3f);
        self.parser.utf8_remaining -= 1;
        if self.parser.utf8_remaining == 0 {
            let codepoint = self.parser.utf8_codepoint;
            if codepoint >= self.parser.utf8_min
                && codepoint <= 0x10ffff
                && !(0xd800..=0xdfff).contains(&codepoint)
            {
                self.put(codepoint);
            } else {
                self.put(0xfffd);
            }
            self.parser.state = ParserState::Ground;
        }
    }

    fn feed_csi(&mut self, byte: u8) {
        match byte {
            b'?' if self.parser.param_count == 0 && !self.parser.has_current => {
                self.parser.private = true;
            }
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
        let first = self.parser.param(0, 1) as usize;
        match (self.parser.private, final_byte) {
            (false, b'A') => self.cursor_row = self.cursor_row.saturating_sub(first),
            (false, b'B') => self.cursor_row = (self.cursor_row + first).min(self.rows - 1),
            (false, b'C') | (false, b'a') => {
                self.cursor_column = (self.cursor_column + first).min(self.columns - 1)
            }
            (false, b'D') => self.cursor_column = self.cursor_column.saturating_sub(first),
            (false, b'G') | (false, b'`') => {
                self.cursor_column = first.saturating_sub(1).min(self.columns - 1)
            }
            (false, b'd') => self.cursor_row = first.saturating_sub(1).min(self.rows - 1),
            (false, b'H') | (false, b'f') => {
                self.cursor_row = self.parser.param(0, 1).saturating_sub(1) as usize;
                self.cursor_column = self.parser.param(1, 1).saturating_sub(1) as usize;
                self.cursor_row = self.cursor_row.min(self.rows - 1);
                self.cursor_column = self.cursor_column.min(self.columns - 1);
            }
            (false, b'J') => self.erase_display(self.parser.param(0, 0)),
            (false, b'K') => self.erase_line(self.parser.param(0, 0)),
            (false, b'P') => self.delete_chars(first),
            (false, b'@') => self.insert_chars(first),
            (false, b'L') => self.insert_lines(first),
            (false, b'M') => self.delete_lines(first),
            (false, b'm') => self.sgr(),
            (false, b'r') => self.cursor_row = self.cursor_row.min(self.rows - 1),
            (false, b's') => {
                self.saved_column = self.cursor_column;
                self.saved_row = self.cursor_row;
            }
            (false, b'u') => {
                self.cursor_column = self.saved_column.min(self.columns - 1);
                self.cursor_row = self.saved_row.min(self.rows - 1);
            }
            (false, b'n') => {}
            (true, b'h') | (true, b'l') => self.set_private_mode(final_byte == b'h'),
            _ => {}
        }
    }

    fn set_private_mode(&mut self, enabled: bool) {
        for index in 0..self.parser.param_count {
            match self.parser.params[index] {
                1 => self.modes.application_cursor = enabled,
                6 => self.modes.origin = enabled,
                25 => {}
                47 | 1047 | 1049 => {
                    self.modes.alternate_screen = enabled;
                    if enabled {
                        self.alternate.fill(Cell::EMPTY);
                    }
                    self.mark_all_dirty();
                }
                2004 => self.modes.bracketed_paste = enabled,
                _ => {}
            }
        }
    }

    fn sgr(&mut self) {
        if self.parser.param_count == 0 {
            self.style = Style::DEFAULT;
            return;
        }
        let mut index = 0;
        while index < self.parser.param_count {
            match self.parser.params[index] {
                0 => self.style = Style::DEFAULT,
                1 => self.style.attributes |= ATTR_BOLD,
                2 => self.style.attributes |= ATTR_DIM,
                4 => self.style.attributes |= ATTR_UNDERLINE,
                7 => self.style.attributes |= ATTR_REVERSE,
                22 => self.style.attributes &= !(ATTR_BOLD | ATTR_DIM),
                24 => self.style.attributes &= !ATTR_UNDERLINE,
                27 => self.style.attributes &= !ATTR_REVERSE,
                30..=37 => {
                    self.style.foreground = ansi_color(self.parser.params[index] - 30, false)
                }
                40..=47 => {
                    self.style.background = ansi_color(self.parser.params[index] - 40, false)
                }
                90..=97 => self.style.foreground = ansi_color(self.parser.params[index] - 90, true),
                100..=107 => {
                    self.style.background = ansi_color(self.parser.params[index] - 100, true)
                }
                39 => self.style.foreground = DEFAULT_FG,
                49 => self.style.background = DEFAULT_BG,
                38 | 48 => {
                    let is_foreground = self.parser.params[index] == 38;
                    if index + 2 < self.parser.param_count && self.parser.params[index + 1] == 5 {
                        let color = palette(self.parser.params[index + 2] as u8);
                        if is_foreground {
                            self.style.foreground = color
                        } else {
                            self.style.background = color
                        }
                        index += 2;
                    } else if index + 4 < self.parser.param_count
                        && self.parser.params[index + 1] == 2
                    {
                        let color = (u32::from(self.parser.params[index + 2]) << 16)
                            | (u32::from(self.parser.params[index + 3]) << 8)
                            | u32::from(self.parser.params[index + 4]);
                        if is_foreground {
                            self.style.foreground = color
                        } else {
                            self.style.background = color
                        }
                        index += 4;
                    }
                }
                _ => {}
            }
            index += 1;
        }
    }

    fn finish_osc(&mut self) {
        if self.parser.osc_len > 2
            && (self.parser.osc[0] == b'0' || self.parser.osc[0] == b'2')
            && self.parser.osc[1] == b';'
        {
            let length = (self.parser.osc_len - 2).min(self.title.len());
            self.title[..length].copy_from_slice(&self.parser.osc[2..2 + length]);
            self.title_len = length;
        }
        self.parser.osc_len = 0;
        self.parser.state = ParserState::Ground;
    }

    fn active(&mut self) -> &mut [Cell; MAX_COLUMNS * MAX_ROWS] {
        if self.modes.alternate_screen { &mut self.alternate } else { &mut self.screen }
    }

    fn index(&self, column: usize, row: usize) -> usize {
        row * MAX_COLUMNS + column
    }

    fn put(&mut self, codepoint: u32) {
        if self.cursor_column >= self.columns {
            if self.modes.wrap {
                self.line_feed();
                self.cursor_column = 0;
            } else {
                self.cursor_column = self.columns - 1;
            }
        }
        let index = self.index(self.cursor_column, self.cursor_row);
        let cell = Cell {
            codepoint,
            foreground: self.style.foreground,
            background: self.style.background,
            attributes: self.style.attributes,
            width: 1,
            reserved: 0,
        };
        if self.modes.insert {
            self.insert_chars(1);
        }
        self.active()[index] = cell;
        self.dirty[index] = true;
        self.cursor_column += 1;
    }

    fn line_feed(&mut self) {
        if self.cursor_row + 1 >= self.rows {
            self.scroll_up(1);
        } else {
            self.cursor_row += 1;
        }
    }

    fn scroll_up(&mut self, lines: usize) {
        let lines = lines.min(self.rows);
        for row in 0..self.rows - lines {
            for column in 0..self.columns {
                let source = self.index(column, row + lines);
                let target = self.index(column, row);
                let value = self.active()[source];
                self.active()[target] = value;
                self.dirty[target] = true;
            }
        }
        for row in self.rows - lines..self.rows {
            for column in 0..self.columns {
                let index = self.index(column, row);
                self.active()[index] = Cell::EMPTY;
                self.dirty[index] = true;
            }
        }
        if !self.modes.alternate_screen {
            self.scrollback_lines = (self.scrollback_lines + lines).min(MAX_SCROLLBACK_LINES);
        }
    }

    fn erase_display(&mut self, mode: u16) {
        match mode {
            0 => {
                self.erase_line(0);
                for row in self.cursor_row + 1..self.rows {
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
                for row in 0..self.rows {
                    self.erase_row(row);
                }
                if mode == 3 {
                    self.scrollback_lines = 0;
                }
            }
            _ => {}
        }
    }

    fn erase_row(&mut self, row: usize) {
        for column in 0..self.columns {
            let index = self.index(column, row);
            self.active()[index] = Cell::EMPTY;
            self.dirty[index] = true;
        }
    }

    fn erase_line(&mut self, mode: u16) {
        let (start, end) = match mode {
            0 => (self.cursor_column, self.columns),
            1 => (0, self.cursor_column + 1),
            2 => (0, self.columns),
            _ => return,
        };
        for column in start..end.min(self.columns) {
            let index = self.index(column, self.cursor_row);
            self.active()[index] = Cell::EMPTY;
            self.dirty[index] = true;
        }
    }

    fn insert_chars(&mut self, count: usize) {
        let cursor_column = self.cursor_column.min(self.columns);
        let count = count.min(self.columns.saturating_sub(cursor_column));
        for column in (cursor_column + count..self.columns).rev() {
            let from = self.index(column - count, self.cursor_row);
            let to = self.index(column, self.cursor_row);
            self.active()[to] = self.active()[from];
            self.dirty[to] = true;
        }
        for column in cursor_column..cursor_column + count {
            let index = self.index(column, self.cursor_row);
            self.active()[index] = Cell::EMPTY;
            self.dirty[index] = true;
        }
    }

    fn delete_chars(&mut self, count: usize) {
        let cursor_column = self.cursor_column.min(self.columns);
        let count = count.min(self.columns.saturating_sub(cursor_column));
        for column in cursor_column..self.columns - count {
            let from = self.index(column + count, self.cursor_row);
            let to = self.index(column, self.cursor_row);
            self.active()[to] = self.active()[from];
            self.dirty[to] = true;
        }
        for column in self.columns - count..self.columns {
            let index = self.index(column, self.cursor_row);
            self.active()[index] = Cell::EMPTY;
            self.dirty[index] = true;
        }
    }

    fn insert_lines(&mut self, count: usize) {
        let count = count.min(self.rows - self.cursor_row);
        for row in (self.cursor_row + count..self.rows).rev() {
            for column in 0..self.columns {
                let from = self.index(column, row - count);
                let to = self.index(column, row);
                self.active()[to] = self.active()[from];
                self.dirty[to] = true;
            }
        }
        for row in self.cursor_row..self.cursor_row + count {
            self.erase_row(row);
        }
    }

    fn delete_lines(&mut self, count: usize) {
        let count = count.min(self.rows - self.cursor_row);
        for row in self.cursor_row..self.rows - count {
            for column in 0..self.columns {
                let from = self.index(column, row + count);
                let to = self.index(column, row);
                self.active()[to] = self.active()[from];
                self.dirty[to] = true;
            }
        }
        for row in self.rows - count..self.rows {
            self.erase_row(row);
        }
    }

    fn mark_all_dirty(&mut self) {
        for row in 0..self.rows {
            for column in 0..self.columns {
                self.dirty[self.index(column, row)] = true;
            }
        }
    }

    /// Return at most 128 dirty cells. Repeated calls drain the dirty set.
    pub fn next_render(&mut self) -> Option<RenderMessage> {
        let mut message = RenderMessage::empty(MessageKind::RenderCells);
        message.columns = self.columns as u16;
        message.rows = self.rows as u16;
        message.cursor_column = self.cursor_column as u16;
        message.cursor_row = self.cursor_row as u16;
        let mut count = 0;
        for row in 0..self.rows {
            for column in 0..self.columns {
                let index = self.index(column, row);
                if self.dirty[index] && count < MAX_RENDER_CELLS {
                    message.positions[count] = index as u16;
                    message.cells[count] = self.active()[index];
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

    /// Convert a semantic input event into the terminal's session byte stream.
    pub fn input(&self, event: &InputMessage) -> Option<StreamMessage> {
        if matches!(event.kind, MessageKind::Text | MessageKind::Paste) {
            let bytes = event.text_bytes()?;
            if self.modes.bracketed_paste && event.kind == MessageKind::Paste {
                let mut stream = StreamMessage::empty(MessageKind::SessionInput);
                let prefix = b"\x1b[200~";
                let suffix = b"\x1b[201~";
                if prefix.len() + bytes.len() + suffix.len() > stream.bytes.len() {
                    return None;
                }
                stream.bytes[..prefix.len()].copy_from_slice(prefix);
                stream.bytes[prefix.len()..prefix.len() + bytes.len()].copy_from_slice(bytes);
                stream.bytes[prefix.len() + bytes.len()..prefix.len() + bytes.len() + suffix.len()]
                    .copy_from_slice(suffix);
                stream.len = (prefix.len() + bytes.len() + suffix.len()) as u16;
                return Some(stream);
            }
            return StreamMessage::from_bytes(MessageKind::SessionInput, bytes);
        }
        if event.kind != MessageKind::Key || event.state == KeyState::Released {
            return None;
        }
        let code = KeyCode::from_raw(event.code);
        if let Some(byte) = code.character_byte() {
            let byte = modified_character(byte, event.modifiers);
            if event.modifiers & MOD_CTRL != 0 {
                return StreamMessage::from_bytes(
                    MessageKind::SessionInput,
                    &[control_byte(byte)?],
                );
            }
            if event.modifiers & MOD_ALT != 0 {
                let bytes = [b'\x1b', byte];
                return StreamMessage::from_bytes(MessageKind::SessionInput, &bytes);
            }
        }
        let bytes: &[u8] = match code {
            KeyCode::Escape => b"\x1b",
            KeyCode::Enter => b"\r",
            KeyCode::Backspace => b"\x7f",
            KeyCode::Tab => b"\t",
            KeyCode::Up => {
                if self.modes.application_cursor {
                    b"\x1bOA"
                } else {
                    b"\x1b[A"
                }
            }
            KeyCode::Down => {
                if self.modes.application_cursor {
                    b"\x1bOB"
                } else {
                    b"\x1b[B"
                }
            }
            KeyCode::Left => {
                if self.modes.application_cursor {
                    b"\x1bOD"
                } else {
                    b"\x1b[D"
                }
            }
            KeyCode::Right => {
                if self.modes.application_cursor {
                    b"\x1bOC"
                } else {
                    b"\x1b[C"
                }
            }
            KeyCode::Home => b"\x1b[H",
            KeyCode::End => b"\x1b[F",
            KeyCode::Delete => b"\x1b[3~",
            KeyCode::PageUp => b"\x1b[5~",
            KeyCode::PageDown => b"\x1b[6~",
            _ => return None,
        };
        StreamMessage::from_bytes(MessageKind::SessionInput, bytes)
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

impl Default for Terminal {
    fn default() -> Self {
        Self::new()
    }
}

const fn ansi_color(index: u16, bright: bool) -> u32 {
    palette((index as u8) + if bright { 8 } else { 0 })
}

const fn palette(index: u8) -> u32 {
    match index {
        0 => 0x0000_0000,
        1 => 0x00aa_0000,
        2 => 0x0000_aa00,
        3 => 0x00aa_5500,
        4 => 0x0000_00aa,
        5 => 0x00aa_00aa,
        6 => 0x0000_aaaa,
        7 => 0x00aa_aaaa,
        8 => 0x0055_5555,
        9 => 0x00ff_5555,
        10 => 0x0055_ff55,
        11 => 0x00ff_ff55,
        12 => 0x0055_55ff,
        13 => 0x00ff_55ff,
        14 => 0x0055_ffff,
        15 => 0x00ff_ffff,
        16..=231 => {
            let value = index - 16;
            let r = value / 36;
            let g = (value / 6) % 6;
            let b = value % 6;
            ((if r == 0 { 0 } else { 55 + r * 40 }) as u32) << 16
                | ((if g == 0 { 0 } else { 55 + g * 40 }) as u32) << 8
                | (if b == 0 { 0 } else { 55 + b * 40 }) as u32
        }
        232..=255 => {
            let gray = 8 + (index - 232) * 10;
            (gray as u32) << 16 | (gray as u32) << 8 | gray as u32
        }
    }
}

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
        assert_eq!(terminal.scrollback_lines(), 0);
        terminal.feed(b"\x1b[31mred\x1b[0m");
        assert!(drain(&mut terminal) > 0);
    }

    #[test]
    fn edit_controls_at_the_right_edge_are_bounded() {
        let mut terminal = Terminal::new();
        terminal.feed(&[b'a'; 80]);
        terminal.feed(b"\x1b[1P\x1b[1@");
        assert_eq!(terminal.cursor(), (80, 0));
    }

    #[test]
    fn cursor_sgr_resize_and_alternate_screen_work() {
        let mut terminal = Terminal::new();
        terminal.feed(b"\x1b[2;4Hx\x1b[?1049h");
        assert_eq!(terminal.cursor(), (4, 1));
        assert!(terminal.modes().alternate_screen);
        terminal.feed(b"\x1b[?1049l");
        assert!(!terminal.modes().alternate_screen);
        assert!(terminal.resize(160, 100).is_ok());
        assert!(terminal.resize(161, 100).is_err());
    }

    #[test]
    fn semantic_input_maps_to_session_bytes() {
        let mut terminal = Terminal::new();
        let key = InputMessage::key(KeyCode::Up, KeyState::Pressed, 0);
        assert_eq!(terminal.input(&key).unwrap().as_bytes(), Some(&b"\x1b[A"[..]));
        let text = InputMessage::text(b"abc").unwrap();
        assert_eq!(terminal.input(&text).unwrap().as_bytes(), Some(&b"abc"[..]));
        terminal.feed(b"\x1b[?2004h");
        let paste = InputMessage::paste(b"abc").unwrap();
        assert_eq!(terminal.input(&paste).unwrap().as_bytes(), Some(&b"\x1b[200~abc\x1b[201~"[..]));
        let ctrl_c = InputMessage::key(KeyCode::character(b'c'), KeyState::Pressed, MOD_CTRL);
        assert_eq!(terminal.input(&ctrl_c).unwrap().as_bytes(), Some(&[0x03][..]));
        let alt_x = InputMessage::key(KeyCode::character(b'x'), KeyState::Pressed, MOD_ALT);
        assert_eq!(terminal.input(&alt_x).unwrap().as_bytes(), Some(&b"\x1bx"[..]));
    }

    #[test]
    fn render_drain_is_chunked() {
        let mut terminal = Terminal::new();
        let mut chunks = 0;
        while let Some(render) = terminal.next_render() {
            assert!(render.count as usize <= MAX_RENDER_CELLS);
            chunks += 1;
        }
        assert!(chunks > 1);
    }
}
