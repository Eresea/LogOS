use crate::{display, input, text};

const ACCENT: [u8; 3] = [61, 220, 151];
const BACKGROUND: [u8; 3] = [12, 18, 30];
const ORIGIN: (usize, usize) = (32, 32);
const CELLS: usize = 64;

#[derive(Clone, Copy)]
pub struct Submission {
    cells: [u8; CELLS],
    length: usize,
}

impl Submission {
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
}

impl Model {
    pub const fn new() -> Self {
        Self { cells: [0; CELLS], length: 0, cursor: 0, caret_visible: true }
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
        let mut column = 0;
        for &glyph in &self.cells[..self.length] {
            if glyph & 0xc0 != 0x80
                && !text.render(
                    display,
                    glyph,
                    ORIGIN.0 + column * text::Service::ADVANCE,
                    ORIGIN.1,
                    ACCENT,
                )
            {
                return false;
            }
            column += usize::from(glyph & 0xc0 != 0x80);
        }
        let caret = if self.caret_visible { ACCENT } else { BACKGROUND };
        let x = ORIGIN.0 + self.columns_before_cursor() * text::Service::ADVANCE;
        let y = ORIGIN.1 + text.metrics().height.saturating_sub(2);
        (0..text::Service::ADVANCE).all(|dx| display.present(x + dx, y, caret))
    }

    pub fn blink(&mut self) {
        self.caret_visible = !self.caret_visible;
    }

    pub fn submit(&mut self) -> Submission {
        let submission = Submission::new(self.cells, self.length);
        self.length = 0;
        self.cursor = 0;
        submission
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
        edited
            && home
            && model.cursor == model.length
            && visible != model.caret_visible
            && model.submit().as_bytes() == b"g"
            && model.submit().as_bytes().is_empty()
            && navigation_ok
    }

    fn columns_before_cursor(&self) -> usize {
        self.cells[..self.cursor].iter().filter(|byte| **byte & 0xc0 != 0x80).count()
    }
}
