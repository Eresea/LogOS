//! Display service state: cells in, pixels out.

use crate::boot_resources::PixelFormat;
use crate::font::{GLYPH_HEIGHT, GLYPH_WIDTH, GlyphCache};
use crate::terminal_abi::{Cell, MAX_COLUMNS, MAX_ROWS, MessageKind, RenderMessage};

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

    pub const fn generation(&self) -> u16 {
        self.generation
    }
    pub const fn size(&self) -> (usize, usize) {
        (self.columns, self.rows)
    }
    pub const fn cursor(&self) -> (usize, usize) {
        (self.cursor_column, self.cursor_row)
    }
    pub const fn applied_cells(&self) -> usize {
        self.applied
    }

    pub fn replace_generation(&mut self, generation: u16) {
        self.generation = generation;
        self.cells.fill(Cell::EMPTY);
        self.dirty.fill(true);
        self.applied = 0;
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
            || message.count as usize > message.cells.len()
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

    /// Rasterize dirty cells into a 32-bit GOP-style framebuffer.
    pub fn render(
        &mut self,
        framebuffer: &mut [u8],
        width: usize,
        height: usize,
        stride: usize,
        format: PixelFormat,
        font: &mut GlyphCache,
    ) -> Result<usize, DisplayError> {
        if self.columns == 0
            || self.rows == 0
            || width < self.columns * GLYPH_WIDTH
            || height < self.rows * GLYPH_HEIGHT
            || stride < width * 4
            || framebuffer.len() < stride * height
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
                let glyph = if let Some(glyph) = font.glyph(glyph_id) {
                    glyph
                } else {
                    let fallback_id = font.lookup(crate::font::REPLACEMENT_SCALAR);
                    font.glyph(fallback_id).unwrap()
                };
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
                        match format {
                            PixelFormat::Rgb8 => framebuffer[pixel..pixel + 4]
                                .copy_from_slice(&[red, green, blue, 0]),
                            PixelFormat::Bgr8 => framebuffer[pixel..pixel + 4]
                                .copy_from_slice(&[blue, green, red, 0]),
                        }
                    }
                }
                self.dirty[index] = false;
                rendered += 1;
            }
        }
        Ok(rendered)
    }

    pub fn cell(&self, column: usize, row: usize) -> Option<Cell> {
        (column < self.columns && row < self.rows).then(|| self.cells[row * MAX_COLUMNS + column])
    }
}

impl Default for Display {
    fn default() -> Self {
        Self::new(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::Terminal;

    #[test]
    fn display_applies_terminal_diffs_and_rejects_stale_pages() {
        let mut terminal = Terminal::new();
        let mut display = Display::new(7);
        while let Some(message) = terminal.next_render() {
            display.apply(7, &message).unwrap();
        }
        assert_eq!(display.size(), (80, 25));
        terminal.feed(b"A");
        while let Some(message) = terminal.next_render() {
            display.apply(7, &message).unwrap();
        }
        assert_eq!(display.cell(0, 0).unwrap().codepoint, b'A' as u32);
        assert_eq!(
            display.apply(6, &RenderMessage::empty(MessageKind::RenderCells)),
            Err(DisplayError::StaleGeneration)
        );
    }

    #[test]
    fn dirty_cells_rasterize_once_with_gop_pixel_order() {
        let mut terminal = Terminal::new();
        let mut display = Display::new(7);
        while let Some(message) = terminal.next_render() {
            display.apply(7, &message).unwrap();
        }
        terminal.feed(b"l");
        while let Some(message) = terminal.next_render() {
            display.apply(7, &message).unwrap();
        }
        let mut font = GlyphCache::new();
        let mut framebuffer = std::vec![0u8; 640 * 400 * 4];
        assert_eq!(
            display
                .render(&mut framebuffer, 640, 400, 640 * 4, PixelFormat::Bgr8, &mut font)
                .unwrap(),
            80 * 25
        );
        assert_eq!(
            display
                .render(&mut framebuffer, 640, 400, 640 * 4, PixelFormat::Bgr8, &mut font)
                .unwrap(),
            0
        );
        assert!(framebuffer.iter().any(|byte| *byte != 0));
    }

    #[test]
    fn malformed_render_is_atomic() {
        let mut display = Display::new(7);
        let mut valid = RenderMessage::empty(MessageKind::RenderCells);
        valid.columns = 80;
        valid.rows = 25;
        valid.count = 1;
        valid.positions[0] = 0;
        valid.cells[0].codepoint = b'A' as u32;
        display.apply(7, &valid).unwrap();
        let applied = display.applied_cells();

        let mut invalid = RenderMessage::empty(MessageKind::FullRedraw);
        invalid.columns = 80;
        invalid.rows = 25;
        invalid.count = 2;
        invalid.positions[0] = 0;
        invalid.positions[1] = (25 * MAX_COLUMNS) as u16;
        invalid.cells[0].codepoint = b'X' as u32;

        assert_eq!(display.apply(7, &invalid), Err(DisplayError::InvalidMessage));
        assert_eq!(display.cell(0, 0).unwrap().codepoint, b'A' as u32);
        assert_eq!(display.applied_cells(), applied);
    }
}
