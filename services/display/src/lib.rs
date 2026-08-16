#![no_std]

#[cfg(test)]
extern crate std;

use logos_abi::{
    CELL_ATTR_BOLD, CELL_ATTR_DIM, CELL_ATTR_UNDERLINE, Cell, MAX_COLUMNS, MAX_RENDER_CELLS,
    MAX_ROWS, MessageKind, RENDER_FLAG_MORE, RenderMessage,
};

pub use logos_abi::FramebufferFormat as PixelFormat;

pub const GLYPH_WIDTH: usize = 8;
pub const GLYPH_HEIGHT: usize = 16;
pub const REPLACEMENT_SCALAR: u32 = 0xfffd;
const CURSOR_WIDTH: usize = 2;
const ASCII_FIRST: u32 = 0x20;
const ASCII_LAST: u32 = 0x7e;
const ASCII_GLYPH_COUNT: usize = (ASCII_LAST - ASCII_FIRST + 1) as usize;
// 8-bit coverage atlas generated from the bundled JetBrains Mono Regular font at 14 px,
// aligned to baseline row 13 so descenders fit inside the 16 px cell.
const FONT_DATA: &[u8] = include_bytes!("jetbrains_mono_8x16.bin");

const _: () = assert!(FONT_DATA.len() == (ASCII_GLYPH_COUNT + 1) * GLYPH_HEIGHT * GLYPH_WIDTH);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Glyph {
    pub rows: [[u8; GLYPH_WIDTH]; GLYPH_HEIGHT],
}

#[derive(Clone, Copy)]
enum GlyphOverlay {
    None,
    Acute,
    Grave,
    Cedilla,
    Degree,
    Diaeresis,
    Section,
}

fn normalize_scalar(scalar: u32) -> u32 {
    if scalar <= 0x10ffff && !(0xd800..=0xdfff).contains(&scalar) {
        scalar
    } else {
        REPLACEMENT_SCALAR
    }
}

fn embedded_glyph(scalar: u32) -> Glyph {
    let (scalar, overlay) = match scalar {
        0x00a7 => ('S' as u32, GlyphOverlay::Section),
        0x00a8 => (' ' as u32, GlyphOverlay::Diaeresis),
        0x00b0 => (' ' as u32, GlyphOverlay::Degree),
        0x00c0 => ('A' as u32, GlyphOverlay::Grave),
        0x00c7 => ('C' as u32, GlyphOverlay::Cedilla),
        0x00c8 => ('E' as u32, GlyphOverlay::Grave),
        0x00c9 => ('E' as u32, GlyphOverlay::Acute),
        0x00d9 => ('U' as u32, GlyphOverlay::Grave),
        0x00e0 => ('a' as u32, GlyphOverlay::Grave),
        0x00e7 => ('c' as u32, GlyphOverlay::Cedilla),
        0x00e8 => ('e' as u32, GlyphOverlay::Grave),
        0x00e9 => ('e' as u32, GlyphOverlay::Acute),
        0x00f9 => ('u' as u32, GlyphOverlay::Grave),
        scalar => (scalar, GlyphOverlay::None),
    };
    let mut glyph = atlas_glyph(scalar);
    apply_overlay(&mut glyph, scalar, overlay);
    glyph
}

fn atlas_glyph(scalar: u32) -> Glyph {
    let scalar = normalize_scalar(scalar);
    let glyph_index = if (ASCII_FIRST..=ASCII_LAST).contains(&scalar) {
        (scalar - ASCII_FIRST) as usize
    } else {
        ASCII_GLYPH_COUNT
    };
    let mut rows = [[0; GLYPH_WIDTH]; GLYPH_HEIGHT];
    let offset = glyph_index * GLYPH_HEIGHT * GLYPH_WIDTH;
    let mut row = 0;
    while row < GLYPH_HEIGHT {
        let start = offset + row * GLYPH_WIDTH;
        rows[row].copy_from_slice(&FONT_DATA[start..start + GLYPH_WIDTH]);
        row += 1;
    }
    Glyph { rows }
}

