use crate::{display, input, text};

const ACCENT: [u8; 3] = [61, 220, 151];
const BACKGROUND: [u8; 3] = [12, 18, 30];
const ORIGIN: (usize, usize) = (32, 32);
const CELLS: usize = 64;
const SCROLLBACK: usize = 8;

#[derive(Clone, Copy)]
pub struct Submission {
    cells: [u8; CELLS],
    length: usize,
}

impl Submission {
    const EMPTY: Self = Self { cells: [0; CELLS], length: 0 };
    pub const fn new(cells: [u8; CELLS], length: usize) -> Self {
        Self { cells, length }
    }

    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() > CELLS {
            return None;
        }
        let mut cells = [0; CELLS];
        cells[..bytes.len()].copy_from_slice(bytes);
        Some(Self::new(cells, bytes.len()))
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.cells[..self.length]
    }
}

pub struct Model {
    cells: [u8; CELLS],
    length: usize,
    cursor: usize,
    caret_visible: bool,
    scrollback: [Submission; SCROLLBACK],
    scrollback_head: usize,
    scrollback_len: usize,
    history_offset: Option<usize>,
}

impl Model {
    pub const fn new() -> Self {
        Self {
            cells: [0; CELLS],
            length: 0,
            cursor: 0,
            caret_visible: true,
            scrollback: [Submission::EMPTY; SCROLLBACK],
            scrollback_head: 0,
            scrollback_len: 0,
            history_offset: None,
        }
    }

    pub fn apply(&mut self, event: input::Event) -> bool {
        match event.pressed() {
            Some((input::LogicalKey::Text(text), _)) => self.insert_utf8(&[text]),
            Some((input::LogicalKey::Backspace, _)) => self.backspace(),
            Some((input::LogicalKey::Delete, _)) => self.delete(),
            Some((input::LogicalKey::Left, _)) if event.control() => self.word_left(),
            Some((input::LogicalKey::Right, _)) if event.control() => self.word_right(),
            Some((input::LogicalKey::Left, _)) => self.move_left(),
            Some((input::LogicalKey::Right, _)) => self.move_right(),
            Some((input::LogicalKey::Home, _)) => {
                self.home();
                true
            }
            Some((input::LogicalKey::End, _)) => {
                self.end();
                true
            }
            Some((input::LogicalKey::Up, _)) => self.history_previous(),
            Some((input::LogicalKey::Down, _)) => self.history_next(),
            _ => false,
        }
    }

    pub fn insert_utf8(&mut self, bytes: &[u8]) -> bool {
        if core::str::from_utf8(bytes).is_err() || self.length + bytes.len() > self.cells.len() {
            return false;
        }
        self.cells.copy_within(self.cursor..self.length, self.cursor + bytes.len());
        self.cells[self.cursor..self.cursor + bytes.len()].copy_from_slice(bytes);
        self.cursor += bytes.len();
        self.length += bytes.len();
        true
    }

    pub fn move_left(&mut self) -> bool {
        let Some(cursor) = self.previous_boundary(self.cursor) else {
            return false;
        };
        self.cursor = cursor;
        true
    }

    pub fn move_right(&mut self) -> bool {
        if self.cursor == self.length {
            return false;
        }
        self.cursor += 1;
        while self.cursor < self.length && self.cells[self.cursor] & 0xc0 == 0x80 {
            self.cursor += 1;
        }
        true
    }

    pub fn home(&mut self) {
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.cursor = self.length;
    }

    fn backspace(&mut self) -> bool {
        let Some(start) = self.previous_boundary(self.cursor) else {
            return false;
        };
        self.cells.copy_within(self.cursor..self.length, start);
        self.length -= self.cursor - start;
        self.cursor = start;
        true
    }

    fn delete(&mut self) -> bool {
        if self.cursor == self.length {
            return false;
        }
        let mut end = self.cursor + 1;
        while end < self.length && self.cells[end] & 0xc0 == 0x80 {
            end += 1;
        }
        self.cells.copy_within(end..self.length, self.cursor);
        self.length -= end - self.cursor;
        true
    }

    fn word_left(&mut self) -> bool {
        let start = self.cursor;
        while self.cursor > 0 && self.cells[self.cursor - 1] == b' ' {
            let _ = self.move_left();
        }
        while self.cursor > 0 && self.cells[self.cursor - 1] != b' ' {
            let _ = self.move_left();
        }
        self.cursor != start
    }

    fn word_right(&mut self) -> bool {
        let start = self.cursor;
        while self.cursor < self.length && self.cells[self.cursor] != b' ' {
            let _ = self.move_right();
        }
        while self.cursor < self.length && self.cells[self.cursor] == b' ' {
            let _ = self.move_right();
        }
        self.cursor != start
    }

    fn previous_boundary(&self, cursor: usize) -> Option<usize> {
        if cursor == 0 {
            return None;
        }
        let mut boundary = cursor - 1;
        while boundary > 0 && self.cells[boundary] & 0xc0 == 0x80 {
            boundary -= 1;
        }
        Some(boundary)
    }

