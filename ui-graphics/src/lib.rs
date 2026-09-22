#![no_std]

use logos_abi::{
    GUI_DRAW_FLAG_MORE, GuiDrawCommand, GuiRect, GuiSceneOp, GuiTransform, MAX_GUI_NODES,
    SurfaceHandle,
};
use logos_ui::{UiComponentTree, UiIcon, UiNode, UiNodeKind, UiRect, UiStyle};

pub const MAX_UI_SCENE_OPS: usize = MAX_GUI_NODES + 2;
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
        let mut delta = Self::new();
        for operation in self
            .as_slice()
            .iter()
            .copied()
            .filter(|operation| operation.operation == logos_abi::GuiNodeOperation::Upsert)
        {
            let unchanged = previous.as_slice().iter().any(|old| {
                old.operation == logos_abi::GuiNodeOperation::Upsert
                    && old.node_id == operation.node_id
                    && old.command == operation.command
            });
            if !unchanged {
                let mut operation = operation;
                operation.flags = GUI_DRAW_FLAG_MORE;
                push(&mut delta, operation)?;
            }
        }
        for old in previous
            .as_slice()
            .iter()
            .copied()
            .filter(|operation| operation.operation == logos_abi::GuiNodeOperation::Upsert)
        {
            let retained = self.as_slice().iter().any(|operation| {
                operation.operation == logos_abi::GuiNodeOperation::Upsert
                    && operation.node_id == old.node_id
            });
            if !retained {
                let mut remove = GuiSceneOp::remove(current.surface, current.frame, old.node_id);
                remove.flags = GUI_DRAW_FLAG_MORE;
                push(&mut delta, remove)?;
            }
        }
        if delta.is_empty() {
            return Ok(delta);
        }
        push(&mut delta, GuiSceneOp::commit(current.surface, current.frame))?;
        delta.ops[delta.len() - 1].flags = 0;
        Ok(delta)
    }
}

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
    if !surface.is_valid() {
        return Err(UiSceneError::InvalidSurface);
    }
    if frame == 0 {
        return Err(UiSceneError::InvalidFrame);
    }

    let mut output = UiSceneFrame::new();
    push(&mut output, clear_op(surface, frame))?;

    for index in 0..logos_ui::MAX_UI_NODES {
        let Ok(handle) = tree.tree().handle_at(index) else { continue };
        let node = tree.tree().node(handle).map_err(|_| UiSceneError::Capacity)?;
        let bounds = visible_bounds(node);
        if bounds.is_empty() {
            continue;
        }
        emit_node(&mut output, surface, frame, index, node, tree, bounds, theme)?;
    }

    if output.len() == 1 {
        push(&mut output, GuiSceneOp::commit(surface, frame))?;
    } else {
        output.ops[output.len() - 1].flags = 0;
    }
    Ok(output)
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
    use logos_ui::{UiBlueprint, UiIcon, UiNodeKind, UiStyle, UiStyleList, UiText};

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
}