fn apply_overlay(glyph: &mut Glyph, scalar: u32, overlay: GlyphOverlay) {
    let uppercase = (b'A' as u32..=b'Z' as u32).contains(&scalar);
    let row = if uppercase { 1 } else { 3 };
    match overlay {
        GlyphOverlay::None => {}
        GlyphOverlay::Acute => {
            glyph.rows[row][4] = 255;
            glyph.rows[row + 1][3] = 255;
        }
        GlyphOverlay::Grave => {
            glyph.rows[row][3] = 255;
            glyph.rows[row + 1][4] = 255;
        }
        GlyphOverlay::Cedilla => {
            glyph.rows[GLYPH_HEIGHT - 2][4] = 255;
            glyph.rows[GLYPH_HEIGHT - 1][3] = 255;
        }
        GlyphOverlay::Diaeresis => {
            glyph.rows[3][2] = 255;
            glyph.rows[3][5] = 255;
        }
        GlyphOverlay::Degree => {
            glyph.rows[3][3] = 255;
            glyph.rows[3][4] = 255;
            glyph.rows[4][2] = 255;
            glyph.rows[4][5] = 255;
            glyph.rows[5][2] = 255;
            glyph.rows[5][5] = 255;
            glyph.rows[6][3] = 255;
            glyph.rows[6][4] = 255;
        }
        GlyphOverlay::Section => {
            glyph.rows[7][3] = 255;
            glyph.rows[8][3] = 255;
            glyph.rows[9][4] = 255;
        }
    }
}

fn blend_channel(background: u8, foreground: u8, coverage: u8) -> u8 {
    let coverage = u32::from(coverage);
    let value = u32::from(background) * (255 - coverage) + u32::from(foreground) * coverage + 127;
    (value / 255) as u8
}

fn blend_color(background: u32, foreground: u32, coverage: u8) -> u32 {
    if coverage == 0 {
        return background;
    }
    if coverage == u8::MAX {
        return foreground;
    }
    let red = blend_channel(
        ((background >> 16) & 0xff) as u8,
        ((foreground >> 16) & 0xff) as u8,
        coverage,
    );
    let green =
        blend_channel(((background >> 8) & 0xff) as u8, ((foreground >> 8) & 0xff) as u8, coverage);
    let blue = blend_channel(background as u8, foreground as u8, coverage);
    u32::from(red) << 16 | u32::from(green) << 8 | u32::from(blue)
}

fn styled_foreground(cell: Cell) -> u32 {
    if cell.attributes & CELL_ATTR_DIM != 0 {
        blend_color(cell.background, cell.foreground, 128)
    } else {
        cell.foreground
    }
}

fn styled_coverage(glyph: &Glyph, row: usize, column: usize, attributes: u16) -> u8 {
    let mut coverage = glyph.rows[row][column];
    if attributes & CELL_ATTR_BOLD != 0 && column > 0 {
        coverage = coverage.max(glyph.rows[row][column - 1]);
    }
    if attributes & CELL_ATTR_UNDERLINE != 0 && row == GLYPH_HEIGHT - 2 {
        coverage = 255;
    }
    coverage
}

fn is_background_only(cell: Cell, background: u32) -> bool {
    cell.codepoint == b' ' as u32
        && cell.attributes & CELL_ATTR_UNDERLINE == 0
        && cell.background == background
}

fn pixel_bytes(color: u32, format: PixelFormat) -> [u8; 4] {
    let red = ((color >> 16) & 0xff) as u8;
    let green = ((color >> 8) & 0xff) as u8;
    let blue = (color & 0xff) as u8;
    match format {
        PixelFormat::Rgb8 => [red, green, blue, 0],
        PixelFormat::Bgr8 => [blue, green, red, 0],
    }
}

