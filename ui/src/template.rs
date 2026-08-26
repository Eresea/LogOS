use crate::runtime::{
    MAX_UI_NODES, TAB_INDEX_NONE, UiBlueprint, UiError, UiInteraction, UiNodeKind,
};
use crate::{UiLayoutAlignment, UiLayoutDirection, UiLayoutStyle};

pub const MAX_UI_NAME_BYTES: usize = 24;
pub const MAX_UI_TEXT_BYTES: usize = 64;
pub const MAX_UI_EXPRESSION_BYTES: usize = 48;
pub const MAX_UI_STYLE_TOKENS: usize = 8;
pub const MAX_UI_STATE_STYLES: usize = 4;
pub const MAX_UI_CONDITIONAL_STYLES: usize = 4;
pub const MAX_UI_BINDINGS: usize = 4;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiName {
    bytes: [u8; MAX_UI_NAME_BYTES],
    len: u8,
}

impl UiName {
    pub const EMPTY: Self = Self { bytes: [0; MAX_UI_NAME_BYTES], len: 0 };

    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.is_empty() || bytes.len() > MAX_UI_NAME_BYTES {
            return None;
        }
        let mut value = Self::EMPTY;
        value.bytes[..bytes.len()].copy_from_slice(bytes);
        value.len = bytes.len() as u8;
        Some(value)
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len as usize]
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiText {
    bytes: [u8; MAX_UI_TEXT_BYTES],
    len: u8,
}

impl UiText {
    pub const EMPTY: Self = Self { bytes: [0; MAX_UI_TEXT_BYTES], len: 0 };

    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() > MAX_UI_TEXT_BYTES {
            return None;
        }
        let mut value = Self::EMPTY;
        value.bytes[..bytes.len()].copy_from_slice(bytes);
        value.len = bytes.len() as u8;
        Some(value)
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len as usize]
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiExpression {
    bytes: [u8; MAX_UI_EXPRESSION_BYTES],
    len: u8,
}

