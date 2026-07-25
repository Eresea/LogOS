use crate::display;

const SCALE: usize = 3;
const WIDTH: usize = 5;

pub struct Service;

impl Service {
    pub const ADVANCE: usize = (WIDTH + 1) * SCALE;

    pub const fn new() -> Self {
        Self
    }

    pub fn render(
        &self,
        display: &mut display::Service,
        text: u8,
        x: usize,
        y: usize,
        color: [u8; 3],
    ) -> bool {
        crate::glyph(text.to_ascii_uppercase()).is_some_and(|rows| {
            rows.iter().enumerate().all(|(row, bits)| {
                (0..WIDTH).all(|column| {
                    bits & (1 << (WIDTH - 1 - column)) == 0
                        || (0..SCALE).all(|dy| {
                            (0..SCALE).all(|dx| {
                                display.present(
                                    x + column * SCALE + dx,
                                    y + row * SCALE + dy,
                                    color,
                                )
                            })
                        })
                })
            })
        })
    }

    pub fn self_check() -> bool {
        crate::glyph(b'G').is_some() && crate::glyph(b'?').is_none() && Self::ADVANCE == 18
    }
}
