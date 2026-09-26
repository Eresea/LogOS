use super::{
    Cell, DEFAULT_SCREEN_HEIGHT, DISPLAY_CELL_HEIGHT, DISPLAY_CELL_WIDTH, MAX_COLUMNS,
    NamespaceCapabilityHandle, NamespaceRights, NamespaceRoot, SessionHandle, UserId,
};

pub const MAX_GUI_SURFACES: usize = 8;
pub const MAX_GUI_DAMAGE_RECTS: usize = 8;
pub const MAX_GUI_COMMANDS: usize = 3;
pub const MAX_GUI_BATCH_FRAGMENTS: usize = 5;
/// Raised from 24 for the retained `TextGrid` node (ADR-0087): a bounded
/// Terminal chrome (title, tab bar) plus one grid node, and headroom for
/// existing Home/Settings scenes, fit comfortably under 48 while still
/// keeping `RenderPlan` (`MAX_GUI_SURFACES * MAX_GUI_NODES`) bounded.
pub const MAX_GUI_NODES: usize = 48;
pub const MAX_GUI_TEXT_BYTES: usize = 32;
/// Text-grid columns share the terminal cell protocol's bound so a future
/// adapter can hand the same `Cell` buffer to either path.
pub const MAX_GUI_TEXT_GRID_COLUMNS: usize = MAX_COLUMNS;
/// Rows are bounded to the 1280x800 Terminal viewport at 8x16 glyphs
/// (`DEFAULT_SCREEN_HEIGHT / DISPLAY_CELL_HEIGHT`), not the legacy
/// protocol's larger scrollback allowance (`MAX_ROWS`): scrollback beyond a
/// row offset is out of scope for this node (see #71).
pub const MAX_GUI_TEXT_GRID_ROWS: usize = DEFAULT_SCREEN_HEIGHT / DISPLAY_CELL_HEIGHT;
/// Concurrently bound grids, one per surface: up to 4 Terminal sessions
/// (ADR-0087/#76), each keeping its own cell content independent of the
/// others — never a single shared slot that the next surface can steal.
pub const MAX_GUI_TEXT_GRIDS: usize = 4;
pub const GUI_DRAW_FLAG_MORE: u8 = 1 << 0;
pub const GUI_SURFACE_FLAG_TERMINAL: u8 = 1 << 0;
pub const GUI_SURFACE_FLAG_CURSOR: u8 = 1 << 1;
pub const GUI_TEXT_FLAG_LIGHT: u32 = 1 << 0;
pub const GUI_TEXT_FLAG_DOUBLE: u32 = 1 << 1;
/// Select the offline Inter body atlas (14 px, ADR-0088) instead of the
/// default JetBrains Mono glyph run. Mutually exclusive with
/// [`GUI_TEXT_FLAG_FONT_INTER_TITLE`].
pub const GUI_TEXT_FLAG_FONT_INTER_BODY: u32 = 1 << 2;
/// Select the offline Inter title atlas (20 px, ADR-0088) instead of the
/// default JetBrains Mono glyph run. Mutually exclusive with
/// [`GUI_TEXT_FLAG_FONT_INTER_BODY`].
pub const GUI_TEXT_FLAG_FONT_INTER_TITLE: u32 = 1 << 3;
const GUI_TEXT_FLAG_FONT_MASK: u32 = GUI_TEXT_FLAG_FONT_INTER_BODY | GUI_TEXT_FLAG_FONT_INTER_TITLE;
pub const MAX_GUI_CORNER_RADIUS: u8 = 32;
pub const MAX_GUI_STROKE_WIDTH: u8 = 8;
pub const MAX_GUI_LINE_WIDTH: u8 = 8;
pub const MAX_GUI_BLUR_RADIUS: u8 = 4;
pub const MAX_GUI_SHADOW_OFFSET: i8 = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct SurfaceHandle {
    pub slot: u16,
    pub generation: u16,
    pub owner: u32,
}

impl SurfaceHandle {
    pub const EMPTY: Self = Self { slot: u16::MAX, generation: 0, owner: 0 };

    pub const fn new(slot: u16, generation: u16, owner: u32) -> Option<Self> {
        if slot == u16::MAX || generation == 0 || owner == 0 {
            None
        } else {
            Some(Self { slot, generation, owner })
        }
    }

