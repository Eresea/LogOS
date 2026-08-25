use super::{NamespaceCapabilityHandle, NamespaceRights, NamespaceRoot, SessionHandle, UserId};

pub const MAX_GUI_SURFACES: usize = 8;
pub const MAX_GUI_DAMAGE_RECTS: usize = 8;
pub const MAX_GUI_COMMANDS: usize = 3;
pub const MAX_GUI_BATCH_FRAGMENTS: usize = 4;
pub const MAX_GUI_TEXT_BYTES: usize = 32;
pub const GUI_DRAW_FLAG_MORE: u8 = 1 << 0;

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
        self.request_id != 0 && self.flags == 0 && self.reserved == 0 && self.reserved_tail == 0
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

    pub const fn clip(bounds: GuiRect) -> Self {
        let mut command = Self::fill_rect(bounds, 0);
        command.kind = GuiDrawKind::ClipRect;
        command
    }

    pub fn glyph_run(x: i32, y: i32, color: u32, text: &[u8]) -> Option<Self> {
        if text.len() > MAX_GUI_TEXT_BYTES {
            return None;
        }
        let mut command = Self::empty(GuiDrawKind::GlyphRun);
        command.x = x;
        command.y = y;
        command.color = color;
        command.text_len = text.len() as u8;
        command.text[..text.len()].copy_from_slice(text);
        Some(command)
    }

    pub const fn is_valid(self) -> bool {
        self.flags == 0
            && self.reserved == 0
            && self.text_len as usize <= MAX_GUI_TEXT_BYTES
            && match self.kind {
                GuiDrawKind::FillRect | GuiDrawKind::StrokeRect | GuiDrawKind::ClipRect => {
                    self.width != 0 && self.height != 0
                }
                GuiDrawKind::Line => self.width != 0 || self.height != 0,
                GuiDrawKind::GlyphRun => self.text_len != 0,
            }
    }
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

const _: () = assert!(core::mem::size_of::<GuiSurfaceRequest>() <= super::MAX_IPC_BYTES);
const _: () = assert!(core::mem::size_of::<GuiSurfaceResponse>() <= super::MAX_IPC_BYTES);
const _: () = assert!(core::mem::size_of::<GuiDrawBatch>() <= super::MAX_IPC_BYTES);
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
    fn session_context_requires_all_authority_handles() {
        assert!(GuiSessionContext::EMPTY.is_clear());
        assert!(!GuiSessionContext::EMPTY.is_authenticated());
    }
}