    pub fn render(&self, display: &mut display::Service, text: &text::Service) -> bool {
        let columns = self.columns(display);
        let mut column = 0;
        let mut row = 0;
        for &glyph in &self.cells[..self.length] {
            if glyph & 0xc0 != 0x80
                && !text.render(
                    display,
                    glyph,
                    ORIGIN.0 + column * text::Service::ADVANCE,
                    ORIGIN.1 + row * text.metrics().height,
                    ACCENT,
                )
            {
                return false;
            }
            column += usize::from(glyph & 0xc0 != 0x80);
            if column == columns {
                column = 0;
                row += 1;
            }
        }
        let caret = if self.caret_visible { ACCENT } else { BACKGROUND };
        let (caret_row, caret_column) = Self::position(self.columns_before_cursor(), columns);
        let x = ORIGIN.0 + caret_column * text::Service::ADVANCE;
        let y =
            ORIGIN.1 + caret_row * text.metrics().height + text.metrics().height.saturating_sub(2);
        (0..text::Service::ADVANCE).all(|dx| display.present(x + dx, y, caret))
    }

    pub fn blink(&mut self) {
        self.caret_visible = !self.caret_visible;
    }

    pub fn submit(&mut self) -> Submission {
        let submission = Submission::new(self.cells, self.length);
        self.push_scrollback(submission);
        self.length = 0;
        self.cursor = 0;
        submission
    }

    pub fn scrollback_len(&self) -> usize {
        self.scrollback_len
    }

    fn push_scrollback(&mut self, submission: Submission) {
        self.scrollback[self.scrollback_head] = submission;
        self.scrollback_head = (self.scrollback_head + 1) % SCROLLBACK;
        self.scrollback_len = (self.scrollback_len + 1).min(SCROLLBACK);
        self.history_offset = None;
    }

    fn latest_scrollback(&self) -> Submission {
        self.scrollback[(self.scrollback_head + SCROLLBACK - 1) % SCROLLBACK]
    }

    fn history_previous(&mut self) -> bool {
        if self.scrollback_len == 0 {
            return false;
        }
        let offset =
            self.history_offset.map_or(0, |offset| (offset + 1).min(self.scrollback_len - 1));
        self.history_offset = Some(offset);
        self.load_history(offset);
        true
    }

    fn history_next(&mut self) -> bool {
        let Some(offset) = self.history_offset else {
            return false;
        };
        if offset == 0 {
            self.history_offset = None;
            self.length = 0;
            self.cursor = 0;
        } else {
            self.history_offset = Some(offset - 1);
            self.load_history(offset - 1);
        }
        true
    }

    fn load_history(&mut self, offset: usize) {
        let index = (self.scrollback_head + SCROLLBACK - 1 - offset) % SCROLLBACK;
        let submission = self.scrollback[index];
        self.cells = submission.cells;
        self.length = submission.length;
        self.cursor = self.length;
    }

    pub fn self_check() -> bool {
        let mut model = Self::new();
        let text = input::Event::Key {
            physical: input::PhysicalKey(0x22),
            logical: input::LogicalKey::Text(b'g'),
            state: input::State::Press,
            modifiers: input::Modifiers::none(),
        };
        let edited = model.apply(text)
            && model.insert_utf8(b"\xc3\xa9")
            && model.move_left()
            && model.insert_utf8(b"b")
            && model.backspace()
            && model.move_right()
            && model.backspace();
        model.home();
        let home = model.cursor == 0;
        model.end();
        let visible = model.caret_visible;
        model.blink();
        let mut navigation = Self::new();
        let inserted = navigation.insert_utf8(b"one two");
        navigation.home();
        let navigation_ok = inserted
            && navigation.word_right()
            && navigation.delete()
            && navigation.word_left()
            && navigation.delete();
        let mut scrollback = Self::new();
        for _ in 0..SCROLLBACK + 1 {
            let _ = scrollback.insert_utf8(b"x");
            let _ = scrollback.submit();
        }
        let history = scrollback.history_previous()
            && scrollback.latest_scrollback().as_bytes() == b"x"
            && scrollback.history_next();
        edited
            && home
            && model.cursor == model.length
            && visible != model.caret_visible
            && model.submit().as_bytes() == b"g"
            && model.submit().as_bytes().is_empty()
            && navigation_ok
            && Self::position(6, 4) == (1, 2)
            && scrollback.scrollback_len() == SCROLLBACK
            && scrollback.latest_scrollback().as_bytes() == b"x"
            && history
    }

    fn columns_before_cursor(&self) -> usize {
        self.cells[..self.cursor].iter().filter(|byte| **byte & 0xc0 != 0x80).count()
    }

    fn columns(&self, display: &display::Service) -> usize {
        let width = display.dimensions().0.saturating_sub(ORIGIN.0 * 2);
        (width / text::Service::ADVANCE).max(1)
    }

    const fn position(column: usize, columns: usize) -> (usize, usize) {
        (column / columns, column % columns)
    }
}