impl UiExpression {
    pub const EMPTY: Self = Self { bytes: [0; MAX_UI_EXPRESSION_BYTES], len: 0 };

    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.is_empty() || bytes.len() > MAX_UI_EXPRESSION_BYTES {
            return None;
        }
        let mut value = Self::EMPTY;
        value.bytes[..bytes.len()].copy_from_slice(bytes);
        value.len = bytes.len() as u8;
        Some(value)
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len as usize]
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UiBindingProperty {
    Value,
    Disabled,
    Form,
    Control,
    CanSubmit,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiBinding {
    pub property: UiBindingProperty,
    pub expression: UiExpression,
}

impl UiBinding {
    pub const EMPTY: Self =
        Self { property: UiBindingProperty::Value, expression: UiExpression::EMPTY };

    pub fn is_present(&self) -> bool {
        !self.expression.as_bytes().is_empty()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiBindingList {
    pub entries: [UiBinding; MAX_UI_BINDINGS],
    pub len: u8,
}

impl UiBindingList {
    pub const EMPTY: Self = Self { entries: [UiBinding::EMPTY; MAX_UI_BINDINGS], len: 0 };

    pub fn contains(&self, property: UiBindingProperty) -> bool {
        self.entries[..self.len as usize].iter().any(|binding| binding.property == property)
    }

    pub fn push(&mut self, binding: UiBinding) -> bool {
        if usize::from(self.len) == MAX_UI_BINDINGS {
            return false;
        }
        self.entries[usize::from(self.len)] = binding;
        self.len += 1;
        true
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UiStyleState {
    Focus,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiStateStyle {
    pub state: UiStyleState,
    pub style: UiStyle,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiStateStyleList {
    pub entries: [UiStateStyle; MAX_UI_STATE_STYLES],
    pub len: u8,
}

impl UiStateStyleList {
    pub const EMPTY: Self = Self {
        entries: [UiStateStyle { state: UiStyleState::Focus, style: UiStyle::FullHeight };
            MAX_UI_STATE_STYLES],
        len: 0,
    };

    pub fn push(&mut self, entry: UiStateStyle) -> bool {
        if usize::from(self.len) == MAX_UI_STATE_STYLES {
            return false;
        }
        self.entries[usize::from(self.len)] = entry;
        self.len += 1;
        true
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiConditionalStyle {
    pub style: UiStyle,
    pub expression: UiExpression,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiConditionalStyleList {
    pub entries: [UiConditionalStyle; MAX_UI_CONDITIONAL_STYLES],
    pub len: u8,
}

impl UiConditionalStyleList {
    pub const EMPTY: Self = Self {
        entries: [UiConditionalStyle {
            style: UiStyle::FullHeight,
            expression: UiExpression::EMPTY,
        }; MAX_UI_CONDITIONAL_STYLES],
        len: 0,
    };

    pub fn push(&mut self, entry: UiConditionalStyle) -> bool {
        if usize::from(self.len) == MAX_UI_CONDITIONAL_STYLES {
            return false;
        }
        self.entries[usize::from(self.len)] = entry;
        self.len += 1;
        true
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UiEventKind {
    Click,
    Submit,
    Changed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiEvent {
    pub kind: UiEventKind,
    pub handler: UiExpression,
}

impl UiEvent {
    pub const EMPTY: Self = Self { kind: UiEventKind::Click, handler: UiExpression::EMPTY };

    pub fn is_present(&self) -> bool {
        !self.handler.as_bytes().is_empty()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UiStyle {
    FullHeight,
    FullWidth,
    FlexX,
    FlexY,
    ItemsCenter,
    JustifyCenter,
    Width96,
    Gap(u8),
    GapX(u8),
    GapY(u8),
    MarginTop4,
    MarginBottom2,
    PaddingX6,
    PaddingY3,
    RoundedLarge,
    BackgroundAccent,
    Text4xl,
    FontLight,
    Opacity50,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiStyleList {
    pub tokens: [UiStyle; MAX_UI_STYLE_TOKENS],
    pub len: u8,
}

impl UiStyleList {
    pub const EMPTY: Self = Self { tokens: [UiStyle::FullHeight; MAX_UI_STYLE_TOKENS], len: 0 };

    pub fn push(&mut self, token: UiStyle) -> bool {
        if usize::from(self.len) == MAX_UI_STYLE_TOKENS {
            return false;
        }
        self.tokens[usize::from(self.len)] = token;
        self.len += 1;
        true
    }

    pub fn to_layout_style(&self) -> UiLayoutStyle {
        let mut layout = UiLayoutStyle::EMPTY;
        for token in self.tokens[..self.len as usize].iter().copied() {
            match token {
                UiStyle::FlexX => layout.direction = UiLayoutDirection::Row,
                UiStyle::FlexY => layout.direction = UiLayoutDirection::Column,
                UiStyle::ItemsCenter => layout.align_items = UiLayoutAlignment::Center,
                UiStyle::JustifyCenter => layout.justify_content = UiLayoutAlignment::Center,
                UiStyle::Gap(value) => layout.gap = spacing_px(value),
                UiStyle::GapX(value) => layout.gap_x = spacing_px(value),
                UiStyle::GapY(value) => layout.gap_y = spacing_px(value),
                UiStyle::PaddingX6 => {
                    layout.padding.left = spacing_px(6);
                    layout.padding.right = spacing_px(6);
                }
                UiStyle::PaddingY3 => {
                    layout.padding.top = spacing_px(3);
                    layout.padding.bottom = spacing_px(3);
                }
                _ => {}
            }
        }
        layout
    }
}

fn spacing_px(value: u8) -> u32 {
    u32::from(value).saturating_mul(4)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiNodeTemplate {
    pub kind: UiNodeKind,
    pub parent: u16,
    pub key: UiName,
    pub text: UiText,
    pub bindings: UiBindingList,
    pub event: UiEvent,
    pub styles: UiStyleList,
    pub state_styles: UiStateStyleList,
    pub conditional_styles: UiConditionalStyleList,
    pub tab_index: i16,
}

impl UiNodeTemplate {
    pub const EMPTY: Self = Self {
        kind: UiNodeKind::Panel,
        parent: u16::MAX,
        key: UiName::EMPTY,
        text: UiText::EMPTY,
        bindings: UiBindingList::EMPTY,
        event: UiEvent::EMPTY,
        styles: UiStyleList::EMPTY,
        state_styles: UiStateStyleList::EMPTY,
        conditional_styles: UiConditionalStyleList::EMPTY,
        tab_index: TAB_INDEX_NONE,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiDocument {
    nodes: [UiNodeTemplate; MAX_UI_NODES],
    count: usize,
}

impl UiDocument {
    pub const EMPTY: Self = Self { nodes: [UiNodeTemplate::EMPTY; MAX_UI_NODES], count: 0 };

    pub const fn node_count(&self) -> usize {
        self.count
    }

    pub fn node(&self, index: usize) -> Option<&UiNodeTemplate> {
        (index < self.count).then(|| &self.nodes[index])
    }

    pub fn to_blueprint(&self) -> Result<UiBlueprint, UiError> {
        let mut blueprint = UiBlueprint::new();
        for index in 0..self.count {
            let node = self.nodes[index];
            let key = node_key(index, &node.key);
            let mut interaction = UiInteraction::for_kind(node.kind);
            interaction.set_tab_index(node.tab_index);
            let layout = node.styles.to_layout_style();
            if node.parent == u16::MAX {
                blueprint.push_root_with_interaction_and_layout(
                    node.kind,
                    key,
                    interaction,
                    layout,
                )?;
            } else {
                blueprint.push_child_with_interaction_and_layout(
                    node.kind,
                    node.parent,
                    key,
                    interaction,
                    layout,
                )?;
            }
        }
        Ok(blueprint)
    }

    pub fn push_node(&mut self, node: UiNodeTemplate) -> Option<u16> {
        if self.count == MAX_UI_NODES {
            return None;
        }
        let index = self.count as u16;
        self.nodes[self.count] = node;
        self.count += 1;
        Some(index)
    }

    pub fn node_mut(&mut self, index: u16) -> Option<&mut UiNodeTemplate> {
        self.nodes.get_mut(usize::from(index))
    }
}

fn node_key(index: usize, name: &UiName) -> u16 {
    let mut hash = 0x811c_u32 ^ index as u32;
    for byte in name.as_bytes() {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    let key = hash as u16;
    if key == 0 { 1 } else { key }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portable_document_builds_runtime_blueprint() {
        let mut document = UiDocument::EMPTY;
        let root = document
            .push_node(UiNodeTemplate { kind: UiNodeKind::Root, ..UiNodeTemplate::EMPTY })
            .unwrap();
        document.push_node(UiNodeTemplate {
            kind: UiNodeKind::Button,
            parent: root,
            ..UiNodeTemplate::EMPTY
        });

        let blueprint = document.to_blueprint().unwrap();
        assert_eq!(blueprint.len(), 2);
        assert_eq!(crate::UiTree::from_blueprint(&blueprint).unwrap().len(), 2);
    }

    #[test]
    fn utility_layout_tokens_survive_blueprint_instantiation() {
        let mut styles = UiStyleList::EMPTY;
        assert!(styles.push(UiStyle::FlexX));
        assert!(styles.push(UiStyle::ItemsCenter));
        assert!(styles.push(UiStyle::JustifyCenter));
        assert!(styles.push(UiStyle::GapX(3)));
        assert!(styles.push(UiStyle::PaddingX6));
        assert!(styles.push(UiStyle::PaddingY3));

        let layout = styles.to_layout_style();
        assert_eq!(layout.direction, crate::UiLayoutDirection::Row);
        assert_eq!(layout.align_items, crate::UiLayoutAlignment::Center);
        assert_eq!(layout.justify_content, crate::UiLayoutAlignment::Center);
        assert_eq!(layout.gap_x, 12);
        assert_eq!(layout.padding, crate::UiEdges::new(24, 24, 12, 12));

        let mut document = UiDocument::EMPTY;
        document.push_node(UiNodeTemplate { styles, ..UiNodeTemplate::EMPTY }).unwrap();
        let blueprint = document.to_blueprint().unwrap();
        let tree = crate::UiTree::from_blueprint(&blueprint).unwrap();
        let root = tree.node(crate::UiNodeHandle { slot: 0, generation: 1 }).unwrap();
        assert_eq!(root.layout, layout);
    }
}
