use crate::runtime::{MAX_UI_NODES, UiError, UiNodeHandle, UiRect, UiTree};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiSize {
    pub width: u32,
    pub height: u32,
}

impl UiSize {
    pub const ZERO: Self = Self { width: 0, height: 0 };

    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiEdges {
    pub left: u32,
    pub right: u32,
    pub top: u32,
    pub bottom: u32,
}

impl UiEdges {
    pub const ZERO: Self = Self { left: 0, right: 0, top: 0, bottom: 0 };

    pub const fn all(value: u32) -> Self {
        Self { left: value, right: value, top: value, bottom: value }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UiLayoutDirection {
    Row,
    Column,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UiLayoutAlignment {
    Start,
    Center,
    End,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UiOverflow {
    Visible,
    Clip,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiLayoutStyle {
    pub direction: UiLayoutDirection,
    pub align_items: UiLayoutAlignment,
    pub justify_content: UiLayoutAlignment,
    pub gap: u32,
    pub gap_x: u32,
    pub gap_y: u32,
    pub padding: UiEdges,
    pub overflow: UiOverflow,
}

impl UiLayoutStyle {
    pub const EMPTY: Self = Self {
        direction: UiLayoutDirection::Column,
        align_items: UiLayoutAlignment::Start,
        justify_content: UiLayoutAlignment::Start,
        gap: 0,
        gap_x: 0,
        gap_y: 0,
        padding: UiEdges::ZERO,
        overflow: UiOverflow::Visible,
    };

    pub const fn row() -> Self {
        Self { direction: UiLayoutDirection::Row, ..Self::EMPTY }
    }

    pub const fn column() -> Self {
        Self::EMPTY
    }

    pub const fn axis_gap(self) -> u32 {
        match self.direction {
            UiLayoutDirection::Row => {
                if self.gap_x == 0 {
                    self.gap
                } else {
                    self.gap_x
                }
            }
            UiLayoutDirection::Column => {
                if self.gap_y == 0 {
                    self.gap
                } else {
                    self.gap_y
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UiLayoutError {
    Stale,
    StackOverflow,
}

#[derive(Clone, Copy)]
struct LayoutTask {
    handle: UiNodeHandle,
    bounds: UiRect,
    clip: UiRect,
}

impl LayoutTask {
    const EMPTY: Self =
        Self { handle: UiNodeHandle::EMPTY, bounds: UiRect::EMPTY, clip: UiRect::EMPTY };
}

/// Stateless, bounded measure/arrange implementation for a `UiTree`.
pub struct UiLayoutEngine;

impl UiLayoutEngine {
    pub const fn new() -> Self {
        Self
    }

    pub fn arrange(
        &self,
        tree: &mut UiTree,
        root: UiNodeHandle,
        viewport: UiRect,
    ) -> Result<usize, UiLayoutError> {
        if tree.node(root).is_err() {
            return Err(UiLayoutError::Stale);
        }

        let mut stack = [LayoutTask::EMPTY; MAX_UI_NODES];
        let mut stack_len = 1;
        stack[0] = LayoutTask { handle: root, bounds: viewport, clip: viewport };
        let mut arranged = 0;

        while stack_len != 0 {
            stack_len -= 1;
            let task = stack[stack_len];
            let style = tree.node(task.handle).map_err(|_| UiLayoutError::Stale)?.layout;
            let own_clip = intersect(task.clip, task.bounds);
            tree.set_bounds(task.handle, task.bounds).map_err(map_tree_error)?;
            tree.set_clip(task.handle, own_clip).map_err(map_tree_error)?;
            arranged += 1;

            let content = inset(task.bounds, style.padding);
            let child_clip = match style.overflow {
                UiOverflow::Visible => own_clip,
                UiOverflow::Clip => intersect(own_clip, content),
            };
            let mut children = [UiNodeHandle::EMPTY; MAX_UI_NODES];
            let child_count = tree.children(task.handle, &mut children).map_err(map_tree_error)?;
            if child_count == 0 {
                continue;
            }

            let gap = style.axis_gap();
            let mut total_main = gap.saturating_mul(child_count.saturating_sub(1) as u32);
            for child in children[..child_count].iter().copied() {
                let size = tree.node(child).map_err(|_| UiLayoutError::Stale)?.intrinsic_size;
                total_main = total_main.saturating_add(main_size(size, style.direction));
            }
            let available_main = main_size_of_rect(content, style.direction);
            let free_main = available_main.saturating_sub(total_main);
            let mut main_offset = alignment_offset(style.justify_content, free_main);

            for child in children[..child_count].iter().copied() {
                let size = tree.node(child).map_err(|_| UiLayoutError::Stale)?.intrinsic_size;
                let child_main = main_size(size, style.direction);
                let child_cross = cross_size(size, style.direction);
                let available_cross = cross_size_of_rect(content, style.direction);
                let cross_offset = alignment_offset(
                    style.align_items,
                    available_cross.saturating_sub(child_cross),
                );
                let bounds = child_bounds(
                    content,
                    style.direction,
                    main_offset,
                    cross_offset,
                    child_main,
                    child_cross,
                );
                if stack_len == MAX_UI_NODES {
                    return Err(UiLayoutError::StackOverflow);
                }
                stack[stack_len] = LayoutTask { handle: child, bounds, clip: child_clip };
                stack_len += 1;
                main_offset = main_offset.saturating_add(child_main).saturating_add(gap);
            }
        }

        Ok(arranged)
    }
}

impl Default for UiLayoutEngine {
    fn default() -> Self {
        Self::new()
    }
}

fn map_tree_error(error: UiError) -> UiLayoutError {
    match error {
        UiError::Stale | UiError::NotFound => UiLayoutError::Stale,
        UiError::Capacity | UiError::InvalidParent | UiError::RootExists => {
            UiLayoutError::StackOverflow
        }
    }
}

fn main_size(size: UiSize, direction: UiLayoutDirection) -> u32 {
    match direction {
        UiLayoutDirection::Row => size.width,
        UiLayoutDirection::Column => size.height,
    }
}

fn cross_size(size: UiSize, direction: UiLayoutDirection) -> u32 {
    match direction {
        UiLayoutDirection::Row => size.height,
        UiLayoutDirection::Column => size.width,
    }
}

fn main_size_of_rect(rect: UiRect, direction: UiLayoutDirection) -> u32 {
    match direction {
        UiLayoutDirection::Row => rect.width,
        UiLayoutDirection::Column => rect.height,
    }
}

fn cross_size_of_rect(rect: UiRect, direction: UiLayoutDirection) -> u32 {
    match direction {
        UiLayoutDirection::Row => rect.height,
        UiLayoutDirection::Column => rect.width,
    }
}

fn alignment_offset(alignment: UiLayoutAlignment, free: u32) -> u32 {
    match alignment {
        UiLayoutAlignment::Start => 0,
        UiLayoutAlignment::Center => free / 2,
        UiLayoutAlignment::End => free,
    }
}

fn child_bounds(
    content: UiRect,
    direction: UiLayoutDirection,
    main_offset: u32,
    cross_offset: u32,
    child_main: u32,
    child_cross: u32,
) -> UiRect {
    match direction {
        UiLayoutDirection::Row => UiRect::new(
            offset(content.x, main_offset),
            offset(content.y, cross_offset),
            child_main,
            child_cross,
        ),
        UiLayoutDirection::Column => UiRect::new(
            offset(content.x, cross_offset),
            offset(content.y, main_offset),
            child_cross,
            child_main,
        ),
    }
}

fn offset(origin: i32, amount: u32) -> i32 {
    origin.saturating_add(amount.min(i32::MAX as u32) as i32)
}

fn inset(rect: UiRect, edges: UiEdges) -> UiRect {
    let left = edges.left.min(rect.width);
    let top = edges.top.min(rect.height);
    let right = edges.right.min(rect.width.saturating_sub(left));
    let bottom = edges.bottom.min(rect.height.saturating_sub(top));
    UiRect::new(
        offset(rect.x, left),
        offset(rect.y, top),
        rect.width.saturating_sub(left).saturating_sub(right),
        rect.height.saturating_sub(top).saturating_sub(bottom),
    )
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
    use crate::runtime::UiNodeKind;

    #[test]
    fn row_layout_applies_padding_gap_and_alignment() {
        let mut tree = UiTree::new();
        let root = tree.insert(UiNodeKind::Root, UiNodeHandle::EMPTY, 1).unwrap();
        let first = tree.insert(UiNodeKind::Panel, root, 2).unwrap();
        let second = tree.insert(UiNodeKind::Panel, root, 3).unwrap();
        let mut style = UiLayoutStyle::row();
        style.padding = UiEdges::all(4);
        style.gap_x = 2;
        style.justify_content = UiLayoutAlignment::Center;
        style.align_items = UiLayoutAlignment::Center;
        tree.set_layout_style(root, style).unwrap();
        tree.set_intrinsic_size(first, UiSize::new(10, 10)).unwrap();
        tree.set_intrinsic_size(second, UiSize::new(20, 12)).unwrap();

        UiLayoutEngine::new().arrange(&mut tree, root, UiRect::new(0, 0, 100, 40)).unwrap();

        assert_eq!(tree.node(first).unwrap().bounds, UiRect::new(34, 15, 10, 10));
        assert_eq!(tree.node(second).unwrap().bounds, UiRect::new(46, 14, 20, 12));
    }

    #[test]
    fn column_layout_uses_axis_specific_gap_and_end_alignment() {
        let mut tree = UiTree::new();
        let root = tree.insert(UiNodeKind::Root, UiNodeHandle::EMPTY, 1).unwrap();
        let first = tree.insert(UiNodeKind::Panel, root, 2).unwrap();
        let second = tree.insert(UiNodeKind::Panel, root, 3).unwrap();
        let mut style = UiLayoutStyle::column();
        style.gap = 9;
        style.gap_y = 3;
        style.align_items = UiLayoutAlignment::End;
        tree.set_layout_style(root, style).unwrap();
        tree.set_intrinsic_size(first, UiSize::new(10, 5)).unwrap();
        tree.set_intrinsic_size(second, UiSize::new(20, 7)).unwrap();

        UiLayoutEngine::new().arrange(&mut tree, root, UiRect::new(10, 20, 50, 40)).unwrap();

        assert_eq!(tree.node(first).unwrap().bounds, UiRect::new(50, 20, 10, 5));
        assert_eq!(tree.node(second).unwrap().bounds, UiRect::new(40, 28, 20, 7));
    }

    #[test]
    fn clipped_parents_limit_child_render_regions() {
        let mut tree = UiTree::new();
        let root = tree.insert(UiNodeKind::Root, UiNodeHandle::EMPTY, 1).unwrap();
        let child = tree.insert(UiNodeKind::Panel, root, 2).unwrap();
        let mut style = UiLayoutStyle::column();
        style.padding = UiEdges::all(2);
        style.overflow = UiOverflow::Clip;
        tree.set_layout_style(root, style).unwrap();
        tree.set_intrinsic_size(child, UiSize::new(40, 40)).unwrap();

        UiLayoutEngine::new().arrange(&mut tree, root, UiRect::new(0, 0, 20, 20)).unwrap();

        assert_eq!(tree.node(child).unwrap().bounds, UiRect::new(2, 2, 40, 40));
        assert_eq!(tree.node(child).unwrap().clip, UiRect::new(2, 2, 16, 16));
    }

    #[test]
    fn stale_roots_are_rejected_without_mutating_the_tree() {
        let mut tree = UiTree::new();
        let stale = UiNodeHandle { slot: 0, generation: 1 };
        assert_eq!(
            UiLayoutEngine::new().arrange(&mut tree, stale, UiRect::new(0, 0, 1, 1)),
            Err(UiLayoutError::Stale)
        );
        assert!(tree.is_empty());
    }
}
