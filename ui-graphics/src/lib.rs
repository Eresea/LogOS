#![no_std]

use logos_abi::{GUI_DRAW_FLAG_MORE, GuiDrawCommand, GuiSceneOp, MAX_GUI_NODES, SurfaceHandle};
use logos_ui::{UiComponentTree, UiNode, UiNodeKind, UiRect, UiStyle};

pub const MAX_UI_SCENE_OPS: usize = MAX_GUI_NODES + 2;
const GUI_GLYPH_WIDTH: usize = 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiSceneTheme {
    pub surface: u32,
    pub panel: u32,
    pub input: u32,
    pub accent: u32,
    pub focus: u32,
    pub text: u32,
}

impl UiSceneTheme {
    pub const DEFAULT: Self = Self {
        surface: 0x101820,
        panel: 0x182535,
        input: 0x263548,
        accent: 0x356bd8,
        focus: 0x4b82f2,
        text: 0xffffff,
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
            push_upsert(
                output,
                surface,
                frame,
                index,
                0,
                GuiDrawCommand::fill_rect(to_gui_rect(bounds), color(theme.surface, node)),
            )?;
        }
        UiNodeKind::Panel | UiNodeKind::Form => {
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
            push_text(output, surface, frame, index, node, node.text.as_bytes(), theme.text, 0)?;
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
            push_text(output, surface, frame, index, node, node.text.as_bytes(), theme.text, 2)?;
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
            push_text(output, surface, frame, index, node, value.as_bytes(), theme.text, 2)?;
        }
    }
    Ok(())
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
        let x_offset = offset.saturating_mul(GUI_GLYPH_WIDTH) as i32;
        let node_id = if chunk == 0 {
            (index as u32).saturating_mul(3).saturating_add(fragment + 1)
        } else {
            0x8000_0000 | index as u32
        };
        let Some(command) = GuiDrawCommand::glyph_run(
            node.bounds.x.saturating_add(4).saturating_add(x_offset),
            node.bounds.y.saturating_add(4),
            color(text_color, node),
            &text[offset..end],
        ) else {
            return Err(UiSceneError::Capacity);
        };
        push_upsert_id(output, surface, frame, node_id, command)?;
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
        GuiDrawCommand::shadow(to_gui_rect(bounds), 0x55000000, radius, 3, 0, 3),
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

fn fill_command(bounds: UiRect, color: u32, node: &UiNode) -> GuiDrawCommand {
    let rect = to_gui_rect(bounds);
    let radius = corner_radius(bounds, node);
    if radius != 0 {
        return GuiDrawCommand::fill_rounded_rect(rect, color, radius);
    }
    GuiDrawCommand::fill_rect(rect, color)
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
    node.styles.contains(UiStyle::RoundedLarge) || node.styles.contains(UiStyle::RoundedFull)
}

fn panel_color(node: &UiNode, theme: UiSceneTheme) -> u32 {
    if node.styles.contains(UiStyle::BackgroundAccent) { theme.accent } else { theme.panel }
}

fn control_color(node: &UiNode, theme: UiSceneTheme) -> u32 {
    if node.interaction.is_focused() {
        theme.focus
    } else if node.styles.contains(UiStyle::BackgroundAccent) {
        theme.accent
    } else {
        theme.input
    }
}

fn color(value: u32, node: &UiNode) -> u32 {
    if node.styles.contains(UiStyle::Opacity50) {
        (value & 0x00ff_ffff) | 0x8000_0000
    } else {
        value
    }
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
    use logos_ui::{UiBlueprint, UiNodeKind, UiText};

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
    fn rejects_more_visual_commands_than_display_can_retain() {
        let mut blueprint = UiBlueprint::new();
        let root = blueprint.push_root(UiNodeKind::Root, 1).unwrap();
        let text = UiText::from_bytes(b"x").unwrap();
        for index in 0..8 {
            let button = blueprint.push_child(UiNodeKind::Button, root, index + 2).unwrap();
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
}
