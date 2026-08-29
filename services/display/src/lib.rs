#![no_std]

extern crate alloc;

#[cfg(test)]
extern crate std;

use alloc::vec::Vec;
use logos_abi::{
    CELL_ATTR_BOLD, CELL_ATTR_DIM, CELL_ATTR_UNDERLINE, Cell, GuiDrawKind, GuiNodeOperation,
    GuiRect, GuiSceneOp, MAX_COLUMNS, MAX_DISPLAY_PRESENT_RECTS, MAX_FRAMEBUFFER_BYTES,
    MAX_GUI_DAMAGE_RECTS, MAX_RENDER_CELLS, MAX_ROWS, MessageKind, RENDER_FLAG_MORE, RenderMessage,
    SurfaceHandle,
};

mod gui;

pub use gui::{GuiRegistryError, GuiRenderBackend, GuiSurfaceRegistry};

pub use logos_abi::FramebufferFormat as PixelFormat;

pub const GLYPH_WIDTH: usize = 8;
pub const GLYPH_HEIGHT: usize = 16;
pub const REPLACEMENT_SCALAR: u32 = 0xfffd;
const CURSOR_WIDTH: usize = 2;
const GUI_TILE_SIZE: u32 = 64;
// Keep incremental GUI slices short enough for input and watchdog progress.
const GUI_TILES_PER_STEP: usize = 4;
const CURSOR_LAYERS: usize = 2;
const CURSOR_DAMAGE_RECTS: usize = 2;
const LOCKSCREEN_OWNER: u32 = 12;
const ATRIUM_OWNER: u32 = 13;
const POINTER_CURSOR_WIDTH: i32 = 18;
const POINTER_CURSOR_HEIGHT: i32 = 24;
const POINTER_CURSOR_MASK: [u32; POINTER_CURSOR_HEIGHT as usize] = [
    0x00001, 0x00003, 0x00007, 0x0000f, 0x0001f, 0x0003f, 0x0007f, 0x000ff, 0x001ff, 0x003ff,
    0x007ff, 0x00fff, 0x01fff, 0x03fff, 0x07fff, 0x0ffff, 0x0ffff, 0x0fff8, 0x0fff8, 0x01ff0,
    0x01ff0, 0x03fe0, 0x03fe0, 0x07fc0,
];
const GLYPH_CACHE_ENTRIES: usize = 32;
const DIRTY_WORDS: usize = (MAX_COLUMNS * MAX_ROWS).div_ceil(usize::BITS as usize);
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
struct GlyphCacheEntry {
    scalar: u32,
    glyph: Glyph,
    valid: bool,
}

impl GlyphCacheEntry {
    const EMPTY: Self =
        Self { scalar: 0, glyph: Glyph { rows: [[0; GLYPH_WIDTH]; GLYPH_HEIGHT] }, valid: false };
}

struct GlyphCache {
    entries: [GlyphCacheEntry; GLYPH_CACHE_ENTRIES],
    next: usize,
}

#[derive(Clone, Copy)]
struct CursorLayer {
    surface: SurfaceHandle,
    x: i32,
    y: i32,
    order: u32,
}

impl CursorLayer {
    const EMPTY: Self = Self { surface: SurfaceHandle::EMPTY, x: 0, y: 0, order: 0 };

    const fn active(self) -> bool {
        self.surface.is_valid()
    }
}

impl GlyphCache {
    const fn new() -> Self {
        Self { entries: [GlyphCacheEntry::EMPTY; GLYPH_CACHE_ENTRIES], next: 0 }
    }

