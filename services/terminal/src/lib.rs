#![no_std]

//! Bounded fixed-size terminal emulator.

#[cfg(test)]
extern crate std;

use logos_abi::{
    CELL_ATTR_BOLD, CELL_ATTR_DIM, CELL_ATTR_UNDERLINE, Cell, DEFAULT_COLUMNS, DEFAULT_ROWS,
    InputMessage, IpcBytes, KeyCode, KeyState, MAX_COLUMNS, MAX_RENDER_CELLS, MOD_ALT,
    MOD_CAPS_LOCK, MOD_CTRL, MOD_SHIFT, MessageKind, RenderMessage,
};

const MAX_PARAMS: usize = 16;
const REPLACEMENT_SCALAR: u32 = 0xfffd;
/// Service-local storage cap; the ABI maximum is a protocol-wide ceiling.
pub const TERMINAL_SCROLLBACK_LINES: usize = 64;
const DEFAULT_FOREGROUND: u32 = 0x00d7_e3f4;
const DEFAULT_BACKGROUND: u32 = 0x000b_1020;
const ANSI_COLORS: [u32; 8] = [
    0x000b_1020,
    0x00ff_6b6b,
    0x007e_d787,
    0x00ff_d866,
    0x0058_a6ff,
    0x00d2_a8ff,
    0x0056_d4dd,
    DEFAULT_FOREGROUND,
];
const ANSI_BRIGHT_COLORS: [u32; 8] = [
    0x0030_3845,
    0x00ff_7b72,
    0x00a5_d6a7,
    0x00f2_cc60,
    0x0079_c0ff,
    0x00d2_a8ff,
    0x00a5_d6ff,
    0x00ffffff,
];

