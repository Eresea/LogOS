pub const MAX_UI_NODES: usize = 32;
const NO_PARENT: u16 = u16::MAX;
pub const TAB_INDEX_NONE: i16 = -1;

use crate::layout::{UiLayoutStyle, UiSize};
use crate::template::{UiStyle, UiStyleList, UiText};

/// Framework-owned logical geometry. Rendering adapters convert this into
/// their platform or compositor rectangle type at the boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl UiRect {
    pub const EMPTY: Self = Self { x: 0, y: 0, width: 0, height: 0 };

    pub const fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self { x, y, width, height }
    }

    pub const fn is_empty(self) -> bool {
        self.width == 0 || self.height == 0
    }

    pub const fn contains(self, x: i32, y: i32) -> bool {
        if self.is_empty() {
            return false;
        }
        let x = x as i64;
        let y = y as i64;
        let left = self.x as i64;
        let top = self.y as i64;
        x >= left && y >= top && x < left + self.width as i64 && y < top + self.height as i64
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum UiNodeKind {
    Root = 1,
    Panel = 2,
    Label = 3,
    Button = 4,
    TextInput = 5,
    Form = 6,
}

impl UiNodeKind {
    pub const fn is_interactive(self) -> bool {
        matches!(self, Self::Button | Self::TextInput)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiInteraction {
    tab_index: i16,
    disabled: bool,
    focused: bool,
}

impl UiInteraction {
    pub const fn for_kind(kind: UiNodeKind) -> Self {
        Self {
            tab_index: if kind.is_interactive() { 0 } else { TAB_INDEX_NONE },
            disabled: false,
            focused: false,
        }
    }

    pub const fn tab_index(self) -> i16 {
        self.tab_index
    }

    pub const fn is_disabled(self) -> bool {
        self.disabled
    }

    pub const fn is_focused(self) -> bool {
        self.focused
    }

    pub const fn is_focusable(self) -> bool {
        self.tab_index >= 0 && !self.disabled
    }

    pub fn set_tab_index(&mut self, tab_index: i16) {
        self.tab_index = tab_index;
    }

    pub fn set_disabled(&mut self, disabled: bool) {
        self.disabled = disabled;
        if disabled {
            self.focused = false;
        }
    }

    pub fn set_focused(&mut self, focused: bool) {
        self.focused = focused && !self.disabled && self.tab_index >= 0;
    }
}

pub trait UiInteractive {
    fn interaction(&self) -> &UiInteraction;
    fn interaction_mut(&mut self) -> &mut UiInteraction;

    fn tab_index(&self) -> i16 {
        self.interaction().tab_index()
    }

    fn is_disabled(&self) -> bool {
        self.interaction().is_disabled()
    }

    fn is_focused(&self) -> bool {
        self.interaction().is_focused()
    }

    fn is_focusable(&self) -> bool {
        self.interaction().is_focusable()
    }

    fn set_tab_index(&mut self, tab_index: i16) {
        self.interaction_mut().set_tab_index(tab_index);
    }

    fn set_disabled(&mut self, disabled: bool) {
        self.interaction_mut().set_disabled(disabled);
    }

    fn set_focused(&mut self, focused: bool) {
        self.interaction_mut().set_focused(focused);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiNodeHandle {
    pub slot: u16,
    pub generation: u16,
}

impl UiNodeHandle {
    pub const EMPTY: Self = Self { slot: u16::MAX, generation: 0 };

    pub const fn is_valid(self) -> bool {
        self.slot != u16::MAX && self.generation != 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiNodeSpec {
    pub kind: UiNodeKind,
    pub parent: u16,
    pub key: u16,
    pub text: UiText,
    pub styles: UiStyleList,
    pub interaction: UiInteraction,
    pub layout: UiLayoutStyle,
    pub intrinsic_size: UiSize,
}

impl UiNodeSpec {
    pub const fn root(kind: UiNodeKind, key: u16) -> Self {
        Self {
            kind,
            parent: NO_PARENT,
            key,
            text: UiText::EMPTY,
            styles: UiStyleList::EMPTY,
            interaction: UiInteraction::for_kind(kind),
            layout: UiLayoutStyle::EMPTY,
            intrinsic_size: UiSize::ZERO,
        }
    }

    pub const fn child(kind: UiNodeKind, parent: u16, key: u16) -> Self {
        Self {
            kind,
            parent,
            key,
            text: UiText::EMPTY,
            styles: UiStyleList::EMPTY,
            interaction: UiInteraction::for_kind(kind),
            layout: UiLayoutStyle::EMPTY,
            intrinsic_size: UiSize::ZERO,
        }
    }

    pub const fn root_with_interaction(
        kind: UiNodeKind,
        key: u16,
        interaction: UiInteraction,
    ) -> Self {
        Self {
            kind,
            parent: NO_PARENT,
            key,
            text: UiText::EMPTY,
            styles: UiStyleList::EMPTY,
            interaction,
            layout: UiLayoutStyle::EMPTY,
            intrinsic_size: UiSize::ZERO,
        }
    }

    pub const fn child_with_interaction(
        kind: UiNodeKind,
        parent: u16,
        key: u16,
        interaction: UiInteraction,
    ) -> Self {
        Self {
            kind,
            parent,
            key,
            text: UiText::EMPTY,
            styles: UiStyleList::EMPTY,
            interaction,
            layout: UiLayoutStyle::EMPTY,
            intrinsic_size: UiSize::ZERO,
        }
    }
}

pub struct UiBlueprint {
    specs: [UiNodeSpec; MAX_UI_NODES],
    count: usize,
}

impl UiBlueprint {
    pub const fn new() -> Self {
        Self { specs: [UiNodeSpec::root(UiNodeKind::Panel, 0); MAX_UI_NODES], count: 0 }
    }

    pub fn push_root(&mut self, kind: UiNodeKind, key: u16) -> Result<u16, UiError> {
        self.push_root_with_interaction(kind, key, UiInteraction::for_kind(kind))
    }

    pub fn push_root_with_interaction(
        &mut self,
        kind: UiNodeKind,
        key: u16,
        interaction: UiInteraction,
    ) -> Result<u16, UiError> {
        self.push_root_with_interaction_and_layout(kind, key, interaction, UiLayoutStyle::EMPTY)
    }

    pub fn push_root_with_interaction_and_layout(
        &mut self,
        kind: UiNodeKind,
        key: u16,
        interaction: UiInteraction,
        layout: UiLayoutStyle,
    ) -> Result<u16, UiError> {
        if self.count != 0 {
            return Err(UiError::RootExists);
        }
        self.push(UiNodeSpec {
            layout,
            ..UiNodeSpec::root_with_interaction(kind, key, interaction)
        })
    }

    pub fn push_child(&mut self, kind: UiNodeKind, parent: u16, key: u16) -> Result<u16, UiError> {
        self.push_child_with_interaction(kind, parent, key, UiInteraction::for_kind(kind))
    }

    pub fn push_child_with_interaction(
        &mut self,
        kind: UiNodeKind,
        parent: u16,
        key: u16,
        interaction: UiInteraction,
    ) -> Result<u16, UiError> {
        self.push_child_with_interaction_and_layout(
            kind,
            parent,
            key,
            interaction,
            UiLayoutStyle::EMPTY,
        )
    }

    pub fn push_child_with_interaction_and_layout(
        &mut self,
        kind: UiNodeKind,
        parent: u16,
        key: u16,
        interaction: UiInteraction,
        layout: UiLayoutStyle,
    ) -> Result<u16, UiError> {
        if usize::from(parent) >= self.count {
            return Err(UiError::InvalidParent);
        }
        self.push(UiNodeSpec {
            layout,
            ..UiNodeSpec::child_with_interaction(kind, parent, key, interaction)
        })
    }

    pub const fn len(&self) -> usize {
        self.count
    }

    pub fn spec(&self, index: usize) -> Option<UiNodeSpec> {
        (index < self.count).then_some(self.specs[index])
    }

    pub fn set_text(&mut self, index: u16, text: UiText) -> Result<(), UiError> {
        let index = usize::from(index);
        if index >= self.count {
            return Err(UiError::NotFound);
        }
        self.specs[index].text = text;
        Ok(())
    }

    pub fn set_styles(&mut self, index: u16, styles: UiStyleList) -> Result<(), UiError> {
        let index = usize::from(index);
        if index >= self.count {
            return Err(UiError::NotFound);
        }
        self.specs[index].styles = styles;
        Ok(())
    }

    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }

    fn push(&mut self, spec: UiNodeSpec) -> Result<u16, UiError> {
        if self.count == MAX_UI_NODES {
            return Err(UiError::Capacity);
        }
        let index = self.count as u16;
        self.specs[self.count] = spec;
        self.count += 1;
        Ok(index)
    }
}

impl Default for UiBlueprint {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UiError {
    Capacity,
    InvalidParent,
    RootExists,
    Stale,
    NotFound,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiNode {
    pub handle: UiNodeHandle,
    pub parent: UiNodeHandle,
    pub kind: UiNodeKind,
    pub key: u16,
    pub order: u32,
    pub text: UiText,
    pub styles: UiStyleList,
    pub bounds: UiRect,
    pub clip: UiRect,
    pub dirty: bool,
    pub interaction: UiInteraction,
    pub layout: UiLayoutStyle,
    pub intrinsic_size: UiSize,
}

impl UiNode {
    const EMPTY: Self = Self {
        handle: UiNodeHandle::EMPTY,
        parent: UiNodeHandle::EMPTY,
        kind: UiNodeKind::Panel,
        key: 0,
        order: 0,
        text: UiText::EMPTY,
        styles: UiStyleList::EMPTY,
        bounds: UiRect::EMPTY,
        clip: UiRect::EMPTY,
        dirty: false,
        interaction: UiInteraction::for_kind(UiNodeKind::Panel),
        layout: UiLayoutStyle::EMPTY,
        intrinsic_size: UiSize::ZERO,
    };
}

pub struct UiTree {
    nodes: [UiNode; MAX_UI_NODES],
    generations: [u16; MAX_UI_NODES],
    count: usize,
    next_order: u32,
}

impl UiTree {
    pub const fn new() -> Self {
        Self {
            nodes: [UiNode::EMPTY; MAX_UI_NODES],
            generations: [0; MAX_UI_NODES],
            count: 0,
            next_order: 0,
        }
    }

    pub fn from_blueprint(blueprint: &UiBlueprint) -> Result<Self, UiError> {
        let mut tree = Self::new();
        for index in 0..blueprint.count {
            let spec = blueprint.specs[index];
            let parent = if spec.parent == NO_PARENT {
                UiNodeHandle::EMPTY
            } else {
                tree.nodes[usize::from(spec.parent)].handle
            };
            tree.insert_with_text_and_interaction_and_layout(
                spec.kind,
                parent,
                spec.key,
                spec.text,
                spec.styles,
                spec.interaction,
                spec.layout,
                spec.intrinsic_size,
            )?;
        }
        Ok(tree)
    }

    pub fn insert(
        &mut self,
        kind: UiNodeKind,
        parent: UiNodeHandle,
        key: u16,
    ) -> Result<UiNodeHandle, UiError> {
        self.insert_with_interaction(kind, parent, key, UiInteraction::for_kind(kind))
    }

    pub fn insert_with_interaction(
        &mut self,
        kind: UiNodeKind,
        parent: UiNodeHandle,
        key: u16,
        interaction: UiInteraction,
    ) -> Result<UiNodeHandle, UiError> {
        self.insert_with_interaction_and_layout(
            kind,
            parent,
            key,
            interaction,
            UiLayoutStyle::EMPTY,
            UiSize::ZERO,
        )
    }

    pub fn insert_with_interaction_and_layout(
        &mut self,
        kind: UiNodeKind,
        parent: UiNodeHandle,
        key: u16,
        interaction: UiInteraction,
        layout: UiLayoutStyle,
        intrinsic_size: UiSize,
    ) -> Result<UiNodeHandle, UiError> {
        self.insert_with_text_and_interaction_and_layout(
            kind,
            parent,
            key,
            UiText::EMPTY,
            UiStyleList::EMPTY,
            interaction,
            layout,
            intrinsic_size,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn insert_with_text_and_interaction_and_layout(
        &mut self,
        kind: UiNodeKind,
        parent: UiNodeHandle,
        key: u16,
        text: UiText,
        styles: UiStyleList,
        interaction: UiInteraction,
        layout: UiLayoutStyle,
        intrinsic_size: UiSize,
    ) -> Result<UiNodeHandle, UiError> {
        if parent.is_valid() && self.lookup(parent).is_err() {
            return Err(UiError::Stale);
        }
        if !parent.is_valid() && self.nodes.iter().any(|node| node.handle.is_valid()) {
            return Err(UiError::RootExists);
        }
        let Some((slot, node)) =
            self.nodes.iter_mut().enumerate().find(|(_, node)| !node.handle.is_valid())
        else {
            return Err(UiError::Capacity);
        };
        self.generations[slot] = self.generations[slot].wrapping_add(1).max(1);
        let handle = UiNodeHandle { slot: slot as u16, generation: self.generations[slot] };
        self.next_order = self.next_order.wrapping_add(1).max(1);
        *node = UiNode {
            handle,
            parent,
            kind,
            key,
            order: self.next_order,
            text,
            styles,
            bounds: UiRect::EMPTY,
            clip: UiRect::EMPTY,
            dirty: true,
            interaction,
            layout,
            intrinsic_size,
        };
        self.count += 1;
        Ok(handle)
    }

    pub fn destroy(&mut self, handle: UiNodeHandle) -> Result<(), UiError> {
        let _ = self.lookup(handle)?;
        loop {
            let child = self
                .nodes
                .iter()
                .find(|node| node.handle.is_valid() && node.parent == handle)
                .map(|node| node.handle);
            let Some(child) = child else { break };
            self.destroy(child)?;
        }
        let node = self.lookup(handle)?;
        self.nodes[usize::from(node.handle.slot)] = UiNode::EMPTY;
        self.count -= 1;
        Ok(())
    }

    pub const fn len(&self) -> usize {
        self.count
    }

    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }

    pub fn node(&self, handle: UiNodeHandle) -> Result<&UiNode, UiError> {
        self.lookup(handle)
    }

    pub fn node_mut(&mut self, handle: UiNodeHandle) -> Result<&mut UiNode, UiError> {
        let index = self.lookup(handle)?.handle.slot as usize;
        Ok(&mut self.nodes[index])
    }

    pub fn handle_at(&self, index: usize) -> Result<UiNodeHandle, UiError> {
        self.nodes
            .get(index)
            .filter(|node| node.handle.is_valid())
            .map(|node| node.handle)
            .ok_or(UiError::NotFound)
    }

    pub fn hit_test(&self, x: i32, y: i32) -> Option<UiNodeHandle> {
        let mut hit = None;
        let mut order = 0;
        for node in self.nodes.iter().filter(|node| node.handle.is_valid()) {
            let clipped = node.clip.is_empty() || node.clip.contains(x, y);
            if node.kind.is_interactive()
                && node.interaction.is_focusable()
                && node.bounds.contains(x, y)
                && clipped
                && node.order >= order
            {
                hit = Some(node.handle);
                order = node.order;
            }
        }
        hit
    }

    pub fn set_bounds(&mut self, handle: UiNodeHandle, bounds: UiRect) -> Result<(), UiError> {
        let node = self.node_mut(handle)?;
        if node.bounds != bounds {
            node.bounds = bounds;
            node.dirty = true;
        }
        Ok(())
    }

    pub fn set_text(&mut self, handle: UiNodeHandle, text: UiText) -> Result<bool, UiError> {
        let node = self.node_mut(handle)?;
        if node.text == text {
            return Ok(false);
        }
        node.text = text;
        node.dirty = true;
        Ok(true)
    }

    pub fn set_styles(
        &mut self,
        handle: UiNodeHandle,
        styles: UiStyleList,
    ) -> Result<bool, UiError> {
        let node = self.node_mut(handle)?;
        if node.styles == styles {
            return Ok(false);
        }
        node.styles = styles;
        node.dirty = true;
        Ok(true)
    }

    pub fn has_style(&self, handle: UiNodeHandle, style: UiStyle) -> Result<bool, UiError> {
        Ok(self.node(handle)?.styles.contains(style))
    }

    pub fn set_clip(&mut self, handle: UiNodeHandle, clip: UiRect) -> Result<(), UiError> {
        let node = self.node_mut(handle)?;
        if node.clip != clip {
            node.clip = clip;
            node.dirty = true;
        }
        Ok(())
    }

    pub fn set_layout_style(
        &mut self,
        handle: UiNodeHandle,
        layout: UiLayoutStyle,
    ) -> Result<(), UiError> {
        let node = self.node_mut(handle)?;
        if node.layout != layout {
            node.layout = layout;
            node.dirty = true;
        }
        Ok(())
    }

    pub fn set_intrinsic_size(
        &mut self,
        handle: UiNodeHandle,
        intrinsic_size: UiSize,
    ) -> Result<(), UiError> {
        let node = self.node_mut(handle)?;
        if node.intrinsic_size != intrinsic_size {
            node.intrinsic_size = intrinsic_size;
            node.dirty = true;
        }
        Ok(())
    }

    pub fn children(
        &self,
        parent: UiNodeHandle,
        output: &mut [UiNodeHandle; MAX_UI_NODES],
    ) -> Result<usize, UiError> {
        let _ = self.lookup(parent)?;
        let mut count = 0;
        for node in self.nodes.iter().filter(|node| node.handle.is_valid() && node.parent == parent)
        {
            output[count] = node.handle;
            count += 1;
        }
        Ok(count)
    }

    pub fn clear_dirty(&mut self, handle: UiNodeHandle) -> Result<(), UiError> {
        self.node_mut(handle)?.dirty = false;
        Ok(())
    }

    fn lookup(&self, handle: UiNodeHandle) -> Result<&UiNode, UiError> {
        let Some(node) = self.nodes.get(usize::from(handle.slot)) else {
            return Err(UiError::Stale);
        };
        if node.handle != handle {
            return Err(UiError::Stale);
        }
        Ok(node)
    }
}

impl UiInteractive for UiNode {
    fn interaction(&self) -> &UiInteraction {
        &self.interaction
    }

    fn interaction_mut(&mut self) -> &mut UiInteraction {
        &mut self.interaction
    }
}

impl Default for UiTree {
    fn default() -> Self {
        Self::new()
    }
}

const _: () = assert!(core::mem::size_of::<UiTree>() <= 8192);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blueprint_instantiates_a_generation_safe_tree() {
        let mut blueprint = UiBlueprint::new();
        let root = blueprint.push_root(UiNodeKind::Root, 1).unwrap();
        let button = blueprint.push_child(UiNodeKind::Button, root, 2).unwrap();
        let mut tree = UiTree::from_blueprint(&blueprint).unwrap();
        assert_eq!(tree.len(), 2);
        let root_handle = tree.node(tree.nodes[usize::from(root)].handle).unwrap().handle;
        let button_handle = tree.nodes[usize::from(button)].handle;
        assert_eq!(tree.node(button_handle).unwrap().parent, root_handle);
        tree.destroy(button_handle).unwrap();
        let replacement = tree.insert(UiNodeKind::Label, root_handle, 3).unwrap();
        assert_eq!(replacement.slot, button_handle.slot);
        assert_ne!(replacement.generation, button_handle.generation);
        assert_eq!(tree.node(button_handle), Err(UiError::Stale));
    }

    #[test]
    fn interactive_nodes_expose_bounded_focus_contract() {
        let mut tree = UiTree::new();
        let root = tree.insert(UiNodeKind::Root, UiNodeHandle::EMPTY, 1).unwrap();
        let button = tree.insert(UiNodeKind::Button, root, 2).unwrap();
        let node = tree.node(button).unwrap();
        assert!(node.is_focusable());
        assert_eq!(node.tab_index(), 0);

        let node = tree.node_mut(button).unwrap();
        node.set_focused(true);
        assert!(node.is_focused());
        node.set_disabled(true);
        assert!(!node.is_focusable());
        assert!(!node.is_focused());
    }

    #[test]
    fn non_interactive_nodes_are_not_focusable_by_default() {
        let mut tree = UiTree::new();
        let root = tree.insert(UiNodeKind::Root, UiNodeHandle::EMPTY, 1).unwrap();
        let label = tree.insert(UiNodeKind::Label, root, 2).unwrap();
        assert!(!tree.node(label).unwrap().is_focusable());
    }

    #[test]
    fn tree_destroy_removes_descendants_and_bounds_are_dirty() {
        let mut tree = UiTree::new();
        let root = tree.insert(UiNodeKind::Root, UiNodeHandle::EMPTY, 1).unwrap();
        let child = tree.insert(UiNodeKind::Panel, root, 2).unwrap();
        let leaf = tree.insert(UiNodeKind::Label, child, 3).unwrap();
        tree.set_bounds(leaf, UiRect::new(4, 5, 20, 10)).unwrap();
        assert!(tree.node(leaf).unwrap().dirty);
        tree.clear_dirty(leaf).unwrap();
        assert!(!tree.node(leaf).unwrap().dirty);
        tree.destroy(root).unwrap();
        assert!(tree.is_empty());
    }

    #[test]
    fn text_is_retained_and_updates_only_when_changed() {
        let mut blueprint = UiBlueprint::new();
        let root = blueprint.push_root(UiNodeKind::Label, 1).unwrap();
        blueprint.set_text(root, UiText::from_bytes(b"Title").unwrap()).unwrap();
        let mut tree = UiTree::from_blueprint(&blueprint).unwrap();
        let handle = tree.handle_at(0).unwrap();
        assert_eq!(tree.node(handle).unwrap().text.as_bytes(), b"Title");

        tree.clear_dirty(handle).unwrap();
        assert_eq!(tree.set_text(handle, UiText::from_bytes(b"Title").unwrap()), Ok(false));
        assert!(!tree.node(handle).unwrap().dirty);
        assert_eq!(tree.set_text(handle, UiText::from_bytes(b"Updated").unwrap()), Ok(true));
        assert!(tree.node(handle).unwrap().dirty);

        tree.destroy(handle).unwrap();
        assert_eq!(
            tree.set_text(handle, UiText::from_bytes(b"stale").unwrap()),
            Err(UiError::Stale)
        );
    }

    #[test]
    fn styles_are_retained_and_update_only_when_changed() {
        let mut tree = UiTree::new();
        let root = tree.insert(UiNodeKind::Panel, UiNodeHandle::EMPTY, 1).unwrap();
        let mut styles = UiStyleList::EMPTY;
        assert!(styles.push(UiStyle::BackgroundAccent));
        tree.clear_dirty(root).unwrap();

        assert_eq!(tree.set_styles(root, styles), Ok(true));
        assert!(tree.has_style(root, UiStyle::BackgroundAccent).unwrap());
        assert!(tree.node(root).unwrap().dirty);
        tree.clear_dirty(root).unwrap();
        assert_eq!(tree.set_styles(root, styles), Ok(false));
        assert!(!tree.node(root).unwrap().dirty);

        tree.destroy(root).unwrap();
        assert_eq!(tree.set_styles(root, styles), Err(UiError::Stale));
        assert_eq!(tree.has_style(root, UiStyle::BackgroundAccent), Err(UiError::Stale));
    }

    #[test]
    fn hit_test_returns_the_topmost_visible_interactive_node() {
        let mut tree = UiTree::new();
        let root = tree.insert(UiNodeKind::Root, UiNodeHandle::EMPTY, 1).unwrap();
        let first = tree.insert(UiNodeKind::Button, root, 2).unwrap();
        let second = tree.insert(UiNodeKind::Button, root, 3).unwrap();
        let bounds = UiRect::new(10, 20, 40, 30);
        tree.set_bounds(first, bounds).unwrap();
        tree.set_bounds(second, bounds).unwrap();

        assert_eq!(tree.hit_test(10, 20), Some(second));
        assert_eq!(tree.hit_test(50, 50), None);

        tree.set_clip(second, UiRect::new(10, 20, 5, 5)).unwrap();
        assert_eq!(tree.hit_test(12, 22), Some(second));
        assert_eq!(tree.hit_test(20, 25), Some(first));

        tree.node_mut(second).unwrap().interaction.set_disabled(true);
        assert_eq!(tree.hit_test(12, 22), Some(first));
    }

    #[test]
    fn hit_test_uses_retained_order_after_slot_reuse() {
        let mut tree = UiTree::new();
        let root = tree.insert(UiNodeKind::Root, UiNodeHandle::EMPTY, 1).unwrap();
        let first = tree.insert(UiNodeKind::Button, root, 2).unwrap();
        let second = tree.insert(UiNodeKind::Button, root, 3).unwrap();
        let bounds = UiRect::new(0, 0, 10, 10);
        tree.set_bounds(first, bounds).unwrap();
        tree.set_bounds(second, bounds).unwrap();
        tree.destroy(first).unwrap();
        let replacement = tree.insert(UiNodeKind::Button, root, 4).unwrap();
        tree.set_bounds(replacement, bounds).unwrap();

        assert_eq!(replacement.slot, first.slot);
        assert!(replacement.generation != first.generation);
        assert_eq!(tree.hit_test(1, 1), Some(replacement));
    }

    #[test]
    fn rectangles_use_half_open_bounds() {
        let rectangle = UiRect::new(-2, 3, 4, 5);
        assert!(rectangle.contains(-2, 3));
        assert!(rectangle.contains(1, 7));
        assert!(!rectangle.contains(2, 7));
        assert!(!rectangle.contains(1, 8));
        assert!(!UiRect::EMPTY.contains(0, 0));
    }

    #[test]
    fn blueprint_rejects_multiple_roots_and_invalid_parents() {
        let mut blueprint = UiBlueprint::new();
        let root = blueprint.push_root(UiNodeKind::Root, 1).unwrap();
        assert_eq!(blueprint.push_root(UiNodeKind::Root, 2), Err(UiError::RootExists));
        assert_eq!(
            blueprint.push_child(UiNodeKind::Button, root + 1, 3),
            Err(UiError::InvalidParent)
        );
    }
}