    fn get(&mut self, scalar: u32) -> Glyph {
        if let Some(entry) = self.entries.iter().find(|entry| entry.valid && entry.scalar == scalar)
        {
            return entry.glyph;
        }
        let glyph = embedded_glyph(scalar);
        self.entries[self.next] = GlyphCacheEntry { scalar, glyph, valid: true };
        self.next = (self.next + 1) % GLYPH_CACHE_ENTRIES;
        glyph
    }
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

fn is_uninitialized(cell: Cell) -> bool {
    cell == Cell::EMPTY
}

#[inline(always)]
fn pixel_bytes(color: u32, format: PixelFormat) -> [u8; 4] {
    let red = ((color >> 16) & 0xff) as u8;
    let green = ((color >> 8) & 0xff) as u8;
    let blue = (color & 0xff) as u8;
    match format {
        PixelFormat::Rgb8 => [red, green, blue, 0],
        PixelFormat::Bgr8 => [blue, green, red, 0],
    }
}

const fn cursor_mask_has(row: i32, column: i32) -> bool {
    row >= 0
        && row < POINTER_CURSOR_HEIGHT
        && column >= 0
        && column < POINTER_CURSOR_WIDTH
        && POINTER_CURSOR_MASK[row as usize] & (1u32 << column) != 0
}

const fn cursor_outline_mask() -> [u32; (POINTER_CURSOR_HEIGHT + 2) as usize] {
    let mut outline = [0; (POINTER_CURSOR_HEIGHT + 2) as usize];
    let mut row = 0;
    while row < POINTER_CURSOR_HEIGHT + 2 {
        let actual_row = row - 1;
        let mut column = 0;
        while column < POINTER_CURSOR_WIDTH + 2 {
            let actual_column = column - 1;
            if !cursor_mask_has(actual_row, actual_column) {
                let mut neighbor_row = -1;
                let mut adjacent = false;
                while neighbor_row <= 1 {
                    let mut neighbor_column = -1;
                    while neighbor_column <= 1 {
                        adjacent |= cursor_mask_has(
                            actual_row + neighbor_row,
                            actual_column + neighbor_column,
                        );
                        neighbor_column += 1;
                    }
                    neighbor_row += 1;
                }
                if adjacent {
                    outline[row as usize] |= 1u32 << column as u32;
                }
            }
            column += 1;
        }
        row += 1;
    }
    outline
}

const POINTER_CURSOR_OUTLINE: [u32; (POINTER_CURSOR_HEIGHT + 2) as usize] = cursor_outline_mask();

#[allow(clippy::too_many_arguments)]
#[inline(always)]
fn draw_cursor_pixel(
    framebuffer: &mut [u8],
    width: usize,
    height: usize,
    stride: usize,
    format: PixelFormat,
    x: i32,
    y: i32,
    color: u32,
    alpha: u8,
    clip: GuiRect,
) {
    if alpha == 0 || !clip.contains(x, y) || x < 0 || y < 0 {
        return;
    }
    let (x, y) = (x as usize, y as usize);
    if x >= width || y >= height {
        return;
    }
    let offset = y * stride + x * 4;
    if alpha == u8::MAX {
        framebuffer[offset..offset + 4].copy_from_slice(&pixel_bytes(color, format));
        return;
    }
    let existing = &framebuffer[offset..offset + 4];
    let (red, green, blue) = match format {
        PixelFormat::Rgb8 => (existing[0], existing[1], existing[2]),
        PixelFormat::Bgr8 => (existing[2], existing[1], existing[0]),
    };
    let blended = (u32::from(blend_channel(red, ((color >> 16) & 0xff) as u8, alpha)) << 16)
        | (u32::from(blend_channel(green, ((color >> 8) & 0xff) as u8, alpha)) << 8)
        | u32::from(blend_channel(blue, color as u8, alpha));
    framebuffer[offset..offset + 4].copy_from_slice(&pixel_bytes(blended, format));
}

#[allow(clippy::too_many_arguments)]
#[inline(always)]
fn draw_cursor_span(
    framebuffer: &mut [u8],
    width: usize,
    height: usize,
    stride: usize,
    format: PixelFormat,
    left: i32,
    right: i32,
    y: i32,
    color: u32,
    alpha: u8,
    clip: GuiRect,
) {
    if y < clip.y || y >= clip.y.saturating_add(clip.height as i32) || y < 0 {
        return;
    }
    let left = left.max(clip.x).max(0).min(width as i32) as usize;
    let right =
        right.min(clip.x.saturating_add(clip.width as i32)).min(width as i32).max(0) as usize;
    if left >= right || y as usize >= height {
        return;
    }
    if alpha == u8::MAX {
        let pixel = pixel_bytes(color, format);
        let start = y as usize * stride + left * 4;
        let end = y as usize * stride + right * 4;
        fill_row(&mut framebuffer[start..end], pixel);
    } else {
        for x in left..right {
            draw_cursor_pixel(
                framebuffer,
                width,
                height,
                stride,
                format,
                x as i32,
                y,
                color,
                alpha,
                clip,
            );
        }
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

fn terminal_cell_rect(surface: GuiRect, row: usize, column: usize) -> GuiRect {
    GuiRect::new(
        surface.x.saturating_add((column * GLYPH_WIDTH) as i32),
        surface.y.saturating_add((row * GLYPH_HEIGHT) as i32),
        GLYPH_WIDTH as u32,
        GLYPH_HEIGHT as u32,
    )
}

fn intersect(left: GuiRect, right: GuiRect) -> GuiRect {
    let x = left.x.max(right.x);
    let y = left.y.max(right.y);
    let right_edge =
        left.x.saturating_add(left.width as i32).min(right.x.saturating_add(right.width as i32));
    let bottom =
        left.y.saturating_add(left.height as i32).min(right.y.saturating_add(right.height as i32));
    if right_edge <= x || bottom <= y {
        GuiRect::EMPTY
    } else {
        GuiRect::new(x, y, (right_edge - x) as u32, (bottom - y) as u32)
    }
}

fn union_rect(left: GuiRect, right: GuiRect) -> GuiRect {
    let x = left.x.min(right.x);
    let y = left.y.min(right.y);
    let right_edge =
        left.x.saturating_add(left.width as i32).max(right.x.saturating_add(right.width as i32));
    let bottom =
        left.y.saturating_add(left.height as i32).max(right.y.saturating_add(right.height as i32));
    GuiRect::new(x, y, right_edge.saturating_sub(x) as u32, bottom.saturating_sub(y) as u32)
}

fn rects_touch(left: GuiRect, right: GuiRect) -> bool {
    let left_right = left.x.saturating_add(left.width as i32);
    let right_right = right.x.saturating_add(right.width as i32);
    let left_bottom = left.y.saturating_add(left.height as i32);
    let right_bottom = right.y.saturating_add(right.height as i32);
    left.x <= right_right
        && right.x <= left_right
        && left.y <= right_bottom
        && right.y <= left_bottom
}

fn append_present_rect(
    damage: &mut [GuiRect; MAX_DISPLAY_PRESENT_RECTS],
    count: &mut usize,
    rect: GuiRect,
) -> bool {
    if rect.is_empty() {
        return true;
    }
    for existing in damage[..*count].iter_mut() {
        if rects_touch(*existing, rect) {
            *existing = union_rect(*existing, rect);
            return true;
        }
    }
    if *count == damage.len() {
        return false;
    }
    damage[*count] = rect;
    *count += 1;
    true
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
    dirty: [u64; DIRTY_WORDS],
    glyph_cache: GlyphCache,
    surface_initialized: bool,
    surface_background: u32,
    cursor_visible: bool,
    gui: GuiSurfaceRegistry,
    gui_background: Option<u32>,
    gui_background_pending: bool,
    gui_damage: [GuiRect; MAX_GUI_DAMAGE_RECTS],
    gui_damage_count: usize,
    gui_tile_index: usize,
    gui_tile_x: i32,
    gui_tile_y: i32,
    cursor_layers: [CursorLayer; CURSOR_LAYERS],
    cursor_order: u32,
    cursor_damage: [GuiRect; CURSOR_DAMAGE_RECTS],
    cursor_damage_count: usize,
    hardware_cursor: bool,
    backbuffer: Option<Vec<u8>>,
    presented: bool,
    presented_full: bool,
    presented_damage: [GuiRect; MAX_DISPLAY_PRESENT_RECTS],
    presented_damage_count: usize,
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
            dirty: [0; DIRTY_WORDS],
            glyph_cache: GlyphCache::new(),
            surface_initialized: false,
            surface_background: 0,
            cursor_visible: true,
            gui: GuiSurfaceRegistry::new(),
            gui_background: None,
            gui_background_pending: false,
            gui_damage: [GuiRect::EMPTY; MAX_GUI_DAMAGE_RECTS],
            gui_damage_count: 0,
            gui_tile_index: 0,
            gui_tile_x: 0,
            gui_tile_y: 0,
            cursor_layers: [CursorLayer::EMPTY; CURSOR_LAYERS],
            cursor_order: 0,
            cursor_damage: [GuiRect::EMPTY; CURSOR_DAMAGE_RECTS],
            cursor_damage_count: 0,
            hardware_cursor: false,
            backbuffer: None,
            presented: false,
            presented_full: false,
            presented_damage: [GuiRect::EMPTY; MAX_DISPLAY_PRESENT_RECTS],
            presented_damage_count: 0,
        }
    }

    pub const fn generation(&self) -> u16 {
        self.generation
    }

    pub const fn presented(&self) -> bool {
        self.presented
    }

    fn mark_dirty(&mut self, index: usize) {
        self.dirty[index / 64] |= 1u64 << (index % 64);
    }

    fn clear_dirty(&mut self, index: usize) {
        self.dirty[index / 64] &= !(1u64 << (index % 64));
    }

    fn set_all_dirty(&mut self, dirty: bool) {
        self.dirty.fill(if dirty { u64::MAX } else { 0 });
    }

    fn cursor_layer_index(owner: u32) -> Option<usize> {
        match owner {
            LOCKSCREEN_OWNER => Some(0),
            ATRIUM_OWNER => Some(1),
            _ => None,
        }
    }

    fn cursor_bounds(x: i32, y: i32) -> GuiRect {
        GuiRect::new(
            x.saturating_sub(1),
            y.saturating_sub(1),
            POINTER_CURSOR_WIDTH as u32 + 3,
            POINTER_CURSOR_HEIGHT as u32 + 3,
        )
    }

    /// Register a compositor-owned cursor surface. Its retained scene stays
    /// empty; motion is painted directly over the composed backbuffer.
    pub fn register_cursor_surface(&mut self, owner: u32, surface: SurfaceHandle) {
        let Some(index) = Self::cursor_layer_index(owner) else { return };
        let old = self.cursor_layers[index];
        if old.active() {
            self.gui.invalidate_rect(Self::cursor_bounds(old.x, old.y));
        }
        self.cursor_order = self.cursor_order.wrapping_add(1).max(1);
        self.cursor_layers[index] =
            CursorLayer { surface, x: 320, y: 200, order: self.cursor_order };
        self.gui.invalidate_rect(Self::cursor_bounds(320, 200));
    }

    pub fn is_cursor_surface(&self, surface: SurfaceHandle) -> bool {
        self.cursor_layers.iter().any(|layer| layer.active() && layer.surface == surface)
    }

    pub fn unregister_cursor_surface(&mut self, surface: SurfaceHandle) {
        for layer in &mut self.cursor_layers {
            if layer.active() && layer.surface == surface {
                let old = *layer;
                self.gui.invalidate_rect(Self::cursor_bounds(old.x, old.y));
                *layer = CursorLayer::EMPTY;
            }
        }
    }

    /// Apply the existing cursor scene message without sending it through the
    /// retained-scene compositor. Returns whether the surface is a cursor.
    pub fn apply_cursor_scene_op(&mut self, op: GuiSceneOp) -> bool {
        let index = if let Some(index) = self
            .cursor_layers
            .iter()
            .position(|layer| layer.active() && layer.surface == op.surface)
        {
            index
        } else {
            let is_cursor_shape = op.operation == GuiNodeOperation::Upsert
                && op.node_id == 1
                && op.command.kind == GuiDrawKind::FillRect
                && op.command.width == 3
                && op.command.height == 14;
            if !is_cursor_shape || !self.gui.contains(op.surface) {
                return false;
            }
            let Some(index) = Self::cursor_layer_index(op.surface.owner) else { return false };
            self.register_cursor_surface(op.surface.owner, op.surface);
            index
        };
        if !op.is_valid() {
            return true;
        }
        if op.operation != GuiNodeOperation::Upsert || op.node_id != 1 {
            return true;
        }
        let old = self.cursor_layers[index];
        if !self.hardware_cursor {
            let old_bounds = Self::cursor_bounds(old.x, old.y);
            let new_bounds = Self::cursor_bounds(op.command.x, op.command.y);
            if self.can_repaint_cursor() {
                self.queue_cursor_damage(old_bounds);
                self.queue_cursor_damage(new_bounds);
            } else {
                self.gui.invalidate_rect(old_bounds);
                self.gui.invalidate_rect(new_bounds);
            }
        }
        self.cursor_layers[index].x = op.command.x;
        self.cursor_layers[index].y = op.command.y;
        self.cursor_order = self.cursor_order.wrapping_add(1).max(1);
        self.cursor_layers[index].order = self.cursor_order;
        true
    }

    fn can_repaint_cursor(&self) -> bool {
        // The framebuffer is updated in this same display task, so cursor
        // repainting remains safe while GUI tiles are being composed. Later
        // GUI tiles restore their own pixels before drawing the current cursor.
        self.backbuffer.is_some() && self.surface_initialized && !self.gui_background_pending
    }

    fn queue_cursor_damage(&mut self, rect: GuiRect) {
        if rect.is_empty() {
            return;
        }
        for existing in self.cursor_damage[..self.cursor_damage_count].iter_mut() {
            if rects_touch(*existing, rect) {
                *existing = union_rect(*existing, rect);
                return;
            }
        }
        if self.cursor_damage_count < CURSOR_DAMAGE_RECTS {
            self.cursor_damage[self.cursor_damage_count] = rect;
            self.cursor_damage_count += 1;
        } else {
            let mut combined = rect;
            for existing in self.cursor_damage[..self.cursor_damage_count].iter().copied() {
                combined = union_rect(combined, existing);
            }
            self.cursor_damage[0] = combined;
            self.cursor_damage_count = 1;
        }
    }

    /// Repaints only queued software-cursor rectangles over the current
    /// backbuffer, avoiding the tiled GUI path for pointer motion.
    pub fn repaint_cursor(
        &mut self,
        framebuffer: &mut [u8],
        width: usize,
        height: usize,
        stride: usize,
        format: PixelFormat,
    ) -> bool {
        if self.hardware_cursor || self.cursor_damage_count == 0 {
            return false;
        }
        let required = match stride.checked_mul(height) {
            Some(required) if stride >= width * 4 && framebuffer.len() >= required => required,
            _ => return false,
        };
        if self.backbuffer.as_ref().is_none_or(|backbuffer| backbuffer.len() < required) {
            return false;
        }
        let screen = GuiRect::new(0, 0, width as u32, height as u32);
        let damage = self.cursor_damage;
        let count = self.cursor_damage_count;
        self.cursor_damage = [GuiRect::EMPTY; CURSOR_DAMAGE_RECTS];
        self.cursor_damage_count = 0;
        for rect in damage[..count].iter().copied() {
            let rect = intersect(rect, screen);
            if rect.is_empty() {
                continue;
            }
            self.present_damage(framebuffer, width, height, stride, &[rect]);
            self.render_cursor_layers(framebuffer, width, height, stride, format, rect);
        }
        true
    }

    fn render_cursor_layers(
        &self,
        framebuffer: &mut [u8],
        width: usize,
        height: usize,
        stride: usize,
        format: PixelFormat,
        clip: GuiRect,
    ) {
        if self.hardware_cursor {
            return;
        }
        let Some(layer) = self.active_cursor_layer() else {
            return;
        };
        let bounds = Self::cursor_bounds(layer.x, layer.y);
        if intersect(bounds, clip).is_empty() {
            return;
        }
        for row in 0..POINTER_CURSOR_HEIGHT {
            let mask = POINTER_CURSOR_MASK[row as usize];
            if mask == 0 {
                continue;
            }
            let left = mask.trailing_zeros() as i32;
            let right = (32 - mask.leading_zeros()) as i32;
            draw_cursor_span(
                framebuffer,
                width,
                height,
                stride,
                format,
                layer.x.saturating_add(left + 1),
                layer.x.saturating_add(right + 1),
                layer.y.saturating_add(row + 2),
                0x000000,
                150,
                clip,
            );
            let mut outline = POINTER_CURSOR_OUTLINE[(row + 1) as usize];
            while outline != 0 {
                let column = outline.trailing_zeros() as i32 - 1;
                outline &= outline - 1;
                draw_cursor_pixel(
                    framebuffer,
                    width,
                    height,
                    stride,
                    format,
                    layer.x.saturating_add(column),
                    layer.y.saturating_add(row),
                    0x101820,
                    u8::MAX,
                    clip,
                );
            }
            draw_cursor_span(
                framebuffer,
                width,
                height,
                stride,
                format,
                layer.x.saturating_add(left),
                layer.x.saturating_add(right),
                layer.y.saturating_add(row),
                0xffffff,
                u8::MAX,
                clip,
            );
        }
    }

    fn ensure_backbuffer(&mut self, bytes: usize) -> Result<(), DisplayError> {
        if bytes > MAX_FRAMEBUFFER_BYTES {
            return Err(DisplayError::InvalidFramebuffer);
        }
        if self.backbuffer.as_ref().is_none_or(|buffer| buffer.len() != bytes) {
            let mut buffer = Vec::new();
            buffer.try_reserve_exact(bytes).map_err(|_| DisplayError::InvalidFramebuffer)?;
            buffer.resize(bytes, 0);
            self.backbuffer = Some(buffer);
        }
        Ok(())
    }

    fn present_all(&mut self, framebuffer: &mut [u8], width: usize, height: usize, stride: usize) {
        let Some(backbuffer) = self.backbuffer.as_ref() else { return };
        let row_bytes = width * 4;
        for row in 0..height {
            let start = row * stride;
            framebuffer[start..start + row_bytes]
                .copy_from_slice(&backbuffer[start..start + row_bytes]);
        }
        self.presented = true;
        self.presented_full = true;
        self.presented_damage_count = 0;
    }

    fn present_damage(
        &mut self,
        framebuffer: &mut [u8],
        width: usize,
        height: usize,
        stride: usize,
        damage: &[GuiRect],
    ) {
        let Some(backbuffer) = self.backbuffer.as_ref() else { return };
        let screen = GuiRect::new(0, 0, width as u32, height as u32);
        for rect in damage.iter().copied() {
            let rect = intersect(rect, screen);
            if rect.is_empty() {
                continue;
            }
            let left = rect.x as usize;
            let right = left + rect.width as usize;
            for row in rect.y as usize..rect.y as usize + rect.height as usize {
                let start = row * stride + left * 4;
                let end = row * stride + right * 4;
                framebuffer[start..end].copy_from_slice(&backbuffer[start..end]);
            }
            self.presented = true;
            if !self.presented_full {
                if self.presented_damage_count < MAX_DISPLAY_PRESENT_RECTS {
                    self.presented_damage[self.presented_damage_count] = rect;
                    self.presented_damage_count += 1;
                } else {
                    self.presented_full = true;
                    self.presented_damage_count = 0;
                }
            }
        }
    }

    /// Rebind the renderer to a replacement producer and invalidate the old
    /// surface so the next full redraw cannot inherit stale cells.
    pub fn replace_generation(&mut self, generation: u16) {
        self.generation = generation;
        self.columns = 0;
        self.rows = 0;
        self.cursor_column = 0;
        self.cursor_row = 0;
        self.cells.fill(Cell::EMPTY);
        self.set_all_dirty(true);
        self.glyph_cache = GlyphCache::new();
        self.surface_initialized = false;
        self.surface_background = 0;
        self.cursor_visible = true;
        self.gui = GuiSurfaceRegistry::new();
        self.gui_background = None;
        self.gui_background_pending = false;
        self.gui_damage = [GuiRect::EMPTY; MAX_GUI_DAMAGE_RECTS];
        self.gui_damage_count = 0;
        self.gui_tile_index = 0;
        self.gui_tile_x = 0;
        self.gui_tile_y = 0;
        self.cursor_layers = [CursorLayer::EMPTY; CURSOR_LAYERS];
        self.cursor_order = 0;
        self.cursor_damage = [GuiRect::EMPTY; CURSOR_DAMAGE_RECTS];
        self.cursor_damage_count = 0;
        self.hardware_cursor = false;
        self.presented = false;
        self.presented_full = false;
        self.presented_damage_count = 0;
        self.presented_damage = [GuiRect::EMPTY; MAX_DISPLAY_PRESENT_RECTS];
    }

    pub fn toggle_cursor(&mut self) -> bool {
        if self.columns == 0 || self.rows == 0 {
            return false;
        }
        self.cursor_visible = !self.cursor_visible;
        self.mark_dirty(self.cursor_row * MAX_COLUMNS + self.cursor_column);
        true
    }

    pub fn set_hardware_cursor(&mut self, active: bool) {
        if self.hardware_cursor == active {
            return;
        }
        for layer in self.cursor_layers.iter().copied().filter(|layer| layer.active()) {
            self.gui.invalidate_rect(Self::cursor_bounds(layer.x, layer.y));
        }
        self.hardware_cursor = active;
    }

    pub fn cursor_position(&self) -> Option<(i16, i16)> {
        let layer = self.active_cursor_layer()?;
        Some((layer.x as i16, layer.y as i16))
    }

    fn active_cursor_layer(&self) -> Option<CursorLayer> {
        let mut active = None;
        for layer in self.cursor_layers.iter().copied().filter(|layer| layer.active()) {
            if active.is_none_or(|current: CursorLayer| layer.order > current.order) {
                active = Some(layer);
            }
        }
        active
    }

    pub fn apply(&mut self, generation: u16, message: &RenderMessage) -> Result<(), DisplayError> {
        if generation != self.generation {
            return Err(DisplayError::StaleGeneration);
        }
        if let Some((surface, _)) = self.gui.terminal_surface() {
            if message.surface != surface {
                return Err(DisplayError::InvalidMessage);
            }
        } else if message.surface.is_valid() {
            return Err(DisplayError::InvalidMessage);
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
            self.set_all_dirty(true);
            self.surface_initialized = false;
            self.surface_background = 0;
        }
        for index in 0..message.count as usize {
            let position = usize::from(message.positions[index]);
            let cell = message.cells[index];
            let was_unrendered = self.cells[position] == Cell::EMPTY;
            self.cells[position] = cell;
            if !(self.surface_initialized
                && was_unrendered
                && is_background_only(cell, self.surface_background))
            {
                self.mark_dirty(position);
            } else {
                self.clear_dirty(position);
            }
        }
        let old_cursor = self.cursor_row * MAX_COLUMNS + self.cursor_column;
        self.columns = columns;
        self.rows = rows;
        self.cursor_column = usize::from(message.cursor_column).min(columns - 1);
        self.cursor_row = usize::from(message.cursor_row).min(rows - 1);
        self.cursor_visible = true;
        self.mark_dirty(old_cursor);
        self.mark_dirty(self.cursor_row * MAX_COLUMNS + self.cursor_column);
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
        self.ensure_backbuffer(required)?;
        let dirty = &mut self.dirty;
        let backbuffer = self.backbuffer.as_mut().unwrap();
        let mut rendered = 0;
        let first_render = !self.surface_initialized;
        let mut present_damage = [GuiRect::EMPTY; MAX_DISPLAY_PRESENT_RECTS];
        let mut present_damage_count = 0;
        let mut present_full = first_render;
        if first_render {
            self.surface_background = self.gui_background.unwrap_or(self.cells[0].background);
            let pixel = pixel_bytes(self.surface_background, format);
            let row_bytes = width * 4;
            for row in 0..height {
                let start = row * stride;
                fill_row(&mut backbuffer[start..start + row_bytes], pixel);
            }
            self.surface_initialized = true;
        }
        if let Some(surface) = self.gui.terminal_bounds() {
            let cell_count = self.rows * MAX_COLUMNS;
            for (word_index, word) in dirty.iter_mut().enumerate() {
                let mut bits = *word;
                while bits != 0 {
                    let bit = bits.trailing_zeros() as usize;
                    bits &= bits - 1;
                    let index = word_index * 64 + bit;
                    if index >= cell_count || index % MAX_COLUMNS >= self.columns {
                        continue;
                    }
                    let row = index / MAX_COLUMNS;
                    let column = index % MAX_COLUMNS;
                    self.gui.invalidate_rect(terminal_cell_rect(surface, row, column));
                    *word &= !(1u64 << bit);
                    rendered += 1;
                }
            }
            return Ok(rendered);
        }
        let cell_count = self.rows * MAX_COLUMNS;
        for (word_index, word) in dirty.iter_mut().enumerate() {
            let mut bits = *word;
            while bits != 0 {
                let bit = bits.trailing_zeros() as usize;
                bits &= bits - 1;
                let index = word_index * 64 + bit;
                if index >= cell_count || index % MAX_COLUMNS >= self.columns {
                    continue;
                }
                let row = index / MAX_COLUMNS;
                let column = index % MAX_COLUMNS;
                let cell_rect = GuiRect::new(
                    (column * GLYPH_WIDTH) as i32,
                    (row * GLYPH_HEIGHT) as i32,
                    GLYPH_WIDTH as u32,
                    GLYPH_HEIGHT as u32,
                );
                if !append_present_rect(&mut present_damage, &mut present_damage_count, cell_rect) {
                    present_full = true;
                }
                if self.gui_background.is_some() {
                    *word &= !(1u64 << bit);
                    rendered += 1;
                    continue;
                }
                self.gui.invalidate_rect(cell_rect);
                let cell = self.cells[index];
                let is_cursor = row == self.cursor_row && column == self.cursor_column;
                if first_render
                    && (is_uninitialized(cell) || is_background_only(cell, self.surface_background))
                    && !(is_cursor && self.cursor_visible)
                {
                    *word &= !(1u64 << bit);
                    rendered += 1;
                    continue;
                }
                let glyph = self.glyph_cache.get(cell.codepoint);
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
                        backbuffer[pixel..pixel + 4].copy_from_slice(&bytes);
                    }
                }
                if is_cursor && self.cursor_visible {
                    let bytes = pixel_bytes(foreground, format);
                    for glyph_row in 1..GLYPH_HEIGHT - 1 {
                        for glyph_column in 0..CURSOR_WIDTH {
                            let pixel = row * GLYPH_HEIGHT * stride
                                + glyph_row * stride
                                + (column * GLYPH_WIDTH + glyph_column) * 4;
                            backbuffer[pixel..pixel + 4].copy_from_slice(&bytes);
                        }
                    }
                }
                *word &= !(1u64 << bit);
                rendered += 1;
            }
        }
        if rendered != 0 {
            let (damage, damage_count) = self.gui.take_damage();
            for rect in damage[..damage_count].iter().copied() {
                if !append_present_rect(&mut present_damage, &mut present_damage_count, rect) {
                    present_full = true;
                }
            }
            rendered += self.gui.render(
                &mut self.glyph_cache,
                backbuffer,
                width,
                height,
                stride,
                format,
                &damage,
                damage_count,
            );
        }
        let present = first_render || rendered != 0;
        if present {
            if present_full {
                self.present_all(framebuffer, width, height, stride);
            } else {
                self.present_damage(
                    framebuffer,
                    width,
                    height,
                    stride,
                    &present_damage[..present_damage_count],
                );
            }
        }
        Ok(rendered)
    }

    pub fn invalidate_terminal(&mut self) {
        self.set_all_dirty(true);
        self.surface_initialized = false;
    }

    pub fn gui(&self) -> &GuiSurfaceRegistry {
        &self.gui
    }

    pub fn gui_mut(&mut self) -> &mut GuiSurfaceRegistry {
        &mut self.gui
    }

    #[allow(clippy::too_many_arguments)]
    fn render_terminal_surface(
        cells: &[Cell; MAX_COLUMNS * MAX_ROWS],
        cell_columns: usize,
        cell_rows: usize,
        cursor_column: usize,
        cursor_row: usize,
        cursor_visible: bool,
        glyph_cache: &mut GlyphCache,
        framebuffer: &mut [u8],
        width: usize,
        height: usize,
        stride: usize,
        format: PixelFormat,
        surface: GuiRect,
        damage: &[GuiRect; logos_abi::MAX_GUI_DAMAGE_RECTS],
        damage_count: usize,
    ) -> usize {
        let screen = GuiRect::new(0, 0, width as u32, height as u32);
        let columns = cell_columns.min(surface.width as usize / GLYPH_WIDTH);
        let rows = cell_rows.min(surface.height as usize / GLYPH_HEIGHT);
        let mut damage_bounds = GuiRect::EMPTY;
        for damage_rect in damage[..damage_count].iter().copied() {
            let clipped = intersect(intersect(damage_rect, surface), screen);
            if clipped.is_empty() {
                continue;
            }
            damage_bounds =
                if damage_bounds.is_empty() { clipped } else { union_rect(damage_bounds, clipped) };
        }
        if damage_bounds.is_empty() {
            return 0;
        }
        let local_left = damage_bounds.x.saturating_sub(surface.x).max(0) as usize;
        let local_top = damage_bounds.y.saturating_sub(surface.y).max(0) as usize;
        let local_right = damage_bounds
            .x
            .saturating_add(damage_bounds.width as i32)
            .saturating_sub(surface.x)
            .max(0) as usize;
        let local_bottom = damage_bounds
            .y
            .saturating_add(damage_bounds.height as i32)
            .saturating_sub(surface.y)
            .max(0) as usize;
        let first_column = (local_left / GLYPH_WIDTH).min(columns);
        let last_column = local_right.saturating_add(GLYPH_WIDTH - 1) / GLYPH_WIDTH;
        let first_row = (local_top / GLYPH_HEIGHT).min(rows);
        let last_row = local_bottom.saturating_add(GLYPH_HEIGHT - 1) / GLYPH_HEIGHT;
        let mut rendered = 0;
        for row in first_row..last_row.min(rows) {
            for column in first_column..last_column.min(columns) {
                let cell = cells[row * MAX_COLUMNS + column];
                let cell_rect = terminal_cell_rect(surface, row, column);
                let is_cursor = row == cursor_row && column == cursor_column;
                let glyph = glyph_cache.get(cell.codepoint);
                let foreground = styled_foreground(cell);
                let background_bytes = pixel_bytes(cell.background, format);
                let foreground_bytes = pixel_bytes(foreground, format);
                for damage_rect in damage[..damage_count].iter().copied() {
                    let clip = intersect(intersect(cell_rect, damage_rect), screen);
                    if clip.is_empty() {
                        continue;
                    }
                    for y in clip.y..clip.y.saturating_add(clip.height as i32) {
                        for x in clip.x..clip.x.saturating_add(clip.width as i32) {
                            let glyph_row = (y - cell_rect.y) as usize;
                            let glyph_column = (x - cell_rect.x) as usize;
                            let coverage =
                                styled_coverage(&glyph, glyph_row, glyph_column, cell.attributes);
                            let offset = y as usize * stride + x as usize * 4;
                            let bytes = match coverage {
                                0 => background_bytes,
                                u8::MAX => foreground_bytes,
                                _ => pixel_bytes(
                                    blend_color(cell.background, foreground, coverage),
                                    format,
                                ),
                            };
                            framebuffer[offset..offset + 4].copy_from_slice(&bytes);
                            rendered += 1;
                        }
                    }
                    if is_cursor && cursor_visible {
                        let cursor_clip = intersect(
                            clip,
                            GuiRect::new(cell_rect.x, cell_rect.y + 1, CURSOR_WIDTH as u32, 14),
                        );
                        let bytes = pixel_bytes(foreground, format);
                        for y in
                            cursor_clip.y..cursor_clip.y.saturating_add(cursor_clip.height as i32)
                        {
                            for x in cursor_clip.x
                                ..cursor_clip.x.saturating_add(cursor_clip.width as i32)
                            {
                                let offset = y as usize * stride + x as usize * 4;
                                framebuffer[offset..offset + 4].copy_from_slice(&bytes);
                            }
                        }
                    }
                }
            }
        }
        rendered
    }

    pub fn render_gui(
        &mut self,
        framebuffer: &mut [u8],
        width: usize,
        height: usize,
        stride: usize,
        format: PixelFormat,
    ) -> Result<usize, DisplayError> {
        let required = stride.checked_mul(height).ok_or(DisplayError::InvalidFramebuffer)?;
        if stride < width * 4 || framebuffer.len() < required {
            return Err(DisplayError::InvalidFramebuffer);
        }
        self.ensure_backbuffer(required)?;
        let background = self.gui.background_color();
        if background != self.gui_background {
            let restoring_terminal = background.is_none();
            self.gui_background = background;
            self.invalidate_terminal();
            self.gui_background_pending = true;
            self.gui_damage = [GuiRect::EMPTY; MAX_GUI_DAMAGE_RECTS];
            self.gui_damage_count = 0;
            if restoring_terminal {
                return self.render(framebuffer, width, height, stride, format);
            }
        }
        let mut background_filled = false;
        if self.gui_background_pending {
            self.surface_background = self.gui_background.unwrap_or(self.cells[0].background);
            let pixel = pixel_bytes(self.surface_background, format);
            let row_bytes = width * 4;
            let backbuffer = self.backbuffer.as_mut().unwrap();
            for row in 0..height {
                let start = row * stride;
                fill_row(&mut backbuffer[start..start + row_bytes], pixel);
            }
            self.present_all(framebuffer, width, height, stride);
            self.gui_background_pending = false;
            background_filled = true;
            self.surface_initialized = true;
            if let Some(surface) = self.gui.terminal_bounds() {
                self.set_all_dirty(true);
                self.gui.invalidate_rect(surface);
            } else {
                self.set_all_dirty(false);
            }
        }
        if self.gui_damage_count == 0 {
            let (damage, count) = self.gui.take_damage();
            self.load_gui_damage(damage, count, width, height);
            if self.gui_damage_count == 0 {
                return Ok(0);
            }
            self.gui_tile_index = 0;
            self.gui_tile_x = self.gui_damage[0].x;
            self.gui_tile_y = self.gui_damage[0].y;
        }
        let mut rendered = 0;
        let screen = GuiRect::new(0, 0, width as u32, height as u32);
        if self.gui_damage_count == 1 && self.gui_damage[0] == screen {
            let damage = self.gui_damage;
            if !background_filled {
                let pixel = pixel_bytes(self.surface_background, format);
                let row_bytes = width * 4;
                let backbuffer = self.backbuffer.as_mut().unwrap();
                for row in 0..height {
                    let start = row * stride;
                    fill_row(&mut backbuffer[start..start + row_bytes], pixel);
                }
            }
            let backbuffer = self.backbuffer.as_mut().unwrap();
            if let Some(surface) = self.gui.terminal_bounds() {
                rendered += Self::render_terminal_surface(
                    &self.cells,
                    self.columns,
                    self.rows,
                    self.cursor_column,
                    self.cursor_row,
                    self.cursor_visible,
                    &mut self.glyph_cache,
                    backbuffer,
                    width,
                    height,
                    stride,
                    format,
                    surface,
                    &damage,
                    1,
                );
            }
            rendered += self.gui.render(
                &mut self.glyph_cache,
                backbuffer,
                width,
                height,
                stride,
                format,
                &damage,
                1,
            );
            self.present_all(framebuffer, width, height, stride);
            self.render_cursor_layers(framebuffer, width, height, stride, format, screen);
            self.gui_damage_count = 0;
            self.gui_tile_index = 0;
            return Ok(rendered);
        }
        for _ in 0..GUI_TILES_PER_STEP {
            if self.gui_tile_index >= self.gui_damage_count {
                break;
            }
            let rect = self.gui_damage[self.gui_tile_index];
            let right = rect.x.saturating_add(rect.width as i32);
            let bottom = rect.y.saturating_add(rect.height as i32);
            let tile = intersect(
                GuiRect::new(self.gui_tile_x, self.gui_tile_y, GUI_TILE_SIZE, GUI_TILE_SIZE),
                intersect(rect, screen),
            );
            if tile.is_empty() {
                self.advance_gui_tile(rect, right, bottom);
                continue;
            }
            let pixel = pixel_bytes(self.surface_background, format);
            let row_bytes = tile.width as usize * 4;
            let backbuffer = self.backbuffer.as_mut().unwrap();
            for row in tile.y as usize..tile.y as usize + tile.height as usize {
                let start = row * stride + tile.x as usize * 4;
                fill_row(&mut backbuffer[start..start + row_bytes], pixel);
            }
            let backbuffer = self.backbuffer.as_mut().unwrap();
            let mut damage = [GuiRect::EMPTY; MAX_GUI_DAMAGE_RECTS];
            damage[0] = tile;
            let terminal = self
                .gui
                .terminal_bounds()
                .map(|surface| {
                    Self::render_terminal_surface(
                        &self.cells,
                        self.columns,
                        self.rows,
                        self.cursor_column,
                        self.cursor_row,
                        self.cursor_visible,
                        &mut self.glyph_cache,
                        backbuffer,
                        width,
                        height,
                        stride,
                        format,
                        surface,
                        &damage,
                        1,
                    )
                })
                .unwrap_or(0);
            rendered += terminal
                + self.gui.render(
                    &mut self.glyph_cache,
                    backbuffer,
                    width,
                    height,
                    stride,
                    format,
                    &damage,
                    1,
                );
            self.present_damage(framebuffer, width, height, stride, &[tile]);
            self.render_cursor_layers(framebuffer, width, height, stride, format, tile);
            self.advance_gui_tile(rect, right, bottom);
        }
        if self.gui_tile_index >= self.gui_damage_count {
            let (next_damage, next_count) = self.gui.take_damage();
            self.load_gui_damage(next_damage, next_count, width, height);
            self.gui_tile_index = 0;
            if self.gui_damage_count != 0 {
                self.gui_tile_x = self.gui_damage[0].x;
                self.gui_tile_y = self.gui_damage[0].y;
            }
        }
        Ok(rendered)
    }

    fn load_gui_damage(
        &mut self,
        damage: [GuiRect; MAX_GUI_DAMAGE_RECTS],
        count: usize,
        width: usize,
        height: usize,
    ) {
        let screen = GuiRect::new(0, 0, width as u32, height as u32);
        let mut clipped = [GuiRect::EMPTY; MAX_GUI_DAMAGE_RECTS];
        let mut clipped_count = 0;
        for rect in damage[..count].iter().copied() {
            let rect = intersect(rect, screen);
            if !rect.is_empty() {
                clipped[clipped_count] = rect;
                clipped_count += 1;
            }
        }
        self.gui_damage = clipped;
        self.gui_damage_count = clipped_count;
    }

    fn advance_gui_tile(&mut self, rect: GuiRect, right: i32, bottom: i32) {
        self.gui_tile_x = self.gui_tile_x.saturating_add(GUI_TILE_SIZE as i32);
        if self.gui_tile_x >= right {
            self.gui_tile_x = rect.x;
            self.gui_tile_y = self.gui_tile_y.saturating_add(GUI_TILE_SIZE as i32);
            if self.gui_tile_y >= bottom {
                self.gui_tile_index += 1;
                if self.gui_tile_index < self.gui_damage_count {
                    let next = self.gui_damage[self.gui_tile_index];
                    self.gui_tile_x = next.x;
                    self.gui_tile_y = next.y;
                }
            }
        }
    }

    pub const fn render_pending(&self) -> bool {
        self.gui_background_pending || self.gui_damage_count != 0
    }

    pub fn take_presented(&mut self) -> bool {
        let (full, _, count) = self.take_presented_damage();
        full || count != 0
    }

    pub fn take_presented_damage(&mut self) -> (bool, [GuiRect; MAX_DISPLAY_PRESENT_RECTS], usize) {
        let result = (self.presented_full, self.presented_damage, self.presented_damage_count);
        self.presented = false;
        self.presented_full = false;
        self.presented_damage_count = 0;
        self.presented_damage = [GuiRect::EMPTY; MAX_DISPLAY_PRESENT_RECTS];
        result
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
    fn glyph_cache_reuses_fixed_atlas_entries() {
        let mut cache = GlyphCache::new();
        let first = cache.get('A' as u32);
        let next = cache.next;
        let second = cache.get('A' as u32);
        assert_eq!(first, second);
        assert_eq!(cache.next, next);
        assert!(cache.entries.iter().any(|entry| entry.valid && entry.scalar == 'A' as u32));
    }

    #[test]
    fn present_signal_is_consumed_once() {
        let mut display = Display::new(1);
        display.ensure_backbuffer(4 * 4 * 4).unwrap();
        let mut framebuffer = [0; 4 * 4 * 4];
        display.present_all(&mut framebuffer, 4, 4, 4 * 4);
        assert!(display.take_presented());
        assert!(!display.take_presented());
    }

    #[test]
    fn damage_present_signal_keeps_bounded_rectangles() {
        let mut display = Display::new(1);
        display.ensure_backbuffer(8 * 8 * 4).unwrap();
        let mut framebuffer = [0; 8 * 8 * 4];
        let damage = [GuiRect::new(2, 3, 4, 2)];
        display.present_damage(&mut framebuffer, 8, 8, 8 * 4, &damage);
        let (full, rects, count) = display.take_presented_damage();
        assert!(!full);
        assert_eq!(count, 1);
        assert_eq!(rects[0], damage[0]);
    }

    #[test]
    fn legacy_cell_render_presents_only_changed_cells() {
        let mut display = Display::new(1);
        let mut message = RenderMessage::empty(MessageKind::RenderCells);
        message.columns = 3;
        message.rows = 1;
        message.count = 1;
        message.positions[0] = 0;
        message.cells[0] = Cell { background: 0x102030, ..Cell::EMPTY };
        let mut framebuffer = std::vec![0; 24 * 16 * 4];
        display.apply(1, &message).unwrap();
        display.render(&mut framebuffer, 24, 16, 24 * 4, PixelFormat::Bgr8).unwrap();
        let _ = display.take_presented_damage();

        message.positions[0] = 2;
        message.cells[0] = Cell { background: 0x304050, ..Cell::EMPTY };
        display.apply(1, &message).unwrap();
        display.render(&mut framebuffer, 24, 16, 24 * 4, PixelFormat::Bgr8).unwrap();
        let (full, rects, count) = display.take_presented_damage();
        assert!(!full);
        assert_eq!(count, 2);
        assert_eq!(rects[0], GuiRect::new(0, 0, 8, 16));
        assert_eq!(rects[1], GuiRect::new(16, 0, 8, 16));
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
    fn legacy_terminal_and_gui_surface_compose_without_idle_redraw() {
        let mut display = Display::new(1);
        let mut terminal = RenderMessage::empty(MessageKind::FullRedraw);
        terminal.columns = 2;
        terminal.rows = 1;
        terminal.count = 1;
        terminal.positions[0] = 0;
        terminal.cells[0] = Cell {
            codepoint: b'T' as u32,
            background: 0x102030,
            foreground: 0xffffff,
            ..Cell::EMPTY
        };
        display.apply(1, &terminal).unwrap();
        let mut framebuffer = std::vec![0; 64 * 32 * 4];
        display.render(&mut framebuffer, 64, 32, 64 * 4, PixelFormat::Bgr8).unwrap();
        let mut root =
            logos_abi::GuiSurfaceRequest::new(logos_abi::GuiSurfaceOperation::CreateRoot, 1);
        root.bounds = logos_abi::GuiRect::new(0, 0, 64, 32);
        let handle = display.gui_mut().create(11, root).unwrap().surface;
        let mut batch =
            logos_abi::GuiDrawBatch::new(handle, 1, logos_abi::GuiRect::new(8, 8, 16, 8));
        assert!(batch.push(logos_abi::GuiDrawCommand::fill_rect(
            logos_abi::GuiRect::new(8, 8, 16, 8),
            0xff0000,
        )));
        display.gui_mut().update(11, batch).unwrap();
        while display.render_gui(&mut framebuffer, 64, 32, 64 * 4, PixelFormat::Bgr8).unwrap() == 0
        {
        }
        display.apply(1, &terminal).unwrap();
        assert!(display.render(&mut framebuffer, 64, 32, 64 * 4, PixelFormat::Bgr8).unwrap() > 0);
        let pixel = (8 * 64 + 8) * 4;
        assert_eq!(&framebuffer[pixel..pixel + 4], &[0, 0, 255, 0]);
        let hidden = logos_abi::GuiDrawBatch::new(handle, 2, logos_abi::GuiRect::new(0, 0, 64, 32));
        display.gui_mut().update(11, hidden).unwrap();
        display.invalidate_terminal();
        display.render(&mut framebuffer, 64, 32, 64 * 4, PixelFormat::Bgr8).unwrap();
        assert_eq!(&framebuffer[pixel..pixel + 4], &[0x30, 0x20, 0x10, 0]);
        assert_eq!(display.render_gui(&mut framebuffer, 64, 32, 64 * 4, PixelFormat::Bgr8), Ok(0));
    }

    #[test]
    fn gui_composition_finishes_full_screen_in_one_bounded_pass() {
        let mut display = Display::new(1);
        let mut root =
            logos_abi::GuiSurfaceRequest::new(logos_abi::GuiSurfaceOperation::CreateRoot, 1);
        root.bounds = logos_abi::GuiRect::new(0, 0, 640, 400);
        let handle = display.gui_mut().create(11, root).unwrap().surface;
        let mut batch = logos_abi::GuiDrawBatch::new(handle, 1, logos_abi::GuiRect::SURFACE);
        assert!(batch.push(logos_abi::GuiDrawCommand::fill_rounded_rect(
            logos_abi::GuiRect::new(0, 0, 640, 400),
            0x203040,
            16,
        )));
        display.gui_mut().update(11, batch).unwrap();
        let mut framebuffer = std::vec![0; 640 * 400 * 4];
        display.render_gui(&mut framebuffer, 640, 400, 640 * 4, PixelFormat::Bgr8).unwrap();
        assert!(!display.render_pending());
        let mut replacement =
            logos_abi::GuiDrawBatch::new(handle, 2, logos_abi::GuiRect::new(24, 24, 16, 16));
        assert!(replacement.push(logos_abi::GuiDrawCommand::fill_rect(
            logos_abi::GuiRect::new(24, 24, 16, 16),
            0xff0000,
        )));
        display.gui_mut().update(11, replacement).unwrap();
        display.render_gui(&mut framebuffer, 640, 400, 640 * 4, PixelFormat::Bgr8).unwrap();
        assert!(!display.render_pending());
        let pixel = (24 * 640 + 24) * 4;
        assert_eq!(&framebuffer[pixel..pixel + 4], &[0, 0, 255, 0]);
    }

    #[test]
    fn terminal_cells_render_inside_the_atrium_surface() {
        let mut display = Display::new(1);
        let mut terminal = RenderMessage::empty(MessageKind::FullRedraw);
        terminal.columns = 2;
        terminal.rows = 1;
        terminal.count = 1;
        terminal.cells[0] = Cell {
            codepoint: b'T' as u32,
            background: 0x102030,
            foreground: 0xffffff,
            ..Cell::EMPTY
        };
        display.apply(1, &terminal).unwrap();

        let mut root =
            logos_abi::GuiSurfaceRequest::new(logos_abi::GuiSurfaceOperation::CreateRoot, 1);
        root.bounds = logos_abi::GuiRect::new(0, 0, 64, 32);
        let root_handle = display.gui_mut().create(11, root).unwrap().surface;
        let mut root_batch =
            logos_abi::GuiDrawBatch::new(root_handle, 1, logos_abi::GuiRect::new(0, 0, 64, 32));
        assert!(root_batch.push(logos_abi::GuiDrawCommand::fill_surface(0x203040)));
        display.gui_mut().update(11, root_batch).unwrap();

        let mut terminal_surface =
            logos_abi::GuiSurfaceRequest::new(logos_abi::GuiSurfaceOperation::CreateModal, 2);
        terminal_surface.flags = logos_abi::GUI_SURFACE_FLAG_TERMINAL;
        terminal_surface.bounds = logos_abi::GuiRect::new(16, 8, 16, 16);
        terminal_surface.z_order = 2;
        let handle = display.gui_mut().create(11, terminal_surface).unwrap().surface;
        assert_eq!(display.gui().terminal_bounds(), Some(terminal_surface.bounds));
        terminal.surface = logos_abi::SurfaceHandle::new(0, 1, 99).unwrap();
        assert_eq!(display.apply(1, &terminal), Err(DisplayError::InvalidMessage));
        terminal.surface = handle;
        display.apply(1, &terminal).unwrap();

        let mut framebuffer = std::vec![0; 64 * 32 * 4];
        loop {
            display.render_gui(&mut framebuffer, 64, 32, 64 * 4, PixelFormat::Bgr8).unwrap();
            if !display.render_pending() {
                break;
            }
        }

        let background = (8 * 64 + 16) * 4;
        assert_eq!(&framebuffer[background..background + 4], &[0x30, 0x20, 0x10, 0]);
        assert!(
            framebuffer[(8 * 64 + 16) * 4..(24 * 64 + 32) * 4]
                .chunks_exact(4)
                .any(|pixel| pixel[..3] == [0xff, 0xff, 0xff])
        );
        assert!(display.gui().contains(handle));
    }

    #[test]
    fn native_pointer_surface_renders_above_lockscreen() {
        let mut display = Display::new(1);
        let mut root =
            logos_abi::GuiSurfaceRequest::new(logos_abi::GuiSurfaceOperation::CreateRoot, 1);
        root.bounds = logos_abi::GuiRect::new(0, 0, 64, 32);
        display.gui_mut().create(11, root).unwrap();

        let mut lockscreen =
            logos_abi::GuiSurfaceRequest::new(logos_abi::GuiSurfaceOperation::CreateModal, 2);
        lockscreen.bounds = root.bounds;
        lockscreen.z_order = 1;
        let lockscreen_handle = display.gui_mut().create(12, lockscreen).unwrap().surface;
        let mut lockscreen_batch =
            logos_abi::GuiDrawBatch::new(lockscreen_handle, 1, logos_abi::GuiRect::SURFACE);
        assert!(lockscreen_batch.push(logos_abi::GuiDrawCommand::fill_surface(0x102030)));
        display.gui_mut().update(12, lockscreen_batch).unwrap();

        let mut cursor =
            logos_abi::GuiSurfaceRequest::new(logos_abi::GuiSurfaceOperation::CreateModal, 3);
        cursor.bounds = root.bounds;
        cursor.z_order = 3;
        let cursor_handle = display.gui_mut().create(13, cursor).unwrap().surface;
        let commands = [
            logos_abi::GuiDrawCommand::fill_rect(logos_abi::GuiRect::new(22, 12, 4, 16), 0x101820),
            logos_abi::GuiDrawCommand::fill_rect(logos_abi::GuiRect::new(20, 10, 3, 14), 0xffffff),
            logos_abi::GuiDrawCommand::fill_rect(logos_abi::GuiRect::new(22, 20, 8, 3), 0xffffff),
        ];
        let mut clear = logos_abi::GuiSceneOp::clear(cursor_handle, 1);
        clear.flags = logos_abi::GUI_DRAW_FLAG_MORE;
        display.gui_mut().apply_scene_op(13, clear).unwrap();
        for (index, command) in commands.into_iter().enumerate() {
            let mut op =
                logos_abi::GuiSceneOp::upsert(cursor_handle, 1, (index + 1) as u32, command);
            if index + 1 < commands.len() {
                op.flags = logos_abi::GUI_DRAW_FLAG_MORE;
            }
            display.gui_mut().apply_scene_op(13, op).unwrap();
        }

        let mut framebuffer = std::vec![0; 64 * 32 * 4];
        loop {
            display.render_gui(&mut framebuffer, 64, 32, 64 * 4, PixelFormat::Bgr8).unwrap();
            if !display.render_pending() {
                break;
            }
        }
        let pixel = (10 * 64 + 20) * 4;
        assert_eq!(&framebuffer[pixel..pixel + 4], &[0xff, 0xff, 0xff, 0]);
    }

    #[test]
    fn compositor_cursor_moves_without_retaining_cursor_pixels() {
        let mut display = Display::new(1);
        let mut root =
            logos_abi::GuiSurfaceRequest::new(logos_abi::GuiSurfaceOperation::CreateRoot, 1);
        root.bounds = logos_abi::GuiRect::new(0, 0, 64, 32);
        display.gui_mut().create(11, root).unwrap();

        let mut lockscreen =
            logos_abi::GuiSurfaceRequest::new(logos_abi::GuiSurfaceOperation::CreateModal, 2);
        lockscreen.bounds = root.bounds;
        lockscreen.z_order = 1;
        let lockscreen_handle = display.gui_mut().create(12, lockscreen).unwrap().surface;
        let mut background =
            logos_abi::GuiDrawBatch::new(lockscreen_handle, 1, logos_abi::GuiRect::SURFACE);
        assert!(background.push(logos_abi::GuiDrawCommand::fill_surface(0x102030)));
        display.gui_mut().update(12, background).unwrap();

        let mut cursor =
            logos_abi::GuiSurfaceRequest::new(logos_abi::GuiSurfaceOperation::CreateModal, 3);
        cursor.bounds = root.bounds;
        cursor.z_order = 3;
        let cursor_handle = display.gui_mut().create(13, cursor).unwrap().surface;
        display.register_cursor_surface(13, cursor_handle);

        let mut framebuffer = std::vec![0; 64 * 32 * 4];
        let mut draw = logos_abi::GuiSceneOp::upsert(
            cursor_handle,
            1,
            1,
            logos_abi::GuiDrawCommand::fill_rect(logos_abi::GuiRect::new(8, 8, 3, 14), 0xffffff),
        );
        assert!(display.apply_cursor_scene_op(draw));
        loop {
            display.render_gui(&mut framebuffer, 64, 32, 64 * 4, PixelFormat::Bgr8).unwrap();
            if !display.render_pending() {
                break;
            }
        }
        let old_pixel = (8 * 64 + 8) * 4;
        assert_eq!(&framebuffer[old_pixel..old_pixel + 3], &[0xff, 0xff, 0xff]);

        draw.frame = 2;
        draw.command.x = 32;
        assert!(display.apply_cursor_scene_op(draw));
        draw.frame = 3;
        draw.command.x = 56;
        assert!(display.apply_cursor_scene_op(draw));
        assert!(!display.render_pending());
        assert!(display.repaint_cursor(&mut framebuffer, 64, 32, 64 * 4, PixelFormat::Bgr8));
        assert!(!display.render_pending());
        let new_pixel = (8 * 64 + 32) * 4;
        assert_eq!(&framebuffer[old_pixel..old_pixel + 3], &[0x30, 0x20, 0x10]);
        assert_eq!(&framebuffer[new_pixel..new_pixel + 3], &[0x30, 0x20, 0x10]);
        let latest_pixel = (8 * 64 + 56) * 4;
        assert_eq!(&framebuffer[latest_pixel..latest_pixel + 3], &[0xff, 0xff, 0xff]);
    }

    #[test]
    fn software_cursor_only_renders_the_topmost_layer() {
        let mut display = Display::new(1);
        display.cursor_layers[0] = CursorLayer {
            surface: SurfaceHandle::new(1, 1, LOCKSCREEN_OWNER).unwrap(),
            x: 8,
            y: 4,
            order: 2,
        };
        display.cursor_layers[1] = CursorLayer {
            surface: SurfaceHandle::new(2, 1, ATRIUM_OWNER).unwrap(),
            x: 32,
            y: 4,
            order: 1,
        };
        let mut framebuffer = std::vec![0; 64 * 32 * 4];
        for pixel in framebuffer.chunks_exact_mut(4) {
            pixel.copy_from_slice(&[0x40, 0x30, 0x20, 0]);
        }

        display.render_cursor_layers(
            &mut framebuffer,
            64,
            32,
            64 * 4,
            PixelFormat::Bgr8,
            GuiRect::new(0, 0, 64, 32),
        );

        assert_eq!(&framebuffer[(4 * 64 + 8) * 4..(4 * 64 + 8) * 4 + 4], &[0xff, 0xff, 0xff, 0]);
        assert_eq!(&framebuffer[(4 * 64 + 32) * 4..(4 * 64 + 32) * 4 + 4], &[0x40, 0x30, 0x20, 0]);
    }

    #[test]
    fn surface_relative_fill_covers_the_actual_framebuffer() {
        let mut display = Display::new(1);
        let mut terminal = RenderMessage::empty(MessageKind::FullRedraw);
        terminal.columns = 12;
        terminal.rows = 4;
        terminal.count = 1;
        terminal.cells[0] = Cell {
            codepoint: b' ' as u32,
            background: 0x102030,
            foreground: 0xffffff,
            ..Cell::EMPTY
        };
        display.apply(1, &terminal).unwrap();
        let mut root =
            logos_abi::GuiSurfaceRequest::new(logos_abi::GuiSurfaceOperation::CreateRoot, 1);
        root.bounds = logos_abi::GuiRect::new(0, 0, 96, 64);
        let handle = display.gui_mut().create(11, root).unwrap().surface;
        let mut batch = logos_abi::GuiDrawBatch::new(handle, 1, logos_abi::GuiRect::SURFACE);
        assert!(batch.push(logos_abi::GuiDrawCommand::fill_surface(0x203040)));
        display.gui_mut().update(11, batch).unwrap();
        let mut framebuffer = std::vec![0; 96 * 64 * 4];
        loop {
            let _ = display.render_gui(&mut framebuffer, 96, 64, 96 * 4, PixelFormat::Bgr8);
            if !display.render_pending() {
                break;
            }
        }
        let pixel = (63 * 96 + 95) * 4;
        assert_eq!(&framebuffer[pixel..pixel + 4], &[0x40, 0x30, 0x20, 0]);
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
        assert_eq!(&framebuffer[8 * 4..9 * 4], &[0x30, 0x20, 0x10, 0]);
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

    #[test]
    fn replacement_generation_drops_old_geometry_and_cursor() {
        let mut display = Display::new(1);
        let mut message = RenderMessage::empty(MessageKind::RenderCells);
        message.columns = 2;
        message.rows = 1;
        message.cursor_column = 1;
        display.apply(1, &message).unwrap();
        display.replace_generation(2);
        assert_eq!(display.columns, 0);
        assert_eq!(display.rows, 0);
        assert_eq!((display.cursor_column, display.cursor_row), (0, 0));
        assert!(!display.toggle_cursor());
    }
}
