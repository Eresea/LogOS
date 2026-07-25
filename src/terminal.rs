use crate::{display, input};

const ACCENT: [u8; 3] = [61, 220, 151];
const ORIGIN: (usize, usize) = (32, 32);
const SCALE: usize = 3;
const CELLS: usize = 16;

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
            input::Event::Text(text)
                if self.length < self.cells.len()
                    && text.is_ascii()
                    && crate::glyph(text.to_ascii_uppercase()).is_some() =>
            {
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

    pub fn render(&self, display: &mut display::Service) -> bool {
        self.cells[..self.length].iter().enumerate().all(|(cell, text)| {
            Self::draw_glyph(display, *text, ORIGIN.0 + cell * 6 * SCALE, ORIGIN.1)
        })
    }

    pub fn self_check() -> bool {
        let mut model = Self::new();
        model.apply(input::Event::Text(b'g'))
            && model.apply(input::Event::Backspace)
            && !model.apply(input::Event::Enter)
            && model.length == 0
    }

    fn draw_glyph(display: &mut display::Service, text: u8, x: usize, y: usize) -> bool {
        crate::glyph(text.to_ascii_uppercase()).is_some_and(|rows| {
            rows.iter().enumerate().all(|(row, bits)| {
                (0..5).all(|column| {
                    bits & (1 << (4 - column)) == 0
                        || (0..SCALE).all(|dy| {
                            (0..SCALE).all(|dx| {
                                display.present(
                                    x + column * SCALE + dx,
                                    y + row * SCALE + dy,
                                    ACCENT,
                                )
                            })
                        })
                })
            })
        })
    }
}
