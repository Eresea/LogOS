#![no_std]

#[cfg(test)]
extern crate std;

use logos_abi::{
    GUI_DRAW_FLAG_MORE, GuiDrawCommand, GuiNodeOperation, GuiRect, GuiSceneOp, GuiTransform,
    IpcStatus, MAX_GUI_NODES, SurfaceHandle,
};
pub use logos_ui::{UiBlueprint, UiComponentTree, UiNodeKind, UiRect, UiText};
use logos_ui::{UiIcon, UiNode, UiStyle};

pub const MAX_UI_SCENE_OPS: usize = MAX_GUI_NODES + 2;
pub const MAX_UI_SCENE_UPSERTS: usize = MAX_GUI_NODES;
pub const MAX_UI_SCENE_PUBLISHER_BYTES: usize = 7_232;
const GUI_GLYPH_WIDTH: usize = 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiSceneTheme {
    pub surface: u32,
    pub panel: u32,
    pub input: u32,
    pub border: u32,
    pub accent: u32,
    pub focus: u32,
    pub text: u32,
    pub muted: u32,
}

impl UiSceneTheme {
    pub const DEFAULT: Self = Self {
        surface: 0x101820,
        panel: 0x182535,
        input: 0x263548,
        border: 0x334155,
        accent: 0x356bd8,
        focus: 0x4b82f2,
        text: 0xffffff,
        muted: 0xb8c7da,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UiSceneError {
    InvalidSurface,
    InvalidFrame,
    InvalidCommand,
    Capacity,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiSceneFrame {
    ops: [GuiSceneOp; MAX_UI_SCENE_OPS],
    len: u8,
}

impl UiSceneFrame {
    const EMPTY_OP: GuiSceneOp = GuiSceneOp::commit(SurfaceHandle::EMPTY, 1);

    pub const fn new() -> Self {
        Self { ops: [Self::EMPTY_OP; MAX_UI_SCENE_OPS], len: 0 }
    }

    pub const fn len(&self) -> usize {
        self.len as usize
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn as_slice(&self) -> &[GuiSceneOp] {
        &self.ops[..self.len as usize]
    }

    pub fn diff_from(&self, previous: &Self) -> Result<Self, UiSceneError> {
        let Some(current) = self.as_slice().first().copied() else {
            return Ok(Self::new());
        };
        if previous.as_slice().first().is_some_and(|old| old.surface != current.surface) {
            return Ok(*self);
        }
        let (removes, upserts) = diff_counts(self, previous);
        if removes + upserts > MAX_GUI_NODES {
            return Ok(*self);
        }
        let mut delta = Self::new();
        for old in previous
            .as_slice()
            .iter()
            .copied()
            .filter(|operation| operation.operation == GuiNodeOperation::Upsert)
        {
            if !has_node(self, old.node_id) {
                let mut remove = GuiSceneOp::remove(current.surface, current.frame, old.node_id);
                remove.flags = GUI_DRAW_FLAG_MORE;
                push(&mut delta, remove)?;
            }
        }
        for operation in self.as_slice().iter().copied().filter(|operation| {
            operation.operation == GuiNodeOperation::Upsert && changed_node(previous, operation)
        }) {
            let mut operation = operation;
            operation.flags = GUI_DRAW_FLAG_MORE;
            push(&mut delta, operation)?;
        }
        if delta.is_empty() {
            return Ok(delta);
        }
        push(&mut delta, GuiSceneOp::commit(current.surface, current.frame))?;
        delta.ops[delta.len() - 1].flags = 0;
        Ok(delta)
    }
}

pub trait UiSceneSink {
    fn send(&mut self, operation: &GuiSceneOp) -> IpcStatus;
}

/// Fixed-memory retained-scene publisher. It completes an in-flight frame
/// before sending a newer coalesced snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiScenePublisher {
    baseline: UiSceneFrame,
    pending: UiSceneFrame,
    baseline_surface: SurfaceHandle,
    baseline_cursor_signature: Option<u64>,
    pending_surface: SurfaceHandle,
    pending_cursor_signature: Option<u64>,
    pending_index: u8,
    baseline_ready: bool,
    pending_ready: bool,
}

impl UiScenePublisher {
    pub const fn new() -> Self {
        Self {
            baseline: UiSceneFrame::new(),
            pending: UiSceneFrame::new(),
            baseline_surface: SurfaceHandle::EMPTY,
            baseline_cursor_signature: None,
            pending_surface: SurfaceHandle::EMPTY,
            pending_cursor_signature: None,
            pending_index: 0,
            baseline_ready: false,
            pending_ready: false,
        }
    }

    pub const fn is_pending(&self) -> bool {
        self.pending_ready
    }

    pub fn is_pending_for(&self, surface: SurfaceHandle, frame: u32) -> bool {
        self.pending_ready
            && self.pending_surface == surface
            && self.pending.as_slice().first().is_some_and(|operation| operation.frame == frame)
    }

    pub fn reset(&mut self) {
        self.baseline_ready = false;
        self.pending_ready = false;
        self.pending_index = 0;
        self.baseline_surface = SurfaceHandle::EMPTY;
        self.pending_surface = SurfaceHandle::EMPTY;
        self.baseline_cursor_signature = None;
        self.pending_cursor_signature = None;
        self.baseline = UiSceneFrame::new();
        self.pending = UiSceneFrame::new();
    }

    /// `cursor_signature` identifies a cursor-owned scene; ordinary surfaces
    /// pass `None` and remain independent of cursor movement.
    /// Pending frames resume from their stored operations; the tree is only
    /// read when staging a new frame.
    pub fn publish<S: UiSceneSink>(
        &mut self,
        surface: SurfaceHandle,
        frame: u32,
        tree: &UiComponentTree,
        theme: UiSceneTheme,
        cursor_signature: Option<u64>,
        sink: &mut S,
    ) -> Result<(IpcStatus, usize), UiSceneError> {
        let mut sent_before = 0;
        if self.pending_ready {
            let pending_frame = self.pending.as_slice()[0].frame;
            let same_frame = pending_frame == frame
                && self.pending_surface == surface
                && self.pending_cursor_signature == cursor_signature;
            let result = self.flush(sink);
            if result.0 != IpcStatus::Ok {
                return Ok(result);
            }
            self.complete()?;
            if same_frame {
                return Ok(result);
            }
            sent_before = result.1;
        }

        emit_into(surface, frame, tree, theme, &mut self.pending)?;
        if cursor_signature.is_none() && has_cursor_shape(&self.pending) {
            return Err(UiSceneError::InvalidCommand);
        }
        let force_full = !self.baseline_ready
            || self.baseline_surface != surface
            || self.baseline_cursor_signature != cursor_signature;
        self.pending_index = 0;
        self.pending_surface = surface;
        self.pending_cursor_signature = cursor_signature;
        if force_full {
            self.pending_ready = true;
        } else {
            let mutations = make_delta_in_place(&self.baseline, &mut self.pending)?;
            self.pending_ready = mutations != 0;
            if !self.pending_ready {
                self.store_full_baseline();
                self.baseline_surface = surface;
                self.baseline_cursor_signature = cursor_signature;
                return Ok((IpcStatus::Ok, sent_before));
            }
        }

        let result = self.flush(sink);
        if result.0 == IpcStatus::Ok {
            self.complete()?;
        }
        Ok((result.0, sent_before + result.1))
    }

    fn flush<S: UiSceneSink>(&mut self, sink: &mut S) -> (IpcStatus, usize) {
        if !self.pending_ready {
            return (IpcStatus::Ok, 0);
        }
        let operations = self.pending.as_slice();
        while usize::from(self.pending_index) < operations.len() {
            let index = usize::from(self.pending_index);
            let status = sink.send(&operations[index]);
            if status != IpcStatus::Ok {
                return (status, index);
            }
            self.pending_index += 1;
        }
        (IpcStatus::Ok, operations.len())
    }

    fn complete(&mut self) -> Result<(), UiSceneError> {
        if self.pending.ops[0].operation == GuiNodeOperation::Clear {
            self.store_full_baseline();
        } else {
            apply_delta(&mut self.baseline, &self.pending)?;
        }
        self.baseline_surface = self.pending_surface;
        self.baseline_cursor_signature = self.pending_cursor_signature;
        self.baseline_ready = true;
        self.pending_ready = false;
        self.pending_index = 0;
        Ok(())
    }

    fn store_full_baseline(&mut self) {
        self.baseline = self.pending;
        if self.baseline.ops[self.baseline.len() - 1].operation == GuiNodeOperation::Commit {
            self.baseline.len -= 1;
        }
    }
}

fn has_cursor_shape(scene: &UiSceneFrame) -> bool {
    scene.as_slice().iter().any(|operation| {
        operation.operation == GuiNodeOperation::Upsert
            && operation.node_id == 1
            && operation.command.kind == logos_abi::GuiDrawKind::FillRect
            && operation.command.width == 3
            && operation.command.height == 14
    })
}

fn apply_delta(baseline: &mut UiSceneFrame, delta: &UiSceneFrame) -> Result<(), UiSceneError> {
    if baseline.is_empty() || delta.len() < 2 {
        return Err(UiSceneError::InvalidCommand);
    }
    let commit = delta.ops[delta.len() - 1];
    if commit.operation != GuiNodeOperation::Commit {
        return Err(UiSceneError::InvalidCommand);
    }
    for operation in &delta.ops[..delta.len() - 1] {
        match operation.operation {
            GuiNodeOperation::Remove => {
                if let Some(index) = baseline.ops[..baseline.len()].iter().position(|old| {
                    old.operation == GuiNodeOperation::Upsert && old.node_id == operation.node_id
                }) {
                    let length = baseline.len();
                    baseline.ops.copy_within(index + 1..length, index);
                    baseline.len -= 1;
                }
            }
            GuiNodeOperation::Upsert => {
                if let Some(index) = baseline.ops[..baseline.len()].iter().position(|old| {
                    old.operation == GuiNodeOperation::Upsert && old.node_id == operation.node_id
                }) {
                    baseline.ops[index] = *operation;
                } else {
                    if baseline.len() >= MAX_UI_SCENE_OPS {
                        return Err(UiSceneError::Capacity);
                    }
                    baseline.ops[baseline.len()] = *operation;
                    baseline.len += 1;
                }
            }
            GuiNodeOperation::Clear => {
                return Err(UiSceneError::InvalidCommand);
            }
            GuiNodeOperation::Commit => {}
        }
    }
    let length = baseline.len();
    for operation in &mut baseline.ops[..length] {
        operation.frame = commit.frame;
    }
    Ok(())
}

impl Default for UiScenePublisher {
    fn default() -> Self {
        Self::new()
    }
}

const _: () = assert!(core::mem::size_of::<UiScenePublisher>() <= MAX_UI_SCENE_PUBLISHER_BYTES);

impl Default for UiSceneFrame {
    fn default() -> Self {
        Self::new()
    }
}

pub fn emit(
    surface: SurfaceHandle,
    frame: u32,
    tree: &UiComponentTree,
    theme: UiSceneTheme,
) -> Result<UiSceneFrame, UiSceneError> {
    let mut output = UiSceneFrame::new();
    emit_into(surface, frame, tree, theme, &mut output)?;
    Ok(output)
}

pub fn emit_into(
    surface: SurfaceHandle,
    frame: u32,
    tree: &UiComponentTree,
    theme: UiSceneTheme,
    output: &mut UiSceneFrame,
) -> Result<(), UiSceneError> {
    if !surface.is_valid() {
        return Err(UiSceneError::InvalidSurface);
    }
    if frame == 0 {
        return Err(UiSceneError::InvalidFrame);
    }

    output.len = 0;
    push(output, clear_op(surface, frame))?;

    for index in 0..logos_ui::MAX_UI_NODES {
        let Ok(handle) = tree.tree().handle_at(index) else { continue };
        let node = tree.tree().node(handle).map_err(|_| UiSceneError::Capacity)?;
        let bounds = visible_bounds(node);
        if bounds.is_empty() {
            continue;
        }
        emit_node(output, surface, frame, index, node, tree, bounds, theme)?;
    }

    if output.len() == 1 {
        push(output, GuiSceneOp::commit(surface, frame))?;
    } else {
        output.ops[output.len() - 1].flags = 0;
    }
    Ok(())
}

fn diff_counts(current: &UiSceneFrame, previous: &UiSceneFrame) -> (usize, usize) {
    let removes = previous
        .as_slice()
        .iter()
        .filter(|old| old.operation == GuiNodeOperation::Upsert && !has_node(current, old.node_id))
        .count();
    let upserts = current
        .as_slice()
        .iter()
        .filter(|operation| {
            operation.operation == GuiNodeOperation::Upsert && changed_node(previous, operation)
        })
        .count();
    (removes, upserts)
}

fn has_node(scene: &UiSceneFrame, node_id: u32) -> bool {
    scene.as_slice().iter().any(|operation| {
        operation.operation == GuiNodeOperation::Upsert && operation.node_id == node_id
    })
}

fn changed_node(previous: &UiSceneFrame, operation: &GuiSceneOp) -> bool {
    !previous.as_slice().iter().any(|old| {
        old.operation == GuiNodeOperation::Upsert
            && old.node_id == operation.node_id
            && old.command == operation.command
    })
}

/// Replaces a full candidate with its compact delta. Returns `MAX_GUI_NODES + 1`
/// when the delta is too large, leaving the full candidate intact for fallback.
fn make_delta_in_place(
    previous: &UiSceneFrame,
    current: &mut UiSceneFrame,
) -> Result<usize, UiSceneError> {
    let (removes, changed) = diff_counts(current, previous);
    if removes + changed > MAX_GUI_NODES {
        return Ok(MAX_GUI_NODES + 1);
    }
    if removes + changed == 0 {
        return Ok(0);
    }

    let mut removed_ids = [0; MAX_GUI_NODES];
    let mut removed_len = 0;
    for old in previous
        .as_slice()
        .iter()
        .filter(|op| op.operation == GuiNodeOperation::Upsert && !has_node(current, op.node_id))
    {
        removed_ids[removed_len] = old.node_id;
        removed_len += 1;
    }

    let original_len = current.len();
    let surface = current.ops[0].surface;
    let frame = current.ops[0].frame;
    let mut write = 1;
    for read in 1..original_len {
        let operation = current.ops[read];
        if operation.operation == GuiNodeOperation::Upsert && changed_node(previous, &operation) {
            current.ops[write] = operation;
            write += 1;
        }
    }
    current.ops.copy_within(1..write, removes);

    for (remove_index, node_id) in removed_ids[..removed_len].iter().copied().enumerate() {
        let mut operation = GuiSceneOp::remove(surface, frame, node_id);
        operation.flags = GUI_DRAW_FLAG_MORE;
        current.ops[remove_index] = operation;
    }

    let delta_len = removes + changed + 1;
    current.ops[delta_len - 1] = GuiSceneOp::commit(surface, frame);
    current.ops[delta_len - 1].flags = 0;
    current.len = delta_len as u8;
    Ok(removes + changed)
}

#[allow(clippy::too_many_arguments)]
fn emit_node(
    output: &mut UiSceneFrame,
    surface: SurfaceHandle,
    frame: u32,
    index: usize,
    node: &UiNode,
    tree: &UiComponentTree,
    bounds: UiRect,
    theme: UiSceneTheme,
) -> Result<(), UiSceneError> {
    match node.kind {
        UiNodeKind::Root => {
            if !node.styles.contains(UiStyle::Transparent) {
                push_upsert(
                    output,
                    surface,
                    frame,
                    index,
                    0,
                    with_transform(
                        GuiDrawCommand::fill_rect(to_gui_rect(bounds), color(theme.surface, node)),
                        node,
                    ),
                )?;
            }
        }
        UiNodeKind::Panel | UiNodeKind::Form | UiNodeKind::RouteFrame => {
            push_shadow(output, surface, frame, index, node, bounds)?;
            push_upsert(
                output,
                surface,
                frame,
                index,
                1,
                fill_command(bounds, panel_color(node, theme), node),
            )?;
        }
        UiNodeKind::Label => {
            push_text(
                output,
                surface,
                frame,
                index,
                node,
                node.text.as_bytes(),
                text_color(node, theme),
                0,
            )?;
        }
        UiNodeKind::Avatar => {
            let size = bounds.width.min(bounds.height).min(64);
            let circle = UiRect::new(
                bounds.x.saturating_add(bounds.width.saturating_sub(size) as i32 / 2),
                bounds.y.saturating_add(bounds.height.saturating_sub(size) as i32 / 2),
                size,
                size,
            );
            push_upsert(
                output,
                surface,
                frame,
                index,
                1,
                with_transform(
                    GuiDrawCommand::fill_rounded_rect(
                        to_gui_rect(circle),
                        color(control_color(node, theme), node),
                        (size / 2) as u8,
                    ),
                    node,
                ),
            )?;
            if node.icon == UiIcon::LogosMark {
                push_upsert(
                    output,
                    surface,
                    frame,
                    index,
                    2,
                    with_transform(
                        GuiDrawCommand::logos_mark(
                            to_gui_rect(circle),
                            color(text_color(node, theme), node),
                        ),
                        node,
                    ),
                )?;
            } else if let Some(symbol) = material_symbol(node.icon) {
                push_upsert(
                    output,
                    surface,
                    frame,
                    index,
                    2,
                    with_transform(
                        material_symbol_command(circle, text_color(node, theme), symbol),
                        node,
                    ),
                )?;
            } else {
                push_avatar_text(
                    output,
                    surface,
                    frame,
                    index,
                    node,
                    circle,
                    node.text.as_bytes(),
                    text_color(node, theme),
                )?;
            }
        }
        UiNodeKind::Button => {
            push_shadow(output, surface, frame, index, node, bounds)?;
            push_upsert(
                output,
                surface,
                frame,
                index,
                1,
                fill_command(bounds, control_color(node, theme), node),
            )?;
            if let Some(symbol) = material_symbol(node.icon) {
                push_upsert(
                    output,
                    surface,
                    frame,
                    index,
                    2,
                    with_transform(
                        material_symbol_command(bounds, text_color(node, theme), symbol),
                        node,
                    ),
                )?;
            } else {
                push_text(
                    output,
                    surface,
                    frame,
                    index,
                    node,
                    node.text.as_bytes(),
                    text_color(node, theme),
                    2,
                )?;
            }
        }
        UiNodeKind::TextInput => {
            push_shadow(output, surface, frame, index, node, bounds)?;
            push_upsert(
                output,
                surface,
                frame,
                index,
                1,
                fill_command(bounds, control_color(node, theme), node),
            )?;
            let value = tree.value(node.handle).unwrap_or(node.text);
            let value = if value.as_bytes().is_empty() { node.text } else { value };
            push_text(
                output,
                surface,
                frame,
                index,
                node,
                value.as_bytes(),
                text_color(node, theme),
                2,
            )?;
        }
    }
    Ok(())
}

fn material_symbol(icon: UiIcon) -> Option<logos_abi::GuiMaterialSymbol> {
    match icon {
        UiIcon::None => None,
        UiIcon::Settings => Some(logos_abi::GuiMaterialSymbol::Settings),
        UiIcon::LogosMark => None,
    }
}

fn material_symbol_command(
    bounds: UiRect,
    color: u32,
    symbol: logos_abi::GuiMaterialSymbol,
) -> GuiDrawCommand {
    let size = bounds.width.min(bounds.height).min(24);
    GuiDrawCommand::material_symbol(
        GuiRect::new(
            bounds.x.saturating_add(bounds.width.saturating_sub(size) as i32 / 2),
            bounds.y.saturating_add(bounds.height.saturating_sub(size) as i32 / 2),
            size,
            size,
        ),
        color,
        symbol,
    )
}

#[allow(clippy::too_many_arguments)]
fn push_avatar_text(
    output: &mut UiSceneFrame,
    surface: SurfaceHandle,
    frame: u32,
    index: usize,
    node: &UiNode,
    bounds: UiRect,
    text: &[u8],
    text_color: u32,
) -> Result<(), UiSceneError> {
    if text.is_empty() {
        return Ok(());
    }
    let scale = text_scale(node) as u32;
    let text_width =
        (text.len() as u32).saturating_mul(GUI_GLYPH_WIDTH as u32).saturating_mul(scale);
    let x = bounds.x.saturating_add(bounds.width.saturating_sub(text_width) as i32 / 2);
    let text_height = logos_display_text_height(scale as usize);
    let y = bounds.y.saturating_add(bounds.height.saturating_sub(text_height) as i32 / 2);
    let Some(command) =
        GuiDrawCommand::glyph_run_styled(x, y, color(text_color, node), text_flags(node), text)
    else {
        return Err(UiSceneError::InvalidCommand);
    };
    push_upsert(output, surface, frame, index, 2, with_transform(command, node))
}

#[allow(clippy::too_many_arguments)]
fn push_text(
    output: &mut UiSceneFrame,
    surface: SurfaceHandle,
    frame: u32,
    index: usize,
    node: &UiNode,
    text: &[u8],
    text_color: u32,
    fragment: u32,
) -> Result<(), UiSceneError> {
    if text.is_empty() {
        return Ok(());
    }
    let mut offset = 0;
    let mut chunk = 0;
    while offset < text.len() {
        let end = offset.saturating_add(logos_abi::MAX_GUI_TEXT_BYTES).min(text.len());
        let scale = text_scale(node);
        let x_offset = offset.saturating_mul(GUI_GLYPH_WIDTH).saturating_mul(scale) as i32;
        let node_id = if chunk == 0 {
            (index as u32).saturating_mul(3).saturating_add(fragment + 1)
        } else {
            0x8000_0000 | index as u32
        };
        let text_height = logos_display_text_height(scale);
        let y =
            node.bounds.y.saturating_add(
                node.bounds.height.saturating_sub(text_height).saturating_div(2) as i32,
            );
        let Some(command) = GuiDrawCommand::glyph_run_styled(
            node.bounds.x.saturating_add(12).saturating_add(x_offset),
            y,
            color(text_color, node),
            text_flags(node),
            &text[offset..end],
        ) else {
            return Err(UiSceneError::Capacity);
        };
        push_upsert_id(output, surface, frame, node_id, with_transform(command, node))?;
        offset = end;
        chunk += 1;
    }
    Ok(())
}

fn push_shadow(
    output: &mut UiSceneFrame,
    surface: SurfaceHandle,
    frame: u32,
    index: usize,
    node: &UiNode,
    bounds: UiRect,
) -> Result<(), UiSceneError> {
    if !has_rounded_style(node) {
        return Ok(());
    }
    let radius = corner_radius(bounds, node);
    push_upsert(
        output,
        surface,
        frame,
        index,
        0,
        with_transform(
            GuiDrawCommand::shadow(to_gui_rect(bounds), 0x55000000, radius, 3, 0, 3),
            node,
        ),
    )
}

fn push_upsert(
    output: &mut UiSceneFrame,
    surface: SurfaceHandle,
    frame: u32,
    index: usize,
    fragment: u32,
    command: GuiDrawCommand,
) -> Result<(), UiSceneError> {
    if !command.is_valid() {
        return Err(UiSceneError::InvalidCommand);
    }
    let node_id = (index as u32).saturating_mul(3).saturating_add(fragment + 1);
    push_upsert_id(output, surface, frame, node_id, command)
}

fn push_upsert_id(
    output: &mut UiSceneFrame,
    surface: SurfaceHandle,
    frame: u32,
    node_id: u32,
    command: GuiDrawCommand,
) -> Result<(), UiSceneError> {
    let mut op = GuiSceneOp::upsert(surface, frame, node_id, command);
    op.flags = GUI_DRAW_FLAG_MORE;
    push(output, op)
}

fn push(output: &mut UiSceneFrame, op: GuiSceneOp) -> Result<(), UiSceneError> {
    if op.operation == GuiNodeOperation::Upsert
        && output
            .as_slice()
            .iter()
            .filter(|operation| operation.operation == GuiNodeOperation::Upsert)
            .count()
            == MAX_UI_SCENE_UPSERTS
    {
        return Err(UiSceneError::Capacity);
    }
    if output.len() >= MAX_UI_SCENE_OPS {
        return Err(UiSceneError::Capacity);
    }
    output.ops[output.len()] = op;
    output.len += 1;
    Ok(())
}

fn clear_op(surface: SurfaceHandle, frame: u32) -> GuiSceneOp {
    let mut op = GuiSceneOp::clear(surface, frame);
    op.flags = GUI_DRAW_FLAG_MORE;
    op
}

fn visible_bounds(node: &UiNode) -> UiRect {
    if node.clip.is_empty() { node.bounds } else { intersect(node.bounds, node.clip) }
}

fn fill_command(bounds: UiRect, raw_color: u32, node: &UiNode) -> GuiDrawCommand {
    let color = color(raw_color, node);
    let rect = to_gui_rect(bounds);
    let radius = corner_radius(bounds, node);
    if radius != 0 {
        return with_transform(GuiDrawCommand::fill_rounded_rect(rect, color, radius), node);
    }
    with_transform(GuiDrawCommand::fill_rect(rect, color), node)
}

fn with_transform(command: GuiDrawCommand, node: &UiNode) -> GuiDrawCommand {
    command.with_transform(GuiTransform {
        translate_x: node.transform.translate_x,
        translate_y: node.transform.translate_y,
        scale_q8_8: node.transform.scale_q8_8,
        rotation_degrees: node.transform.rotation_degrees,
        reserved: 0,
    })
}

fn corner_radius(bounds: UiRect, node: &UiNode) -> u8 {
    if node.styles.contains(UiStyle::RoundedFull) {
        bounds.width.min(bounds.height).min(64) as u8 / 2
    } else if node.styles.contains(UiStyle::RoundedLarge) {
        bounds.width.min(bounds.height).min(24) as u8 / 2
    } else {
        0
    }
}

fn has_rounded_style(node: &UiNode) -> bool {
    node.styles.contains(UiStyle::RoundedLarge)
}

fn panel_color(node: &UiNode, theme: UiSceneTheme) -> u32 {
    if node.styles.contains(UiStyle::BackgroundAccent) { theme.accent } else { theme.panel }
}

fn control_color(node: &UiNode, theme: UiSceneTheme) -> u32 {
    if node.interaction.is_focused() || node.interaction.is_pressed() {
        theme.focus
    } else if node.interaction.is_hovered() || node.styles.contains(UiStyle::BackgroundAccent) {
        theme.accent
    } else {
        theme.input
    }
}

fn text_color(node: &UiNode, theme: UiSceneTheme) -> u32 {
    if node.styles.contains(UiStyle::TextMuted) { theme.muted } else { theme.text }
}

fn text_scale(node: &UiNode) -> usize {
    if node.styles.contains(UiStyle::Text4xl) { 2 } else { 1 }
}

fn text_flags(node: &UiNode) -> u32 {
    let mut flags = 0;
    if node.styles.contains(UiStyle::FontLight) {
        flags |= logos_abi::GUI_TEXT_FLAG_LIGHT;
    }
    if text_scale(node) == 2 {
        flags |= logos_abi::GUI_TEXT_FLAG_DOUBLE;
    }
    flags
}

const fn logos_display_text_height(scale: usize) -> u32 {
    (16 * scale) as u32
}

fn color(value: u32, node: &UiNode) -> u32 {
    let style_alpha = if node.styles.contains(UiStyle::Opacity50) { 128 } else { 255 };
    let motion_alpha = (u32::from(node.opacity_q16) * style_alpha / 65_535) as u8;
    (value & 0x00ff_ffff) | (u32::from(motion_alpha.max(1)) << 24)
}

fn to_gui_rect(rect: UiRect) -> logos_abi::GuiRect {
    logos_abi::GuiRect::new(rect.x, rect.y, rect.width, rect.height)
}

fn intersect(left: UiRect, right: UiRect) -> UiRect {
    let x = left.x.max(right.x);
    let y = left.y.max(right.y);
    let right_edge = left
        .x
        .saturating_add(left.width.min(i32::MAX as u32) as i32)
        .min(right.x.saturating_add(right.width.min(i32::MAX as u32) as i32));
    let bottom = left
        .y
        .saturating_add(left.height.min(i32::MAX as u32) as i32)
        .min(right.y.saturating_add(right.height.min(i32::MAX as u32) as i32));
    if right_edge <= x || bottom <= y {
        UiRect::EMPTY
    } else {
        UiRect::new(x, y, (right_edge - x) as u32, (bottom - y) as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use logos_abi::{GuiSurfaceOperation, GuiSurfaceRequest};
    use logos_display::{Display, GuiSurfaceRegistry, PixelFormat};
    use logos_ui::{UiBlueprint, UiIcon, UiNodeKind, UiStyle, UiStyleList, UiText};
    use logos_ui_compiler::UiBuild;
    use std::vec::Vec;

    const TEST_BOUNDS: GuiRect = GuiRect::new(0, 0, 1024, 768);
    const TEST_OWNER: u32 = 12;

    struct RegistrySink<'a> {
        registry: &'a mut GuiSurfaceRegistry,
        fail_at: Option<usize>,
        calls: usize,
        failed: bool,
        seen: Vec<GuiSceneOp>,
    }

    impl UiSceneSink for RegistrySink<'_> {
        fn send(&mut self, operation: &GuiSceneOp) -> IpcStatus {
            self.seen.push(*operation);
            let index = self.calls;
            self.calls += 1;
            if !self.failed && self.fail_at == Some(index) {
                self.failed = true;
                return IpcStatus::Full;
            }
            match self.registry.apply_scene_op(TEST_OWNER, *operation) {
                Ok(()) => IpcStatus::Ok,
                Err(_) => IpcStatus::Malformed,
            }
        }
    }

    fn registry_surface(registry: &mut GuiSurfaceRegistry) -> SurfaceHandle {
        let mut root = GuiSurfaceRequest::new(GuiSurfaceOperation::CreateRoot, 1);
        root.bounds = TEST_BOUNDS;
        registry.create(TEST_OWNER, root).unwrap();
        let mut request = GuiSurfaceRequest::new(GuiSurfaceOperation::CreateModal, 2);
        request.bounds = TEST_BOUNDS;
        request.z_order = 1;
        registry.create(TEST_OWNER, request).unwrap().surface
    }

    fn display_surface(display: &mut Display) -> SurfaceHandle {
        let mut root = GuiSurfaceRequest::new(GuiSurfaceOperation::CreateRoot, 1);
        root.bounds = TEST_BOUNDS;
        display.gui_mut().create(TEST_OWNER, root).unwrap();
        let mut request = GuiSurfaceRequest::new(GuiSurfaceOperation::CreateModal, 2);
        request.bounds = TEST_BOUNDS;
        request.z_order = 1;
        display.gui_mut().create(TEST_OWNER, request).unwrap().surface
    }

    fn ready_registry_sink(registry: &mut GuiSurfaceRegistry) -> RegistrySink<'_> {
        RegistrySink { registry, fail_at: None, calls: 0, failed: false, seen: Vec::new() }
    }

    fn sample_tree() -> UiComponentTree {
        let mut blueprint = UiBlueprint::new();
        let root = blueprint.push_root(UiNodeKind::Root, 1).unwrap();
        let label = blueprint.push_child(UiNodeKind::Label, root, 2).unwrap();
        let button = blueprint.push_child(UiNodeKind::Button, root, 3).unwrap();
        blueprint.set_text(label, UiText::from_bytes(b"Hello").unwrap()).unwrap();
        blueprint.set_text(button, UiText::from_bytes(b"Go").unwrap()).unwrap();
        UiComponentTree::from_blueprint(&blueprint).unwrap()
    }

    fn set_bounds(tree: &mut UiComponentTree, index: usize, bounds: UiRect) {
        let handle = tree.tree().handle_at(index).unwrap();
        tree.tree_mut().set_bounds(handle, bounds).unwrap();
    }

    fn app_tree(build: &UiBuild) -> UiComponentTree {
        let mut tree = UiComponentTree::new();
        tree.reset_from_document(&build.document).unwrap();
        let layout = logos_shell::LoginLayout::from_build(build, TEST_BOUNDS).unwrap();
        let conditions = logos_ui::UiStyleConditions::EMPTY;
        for index in 0..build.document.node_count() {
            let handle = tree.tree().handle_at(index).unwrap();
            let node = layout.node(index as u16).unwrap();
            tree.apply_document_styles(&build.document, index as u16, &conditions).unwrap();
            let bounds = if index == 0 {
                UiRect::new(0, 0, TEST_BOUNDS.width, TEST_BOUNDS.height)
            } else {
                UiRect::new(node.bounds.x, node.bounds.y, node.bounds.width, node.bounds.height)
            };
            tree.tree_mut().set_bounds(handle, bounds).unwrap();
            let focused =
                build.document.node(index).is_some_and(|node| node.key.as_bytes() == b"username");
            tree.tree_mut().set_focused(handle, focused).unwrap();
            if build.document.node(index).is_some_and(|node| node.key.as_bytes() == b"submit") {
                tree.set_disabled(handle, true).unwrap();
            }
        }
        tree
    }

    fn apply_scene(registry: &mut GuiSurfaceRegistry, scene: &UiSceneFrame) {
        for operation in scene.as_slice() {
            registry.apply_scene_op(TEST_OWNER, *operation).unwrap();
        }
    }

    fn apply_display_scene(display: &mut Display, scene: &UiSceneFrame) {
        apply_scene(display.gui_mut(), scene);
    }

    fn pixels(display: &mut Display) -> Vec<u8> {
        display.gui_mut().invalidate_rect(TEST_BOUNDS);
        let mut framebuffer =
            std::vec![0; TEST_BOUNDS.width as usize * TEST_BOUNDS.height as usize * 4];
        while display.render_pending() {
            display
                .render_gui(
                    &mut framebuffer,
                    TEST_BOUNDS.width as usize,
                    TEST_BOUNDS.height as usize,
                    TEST_BOUNDS.width as usize * 4,
                    PixelFormat::Bgr8,
                )
                .unwrap();
        }
        framebuffer
    }

    #[test]
    fn emits_atomic_scene_with_stable_fragment_ids() {
        let mut tree = sample_tree();
        set_bounds(&mut tree, 0, UiRect::new(0, 0, 100, 80));
        set_bounds(&mut tree, 1, UiRect::new(8, 8, 40, 16));
        set_bounds(&mut tree, 2, UiRect::new(8, 32, 60, 24));

        let surface = SurfaceHandle::new(1, 1, 7).unwrap();
        let scene = emit(surface, 4, &tree, UiSceneTheme::DEFAULT).unwrap();
        assert_eq!(scene.len(), 5);
        assert_eq!(scene.as_slice()[0].operation, logos_abi::GuiNodeOperation::Clear);
        assert_eq!(scene.as_slice()[1].node_id, 1);
        assert_eq!(scene.as_slice()[2].node_id, 4);
        assert_eq!(scene.as_slice()[3].node_id, 8);
        assert_eq!(scene.as_slice()[4].node_id, 9);
        assert_eq!(scene.as_slice()[0].flags, GUI_DRAW_FLAG_MORE);
        assert_eq!(scene.as_slice()[4].flags, 0);
        assert!(scene.as_slice().iter().all(|op| op.is_valid()));
    }

    #[test]
    fn scene_diff_emits_only_changed_nodes_and_commit() {
        let mut tree = sample_tree();
        set_bounds(&mut tree, 0, UiRect::new(0, 0, 100, 80));
        set_bounds(&mut tree, 1, UiRect::new(8, 8, 40, 16));
        set_bounds(&mut tree, 2, UiRect::new(8, 32, 60, 24));
        let surface = SurfaceHandle::new(1, 1, 7).unwrap();
        let previous = emit(surface, 4, &tree, UiSceneTheme::DEFAULT).unwrap();
        let label = tree.tree().handle_at(1).unwrap();
        tree.set_text(label, UiText::from_bytes(b"World").unwrap()).unwrap();
        let current = emit(surface, 5, &tree, UiSceneTheme::DEFAULT).unwrap();

        let delta = current.diff_from(&previous).unwrap();
        assert_eq!(delta.len(), 2);
        assert_eq!(delta.as_slice()[0].operation, logos_abi::GuiNodeOperation::Upsert);
        assert_eq!(delta.as_slice()[0].node_id, 4);
        assert_eq!(delta.as_slice()[1].operation, logos_abi::GuiNodeOperation::Commit);
        assert_eq!(delta.as_slice()[0].flags, GUI_DRAW_FLAG_MORE);
        assert_eq!(delta.as_slice()[1].flags, 0);
    }

    #[test]
    fn transparent_root_does_not_paint_over_composed_surfaces() {
        let mut blueprint = UiBlueprint::new();
        let root = blueprint.push_root(UiNodeKind::Root, 1).unwrap();
        let mut styles = UiStyleList::EMPTY;
        assert!(styles.push(UiStyle::Transparent));
        blueprint.set_styles(root, styles).unwrap();
        let mut tree = UiComponentTree::from_blueprint(&blueprint).unwrap();
        set_bounds(&mut tree, 0, UiRect::new(0, 0, 100, 80));

        let surface = SurfaceHandle::new(1, 1, 7).unwrap();
        let scene = emit(surface, 4, &tree, UiSceneTheme::DEFAULT).unwrap();
        assert_eq!(scene.len(), 2);
        assert_eq!(scene.as_slice()[1].operation, logos_abi::GuiNodeOperation::Commit);
    }

    #[test]
    fn rejects_more_visual_commands_than_display_can_retain() {
        let mut blueprint = UiBlueprint::new();
        let root = blueprint.push_root(UiNodeKind::Root, 1).unwrap();
        let text = UiText::from_bytes(b"x").unwrap();
        let button_count = logos_abi::MAX_GUI_NODES / 2 + 1;
        for index in 0..button_count {
            let button = blueprint.push_child(UiNodeKind::Button, root, index as u16 + 2).unwrap();
            blueprint.set_text(button, text).unwrap();
        }
        let mut tree = UiComponentTree::from_blueprint(&blueprint).unwrap();
        for index in 0..tree.tree().len() {
            set_bounds(&mut tree, index, UiRect::new(0, index as i32, 20, 20));
        }
        let surface = SurfaceHandle::new(1, 1, 7).unwrap();
        assert_eq!(emit(surface, 1, &tree, UiSceneTheme::DEFAULT), Err(UiSceneError::Capacity));
    }

    #[test]
    fn clips_commands_to_node_clip() {
        let mut tree = sample_tree();
        set_bounds(&mut tree, 0, UiRect::new(0, 0, 100, 80));
        set_bounds(&mut tree, 1, UiRect::new(8, 8, 40, 16));
        set_bounds(&mut tree, 2, UiRect::new(8, 32, 60, 24));
        let root = tree.tree().handle_at(0).unwrap();
        tree.tree_mut().set_clip(root, UiRect::new(0, 0, 50, 40)).unwrap();
        let surface = SurfaceHandle::new(1, 1, 7).unwrap();
        let scene = emit(surface, 1, &tree, UiSceneTheme::DEFAULT).unwrap();
        assert_eq!(scene.as_slice()[1].command.width, 50);
    }

    #[test]
    fn rounded_surface_emits_fixed_radius_and_shadow_before_fill() {
        let mut blueprint = UiBlueprint::new();
        let root = blueprint.push_root(UiNodeKind::Root, 1).unwrap();
        let button = blueprint.push_child(UiNodeKind::Button, root, 2).unwrap();
        blueprint.set_text(button, UiText::from_bytes(b"Go").unwrap()).unwrap();
        let mut styles = logos_ui::UiStyleList::EMPTY;
        assert!(styles.push(logos_ui::UiStyle::RoundedLarge));
        blueprint.set_styles(button, styles).unwrap();
        let mut tree = UiComponentTree::from_blueprint(&blueprint).unwrap();
        set_bounds(&mut tree, 0, UiRect::new(0, 0, 100, 60));
        set_bounds(&mut tree, 1, UiRect::new(8, 8, 80, 24));

        let surface = SurfaceHandle::new(1, 1, 7).unwrap();
        let scene = emit(surface, 1, &tree, UiSceneTheme::DEFAULT).unwrap();
        assert_eq!(scene.as_slice()[2].command.kind, logos_abi::GuiDrawKind::Shadow);
        assert_eq!(scene.as_slice()[3].command.kind, logos_abi::GuiDrawKind::FillRoundedRect);
        assert_eq!(scene.as_slice()[3].command.corner_radius(), 12);
        assert_eq!(scene.as_slice()[2].command.shadow_blur(), 3);
        assert!(scene.as_slice().iter().all(|op| op.is_valid()));
    }

    #[test]
    fn hovered_button_uses_accent_without_focus_flash() {
        let mut tree = sample_tree();
        set_bounds(&mut tree, 0, UiRect::new(0, 0, 100, 60));
        set_bounds(&mut tree, 1, UiRect::new(8, 8, 40, 16));
        set_bounds(&mut tree, 2, UiRect::new(8, 32, 60, 24));
        let button = tree.tree().handle_at(2).unwrap();
        tree.tree_mut().set_hovered(button, true).unwrap();

        let surface = SurfaceHandle::new(1, 1, 7).unwrap();
        let scene = emit(surface, 1, &tree, UiSceneTheme::DEFAULT).unwrap();
        let fill = scene.as_slice().iter().find(|operation| operation.node_id == 8).unwrap();
        assert_eq!(fill.command.color_rgb(), UiSceneTheme::DEFAULT.accent);
    }

    #[test]
    fn empty_input_value_does_not_create_an_invalid_glyph() {
        let mut blueprint = UiBlueprint::new();
        let root = blueprint.push_root(UiNodeKind::Root, 1).unwrap();
        let input = blueprint.push_child(UiNodeKind::TextInput, root, 2).unwrap();
        let mut tree = UiComponentTree::from_blueprint(&blueprint).unwrap();
        for index in 0..tree.tree().len() {
            let handle = tree.tree().handle_at(index).unwrap();
            tree.tree_mut().set_bounds(handle, UiRect::new(0, 0, 40, 20)).unwrap();
        }
        let input_handle = tree.tree().handle_at(usize::from(input)).unwrap();
        tree.tree_mut().set_focused(input_handle, true).unwrap();
        let surface = SurfaceHandle::new(1, 1, 7).unwrap();
        let scene = emit(surface, 1, &tree, UiSceneTheme::DEFAULT).unwrap();
        assert!(scene.as_slice().iter().all(|op| op.is_valid()));
    }

    #[test]
    fn long_labels_split_into_bounded_glyph_runs() {
        let mut tree = sample_tree();
        let label = tree.tree().handle_at(1).unwrap();
        tree.set_text(label, UiText::from_bytes(b"This account will own this system.").unwrap())
            .unwrap();
        set_bounds(&mut tree, 0, UiRect::new(0, 0, 400, 40));
        set_bounds(&mut tree, 1, UiRect::new(0, 0, 400, 40));
        set_bounds(&mut tree, 2, UiRect::new(0, 0, 400, 40));
        let surface = SurfaceHandle::new(1, 1, 7).unwrap();
        let scene = emit(surface, 1, &tree, UiSceneTheme::DEFAULT).unwrap();
        assert_eq!(scene.len(), 6);
        assert!(scene.as_slice().iter().all(|op| op.is_valid()));
        assert_eq!(scene.as_slice()[1].node_id, 1);
        assert_eq!(scene.as_slice()[2].node_id, 4);
        assert_eq!(scene.as_slice()[3].node_id, 0x8000_0001);
    }

    #[test]
    fn text_styles_emit_scaled_and_vertically_centered_glyphs() {
        let mut blueprint = UiBlueprint::new();
        let root = blueprint.push_root(UiNodeKind::Root, 1).unwrap();
        let button = blueprint.push_child(UiNodeKind::Button, root, 2).unwrap();
        blueprint.set_text(button, UiText::from_bytes(b"Open").unwrap()).unwrap();
        let mut styles = logos_ui::UiStyleList::EMPTY;
        assert!(styles.push(UiStyle::Text4xl));
        blueprint.set_styles(button, styles).unwrap();
        let mut tree = UiComponentTree::from_blueprint(&blueprint).unwrap();
        set_bounds(&mut tree, 0, UiRect::new(0, 0, 100, 80));
        set_bounds(&mut tree, 1, UiRect::new(8, 8, 80, 48));

        let surface = SurfaceHandle::new(1, 1, 7).unwrap();
        let scene = emit(surface, 1, &tree, UiSceneTheme::DEFAULT).unwrap();
        let text = scene
            .as_slice()
            .iter()
            .find(|operation| operation.command.kind == logos_abi::GuiDrawKind::GlyphRun)
            .unwrap();
        assert_eq!(text.command.auxiliary, logos_abi::GUI_TEXT_FLAG_DOUBLE);
        assert_eq!(text.command.x, 20);
        assert_eq!(text.command.y, 16);
    }

    #[test]
    fn buttons_emit_material_symbols_without_dropping_semantic_text() {
        let mut blueprint = UiBlueprint::new();
        let root = blueprint.push_root(UiNodeKind::Root, 1).unwrap();
        let button = blueprint.push_child(UiNodeKind::Button, root, 2).unwrap();
        blueprint.set_text(button, UiText::from_bytes(b"Settings").unwrap()).unwrap();
        blueprint.set_icon(button, UiIcon::Settings).unwrap();
        let mut tree = UiComponentTree::from_blueprint(&blueprint).unwrap();
        set_bounds(&mut tree, 0, UiRect::new(0, 0, 100, 80));
        set_bounds(&mut tree, 1, UiRect::new(8, 8, 40, 40));

        let surface = SurfaceHandle::new(1, 1, 7).unwrap();
        let scene = emit(surface, 1, &tree, UiSceneTheme::DEFAULT).unwrap();
        let icon = scene
            .as_slice()
            .iter()
            .find(|operation| operation.command.kind == logos_abi::GuiDrawKind::MaterialSymbol)
            .unwrap();
        assert_eq!(icon.command.auxiliary, logos_abi::GuiMaterialSymbol::Settings as u32);
        assert_eq!(icon.command.width, 24);
        assert_eq!(icon.command.height, 24);
    }

    #[test]
    fn emit_into_matches_allocating_wrapper() {
        let mut tree = sample_tree();
        for index in 0..tree.tree().len() {
            set_bounds(&mut tree, index, UiRect::new(0, index as i32 * 20, 100, 20));
        }
        let surface = SurfaceHandle::new(1, 1, TEST_OWNER).unwrap();
        let expected = emit(surface, 1, &tree, UiSceneTheme::DEFAULT).unwrap();
        let mut actual = UiSceneFrame::new();
        emit_into(surface, 1, &tree, UiSceneTheme::DEFAULT, &mut actual).unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn publisher_resumes_after_full_at_every_operation_index() {
        let mut tree = sample_tree();
        for index in 0..tree.tree().len() {
            set_bounds(&mut tree, index, UiRect::new(0, index as i32 * 20, 100, 20));
        }
        let surface = SurfaceHandle::new(1, 1, TEST_OWNER).unwrap();
        let scene = emit(surface, 1, &tree, UiSceneTheme::DEFAULT).unwrap();

        for fail_at in 0..scene.len() {
            let mut registry = GuiSurfaceRegistry::new();
            assert_eq!(registry_surface(&mut registry), surface);
            let mut publisher = UiScenePublisher::new();
            let mut sink = RegistrySink {
                registry: &mut registry,
                fail_at: Some(fail_at),
                calls: 0,
                failed: false,
                seen: Vec::new(),
            };
            assert_eq!(
                publisher
                    .publish(surface, 1, &tree, UiSceneTheme::DEFAULT, None, &mut sink)
                    .unwrap(),
                (IpcStatus::Full, fail_at)
            );
            assert!(publisher.is_pending());
            sink.fail_at = None;
            assert_eq!(
                publisher
                    .publish(surface, 1, &tree, UiSceneTheme::DEFAULT, None, &mut sink)
                    .unwrap(),
                (IpcStatus::Ok, scene.len())
            );
            assert_eq!(sink.registry.active_frame(surface), Some(1));
        }
    }

    #[test]
    fn publisher_rebinds_destroyed_surface_with_full_frame() {
        let mut registry = GuiSurfaceRegistry::new();
        let surface = registry_surface(&mut registry);
        let mut tree = sample_tree();
        for index in 0..tree.tree().len() {
            set_bounds(&mut tree, index, UiRect::new(0, index as i32 * 20, 100, 20));
        }
        let mut publisher = UiScenePublisher::new();
        let mut sink = ready_registry_sink(&mut registry);
        assert_eq!(
            publisher.publish(surface, 1, &tree, UiSceneTheme::DEFAULT, None, &mut sink).unwrap().0,
            IpcStatus::Ok
        );
        sink.registry.destroy(TEST_OWNER, surface).unwrap();
        let mut request = GuiSurfaceRequest::new(GuiSurfaceOperation::CreateModal, 3);
        request.bounds = TEST_BOUNDS;
        request.z_order = 1;
        let rebound = sink.registry.create(TEST_OWNER, request).unwrap().surface;
        assert_eq!(rebound.slot, surface.slot);
        assert_ne!(rebound.generation, surface.generation);
        sink.seen.clear();
        publisher.publish(rebound, 2, &tree, UiSceneTheme::DEFAULT, None, &mut sink).unwrap();
        assert_eq!(sink.seen[0].operation, GuiNodeOperation::Clear);
        assert_eq!(sink.registry.active_frame(rebound), Some(2));
    }

    #[test]
    fn publisher_coalesces_newer_tree_while_a_frame_is_pending() {
        let mut registry = GuiSurfaceRegistry::new();
        let surface = registry_surface(&mut registry);
        let mut tree = sample_tree();
        for index in 0..tree.tree().len() {
            set_bounds(&mut tree, index, UiRect::new(0, index as i32 * 20, 100, 20));
        }
        let mut publisher = UiScenePublisher::new();
        let mut sink = ready_registry_sink(&mut registry);
        publisher.publish(surface, 1, &tree, UiSceneTheme::DEFAULT, None, &mut sink).unwrap();

        let label = tree.tree().handle_at(1).unwrap();
        tree.set_text(label, UiText::from_bytes(b"Intermediate").unwrap()).unwrap();
        sink.calls = 0;
        sink.fail_at = Some(1);
        assert_eq!(
            publisher.publish(surface, 2, &tree, UiSceneTheme::DEFAULT, None, &mut sink).unwrap().0,
            IpcStatus::Full
        );
        tree.set_text(label, UiText::from_bytes(b"Latest").unwrap()).unwrap();
        sink.seen.clear();
        sink.calls = 0;
        sink.fail_at = None;
        assert_eq!(
            publisher.publish(surface, 3, &tree, UiSceneTheme::DEFAULT, None, &mut sink).unwrap().0,
            IpcStatus::Ok
        );
        let frame_two_end = sink
            .seen
            .iter()
            .position(|operation| {
                operation.operation == GuiNodeOperation::Commit && operation.frame == 2
            })
            .unwrap();
        let frame_three_start =
            sink.seen.iter().position(|operation| operation.frame == 3).unwrap();
        assert!(frame_two_end < frame_three_start);
        assert!(sink.seen.iter().any(|operation| operation.frame == 2));
        assert!(sink.seen.iter().any(|operation| operation.frame == 3));
        assert_ne!(sink.seen[0].operation, GuiNodeOperation::Clear);
        assert_eq!(sink.registry.active_frame(surface), Some(3));
    }

    #[test]
    fn same_frame_resume_with_changed_tree_keeps_display_in_sync() {
        let mut tree = sample_tree();
        for index in 0..tree.tree().len() {
            set_bounds(&mut tree, index, UiRect::new(0, index as i32 * 20, 100, 20));
        }
        let mut actual = std::boxed::Box::new(Display::new(1));
        let surface = display_surface(&mut actual);
        let mut publisher = UiScenePublisher::new();
        {
            let mut sink = RegistrySink {
                registry: actual.gui_mut(),
                fail_at: None,
                calls: 0,
                failed: false,
                seen: Vec::new(),
            };
            publisher.publish(surface, 1, &tree, UiSceneTheme::DEFAULT, None, &mut sink).unwrap();
            let label = tree.tree().handle_at(1).unwrap();
            tree.set_text(label, UiText::from_bytes(b"BBBB").unwrap()).unwrap();
            sink.calls = 0;
            sink.fail_at = Some(0);
            assert_eq!(
                publisher
                    .publish(surface, 2, &tree, UiSceneTheme::DEFAULT, None, &mut sink)
                    .unwrap(),
                (IpcStatus::Full, 0)
            );
            tree.set_text(label, UiText::from_bytes(b"CCCC").unwrap()).unwrap();
            sink.fail_at = None;
            sink.calls = 0;
            publisher.publish(surface, 2, &tree, UiSceneTheme::DEFAULT, None, &mut sink).unwrap();
            let result = publisher
                .publish(surface, 3, &tree, UiSceneTheme::DEFAULT, None, &mut sink)
                .unwrap();
            assert_eq!(result.0, IpcStatus::Ok);
            assert!(result.1 > 0);
        }

        let mut expected = std::boxed::Box::new(Display::new(1));
        let expected_surface = display_surface(&mut expected);
        apply_display_scene(
            &mut expected,
            &emit(expected_surface, 3, &tree, UiSceneTheme::DEFAULT).unwrap(),
        );
        assert_eq!(pixels(&mut expected), pixels(&mut actual));
    }

    #[test]
    fn delta_keeps_last_full_frame_node_in_baseline() {
        let mut tree = sample_tree();
        set_bounds(&mut tree, 0, UiRect::new(0, 0, 200, 80));
        set_bounds(&mut tree, 1, UiRect::new(8, 8, 120, 16));
        set_bounds(&mut tree, 2, UiRect::new(8, 32, 120, 24));
        let mut actual = std::boxed::Box::new(Display::new(1));
        let mut expected = std::boxed::Box::new(Display::new(1));
        let surface = display_surface(&mut actual);
        assert_eq!(display_surface(&mut expected), surface);
        let mut publisher = UiScenePublisher::new();

        for (frame, mutation) in [
            (1, 0), // Initial full scene; node 9 is the final Upsert.
            (2, 1), // Change the label, leaving node 9 unchanged.
            (3, 2), // Remove the button's text node 9.
            (4, 3), // Re-add node 9 with new text.
        ] {
            let button = tree.tree().handle_at(2).unwrap();
            if mutation == 1 {
                let label = tree.tree().handle_at(1).unwrap();
                tree.set_text(label, UiText::from_bytes(b"Changed").unwrap()).unwrap();
            } else if mutation == 2 {
                tree.set_text(button, UiText::EMPTY).unwrap();
            } else if mutation == 3 {
                tree.set_text(button, UiText::from_bytes(b"Again").unwrap()).unwrap();
            }

            let full = emit(surface, frame, &tree, UiSceneTheme::DEFAULT).unwrap();
            apply_display_scene(&mut expected, &full);
            let mut sink = RegistrySink {
                registry: actual.gui_mut(),
                fail_at: None,
                calls: 0,
                failed: false,
                seen: Vec::new(),
            };
            publisher
                .publish(surface, frame, &tree, UiSceneTheme::DEFAULT, None, &mut sink)
                .unwrap();
            if mutation == 2 {
                assert!(sink.seen.iter().any(|operation| {
                    operation.operation == GuiNodeOperation::Remove && operation.node_id == 9
                }));
            } else if mutation == 3 {
                assert!(sink.seen.iter().any(|operation| {
                    operation.operation == GuiNodeOperation::Upsert && operation.node_id == 9
                }));
            }
            drop(sink);
            assert_eq!(pixels(&mut expected), pixels(&mut actual));
        }
    }

    #[test]
    fn publisher_diffs_swapped_node_ids_by_id_and_command() {
        let surface = SurfaceHandle::new(1, 1, TEST_OWNER).unwrap();
        let mut previous = UiSceneFrame::new();
        push(&mut previous, clear_op(surface, 1)).unwrap();
        push_upsert_id(
            &mut previous,
            surface,
            1,
            10,
            GuiDrawCommand::fill_rect(GuiRect::new(0, 0, 10, 10), 0xff0000),
        )
        .unwrap();
        push_upsert_id(
            &mut previous,
            surface,
            1,
            20,
            GuiDrawCommand::fill_rect(GuiRect::new(10, 0, 10, 10), 0x00ff00),
        )
        .unwrap();
        push(&mut previous, GuiSceneOp::commit(surface, 1)).unwrap();
        let mut current = UiSceneFrame::new();
        push(&mut current, clear_op(surface, 2)).unwrap();
        push_upsert_id(
            &mut current,
            surface,
            2,
            10,
            GuiDrawCommand::fill_rect(GuiRect::new(10, 0, 10, 10), 0x00ff00),
        )
        .unwrap();
        push_upsert_id(
            &mut current,
            surface,
            2,
            20,
            GuiDrawCommand::fill_rect(GuiRect::new(0, 0, 10, 10), 0xff0000),
        )
        .unwrap();
        push(&mut current, GuiSceneOp::commit(surface, 2)).unwrap();

        let delta = current.diff_from(&previous).unwrap();
        assert_eq!(delta.len(), 3);
        assert_eq!(delta.as_slice()[0].node_id, 10);
        assert_eq!(delta.as_slice()[1].node_id, 20);
        assert!(
            delta.as_slice()[..2]
                .iter()
                .all(|operation| operation.operation == GuiNodeOperation::Upsert)
        );
    }

    #[test]
    fn scene_delta_places_removes_before_upserts() {
        let surface = SurfaceHandle::new(1, 1, TEST_OWNER).unwrap();
        let mut previous = UiSceneFrame::new();
        push(&mut previous, clear_op(surface, 1)).unwrap();
        for node_id in [10, 20] {
            push_upsert_id(
                &mut previous,
                surface,
                1,
                node_id,
                GuiDrawCommand::fill_rect(GuiRect::new(node_id as i32, 0, 10, 10), 0xff0000),
            )
            .unwrap();
        }
        push(&mut previous, GuiSceneOp::commit(surface, 1)).unwrap();

        let mut current = UiSceneFrame::new();
        push(&mut current, clear_op(surface, 2)).unwrap();
        push_upsert_id(
            &mut current,
            surface,
            2,
            20,
            GuiDrawCommand::fill_rect(GuiRect::new(20, 0, 10, 10), 0x00ff00),
        )
        .unwrap();
        push(&mut current, GuiSceneOp::commit(surface, 2)).unwrap();

        let delta = current.diff_from(&previous).unwrap();
        assert_eq!(delta.as_slice()[0].operation, GuiNodeOperation::Remove);
        assert_eq!(delta.as_slice()[0].node_id, 10);
        assert_eq!(delta.as_slice()[1].operation, GuiNodeOperation::Upsert);
        assert_eq!(delta.as_slice()[2].operation, GuiNodeOperation::Commit);
    }

    #[test]
    fn oversized_delta_falls_back_to_full_frame() {
        let surface = SurfaceHandle::new(1, 1, TEST_OWNER).unwrap();
        let mut previous = UiSceneFrame::new();
        push(&mut previous, clear_op(surface, 1)).unwrap();
        for node_id in 1..=MAX_GUI_NODES as u32 {
            push_upsert_id(
                &mut previous,
                surface,
                1,
                node_id,
                GuiDrawCommand::fill_rect(GuiRect::new(node_id as i32, 0, 1, 1), 0xff0000),
            )
            .unwrap();
        }
        push(&mut previous, GuiSceneOp::commit(surface, 1)).unwrap();
        let mut current = UiSceneFrame::new();
        push(&mut current, clear_op(surface, 2)).unwrap();
        for node_id in 101..101 + MAX_GUI_NODES as u32 {
            push_upsert_id(
                &mut current,
                surface,
                2,
                node_id,
                GuiDrawCommand::fill_rect(GuiRect::new(node_id as i32, 0, 1, 1), 0x00ff00),
            )
            .unwrap();
        }
        push(&mut current, GuiSceneOp::commit(surface, 2)).unwrap();

        let fallback = current.diff_from(&previous).unwrap();
        assert_eq!(fallback, current);
        assert_eq!(fallback.len(), MAX_GUI_NODES + 2);
    }

    #[test]
    fn non_cursor_scene_keeps_incremental_delta() {
        let mut registry = GuiSurfaceRegistry::new();
        let surface = registry_surface(&mut registry);
        let mut tree = sample_tree();
        for index in 0..tree.tree().len() {
            set_bounds(&mut tree, index, UiRect::new(0, index as i32 * 20, 100, 20));
        }
        let mut publisher = UiScenePublisher::new();
        let mut sink = ready_registry_sink(&mut registry);
        publisher.publish(surface, 1, &tree, UiSceneTheme::DEFAULT, None, &mut sink).unwrap();
        let label = tree.tree().handle_at(1).unwrap();
        tree.set_text(label, UiText::from_bytes(b"Changed").unwrap()).unwrap();
        sink.seen.clear();
        publisher.publish(surface, 2, &tree, UiSceneTheme::DEFAULT, None, &mut sink).unwrap();
        assert!(!sink.seen.iter().any(|operation| operation.operation == GuiNodeOperation::Clear));
    }

    #[test]
    fn changed_cursor_signature_forces_a_full_frame() {
        let mut registry = GuiSurfaceRegistry::new();
        let surface = registry_surface(&mut registry);
        let mut tree = sample_tree();
        for index in 0..tree.tree().len() {
            set_bounds(&mut tree, index, UiRect::new(0, index as i32 * 20, 100, 20));
        }
        let mut publisher = UiScenePublisher::new();
        let mut sink = ready_registry_sink(&mut registry);
        publisher.publish(surface, 1, &tree, UiSceneTheme::DEFAULT, Some(1), &mut sink).unwrap();
        sink.seen.clear();
        publisher.publish(surface, 2, &tree, UiSceneTheme::DEFAULT, Some(2), &mut sink).unwrap();
        assert_eq!(sink.seen[0].operation, GuiNodeOperation::Clear);
    }

    #[test]
    fn non_cursor_publisher_rejects_display_cursor_shape() {
        let mut blueprint = UiBlueprint::new();
        blueprint.push_root(UiNodeKind::Root, 1).unwrap();
        let mut tree = UiComponentTree::from_blueprint(&blueprint).unwrap();
        set_bounds(&mut tree, 0, UiRect::new(0, 0, 3, 14));
        let surface = SurfaceHandle::new(1, 1, TEST_OWNER).unwrap();
        let scene = emit(surface, 1, &tree, UiSceneTheme::DEFAULT).unwrap();
        assert!(scene.as_slice().iter().any(|operation| {
            operation.operation == GuiNodeOperation::Upsert
                && operation.node_id == 1
                && operation.command.kind == logos_abi::GuiDrawKind::FillRect
                && operation.command.width == 3
                && operation.command.height == 14
        }));

        let mut registry = GuiSurfaceRegistry::new();
        registry_surface(&mut registry);
        let mut sink = ready_registry_sink(&mut registry);
        let mut publisher = UiScenePublisher::new();
        assert_eq!(
            publisher.publish(surface, 1, &tree, UiSceneTheme::DEFAULT, None, &mut sink),
            Err(UiSceneError::InvalidCommand)
        );
        assert!(sink.seen.is_empty());
        assert!(!publisher.is_pending());
    }

    fn app_username_handle(tree: &UiComponentTree, build: &UiBuild) -> logos_ui::UiNodeHandle {
        let index = build.document.node_index_by_name(b"username").unwrap();
        tree.tree().handle_at(usize::from(index)).unwrap()
    }

    fn set_app_username(tree: &mut UiComponentTree, build: &UiBuild) {
        let handle = app_username_handle(tree, build);
        tree.set_value(handle, UiText::from_bytes(b"alice").unwrap()).unwrap();
    }

    fn set_app_failure_title(tree: &mut UiComponentTree, build: &UiBuild) {
        let index = build.document.node_index_by_name(b"title").unwrap();
        let handle = tree.tree().handle_at(usize::from(index)).unwrap();
        tree.set_text(handle, UiText::from_bytes(b"Sign-in failed").unwrap()).unwrap();
    }

    fn assert_initial_app_publication_at_every_index(build: &UiBuild) {
        let surface = SurfaceHandle::new(1, 1, TEST_OWNER).unwrap();
        let tree = app_tree(build);
        let full = emit(surface, 1, &tree, UiSceneTheme::DEFAULT).unwrap();
        for fail_at in 0..full.len() {
            let mut expected = std::boxed::Box::new(Display::new(1));
            let mut actual = std::boxed::Box::new(Display::new(1));
            let expected_surface = display_surface(&mut expected);
            let actual_surface = display_surface(&mut actual);
            assert_eq!(expected_surface, surface);
            assert_eq!(actual_surface, surface);
            apply_display_scene(&mut expected, &full);
            let mut publisher = UiScenePublisher::new();
            let mut sink = RegistrySink {
                registry: actual.gui_mut(),
                fail_at: Some(fail_at),
                calls: 0,
                failed: false,
                seen: Vec::new(),
            };
            assert_eq!(
                publisher
                    .publish(surface, 1, &tree, UiSceneTheme::DEFAULT, None, &mut sink)
                    .unwrap(),
                (IpcStatus::Full, fail_at)
            );
            sink.fail_at = None;
            sink.calls = 0;
            assert_eq!(
                publisher
                    .publish(surface, 1, &tree, UiSceneTheme::DEFAULT, None, &mut sink)
                    .unwrap(),
                (IpcStatus::Ok, full.len())
            );
            drop(sink);
            assert_eq!(pixels(&mut expected), pixels(&mut actual));
        }
    }

    fn assert_app_transition_at_every_index(build: &UiBuild, frame: u32) {
        let surface = SurfaceHandle::new(1, 1, TEST_OWNER).unwrap();
        let mut measured_tree = app_tree(build);
        let mut previous = emit(surface, 1, &measured_tree, UiSceneTheme::DEFAULT).unwrap();
        if frame == 3 {
            set_app_username(&mut measured_tree, build);
            previous = emit(surface, 2, &measured_tree, UiSceneTheme::DEFAULT).unwrap();
            set_app_failure_title(&mut measured_tree, build);
        } else {
            set_app_username(&mut measured_tree, build);
        }
        let current = emit(surface, frame, &measured_tree, UiSceneTheme::DEFAULT).unwrap();
        let delta = current.diff_from(&previous).unwrap();
        assert!(delta.len() > 1);

        if frame == 2 {
            let empty = emit(surface, 1, &app_tree(build), UiSceneTheme::DEFAULT).unwrap();
            assert!(current.as_slice().iter().any(|operation| {
                operation.operation == GuiNodeOperation::Upsert
                    && operation.command.kind == logos_abi::GuiDrawKind::GlyphRun
                    && !has_node(&empty, operation.node_id)
            }));
        }

        for fail_at in 0..delta.len() {
            let mut expected = std::boxed::Box::new(Display::new(1));
            let mut actual = std::boxed::Box::new(Display::new(1));
            let expected_surface = display_surface(&mut expected);
            let actual_surface = display_surface(&mut actual);
            let mut tree = app_tree(build);
            let mut publisher = UiScenePublisher::new();
            let initial = emit(surface, 1, &tree, UiSceneTheme::DEFAULT).unwrap();
            apply_display_scene(&mut expected, &initial);
            {
                let mut sink = RegistrySink {
                    registry: actual.gui_mut(),
                    fail_at: None,
                    calls: 0,
                    failed: false,
                    seen: Vec::new(),
                };
                publisher
                    .publish(actual_surface, 1, &tree, UiSceneTheme::DEFAULT, None, &mut sink)
                    .unwrap();

                if frame == 3 {
                    set_app_username(&mut tree, build);
                    let username = emit(surface, 2, &tree, UiSceneTheme::DEFAULT).unwrap();
                    apply_display_scene(&mut expected, &username);
                    publisher
                        .publish(actual_surface, 2, &tree, UiSceneTheme::DEFAULT, None, &mut sink)
                        .unwrap();
                    set_app_failure_title(&mut tree, build);
                } else {
                    set_app_username(&mut tree, build);
                }

                sink.fail_at = Some(fail_at);
                sink.calls = 0;
                assert_eq!(
                    publisher
                        .publish(
                            actual_surface,
                            frame,
                            &tree,
                            UiSceneTheme::DEFAULT,
                            None,
                            &mut sink
                        )
                        .unwrap(),
                    (IpcStatus::Full, fail_at)
                );
                apply_display_scene(&mut expected, &current);
                sink.fail_at = None;
                sink.calls = 0;
                assert_eq!(
                    publisher
                        .publish(
                            actual_surface,
                            frame,
                            &tree,
                            UiSceneTheme::DEFAULT,
                            None,
                            &mut sink
                        )
                        .unwrap()
                        .0,
                    IpcStatus::Ok
                );
            }
            assert_eq!(expected_surface, surface);
            assert_eq!(pixels(&mut expected), pixels(&mut actual));
        }
    }

    #[test]
    fn login_and_claim_publication_matches_pixels_after_full_at_every_op_index() {
        for build in
            [logos_ui_compiler::compile_login_page(), logos_ui_compiler::compile_register_page()]
        {
            assert_initial_app_publication_at_every_index(&build);
            assert_app_transition_at_every_index(&build, 2);
            assert_app_transition_at_every_index(&build, 3);
        }
    }
}