const fn blank_cell() -> Cell {
    Cell {
        codepoint: b' ' as u32,
        foreground: DEFAULT_FOREGROUND,
        background: DEFAULT_BACKGROUND,
        attributes: 0,
        width: 1,
        reserved: 0,
    }
}

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
    foreground: u32,
    background: u32,
    attributes: u16,
    utf8_codepoint: u32,
    utf8_remaining: u8,
    utf8_min: u32,
    scrollback: [Cell; DEFAULT_COLUMNS * TERMINAL_SCROLLBACK_LINES],
    scrollback_start: usize,
    scrollback_len: usize,
    view_offset: usize,
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

    pub fn input(&mut self, event: &InputMessage) -> Option<IpcBytes> {
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
            screen: [blank_cell(); CELL_COUNT],
            dirty: [true; CELL_COUNT],
            parser: Parser::new(),
            foreground: DEFAULT_FOREGROUND,
            background: DEFAULT_BACKGROUND,
            attributes: 0,
            utf8_codepoint: 0,
            utf8_remaining: 0,
            utf8_min: 0,
            scrollback: [blank_cell(); DEFAULT_COLUMNS * TERMINAL_SCROLLBACK_LINES],
            scrollback_start: 0,
            scrollback_len: 0,
            view_offset: 0,
        }
    }

    pub const fn cursor(&self) -> (usize, usize) {
        (self.cursor_column, self.cursor_row)
    }

    pub fn reset(&mut self) {
        self.cursor_column = 0;
        self.cursor_row = 0;
        self.parser = Parser::new();
        self.foreground = DEFAULT_FOREGROUND;
        self.background = DEFAULT_BACKGROUND;
        self.attributes = 0;
        self.utf8_codepoint = 0;
        self.utf8_remaining = 0;
        self.utf8_min = 0;
        self.scrollback_start = 0;
        self.scrollback_len = 0;
        self.view_offset = 0;
        self.screen.fill(blank_cell());
        self.mark_all_dirty();
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        self.show_live_view();
        for &byte in bytes {
            self.feed_byte(byte);
        }
    }

    fn feed_byte(&mut self, byte: u8) {
        match self.parser.state {
            ParserState::Ground => self.feed_ground_byte(byte),
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

    fn feed_ground_byte(&mut self, byte: u8) {
        if self.utf8_remaining != 0 {
            if byte & 0xc0 == 0x80 {
                self.utf8_codepoint = (self.utf8_codepoint << 6) | u32::from(byte & 0x3f);
                self.utf8_remaining -= 1;
                if self.utf8_remaining == 0 {
                    let scalar = self.utf8_codepoint;
                    let valid = scalar >= self.utf8_min
                        && scalar <= 0x10ffff
                        && !(0xd800..=0xdfff).contains(&scalar);
                    self.utf8_codepoint = 0;
                    self.utf8_min = 0;
                    self.put(if valid { scalar } else { REPLACEMENT_SCALAR });
                }
                return;
            }
            self.utf8_codepoint = 0;
            self.utf8_remaining = 0;
            self.utf8_min = 0;
            self.put(REPLACEMENT_SCALAR);
            self.feed_ground_byte(byte);
            return;
        }
        match byte {
            0x1b => self.parser.state = ParserState::Escape,
            0x08 | 0x7f => self.cursor_column = self.cursor_column.saturating_sub(1),
            0x09 => {
                self.cursor_column =
                    ((self.cursor_column / 8) + 1).saturating_mul(8).min(DEFAULT_COLUMNS - 1)
            }
            0x0a..=0x0c => self.line_feed(),
            0x0d => self.cursor_column = 0,
            0x20..=0x7e => self.put(byte as u32),
            0xc2..=0xdf => {
                self.utf8_codepoint = u32::from(byte & 0x1f);
                self.utf8_remaining = 1;
                self.utf8_min = 0x80;
            }
            0xe0..=0xef => {
                self.utf8_codepoint = u32::from(byte & 0x0f);
                self.utf8_remaining = 2;
                self.utf8_min = 0x800;
            }
            0xf0..=0xf4 => {
                self.utf8_codepoint = u32::from(byte & 0x07);
                self.utf8_remaining = 3;
                self.utf8_min = 0x10000;
            }
            0x80..=0xbf | 0xc0..=0xc1 | 0xf5..=0xff => self.put(REPLACEMENT_SCALAR),
            _ => {}
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
            b'm' => self.apply_sgr(),
            _ => {}
        }
    }

    fn apply_sgr(&mut self) {
        if self.parser.param_count == 0 {
            self.reset_style();
            return;
        }
        for index in 0..self.parser.param_count {
            match self.parser.params[index] {
                0 => self.reset_style(),
                1 => self.attributes |= CELL_ATTR_BOLD,
                2 => self.attributes |= CELL_ATTR_DIM,
                4 => self.attributes |= CELL_ATTR_UNDERLINE,
                22 => self.attributes &= !(CELL_ATTR_BOLD | CELL_ATTR_DIM),
                24 => self.attributes &= !CELL_ATTR_UNDERLINE,
                30..=37 => self.foreground = ANSI_COLORS[(self.parser.params[index] - 30) as usize],
                39 => self.foreground = DEFAULT_FOREGROUND,
                40..=47 => self.background = ANSI_COLORS[(self.parser.params[index] - 40) as usize],
                49 => self.background = DEFAULT_BACKGROUND,
                90..=97 => {
                    self.foreground = ANSI_BRIGHT_COLORS[(self.parser.params[index] - 90) as usize]
                }
                100..=107 => {
                    self.background = ANSI_BRIGHT_COLORS[(self.parser.params[index] - 100) as usize]
                }
                _ => {}
            }
        }
    }

    fn reset_style(&mut self) {
        self.foreground = DEFAULT_FOREGROUND;
        self.background = DEFAULT_BACKGROUND;
        self.attributes = 0;
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
        self.screen[index] = Cell {
            codepoint,
            foreground: self.foreground,
            background: self.background,
            attributes: self.attributes,
            ..blank_cell()
        };
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
        self.store_scrollback_line(0);
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
            self.screen[index] = blank_cell();
            self.dirty[index] = true;
        }
    }

    fn store_scrollback_line(&mut self, row: usize) {
        let slot = if self.scrollback_len < TERMINAL_SCROLLBACK_LINES {
            (self.scrollback_start + self.scrollback_len) % TERMINAL_SCROLLBACK_LINES
        } else {
            let slot = self.scrollback_start;
            self.scrollback_start = (self.scrollback_start + 1) % TERMINAL_SCROLLBACK_LINES;
            slot
        };
        let source = row * DEFAULT_COLUMNS;
        let target = slot * DEFAULT_COLUMNS;
        self.scrollback[target..target + DEFAULT_COLUMNS]
            .copy_from_slice(&self.screen[source..source + DEFAULT_COLUMNS]);
        self.scrollback_len = self.scrollback_len.saturating_add(1).min(TERMINAL_SCROLLBACK_LINES);
    }

    fn show_live_view(&mut self) {
        if self.view_offset != 0 {
            self.view_offset = 0;
            self.mark_all_dirty();
        }
    }

    fn scroll_view(&mut self, lines: isize) {
        let old_offset = self.view_offset;
        if lines.is_positive() {
            self.view_offset =
                self.view_offset.saturating_add(lines as usize).min(self.scrollback_len);
        } else {
            self.view_offset = self.view_offset.saturating_sub(lines.unsigned_abs());
        }
        if old_offset != self.view_offset {
            self.mark_all_dirty();
        }
    }

    fn visible_cell(&self, row: usize, column: usize) -> Cell {
        if self.view_offset == 0 {
            return self.screen[Self::index(column, row)];
        }
        let top_line = self.scrollback_len.saturating_sub(self.view_offset);
        let line = top_line + row;
        if line < self.scrollback_len {
            let slot = (self.scrollback_start + line) % TERMINAL_SCROLLBACK_LINES;
            self.scrollback[slot * DEFAULT_COLUMNS + column]
        } else {
            self.screen[(line - self.scrollback_len) * DEFAULT_COLUMNS + column]
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
            self.screen[index] = blank_cell();
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
            self.screen[index] = blank_cell();
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
                    message.positions[count] = (row * MAX_COLUMNS + column) as u16;
                    message.cells[count] = self.visible_cell(row, column);
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

    pub fn input(&mut self, event: &InputMessage) -> Option<IpcBytes> {
        if event.kind == MessageKind::Key
            && matches!(event.state, KeyState::Pressed | KeyState::Repeat)
        {
            match KeyCode::from_raw(event.code) {
                KeyCode::PageUp => {
                    self.scroll_view(DEFAULT_ROWS.saturating_sub(1) as isize);
                    return None;
                }
                KeyCode::PageDown => {
                    self.scroll_view(-(DEFAULT_ROWS.saturating_sub(1) as isize));
                    return None;
                }
                _ => {}
            }
        }
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
            KeyCode::Left if event.modifiers & MOD_CTRL != 0 => b"\x1b[1;5D",
            KeyCode::Right if event.modifiers & MOD_CTRL != 0 => b"\x1b[1;5C",
            KeyCode::Left => b"\x1b[D",
            KeyCode::Right => b"\x1b[C",
            KeyCode::Home => b"\x1b[H",
            KeyCode::End => b"\x1b[F",
            KeyCode::Delete => b"\x1b[3~",
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
    fn utf8_text_decodes_to_one_terminal_cell() {
        let mut terminal = Terminal::new();
        terminal.feed(b"\xc3\xa9");
        assert_eq!(terminal.screen[0].codepoint, 0xe9);
        assert_eq!(terminal.cursor(), (1, 0));
    }

    #[test]
    fn cursor_and_erase_are_bounded() {
        let mut terminal = Terminal::new();
        terminal.feed(b"\x1b[2;4Hx\x1b[2K");
        assert_eq!(terminal.cursor(), (4, 1));
        assert!(drain(&mut terminal) > 0);
    }

    #[test]
    fn ansi_sgr_applies_theme_colors_to_cells() {
        let mut terminal = Terminal::new();
        drain(&mut terminal);
        terminal.feed(b"\x1b[31mred\x1b[0m");
        assert_eq!(terminal.screen[0].foreground, ANSI_COLORS[1]);
        assert_eq!(terminal.screen[3].foreground, DEFAULT_FOREGROUND);
        assert_eq!(terminal.screen[0].background, DEFAULT_BACKGROUND);
    }

    #[test]
    fn page_navigation_uses_bounded_scrollback() {
        let mut terminal = Terminal::new();
        for _ in 0..(DEFAULT_ROWS + 2) {
            terminal.feed(b"x\n");
        }
        drain(&mut terminal);
        let event = InputMessage::key(KeyCode::PageUp, KeyState::Pressed, 0);
        assert!(terminal.input(&event).is_none());
        assert!(terminal.view_offset > 0);
        assert!(drain(&mut terminal) > 0);
        let event = InputMessage::key(KeyCode::PageUp, KeyState::Repeat, 0);
        assert!(terminal.input(&event).is_none());
        let event = InputMessage::key(KeyCode::PageDown, KeyState::Pressed, 0);
        assert!(terminal.input(&event).is_none());
        assert_eq!(terminal.view_offset, 0);
    }

    #[test]
    fn semantic_input_is_small_and_stable() {
        let mut terminal = Terminal::new();
        let event = InputMessage::key(KeyCode::Up, KeyState::Pressed, 0);
        assert_eq!(terminal.input(&event).unwrap().as_bytes(), Some(&b"\x1b[A"[..]));
    }

    #[test]
    fn ctrl_arrows_use_word_navigation_sequences() {
        let mut terminal = Terminal::new();
        let left = InputMessage::key(KeyCode::Left, KeyState::Pressed, MOD_CTRL);
        assert_eq!(terminal.input(&left).unwrap().as_bytes(), Some(&b"\x1b[1;5D"[..]));
        let right = InputMessage::key(KeyCode::Right, KeyState::Pressed, MOD_CTRL);
        assert_eq!(terminal.input(&right).unwrap().as_bytes(), Some(&b"\x1b[1;5C"[..]));
        let home = InputMessage::key(KeyCode::Home, KeyState::Pressed, 0);
        assert_eq!(terminal.input(&home).unwrap().as_bytes(), Some(&b"\x1b[H"[..]));
        let end = InputMessage::key(KeyCode::End, KeyState::Pressed, 0);
        assert_eq!(terminal.input(&end).unwrap().as_bytes(), Some(&b"\x1b[F"[..]));
    }

    #[test]
    fn render_is_chunked() {
        let mut terminal = Terminal::new();
        assert!(drain(&mut terminal) > 1);
        assert!(terminal.next_render().is_none());
    }

    #[test]
    fn render_positions_use_display_stride() {
        let mut terminal = Terminal::new();
        let mut saw_second_row = false;
        while let Some(message) = terminal.next_render() {
            for index in 0..message.count as usize {
                if message.positions[index] == MAX_COLUMNS as u16 {
                    saw_second_row = true;
                }
            }
        }
        assert!(saw_second_row);
    }
}
