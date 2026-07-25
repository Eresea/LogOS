use crate::display;

const SCALE: usize = 3;
const WIDTH: usize = 5;
const HEIGHT: usize = 7;

const SPACE: [u8; HEIGHT] = [0; HEIGHT];
const UNKNOWN: [u8; HEIGHT] = [0b11111, 0b10001, 0b00110, 0b00100, 0b00100, 0, 0b00100];
const A: [u8; HEIGHT] = [0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0];
const B: [u8; HEIGHT] = [0b11110, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110, 0];
const C: [u8; HEIGHT] = [0b01111, 0b10000, 0b10000, 0b10000, 0b10000, 0b01111, 0];
const D: [u8; HEIGHT] = [0b11110, 0b10001, 0b10001, 0b10001, 0b10001, 0b11110, 0];
const E: [u8; HEIGHT] = [0b11111, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111, 0];
const F: [u8; HEIGHT] = [0b11111, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000, 0];
const G: [u8; HEIGHT] = [0b01110, 0b10000, 0b10111, 0b10001, 0b10001, 0b01110, 0];
const H: [u8; HEIGHT] = [0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001, 0];
const I: [u8; HEIGHT] = [0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b11111, 0];
const K: [u8; HEIGHT] = [0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001];
const L: [u8; HEIGHT] = [0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111, 0];
const M: [u8; HEIGHT] = [0b10001, 0b11011, 0b10101, 0b10001, 0b10001, 0b10001, 0];
const N: [u8; HEIGHT] = [0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001, 0];
const O: [u8; HEIGHT] = [0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110, 0];
const P: [u8; HEIGHT] = [0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0];
const R: [u8; HEIGHT] = [0b11110, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001, 0];
const S: [u8; HEIGHT] = [0b01111, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110, 0];
const T: [u8; HEIGHT] = [0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0];
const U: [u8; HEIGHT] = [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110, 0];
const V: [u8; HEIGHT] = [0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100, 0];
const W: [u8; HEIGHT] = [0b10001, 0b10001, 0b10101, 0b11011, 0b10001, 0, 0];
const X: [u8; HEIGHT] = [0b10001, 0b01010, 0b00100, 0b00100, 0b01010, 0b10001, 0];
const Y: [u8; HEIGHT] = [0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0, 0];
const ZERO: [u8; HEIGHT] = [0b01110, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110, 0];
const ONE: [u8; HEIGHT] = [0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b01110, 0];

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Metrics {
    pub width: usize,
    pub height: usize,
    pub advance: usize,
}

pub struct Service;

impl Service {
    pub const ADVANCE: usize = (WIDTH + 1) * SCALE;

    pub const fn new() -> Self {
        Self
    }

    pub const fn metrics(&self) -> Metrics {
        Metrics { width: WIDTH * SCALE, height: HEIGHT * SCALE, advance: Self::ADVANCE }
    }

    pub fn render(
        &self,
        display: &mut display::Service,
        text: u8,
        x: usize,
        y: usize,
        color: [u8; 3],
    ) -> bool {
        self.glyph(text).iter().enumerate().all(|(row, bits)| {
            (0..WIDTH).all(|column| {
                bits & (1 << (WIDTH - 1 - column)) == 0
                    || (0..SCALE).all(|dy| {
                        (0..SCALE).all(|dx| {
                            display.present(x + column * SCALE + dx, y + row * SCALE + dy, color)
                        })
                    })
            })
        })
    }

    pub fn self_check() -> bool {
        Self::new().metrics() == Metrics { width: 15, height: 21, advance: 18 }
            && Self::new().glyph(b'G') == &G
            && Self::new().glyph(b'?') == &UNKNOWN
    }

    fn glyph(&self, text: u8) -> &'static [u8; HEIGHT] {
        match text.to_ascii_uppercase() {
            b'A' => &A,
            b'B' => &B,
            b'C' => &C,
            b'D' => &D,
            b'E' => &E,
            b'F' => &F,
            b'G' => &G,
            b'H' => &H,
            b'I' => &I,
            b'K' => &K,
            b'L' => &L,
            b'M' => &M,
            b'N' => &N,
            b'O' => &O,
            b'P' => &P,
            b'R' => &R,
            b'S' => &S,
            b'T' => &T,
            b'U' => &U,
            b'V' => &V,
            b'W' => &W,
            b'X' => &X,
            b'Y' => &Y,
            b'0' => &ZERO,
            b'1' => &ONE,
            b' ' => &SPACE,
            _ => &UNKNOWN,
        }
    }
}