    pub const fn is_valid(self) -> bool {
        self.slot != u16::MAX && self.generation != 0 && self.owner != 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct GuiRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl GuiRect {
    pub const EMPTY: Self = Self { x: 0, y: 0, width: 0, height: 0 };
    pub const SURFACE: Self = Self { x: 0, y: 0, width: i32::MAX as u32, height: i32::MAX as u32 };

    pub const fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self { x, y, width, height }
    }

    pub const fn is_empty(self) -> bool {
        self.width == 0 || self.height == 0
    }

    pub const fn contains(self, x: i32, y: i32) -> bool {
        x >= self.x
            && y >= self.y
            && (x - self.x) < self.width as i32
            && (y - self.y) < self.height as i32
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum GuiSurfaceOperation {
    CreateRoot = 1,
    CreateModal = 2,
    Update = 3,
    Focus = 4,
    Destroy = 5,
    ToggleFps = 6,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum GuiStatus {
    Ok = 0,
    Stale = 1,
    Malformed = 2,
    Capacity = 3,
    Unauthorized = 4,
    Backpressure = 5,
    NotFound = 6,
}

impl GuiStatus {
    pub const fn from_raw(raw: u8) -> Option<Self> {
        match raw {
            0 => Some(Self::Ok),
            1 => Some(Self::Stale),
            2 => Some(Self::Malformed),
            3 => Some(Self::Capacity),
            4 => Some(Self::Unauthorized),
            5 => Some(Self::Backpressure),
            6 => Some(Self::NotFound),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct GuiSurfaceRequest {
    pub operation: GuiSurfaceOperation,
    pub flags: u8,
    pub reserved: u16,
    pub request_id: u32,
    pub surface: SurfaceHandle,
    pub bounds: GuiRect,
    pub z_order: i16,
    pub reserved_tail: u16,
}

impl GuiSurfaceRequest {
    pub const fn new(operation: GuiSurfaceOperation, request_id: u32) -> Self {
        Self {
            operation,
            flags: 0,
            reserved: 0,
            request_id,
            surface: SurfaceHandle::EMPTY,
            bounds: GuiRect::EMPTY,
            z_order: 0,
            reserved_tail: 0,
        }
    }

    pub const fn is_valid(self) -> bool {
        self.request_id != 0
            && self.flags & !(GUI_SURFACE_FLAG_TERMINAL | GUI_SURFACE_FLAG_CURSOR) == 0
            && self.reserved == 0
            && self.reserved_tail == 0
            && match self.operation {
                GuiSurfaceOperation::ToggleFps => {
                    self.flags == 0
                        && self.surface.slot == u16::MAX
                        && self.surface.generation == 0
                        && self.surface.owner == 0
                        && self.bounds.x == 0
                        && self.bounds.y == 0
                        && self.bounds.width == 0
                        && self.bounds.height == 0
                        && self.z_order == 0
                }
                _ => true,
            }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct GuiSurfaceResponse {
    pub operation: GuiSurfaceOperation,
    pub status: GuiStatus,
    pub reserved: u16,
    pub request_id: u32,
    pub surface: SurfaceHandle,
}

impl GuiSurfaceResponse {
    pub const fn new(request: GuiSurfaceRequest, status: GuiStatus) -> Self {
        Self {
            operation: request.operation,
            status,
            reserved: 0,
            request_id: request.request_id,
            surface: SurfaceHandle::EMPTY,
        }
    }

    pub const fn is_valid_for(self, request: GuiSurfaceRequest) -> bool {
        self.operation as u8 == request.operation as u8
            && self.request_id == request.request_id
            && self.reserved == 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum GuiDrawKind {
    FillRect = 1,
    StrokeRect = 2,
    Line = 3,
    ClipRect = 4,
    GlyphRun = 5,
    FillRoundedRect = 6,
    StrokeRoundedRect = 7,
    Shadow = 8,
    LogosMark = 9,
    MaterialSymbol = 10,
    TextGrid = 11,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum GuiMaterialSymbol {
    Settings = 1,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct GuiTransform {
    pub translate_x: i16,
    pub translate_y: i16,
    pub scale_q8_8: u16,
    pub rotation_degrees: i16,
    pub reserved: u16,
}

impl GuiTransform {
    pub const IDENTITY: Self =
        Self { translate_x: 0, translate_y: 0, scale_q8_8: 256, rotation_degrees: 0, reserved: 0 };

    pub const fn is_identity(self) -> bool {
        self.translate_x == 0
            && self.translate_y == 0
            && self.scale_q8_8 == 256
            && self.rotation_degrees == 0
    }

    pub const fn is_valid(self) -> bool {
        self.scale_q8_8 != 0 && self.scale_q8_8 <= 1_024 && self.reserved == 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct GuiDrawCommand {
    pub kind: GuiDrawKind,
    pub flags: u8,
    pub text_len: u8,
    pub reserved: u8,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub color: u32,
    pub auxiliary: u32,
    pub text: [u8; MAX_GUI_TEXT_BYTES],
    pub transform: GuiTransform,
}

impl GuiDrawCommand {
    pub const fn empty(kind: GuiDrawKind) -> Self {
        Self {
            kind,
            flags: 0,
            text_len: 0,
            reserved: 0,
            x: 0,
            y: 0,
            width: 0,
            height: 0,
            color: 0,
            auxiliary: 0,
            text: [0; MAX_GUI_TEXT_BYTES],
            transform: GuiTransform::IDENTITY,
        }
    }

    pub const fn fill_rect(bounds: GuiRect, color: u32) -> Self {
        let mut command = Self::empty(GuiDrawKind::FillRect);
        command.x = bounds.x;
        command.y = bounds.y;
        command.width = bounds.width;
        command.height = bounds.height;
        command.color = color;
        command
    }

    pub const fn fill_surface(color: u32) -> Self {
        Self::fill_rect(GuiRect::SURFACE, color)
    }

    pub const fn logos_mark(bounds: GuiRect, color: u32) -> Self {
        let mut command = Self::fill_rect(bounds, color);
        command.kind = GuiDrawKind::LogosMark;
        command
    }

    pub const fn material_symbol(bounds: GuiRect, color: u32, symbol: GuiMaterialSymbol) -> Self {
        let mut command = Self::fill_rect(bounds, color);
        command.kind = GuiDrawKind::MaterialSymbol;
        command.auxiliary = symbol as u32;
        command
    }

    /// A retained text-grid node: `bounds` must exactly match `columns` by
    /// `rows` whole 8x16 cells, both within the bounded grid size. The
    /// command carries only this descriptor; cell content is delivered
    /// separately through [`GuiTextGridRow`] and stored by the Display
    /// backend, addressed by the node's (surface, node_id).
    pub const fn text_grid(bounds: GuiRect, columns: u16, rows: u16) -> Option<Self> {
        if columns == 0
            || rows == 0
            || columns as usize > MAX_GUI_TEXT_GRID_COLUMNS
            || rows as usize > MAX_GUI_TEXT_GRID_ROWS
            || bounds.width != columns as u32 * DISPLAY_CELL_WIDTH as u32
            || bounds.height != rows as u32 * DISPLAY_CELL_HEIGHT as u32
        {
            return None;
        }
        let mut command = Self::empty(GuiDrawKind::TextGrid);
        command.x = bounds.x;
        command.y = bounds.y;
        command.width = bounds.width;
        command.height = bounds.height;
        command.auxiliary = columns as u32 | ((rows as u32) << 16);
        Some(command)
    }

    pub const fn text_grid_columns(self) -> u16 {
        self.auxiliary as u16
    }

    pub const fn text_grid_rows(self) -> u16 {
        (self.auxiliary >> 16) as u16
    }

    pub const fn stroke_rect(bounds: GuiRect, color: u32, width: u32) -> Self {
        let mut command = Self::fill_rect(bounds, color);
        command.kind = GuiDrawKind::StrokeRect;
        command.auxiliary = width;
        command
    }

    pub const fn line(x: i32, y: i32, width: u32, height: u32, color: u32) -> Self {
        let mut command = Self::empty(GuiDrawKind::Line);
        command.x = x;
        command.y = y;
        command.width = width;
        command.height = height;
        command.color = color;
        command
    }

    pub const fn line_with_width(
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        color: u32,
        line_width: u8,
    ) -> Self {
        let mut command = Self::line(x, y, width, height, color);
        command.auxiliary = line_width as u32;
        command
    }

    pub const fn fill_rounded_rect(bounds: GuiRect, color: u32, radius: u8) -> Self {
        let mut command = Self::fill_rect(bounds, color);
        command.kind = GuiDrawKind::FillRoundedRect;
        command.auxiliary = radius as u32;
        command
    }

    pub const fn stroke_rounded_rect(bounds: GuiRect, color: u32, radius: u8, width: u8) -> Self {
        let mut command = Self::fill_rect(bounds, color);
        command.kind = GuiDrawKind::StrokeRoundedRect;
        command.auxiliary = radius as u32 | ((width as u32) << 8);
        command
    }

    pub const fn shadow(
        bounds: GuiRect,
        color: u32,
        radius: u8,
        blur: u8,
        offset_x: i8,
        offset_y: i8,
    ) -> Self {
        let mut command = Self::fill_rect(bounds, color);
        command.kind = GuiDrawKind::Shadow;
        command.auxiliary = radius as u32
            | ((blur as u32) << 8)
            | ((offset_x as u8 as u32) << 16)
            | ((offset_y as u8 as u32) << 24);
        command
    }

    pub const fn clip(bounds: GuiRect) -> Self {
        let mut command = Self::fill_rect(bounds, 0);
        command.kind = GuiDrawKind::ClipRect;
        command
    }

    pub fn glyph_run(x: i32, y: i32, color: u32, text: &[u8]) -> Option<Self> {
        Self::glyph_run_styled(x, y, color, 0, text)
    }

    pub fn glyph_run_styled(
        x: i32,
        y: i32,
        color: u32,
        text_flags: u32,
        text: &[u8],
    ) -> Option<Self> {
        if text.len() > MAX_GUI_TEXT_BYTES {
            return None;
        }
        if !valid_text_flags(text_flags) {
            return None;
        }
        let mut command = Self::empty(GuiDrawKind::GlyphRun);
        command.x = x;
        command.y = y;
        command.color = color;
        command.auxiliary = text_flags;
        command.text_len = text.len() as u8;
        command.text[..text.len()].copy_from_slice(text);
        Some(command)
    }

    pub const fn color_rgb(self) -> u32 {
        self.color & 0x00ff_ffff
    }

    pub const fn color_alpha(self) -> u8 {
        let alpha = (self.color >> 24) as u8;
        if alpha == 0 { u8::MAX } else { alpha }
    }

    pub const fn is_identity_transform(self) -> bool {
        self.transform.is_identity()
    }

    pub const fn with_transform(mut self, transform: GuiTransform) -> Self {
        self.transform = transform;
        self
    }

    pub const fn corner_radius(self) -> u8 {
        self.auxiliary as u8
    }

    pub const fn stroke_width(self) -> u8 {
        (self.auxiliary >> 8) as u8
    }

    pub const fn line_width(self) -> u8 {
        let width = self.auxiliary as u8;
        if width == 0 { 1 } else { width }
    }

    pub const fn shadow_blur(self) -> u8 {
        (self.auxiliary >> 8) as u8
    }

    pub const fn shadow_offset_x(self) -> i8 {
        ((self.auxiliary >> 16) as u8) as i8
    }

    pub const fn shadow_offset_y(self) -> i8 {
        ((self.auxiliary >> 24) as u8) as i8
    }

    pub const fn is_valid(self) -> bool {
        self.flags == 0
            && self.reserved == 0
            && self.transform.is_valid()
            && self.text_len as usize <= MAX_GUI_TEXT_BYTES
            && match self.kind {
                GuiDrawKind::FillRect | GuiDrawKind::StrokeRect | GuiDrawKind::ClipRect => {
                    self.text_len == 0 && self.width != 0 && self.height != 0
                }
                GuiDrawKind::Line => {
                    self.text_len == 0
                        && (self.width != 0 || self.height != 0)
                        && self.auxiliary <= MAX_GUI_LINE_WIDTH as u32
                }
                GuiDrawKind::GlyphRun => self.text_len != 0 && valid_text_flags(self.auxiliary),
                GuiDrawKind::FillRoundedRect => {
                    self.text_len == 0
                        && self.auxiliary >> 8 == 0
                        && valid_corner_radius(self.width, self.height, self.corner_radius())
                }
                GuiDrawKind::StrokeRoundedRect => {
                    self.text_len == 0
                        && self.auxiliary >> 16 == 0
                        && self.stroke_width() != 0
                        && self.stroke_width() <= MAX_GUI_STROKE_WIDTH
                        && self.stroke_width() as u32 <= min_u32(self.width, self.height)
                        && valid_corner_radius(self.width, self.height, self.corner_radius())
                }
                GuiDrawKind::Shadow => {
                    self.text_len == 0
                        && self.shadow_blur() <= MAX_GUI_BLUR_RADIUS
                        && valid_corner_radius(self.width, self.height, self.corner_radius())
                        && self.shadow_offset_x().unsigned_abs() <= MAX_GUI_SHADOW_OFFSET as u8
                        && self.shadow_offset_y().unsigned_abs() <= MAX_GUI_SHADOW_OFFSET as u8
                }
                GuiDrawKind::LogosMark => {
                    self.text_len == 0
                        && self.width != 0
                        && self.height != 0
                        && self.width <= 512
                        && self.height <= 512
                        && self.auxiliary == 0
                }
                GuiDrawKind::MaterialSymbol => {
                    self.text_len == 0
                        && self.width != 0
                        && self.height != 0
                        && self.auxiliary == GuiMaterialSymbol::Settings as u32
                }
                GuiDrawKind::TextGrid => {
                    let columns = self.text_grid_columns();
                    let rows = self.text_grid_rows();
                    self.text_len == 0
                        && self.is_identity_transform()
                        && columns != 0
                        && rows != 0
                        && columns as usize <= MAX_GUI_TEXT_GRID_COLUMNS
                        && rows as usize <= MAX_GUI_TEXT_GRID_ROWS
                        && self.width == columns as u32 * DISPLAY_CELL_WIDTH as u32
                        && self.height == rows as u32 * DISPLAY_CELL_HEIGHT as u32
                }
            }
    }
}

const fn valid_corner_radius(width: u32, height: u32, radius: u8) -> bool {
    radius <= MAX_GUI_CORNER_RADIUS
        && radius as u32 <= min_u32(width, height) / 2
        && width != 0
        && height != 0
}

const fn min_u32(left: u32, right: u32) -> u32 {
    if left < right { left } else { right }
}

const fn valid_text_flags(flags: u32) -> bool {
    flags & !(GUI_TEXT_FLAG_LIGHT | GUI_TEXT_FLAG_DOUBLE | GUI_TEXT_FLAG_FONT_MASK) == 0
        && flags & GUI_TEXT_FLAG_FONT_MASK != GUI_TEXT_FLAG_FONT_MASK
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct GuiDrawBatch {
    pub surface: SurfaceHandle,
    pub sequence: u32,
    pub command_count: u8,
    pub flags: u8,
    pub reserved: u16,
    pub damage: GuiRect,
    pub commands: [GuiDrawCommand; MAX_GUI_COMMANDS],
}

impl GuiDrawBatch {
    pub const fn new(surface: SurfaceHandle, sequence: u32, damage: GuiRect) -> Self {
        Self {
            surface,
            sequence,
            command_count: 0,
            flags: 0,
            reserved: 0,
            damage,
            commands: [GuiDrawCommand::empty(GuiDrawKind::FillRect); MAX_GUI_COMMANDS],
        }
    }

    pub fn push(&mut self, command: GuiDrawCommand) -> bool {
        let index = self.command_count as usize;
        if index >= MAX_GUI_COMMANDS || !command.is_valid() {
            return false;
        }
        self.commands[index] = command;
        self.command_count += 1;
        true
    }

    pub const fn is_valid(self) -> bool {
        if !self.surface.is_valid()
            || self.sequence == 0
            || self.command_count as usize > MAX_GUI_COMMANDS
            || self.flags & !GUI_DRAW_FLAG_MORE != 0
            || self.reserved != 0
            || self.damage.is_empty()
        {
            return false;
        }
        let mut index = 0;
        while index < self.command_count as usize {
            if !self.commands[index].is_valid() {
                return false;
            }
            index += 1;
        }
        true
    }
}

/// One retained-scene mutation. A frame is staged while `GUI_DRAW_FLAG_MORE`
/// is set and becomes visible when the final operation clears it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum GuiNodeOperation {
    Upsert = 1,
    Remove = 2,
    Clear = 3,
    Commit = 4,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct GuiSceneOp {
    pub surface: SurfaceHandle,
    pub frame: u32,
    pub node_id: u32,
    pub operation: GuiNodeOperation,
    pub flags: u8,
    pub reserved: u16,
    pub command: GuiDrawCommand,
}

impl GuiSceneOp {
    pub const fn upsert(
        surface: SurfaceHandle,
        frame: u32,
        node_id: u32,
        command: GuiDrawCommand,
    ) -> Self {
        Self {
            surface,
            frame,
            node_id,
            operation: GuiNodeOperation::Upsert,
            flags: 0,
            reserved: 0,
            command,
        }
    }

    pub const fn remove(surface: SurfaceHandle, frame: u32, node_id: u32) -> Self {
        Self {
            surface,
            frame,
            node_id,
            operation: GuiNodeOperation::Remove,
            flags: 0,
            reserved: 0,
            command: GuiDrawCommand::empty(GuiDrawKind::FillRect),
        }
    }

    pub const fn clear(surface: SurfaceHandle, frame: u32) -> Self {
        Self {
            surface,
            frame,
            node_id: 1,
            operation: GuiNodeOperation::Clear,
            flags: 0,
            reserved: 0,
            command: GuiDrawCommand::empty(GuiDrawKind::FillRect),
        }
    }

    pub const fn commit(surface: SurfaceHandle, frame: u32) -> Self {
        Self {
            surface,
            frame,
            node_id: 1,
            operation: GuiNodeOperation::Commit,
            flags: 0,
            reserved: 0,
            command: GuiDrawCommand::empty(GuiDrawKind::FillRect),
        }
    }

    pub const fn is_valid(self) -> bool {
        self.surface.is_valid()
            && self.frame != 0
            && self.node_id != 0
            && self.flags & !GUI_DRAW_FLAG_MORE == 0
            && self.reserved == 0
            && match self.operation {
                GuiNodeOperation::Upsert => self.command.is_valid(),
                GuiNodeOperation::Remove | GuiNodeOperation::Clear | GuiNodeOperation::Commit => {
                    is_zero_command(self.command)
                }
            }
    }
}

const fn is_zero_command(command: GuiDrawCommand) -> bool {
    matches!(command.kind, GuiDrawKind::FillRect)
        && command.flags == 0
        && command.text_len == 0
        && command.reserved == 0
        && command.x == 0
        && command.y == 0
        && command.width == 0
        && command.height == 0
        && command.color == 0
        && command.auxiliary == 0
        && zero_text(command.text)
}

const fn zero_text(text: [u8; MAX_GUI_TEXT_BYTES]) -> bool {
    let mut index = 0;
    while index < MAX_GUI_TEXT_BYTES {
        if text[index] != 0 {
            return false;
        }
        index += 1;
    }
    true
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum GuiHookKind {
    Invalidate = 1,
    Refresh = 2,
    Section = 3,
    Session = 4,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct GuiHook {
    pub kind: GuiHookKind,
    pub flags: u8,
    pub reserved: u16,
    pub request_id: u32,
    pub surface: SurfaceHandle,
    pub damage: GuiRect,
    pub deadline: u64,
}

impl GuiHook {
    pub const fn new(kind: GuiHookKind, request_id: u32) -> Self {
        Self {
            kind,
            flags: 0,
            reserved: 0,
            request_id,
            surface: SurfaceHandle::EMPTY,
            damage: GuiRect::EMPTY,
            deadline: 0,
        }
    }

    pub const fn is_valid(self) -> bool {
        self.flags == 0 && self.reserved == 0 && self.request_id != 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct GuiSessionContext {
    pub session: SessionHandle,
    pub user: UserId,
    pub capability: NamespaceCapabilityHandle,
    pub root: NamespaceRoot,
    pub rights: NamespaceRights,
    pub reserved: [u8; 3],
}

impl GuiSessionContext {
    pub const EMPTY: Self = Self {
        session: SessionHandle::EMPTY,
        user: UserId::EMPTY,
        capability: NamespaceCapabilityHandle::EMPTY,
        root: NamespaceRoot::EMPTY,
        rights: NamespaceRights::NONE,
        reserved: [0; 3],
    };

    pub const fn is_authenticated(self) -> bool {
        self.session.is_valid()
            && self.user.is_valid()
            && self.capability.is_valid()
            && self.root.is_valid()
            && self.rights.is_valid()
    }

    pub const fn is_clear(self) -> bool {
        !self.session.is_valid()
            && !self.user.is_valid()
            && !self.capability.is_valid()
            && !self.root.is_valid()
            && self.rights.0 == NamespaceRights::NONE.0
            && self.reserved[0] == 0
            && self.reserved[1] == 0
            && self.reserved[2] == 0
    }
}

/// Bounded dirty-row delivery for a retained [`GuiDrawKind::TextGrid`] node.
/// Like [`super::RenderMessage`], this carries real cell content and is
/// exempt from `MAX_IPC_BYTES`: it is not sent over the small scene-op
/// ring, only addressed to an already-published grid node by
/// `(surface, node_id)`. Sending one row at a time is the "dirty-row"
/// update: a single changed line never resends the whole grid.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct GuiTextGridRow {
    pub surface: SurfaceHandle,
    pub node_id: u32,
    pub row: u16,
    pub cell_count: u16,
    pub cells: [Cell; MAX_GUI_TEXT_GRID_COLUMNS],
}

impl GuiTextGridRow {
    pub const EMPTY: Self = Self {
        surface: SurfaceHandle::EMPTY,
        node_id: 0,
        row: 0,
        cell_count: 0,
        cells: [Cell::EMPTY; MAX_GUI_TEXT_GRID_COLUMNS],
    };

    pub const fn is_valid(&self) -> bool {
        if !self.surface.is_valid()
            || self.node_id == 0
            || self.row as usize >= MAX_GUI_TEXT_GRID_ROWS
            || self.cell_count as usize > MAX_GUI_TEXT_GRID_COLUMNS
        {
            return false;
        }
        let mut index = 0;
        while index < self.cell_count as usize {
            if !self.cells[index].is_valid() {
                return false;
            }
            index += 1;
        }
        true
    }
}

const _: () = assert!(core::mem::size_of::<GuiSurfaceRequest>() <= super::MAX_IPC_BYTES);
const _: () = assert!(core::mem::size_of::<GuiSurfaceResponse>() <= super::MAX_IPC_BYTES);
const _: () = assert!(core::mem::size_of::<GuiDrawBatch>() <= super::MAX_IPC_BYTES);
const _: () = assert!(core::mem::size_of::<GuiSceneOp>() <= super::MAX_IPC_BYTES);
const _: () = assert!(core::mem::size_of::<GuiHook>() <= super::MAX_IPC_BYTES);
const _: () = assert!(core::mem::size_of::<GuiSessionContext>() <= super::MAX_IPC_BYTES);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draw_batches_are_bounded_and_validate_every_command() {
        let surface = SurfaceHandle::new(0, 1, 1).unwrap();
        let mut batch = GuiDrawBatch::new(surface, 1, GuiRect::new(0, 0, 100, 100));
        assert!(batch.push(GuiDrawCommand::fill_rect(GuiRect::new(0, 0, 10, 10), 0x112233)));
        assert!(batch.push(GuiDrawCommand::glyph_run(2, 2, 0xffffff, b"LogOS").unwrap()));
        assert!(batch.is_valid());
        assert!(!batch.push(GuiDrawCommand::empty(GuiDrawKind::GlyphRun)));
        batch.flags = GUI_DRAW_FLAG_MORE;
        assert!(batch.is_valid());
        batch.flags = u8::MAX;
        assert!(!batch.is_valid());
    }

    #[test]
    fn retained_scene_operations_are_stable_and_commit_aware() {
        let surface = SurfaceHandle::new(0, 1, 1).unwrap();
        let mut op = GuiSceneOp::upsert(
            surface,
            7,
            42,
            GuiDrawCommand::fill_rect(GuiRect::new(0, 0, 10, 10), 0x112233),
        );
        assert!(op.is_valid());
        op.flags = GUI_DRAW_FLAG_MORE;
        assert!(op.is_valid());
        op.node_id = 0;
        assert!(!op.is_valid());
        assert!(GuiSceneOp::remove(surface, 7, 42).is_valid());
        assert!(GuiSceneOp::clear(surface, 7).is_valid());
        assert!(GuiSceneOp::commit(surface, 7).is_valid());
    }

    #[test]
    fn modern_commands_pack_bounded_geometry_and_legacy_colors_stay_opaque() {
        let rounded = GuiDrawCommand::fill_rounded_rect(GuiRect::new(0, 0, 40, 20), 0x112233, 8);
        assert!(rounded.is_valid());
        assert_eq!(rounded.corner_radius(), 8);
        assert_eq!(rounded.color_rgb(), 0x112233);
        assert_eq!(rounded.color_alpha(), u8::MAX);

        let stroke =
            GuiDrawCommand::stroke_rounded_rect(GuiRect::new(0, 0, 40, 20), 0x445566, 8, 2);
        assert!(stroke.is_valid());
        assert_eq!(stroke.stroke_width(), 2);

        let shadow = GuiDrawCommand::shadow(GuiRect::new(10, 12, 40, 20), 0x55000000, 8, 4, -3, 5);
        assert!(shadow.is_valid());
        assert_eq!(shadow.color_alpha(), 0x55);
        assert_eq!(shadow.shadow_offset_x(), -3);
        assert_eq!(shadow.shadow_offset_y(), 5);

        let mark = GuiDrawCommand::logos_mark(GuiRect::new(0, 0, 112, 112), 0xf2a33a);
        assert!(mark.is_valid());
        assert_eq!(mark.kind, GuiDrawKind::LogosMark);
    }

    #[test]
    fn modern_commands_reject_overflowing_geometry() {
        assert!(
            !GuiDrawCommand::fill_rounded_rect(
                GuiRect::new(0, 0, 40, 20),
                0xffffff,
                MAX_GUI_CORNER_RADIUS + 1,
            )
            .is_valid()
        );
        assert!(
            !GuiDrawCommand::stroke_rounded_rect(
                GuiRect::new(0, 0, 40, 20),
                0xffffff,
                8,
                MAX_GUI_STROKE_WIDTH + 1,
            )
            .is_valid()
        );
        assert!(
            !GuiDrawCommand::shadow(
                GuiRect::new(0, 0, 40, 20),
                0xffffff,
                8,
                MAX_GUI_BLUR_RADIUS + 1,
                0,
                0,
            )
            .is_valid()
        );
    }

    #[test]
    fn styled_glyph_runs_carry_only_supported_font_flags() {
        let regular = GuiDrawCommand::glyph_run(0, 0, 0xffffff, b"A").unwrap();
        let light =
            GuiDrawCommand::glyph_run_styled(0, 0, 0xffffff, GUI_TEXT_FLAG_LIGHT, b"A").unwrap();
        assert_eq!(regular.auxiliary, 0);
        assert_eq!(light.auxiliary, GUI_TEXT_FLAG_LIGHT);
        assert!(light.is_valid());
        let body =
            GuiDrawCommand::glyph_run_styled(0, 0, 0xffffff, GUI_TEXT_FLAG_FONT_INTER_BODY, b"A")
                .unwrap();
        assert!(body.is_valid());
        let title =
            GuiDrawCommand::glyph_run_styled(0, 0, 0xffffff, GUI_TEXT_FLAG_FONT_INTER_TITLE, b"A")
                .unwrap();
        assert!(title.is_valid());
        assert!(GuiDrawCommand::glyph_run_styled(0, 0, 0xffffff, 1 << 4, b"A").is_none());
        assert!(
            GuiDrawCommand::glyph_run_styled(
                0,
                0,
                0xffffff,
                GUI_TEXT_FLAG_FONT_INTER_BODY | GUI_TEXT_FLAG_FONT_INTER_TITLE,
                b"A",
            )
            .is_none()
        );
    }

    #[test]
    fn terminal_surface_flag_is_explicitly_bounded() {
        let mut request = GuiSurfaceRequest::new(GuiSurfaceOperation::CreateModal, 1);
        request.flags = GUI_SURFACE_FLAG_TERMINAL;
        assert!(request.is_valid());
        request.flags = GUI_SURFACE_FLAG_CURSOR;
        assert!(request.is_valid());
        request.flags = u8::MAX;
        assert!(!request.is_valid());
    }

    #[test]
    fn fps_toggle_request_is_bounded_and_has_no_surface_payload() {
        let request = GuiSurfaceRequest::new(GuiSurfaceOperation::ToggleFps, 1);
        assert!(request.is_valid());
        let mut malformed = request;
        malformed.z_order = 1;
        assert!(!malformed.is_valid());
    }

    #[test]
    fn session_context_requires_all_authority_handles() {
        assert!(GuiSessionContext::EMPTY.is_clear());
        assert!(!GuiSessionContext::EMPTY.is_authenticated());
    }

    #[test]
    fn text_grid_command_requires_exact_cell_aligned_bounds() {
        let bounds = GuiRect::new(0, 0, 80, 32);
        let command = GuiDrawCommand::text_grid(bounds, 10, 2).unwrap();
        assert!(command.is_valid());
        assert_eq!(command.text_grid_columns(), 10);
        assert_eq!(command.text_grid_rows(), 2);

        assert!(GuiDrawCommand::text_grid(bounds, 11, 2).is_none());
        assert!(GuiDrawCommand::text_grid(bounds, 10, 3).is_none());
        assert!(GuiDrawCommand::text_grid(bounds, 0, 2).is_none());
        assert!(GuiDrawCommand::text_grid(bounds, 10, 0).is_none());
    }

    #[test]
    fn text_grid_command_rejects_oversized_grids() {
        let oversized_columns = GuiRect::new(
            0,
            0,
            (MAX_GUI_TEXT_GRID_COLUMNS as u32 + 1) * DISPLAY_CELL_WIDTH as u32,
            DISPLAY_CELL_HEIGHT as u32,
        );
        assert!(
            GuiDrawCommand::text_grid(oversized_columns, MAX_GUI_TEXT_GRID_COLUMNS as u16 + 1, 1,)
                .is_none()
        );
        let oversized_rows = GuiRect::new(
            0,
            0,
            DISPLAY_CELL_WIDTH as u32,
            (MAX_GUI_TEXT_GRID_ROWS as u32 + 1) * DISPLAY_CELL_HEIGHT as u32,
        );
        assert!(
            GuiDrawCommand::text_grid(oversized_rows, 1, MAX_GUI_TEXT_GRID_ROWS as u16 + 1)
                .is_none()
        );

        let mut malformed = GuiDrawCommand::text_grid(GuiRect::new(0, 0, 80, 32), 10, 2).unwrap();
        malformed.text_len = 1;
        assert!(!malformed.is_valid());
    }

    #[test]
    fn text_grid_row_rejects_out_of_bounds_and_unknown_attrs() {
        let surface = SurfaceHandle::new(0, 1, 1).unwrap();
        let mut row = GuiTextGridRow::EMPTY;
        row.surface = surface;
        row.node_id = 4;
        row.row = 0;
        row.cell_count = 1;
        row.cells[0] = super::Cell::EMPTY;
        assert!(row.is_valid());

        let mut out_of_bounds_row = row;
        out_of_bounds_row.row = MAX_GUI_TEXT_GRID_ROWS as u16;
        assert!(!out_of_bounds_row.is_valid());

        let mut oversized = row;
        oversized.cell_count = MAX_GUI_TEXT_GRID_COLUMNS as u16 + 1;
        assert!(!oversized.is_valid());

        let mut unknown_attrs = row;
        unknown_attrs.cells[0].attributes = 1 << 15;
        assert!(!unknown_attrs.is_valid());

        let mut no_node = row;
        no_node.node_id = 0;
        assert!(!no_node.is_valid());
    }

    #[test]
    fn scene_with_forty_eight_nodes_publishes_within_budget() {
        let surface = SurfaceHandle::new(0, 1, 1).unwrap();
        for node_id in 1..=MAX_GUI_NODES as u32 {
            let op = GuiSceneOp::upsert(
                surface,
                1,
                node_id,
                GuiDrawCommand::fill_rect(GuiRect::new(0, node_id as i32, 4, 1), 0x112233),
            );
            assert!(op.is_valid());
        }
        let text_grid = GuiSceneOp::upsert(
            surface,
            1,
            MAX_GUI_NODES as u32 + 1,
            GuiDrawCommand::text_grid(GuiRect::new(0, 0, 80, 32), 10, 2).unwrap(),
        );
        assert!(text_grid.is_valid());
    }
}
