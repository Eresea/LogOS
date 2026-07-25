use crate::{display, input, text};

const ACCENT: [u8; 3] = [61, 220, 151];
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
}

impl Model {
    pub const fn new() -> Self {
        Self { cells: [0; CELLS], length: 0, cursor: 0 }
    }

    pub fn apply(&mut self, event: input::Event) -> bool {
        event.text().is_some_and(|text| self.insert_utf8(&[text]))
            || event.is_backspace() && self.backspace()
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
        self.cells[..self.length].iter().enumerate().all(|(cell, glyph)| {
            text.render(display, *glyph, ORIGIN.0 + cell * text::Service::ADVANCE, ORIGIN.1, ACCENT)
        })
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
        edited
            && home
            && model.cursor == model.length
            && model.submit().as_bytes() == b"g"
            && model.submit().as_bytes().is_empty()
    }
}
