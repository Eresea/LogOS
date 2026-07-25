use crate::{display, input, text};

const ACCENT: [u8; 3] = [61, 220, 151];
const ORIGIN: (usize, usize) = (32, 32);
const CELLS: usize = 16;

#[derive(Clone, Copy)]
pub struct Submission {
    cells: [u8; CELLS],
    length: usize,
}

impl Submission {
    pub const fn new(cells: [u8; CELLS], length: usize) -> Self {
        Self { cells, length }
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.cells[..self.length]
    }
}

pub struct Model {
    cells: [u8; CELLS],
    length: usize,
}

impl Model {
    pub const fn new() -> Self {
        Self { cells: [0; CELLS], length: 0 }
    }

    pub fn apply(&mut self, event: input::Event) -> bool {
        match event {
            input::Event::Text(text) if self.length < self.cells.len() && text.is_ascii() => {
                self.cells[self.length] = text;
                self.length += 1;
                true
            }
            input::Event::Backspace if self.length > 0 => {
                self.length -= 1;
                true
            }
            _ => false,
        }
    }

    pub fn render(&self, display: &mut display::Service, text: &text::Service) -> bool {
        self.cells[..self.length].iter().enumerate().all(|(cell, glyph)| {
            text.render(display, *glyph, ORIGIN.0 + cell * text::Service::ADVANCE, ORIGIN.1, ACCENT)
        })
    }

    pub fn submit(&mut self) -> Submission {
        let submission = Submission::new(self.cells, self.length);
        self.length = 0;
        submission
    }

    pub fn self_check() -> bool {
        let mut model = Self::new();
        model.apply(input::Event::Text(b'g'))
            && model.apply(input::Event::Backspace)
            && !model.apply(input::Event::Enter)
            && model.length == 0
            && model.submit().as_bytes().is_empty()
    }
}
