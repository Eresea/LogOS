#![no_std]

#[cfg(test)]
extern crate std;

use logos_abi::{
    Cell, MAX_COLUMNS, MAX_GLYPH_CACHE, MAX_RENDER_CELLS, MAX_ROWS, MessageKind, RenderMessage,
};

pub use logos_abi::FramebufferFormat as PixelFormat;

pub const GLYPH_WIDTH: usize = 8;
pub const GLYPH_HEIGHT: usize = 16;
pub const REPLACEMENT_SCALAR: u32 = 0xfffd;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Glyph {
    pub rows: [u8; GLYPH_HEIGHT],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GlyphId(u16);

impl GlyphId {
    pub const fn raw(self) -> usize {
        self.0 as usize
    }
}

#[derive(Clone, Copy)]
struct GlyphSlot {
    valid: bool,
    scalar: u32,
    glyph: Glyph,
}

impl GlyphSlot {
    const EMPTY: Self = Self { valid: false, scalar: 0, glyph: Glyph { rows: [0; GLYPH_HEIGHT] } };
}

pub struct GlyphCache {
    slots: [GlyphSlot; MAX_GLYPH_CACHE],
    next: usize,
}

impl GlyphCache {
    pub const fn new() -> Self {
        Self { slots: [GlyphSlot::EMPTY; MAX_GLYPH_CACHE], next: 0 }
    }

    pub fn lookup(&mut self, scalar: u32) -> GlyphId {
        let scalar = normalize_scalar(scalar);
        if let Some(index) = self.slots.iter().position(|slot| slot.valid && slot.scalar == scalar)
        {
            return GlyphId(index as u16);
        }
        let index = self.next;
        self.next = (self.next + 1) % MAX_GLYPH_CACHE;
        self.slots[index] = GlyphSlot { valid: true, scalar, glyph: embedded_glyph(scalar) };
        GlyphId(index as u16)
    }

    pub fn glyph(&self, id: GlyphId) -> Option<Glyph> {
        self.slots.get(id.raw()).filter(|slot| slot.valid).map(|slot| slot.glyph)
    }

    pub fn len(&self) -> usize {
        self.slots.iter().fold(0, |count, slot| count + slot.valid as usize)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Default for GlyphCache {
    fn default() -> Self {
        Self::new()
    }
}

fn normalize_scalar(scalar: u32) -> u32 {
    if scalar <= 0x10ffff && !(0xd800..=0xdfff).contains(&scalar) {
        scalar
    } else {
        REPLACEMENT_SCALAR
    }
}

fn embedded_glyph(scalar: u32) -> Glyph {
    let byte = if scalar >= 'A' as u32 && scalar <= 'Z' as u32 {
        (scalar as u8).to_ascii_lowercase()
    } else {
        scalar as u8
    };
    let pattern = match byte {
        b'a' => [14, 17, 31, 17, 17, 17, 0],
        b'b' => [30, 17, 17, 30, 17, 17, 30],
        b'c' => [14, 17, 16, 16, 16, 17, 14],
        b'd' => [30, 17, 17, 17, 17, 17, 30],
        b'e' => [31, 16, 16, 30, 16, 16, 31],
        b'f' => [31, 16, 16, 30, 16, 16, 16],
        b'g' => [14, 17, 16, 23, 17, 17, 14],
        b'h' => [17, 17, 17, 31, 17, 17, 17],
        b'i' => [31, 4, 4, 4, 4, 4, 31],
        b'j' => [1, 1, 1, 1, 17, 17, 14],
        b'k' => [17, 18, 20, 24, 20, 18, 17],
        b'l' => [16, 16, 16, 16, 16, 16, 31],
        b'm' => [17, 27, 21, 21, 17, 17, 17],
        b'n' => [17, 25, 21, 19, 17, 17, 17],
        b'o' => [14, 17, 17, 17, 17, 17, 14],
        b'p' => [30, 17, 17, 30, 16, 16, 16],
        b'q' => [14, 17, 17, 17, 21, 18, 13],
        b'r' => [30, 17, 17, 30, 20, 18, 17],
        b's' => [15, 16, 16, 14, 1, 1, 30],
        b't' => [31, 4, 4, 4, 4, 4, 4],
        b'u' => [17, 17, 17, 17, 17, 17, 14],
        b'v' => [17, 17, 17, 17, 17, 10, 4],
        b'w' => [17, 17, 17, 21, 21, 21, 10],
        b'x' => [17, 17, 10, 4, 10, 17, 17],
        b'y' => [17, 17, 10, 4, 4, 4, 4],
        b'z' => [31, 1, 2, 4, 8, 16, 31],
        b'0' => [14, 17, 19, 21, 25, 17, 14],
        b'1' => [4, 12, 4, 4, 4, 4, 14],
        b'2' => [14, 17, 1, 2, 4, 8, 31],
        b'3' => [30, 1, 1, 14, 1, 1, 30],
        b'4' => [2, 6, 10, 18, 31, 2, 2],
        b'5' => [31, 16, 16, 30, 1, 1, 30],
        b'6' => [14, 16, 16, 30, 17, 17, 14],
        b'7' => [31, 1, 2, 4, 8, 8, 8],
        b'8' => [14, 17, 17, 14, 17, 17, 14],
        b'9' => [14, 17, 17, 15, 1, 1, 14],
        b' ' => [0; 7],
        b'>' => [16, 8, 4, 2, 4, 8, 16],
        b'<' => [1, 2, 4, 8, 4, 2, 1],
        b'-' => [0, 0, 0, 31, 0, 0, 0],
        b'_' => [0, 0, 0, 0, 0, 0, 31],
        _ => [31, 17, 21, 17, 21, 17, 31],
    };
    let mut rows = [0; GLYPH_HEIGHT];
    let mut row = 0;
    while row < pattern.len() {
        rows[row * 2] = pattern[row];
        rows[row * 2 + 1] = pattern[row];
        row += 1;
    }
    Glyph { rows }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DisplayError {
    InvalidMessage,
    StaleGeneration,
    InvalidFramebuffer,
}

pub struct Display {
    generation: u16,
    columns: usize,
    rows: usize,
    cursor_column: usize,
    cursor_row: usize,
    cells: [Cell; MAX_COLUMNS * MAX_ROWS],
    dirty: [bool; MAX_COLUMNS * MAX_ROWS],
    applied: usize,
}

impl Display {
    pub const fn new(generation: u16) -> Self {
        Self {
            generation,
            columns: 0,
            rows: 0,
            cursor_column: 0,
            cursor_row: 0,
            cells: [Cell::EMPTY; MAX_COLUMNS * MAX_ROWS],
            dirty: [false; MAX_COLUMNS * MAX_ROWS],
            applied: 0,
        }
    }

    pub fn apply(&mut self, generation: u16, message: &RenderMessage) -> Result<(), DisplayError> {
        if generation != self.generation {
            return Err(DisplayError::StaleGeneration);
        }
        if !matches!(message.kind, MessageKind::RenderCells | MessageKind::FullRedraw) {
            return Err(DisplayError::InvalidMessage);
        }
        let columns = usize::from(message.columns);
        let rows = usize::from(message.rows);
        if columns == 0
            || columns > MAX_COLUMNS
            || rows == 0
            || rows > MAX_ROWS
            || message.count as usize > MAX_RENDER_CELLS
        {
            return Err(DisplayError::InvalidMessage);
        }
        for index in 0..message.count as usize {
            let position = usize::from(message.positions[index]);
            if position >= rows * MAX_COLUMNS || position % MAX_COLUMNS >= columns {
                return Err(DisplayError::InvalidMessage);
            }
        }
        if message.kind == MessageKind::FullRedraw {
            self.cells.fill(Cell::EMPTY);
            self.dirty.fill(true);
        }
        for index in 0..message.count as usize {
            let position = usize::from(message.positions[index]);
            self.cells[position] = message.cells[index];
            self.dirty[position] = true;
        }
        let old_cursor = self.cursor_row * MAX_COLUMNS + self.cursor_column;
        self.columns = columns;
        self.rows = rows;
        self.cursor_column = usize::from(message.cursor_column).min(columns - 1);
        self.cursor_row = usize::from(message.cursor_row).min(rows - 1);
        self.dirty[old_cursor] = true;
        self.dirty[self.cursor_row * MAX_COLUMNS + self.cursor_column] = true;
        self.applied += message.count as usize;
        Ok(())
    }

    pub fn render(
        &mut self,
        framebuffer: &mut [u8],
        width: usize,
        height: usize,
        stride: usize,
        format: PixelFormat,
        font: &mut GlyphCache,
    ) -> Result<usize, DisplayError> {
        let required = stride.checked_mul(height).ok_or(DisplayError::InvalidFramebuffer)?;
        if self.columns == 0
            || self.rows == 0
            || width < self.columns * GLYPH_WIDTH
            || height < self.rows * GLYPH_HEIGHT
            || stride < width * 4
            || framebuffer.len() < required
        {
            return Err(DisplayError::InvalidFramebuffer);
        }
        let mut rendered = 0;
        for row in 0..self.rows {
            for column in 0..self.columns {
                let index = row * MAX_COLUMNS + column;
                if !self.dirty[index] {
                    continue;
                }
                let cell = self.cells[index];
                let glyph_id = font.lookup(cell.codepoint);
                let glyph = font.glyph(glyph_id).ok_or(DisplayError::InvalidFramebuffer)?;
                for glyph_row in 0..GLYPH_HEIGHT {
                    for glyph_column in 0..GLYPH_WIDTH {
                        let foreground = glyph.rows[glyph_row] & (1 << (7 - glyph_column)) != 0;
                        let color = if foreground { cell.foreground } else { cell.background };
                        let pixel = row * GLYPH_HEIGHT * stride
                            + glyph_row * stride
                            + (column * GLYPH_WIDTH + glyph_column) * 4;
                        let red = ((color >> 16) & 0xff) as u8;
                        let green = ((color >> 8) & 0xff) as u8;
                        let blue = (color & 0xff) as u8;
                        let bytes = match format {
                            PixelFormat::Rgb8 => [red, green, blue, 0],
                            PixelFormat::Bgr8 => [blue, green, red, 0],
                        };
                        framebuffer[pixel..pixel + 4].copy_from_slice(&bytes);
                    }
                }
                self.dirty[index] = false;
                rendered += 1;
            }
        }
        Ok(rendered)
    }
}

impl Default for Display {
    fn default() -> Self {
        Self::new(1)
    }
}

const _: () = assert!(core::mem::size_of::<Display>() <= logos_abi::MAX_SERVICE_IMAGE_BYTES);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_and_cache_are_bounded() {
        let mut cache = GlyphCache::new();
        assert_eq!(cache.lookup(0xd800), cache.lookup(REPLACEMENT_SCALAR));
        for scalar in 0..MAX_GLYPH_CACHE as u32 {
            cache.lookup(scalar + 0x100);
        }
        assert_eq!(cache.len(), MAX_GLYPH_CACHE);
    }

    #[test]
    fn dirty_cells_render_once() {
        let mut display = Display::new(7);
        let mut message = RenderMessage::empty(MessageKind::FullRedraw);
        message.columns = 80;
        message.rows = 25;
        display.apply(7, &message).unwrap();
        let mut framebuffer = std::vec![0; 640 * 400 * 4];
        let mut font = GlyphCache::new();
        assert_eq!(
            display.render(&mut framebuffer, 640, 400, 640 * 4, PixelFormat::Bgr8, &mut font),
            Ok(80 * 25)
        );
        assert_eq!(
            display.render(&mut framebuffer, 640, 400, 640 * 4, PixelFormat::Bgr8, &mut font),
            Ok(0)
        );
    }
}