fn fill_row(row: &mut [u8], pixel: [u8; 4]) {
    row[..pixel.len()].copy_from_slice(&pixel);
    let mut filled = pixel.len();
    while filled < row.len() {
        let copy_len = filled.min(row.len() - filled);
        row.copy_within(..copy_len, filled);
        filled += copy_len;
    }
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
    surface_initialized: bool,
    surface_background: u32,
    cursor_visible: bool,
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
            surface_initialized: false,
            surface_background: 0,
            cursor_visible: true,
        }
    }

    pub const fn generation(&self) -> u16 {
        self.generation
    }

    /// Rebind the renderer to a replacement producer and invalidate the old
    /// surface so the next full redraw cannot inherit stale cells.
    pub fn replace_generation(&mut self, generation: u16) {
        self.generation = generation;
        self.cells.fill(Cell::EMPTY);
        self.dirty.fill(true);
        self.surface_initialized = false;
        self.surface_background = 0;
        self.cursor_visible = true;
    }

    pub fn toggle_cursor(&mut self) -> bool {
        if self.columns == 0 || self.rows == 0 {
            return false;
        }
        self.cursor_visible = !self.cursor_visible;
        self.dirty[self.cursor_row * MAX_COLUMNS + self.cursor_column] = true;
        true
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
            || message.flags & !RENDER_FLAG_MORE != 0
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
            self.surface_initialized = false;
            self.surface_background = 0;
        }
        for index in 0..message.count as usize {
            let position = usize::from(message.positions[index]);
            let cell = message.cells[index];
            let was_unrendered = self.cells[position] == Cell::EMPTY;
            self.cells[position] = cell;
            self.dirty[position] = !(self.surface_initialized
                && was_unrendered
                && is_background_only(cell, self.surface_background));
        }
        let old_cursor = self.cursor_row * MAX_COLUMNS + self.cursor_column;
        self.columns = columns;
        self.rows = rows;
        self.cursor_column = usize::from(message.cursor_column).min(columns - 1);
        self.cursor_row = usize::from(message.cursor_row).min(rows - 1);
        self.cursor_visible = true;
        self.dirty[old_cursor] = true;
        self.dirty[self.cursor_row * MAX_COLUMNS + self.cursor_column] = true;
        Ok(())
    }

    pub fn render(
        &mut self,
        framebuffer: &mut [u8],
        width: usize,
        height: usize,
        stride: usize,
        format: PixelFormat,
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
        let first_render = !self.surface_initialized;
        if first_render {
            self.surface_background = self.cells[0].background;
            let pixel = pixel_bytes(self.surface_background, format);
            let row_bytes = width * 4;
            for row in 0..height {
                let start = row * stride;
                fill_row(&mut framebuffer[start..start + row_bytes], pixel);
            }
            self.surface_initialized = true;
        }
        for row in 0..self.rows {
            for column in 0..self.columns {
                let index = row * MAX_COLUMNS + column;
                if !self.dirty[index] {
                    continue;
                }
                let cell = self.cells[index];
                let is_cursor = row == self.cursor_row && column == self.cursor_column;
                if first_render
                    && is_background_only(cell, self.surface_background)
                    && !(is_cursor && self.cursor_visible)
                {
                    self.dirty[index] = false;
                    rendered += 1;
                    continue;
                }
                let glyph = embedded_glyph(cell.codepoint);
                let foreground = styled_foreground(cell);
                for glyph_row in 0..GLYPH_HEIGHT {
                    for glyph_column in 0..GLYPH_WIDTH {
                        let color = blend_color(
                            cell.background,
                            foreground,
                            styled_coverage(&glyph, glyph_row, glyph_column, cell.attributes),
                        );
                        let pixel = row * GLYPH_HEIGHT * stride
                            + glyph_row * stride
                            + (column * GLYPH_WIDTH + glyph_column) * 4;
                        let bytes = pixel_bytes(color, format);
                        framebuffer[pixel..pixel + 4].copy_from_slice(&bytes);
                    }
                }
                if is_cursor && self.cursor_visible {
                    let bytes = pixel_bytes(foreground, format);
                    for glyph_row in 1..GLYPH_HEIGHT - 1 {
                        for glyph_column in 0..CURSOR_WIDTH {
                            let pixel = row * GLYPH_HEIGHT * stride
                                + glyph_row * stride
                                + (column * GLYPH_WIDTH + glyph_column) * 4;
                            framebuffer[pixel..pixel + 4].copy_from_slice(&bytes);
                        }
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
    fn invalid_scalars_use_the_replacement_glyph() {
        assert_eq!(embedded_glyph(0xd800), embedded_glyph(REPLACEMENT_SCALAR));
    }

    #[test]
    fn jetbrains_mono_preserves_ascii_case_and_coverage() {
        assert_ne!(embedded_glyph('A' as u32), embedded_glyph('a' as u32));
        assert_ne!(embedded_glyph('0' as u32), embedded_glyph('O' as u32));
        assert_ne!(embedded_glyph('{' as u32), embedded_glyph('}' as u32));
        assert_eq!(embedded_glyph(0x1f600), embedded_glyph(REPLACEMENT_SCALAR));
        assert!(
            embedded_glyph('A' as u32)
                .rows
                .iter()
                .flatten()
                .any(|coverage| *coverage > 0 && *coverage < 255)
        );
        assert!(
            embedded_glyph('g' as u32).rows[GLYPH_HEIGHT - 1].iter().any(|coverage| *coverage > 0)
        );
    }

    #[test]
    fn common_french_accents_have_deterministic_glyphs() {
        assert_ne!(embedded_glyph(0xe9), embedded_glyph('e' as u32));
        assert_ne!(embedded_glyph(0xe7), embedded_glyph('c' as u32));
    }

    #[test]
    fn azerty_symbols_have_deterministic_glyphs() {
        assert_ne!(embedded_glyph(0xa7), embedded_glyph(REPLACEMENT_SCALAR));
        assert_ne!(embedded_glyph(0xa8), embedded_glyph(REPLACEMENT_SCALAR));
        assert_ne!(embedded_glyph(0xb0), embedded_glyph(REPLACEMENT_SCALAR));
    }

    #[test]
    fn coverage_blends_background_and_foreground() {
        assert_eq!(blend_color(0x102030, 0xe0d0c0, 0), 0x102030);
        assert_eq!(blend_color(0x102030, 0xe0d0c0, 255), 0xe0d0c0);
        assert_eq!(blend_color(0x000000, 0xffffff, 128), 0x808080);
    }

    #[test]
    fn cell_attributes_change_raster_style() {
        let cell = Cell {
            foreground: 0xe0d0c0,
            background: 0x102030,
            attributes: CELL_ATTR_DIM,
            ..Cell::EMPTY
        };
        assert_eq!(styled_foreground(cell), blend_color(0x102030, 0xe0d0c0, 128));
        let glyph = embedded_glyph('e' as u32);
        assert_eq!(styled_coverage(&glyph, GLYPH_HEIGHT - 2, 0, CELL_ATTR_UNDERLINE), 255);
    }

    #[test]
    fn dirty_cells_render_once() {
        let mut display = Display::new(7);
        let mut message = RenderMessage::empty(MessageKind::FullRedraw);
        message.columns = 80;
        message.rows = 25;
        display.apply(7, &message).unwrap();
        let mut framebuffer = std::vec![0; 640 * 400 * 4];
        assert_eq!(
            display.render(&mut framebuffer, 640, 400, 640 * 4, PixelFormat::Bgr8),
            Ok(80 * 25)
        );
        assert_eq!(display.render(&mut framebuffer, 640, 400, 640 * 4, PixelFormat::Bgr8), Ok(0));
    }

    #[test]
    fn initial_render_fills_background_before_incremental_cells() {
        let mut display = Display::new(1);
        let mut message = RenderMessage::empty(MessageKind::RenderCells);
        message.columns = 2;
        message.rows = 1;
        message.count = 1;
        message.positions[0] = 0;
        message.cells[0] = Cell { background: 0x102030, ..Cell::EMPTY };
        display.apply(1, &message).unwrap();

        let mut framebuffer = std::vec![0; 24 * 32 * 4];
        assert_eq!(display.render(&mut framebuffer, 24, 32, 24 * 4, PixelFormat::Bgr8), Ok(1));
        assert_eq!(&framebuffer[..4], &[0x30, 0x20, 0x10, 0]);
        let last_pixel = (24 * 32 - 1) * 4;
        assert_eq!(&framebuffer[last_pixel..last_pixel + 4], &[0x30, 0x20, 0x10, 0]);
    }

    #[test]
    fn cursor_caret_renders_and_blinks() {
        let mut display = Display::new(1);
        let mut message = RenderMessage::empty(MessageKind::RenderCells);
        message.columns = 1;
        message.rows = 1;
        message.count = 1;
        message.positions[0] = 0;
        message.cells[0] = Cell { foreground: 0xe0d0c0, background: 0x102030, ..Cell::EMPTY };
        display.apply(1, &message).unwrap();

        let mut framebuffer = std::vec![0; 8 * 16 * 4];
        display.render(&mut framebuffer, 8, 16, 8 * 4, PixelFormat::Bgr8).unwrap();
        assert_eq!(&framebuffer[(4 * 8) * 4..(4 * 8 + 1) * 4], &[0xc0, 0xd0, 0xe0, 0]);

        assert!(display.toggle_cursor());
        display.render(&mut framebuffer, 8, 16, 8 * 4, PixelFormat::Bgr8).unwrap();
        assert_eq!(&framebuffer[(4 * 8) * 4..(4 * 8 + 1) * 4], &[0x30, 0x20, 0x10, 0]);
    }

    #[test]
    fn replacement_generation_rejects_old_messages() {
        let mut display = Display::new(1);
        display.replace_generation(2);
        let message = RenderMessage::empty(MessageKind::FullRedraw);
        assert_eq!(display.apply(1, &message), Err(DisplayError::StaleGeneration));
        assert_eq!(display.apply(2, &message), Err(DisplayError::InvalidMessage));
    }
}
