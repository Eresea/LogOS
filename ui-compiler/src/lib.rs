#![no_std]

#[cfg(test)]
extern crate std;

use logos_ui::{MAX_UI_NODES, UiBlueprint, UiError, UiInteraction, UiNodeKind};

pub const MAX_UI_SOURCE_BYTES: usize = 4096;
pub const MAX_UI_NAME_BYTES: usize = 24;
pub const MAX_UI_TEXT_BYTES: usize = 64;
pub const MAX_UI_EXPRESSION_BYTES: usize = 48;
pub const MAX_UI_STYLE_TOKENS: usize = 8;
pub const MAX_UI_STATE_STYLES: usize = 4;
pub const MAX_UI_CONDITIONAL_STYLES: usize = 4;
pub const MAX_UI_BINDINGS: usize = 4;
pub const MAX_UI_DIAGNOSTICS: usize = 16;

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

    fn from_bytes(bytes: &[u8]) -> Option<Self> {
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
    const EMPTY: Self = Self { bytes: [0; MAX_UI_EXPRESSION_BYTES], len: 0 };

    fn from_bytes(bytes: &[u8]) -> Option<Self> {
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
    const EMPTY: Self =
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
    const EMPTY: Self = Self { entries: [UiBinding::EMPTY; MAX_UI_BINDINGS], len: 0 };

    fn contains(&self, property: UiBindingProperty) -> bool {
        self.entries[..self.len as usize].iter().any(|binding| binding.property == property)
    }

    fn push(&mut self, binding: UiBinding) -> bool {
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
    const EMPTY: Self = Self {
        entries: [UiStateStyle { state: UiStyleState::Focus, style: UiStyle::FullHeight };
            MAX_UI_STATE_STYLES],
        len: 0,
    };

    fn push(&mut self, entry: UiStateStyle) -> bool {
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
    const EMPTY: Self = Self {
        entries: [UiConditionalStyle {
            style: UiStyle::FullHeight,
            expression: UiExpression::EMPTY,
        }; MAX_UI_CONDITIONAL_STYLES],
        len: 0,
    };

    fn push(&mut self, entry: UiConditionalStyle) -> bool {
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
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiEvent {
    pub kind: UiEventKind,
    pub handler: UiExpression,
}

impl UiEvent {
    const EMPTY: Self = Self { kind: UiEventKind::Click, handler: UiExpression::EMPTY };

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
    const EMPTY: Self = Self { tokens: [UiStyle::FullHeight; MAX_UI_STYLE_TOKENS], len: 0 };

    fn push(&mut self, token: UiStyle) -> bool {
        if usize::from(self.len) == MAX_UI_STYLE_TOKENS {
            return false;
        }
        self.tokens[usize::from(self.len)] = token;
        self.len += 1;
        true
    }
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
    const EMPTY: Self = Self {
        kind: UiNodeKind::Panel,
        parent: u16::MAX,
        key: UiName::EMPTY,
        text: UiText::EMPTY,
        bindings: UiBindingList::EMPTY,
        event: UiEvent::EMPTY,
        styles: UiStyleList::EMPTY,
        state_styles: UiStateStyleList::EMPTY,
        conditional_styles: UiConditionalStyleList::EMPTY,
        tab_index: logos_ui::TAB_INDEX_NONE,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiDocument {
    nodes: [UiNodeTemplate; MAX_UI_NODES],
    count: usize,
}

impl UiDocument {
    const EMPTY: Self = Self { nodes: [UiNodeTemplate::EMPTY; MAX_UI_NODES], count: 0 };

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
            if node.parent == u16::MAX {
                blueprint.push_root_with_interaction(node.kind, key, interaction)?;
            } else {
                blueprint.push_child_with_interaction(node.kind, node.parent, key, interaction)?;
            }
        }
        Ok(blueprint)
    }

    fn push(&mut self, node: UiNodeTemplate) -> Option<u16> {
        if self.count == MAX_UI_NODES {
            return None;
        }
        let index = self.count as u16;
        self.nodes[self.count] = node;
        self.count += 1;
        Some(index)
    }

    fn node_mut(&mut self, index: u16) -> Option<&mut UiNodeTemplate> {
        self.nodes.get_mut(usize::from(index))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UiDiagnosticKind {
    SourceTooLarge,
    UnexpectedToken,
    UnexpectedEnd,
    UnknownElement,
    UnknownAttribute,
    UnknownBinding,
    UnknownEvent,
    UnknownStyle,
    MissingValue,
    InvalidValue,
    MismatchedClose,
    TextNotAllowed,
    Capacity,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiDiagnostic {
    pub kind: UiDiagnosticKind,
    pub offset: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiDiagnostics {
    entries: [UiDiagnostic; MAX_UI_DIAGNOSTICS],
    count: usize,
}

impl UiDiagnostics {
    const EMPTY: Self = Self {
        entries: [UiDiagnostic { kind: UiDiagnosticKind::UnexpectedToken, offset: 0 };
            MAX_UI_DIAGNOSTICS],
        count: 0,
    };

    pub const fn len(&self) -> usize {
        self.count
    }

    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }

    pub fn get(&self, index: usize) -> Option<UiDiagnostic> {
        (index < self.count).then(|| self.entries[index])
    }

    fn push(&mut self, kind: UiDiagnosticKind, offset: usize) {
        if self.count == MAX_UI_DIAGNOSTICS {
            return;
        }
        self.entries[self.count] =
            UiDiagnostic { kind, offset: offset.min(usize::from(u16::MAX)) as u16 };
        self.count += 1;
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiBuild {
    pub document: UiDocument,
    pub diagnostics: UiDiagnostics,
}

impl UiBuild {
    pub const fn is_valid(&self) -> bool {
        self.diagnostics.is_empty() && self.document.node_count() != 0
    }
}

pub fn lint(source: &str) -> UiDiagnostics {
    compile(source).diagnostics
}

pub const LOGIN_PAGE_SOURCE: &str = include_str!("../examples/login.ui");
pub const REGISTER_PAGE_SOURCE: &str = include_str!("../examples/register.ui");

pub fn compile_login_page() -> UiBuild {
    compile(LOGIN_PAGE_SOURCE)
}

pub fn compile_register_page() -> UiBuild {
    compile(REGISTER_PAGE_SOURCE)
}

pub fn compile(source: &str) -> UiBuild {
    if source.len() > MAX_UI_SOURCE_BYTES {
        let mut diagnostics = UiDiagnostics::EMPTY;
        diagnostics.push(UiDiagnosticKind::SourceTooLarge, MAX_UI_SOURCE_BYTES);
        return UiBuild { document: UiDocument::EMPTY, diagnostics };
    }
    let mut parser = Parser {
        bytes: source.as_bytes(),
        position: 0,
        document: UiDocument::EMPTY,
        diagnostics: UiDiagnostics::EMPTY,
    };
    parser.parse_document();
    UiBuild { document: parser.document, diagnostics: parser.diagnostics }
}

struct Parser<'a> {
    bytes: &'a [u8],
    position: usize,
    document: UiDocument,
    diagnostics: UiDiagnostics,
}

impl Parser<'_> {
    fn parse_document(&mut self) {
        self.skip_space();
        if self.position == self.bytes.len() {
            self.diagnostics.push(UiDiagnosticKind::UnexpectedEnd, self.position);
            return;
        }
        if self.peek() != Some(b'<') {
            self.diagnostics.push(UiDiagnosticKind::UnexpectedToken, self.position);
            return;
        }
        self.parse_element(u16::MAX);
        self.skip_space();
        if self.position != self.bytes.len() {
            self.diagnostics.push(UiDiagnosticKind::UnexpectedToken, self.position);
        }
    }

    fn parse_element(&mut self, parent: u16) -> Option<u16> {
        let start = self.position;
        if !self.consume_byte(b'<') || self.peek() == Some(b'/') {
            self.diagnostics.push(UiDiagnosticKind::UnexpectedToken, start);
            return None;
        }
        let name = self.read_name();
        let Some(name) = name else {
            self.diagnostics.push(UiDiagnosticKind::UnexpectedToken, start);
            return None;
        };
        let kind = element_kind(name.as_bytes());
        if kind.is_none() {
            self.diagnostics.push(UiDiagnosticKind::UnknownElement, start);
        }
        let node_index = if let Some(kind) = kind {
            let node = UiNodeTemplate {
                kind,
                parent,
                tab_index: if kind.is_interactive() { 0 } else { logos_ui::TAB_INDEX_NONE },
                ..UiNodeTemplate::EMPTY
            };
            let index = self.document.push(node);
            if index.is_none() {
                self.diagnostics.push(UiDiagnosticKind::Capacity, start);
            }
            index
        } else {
            None
        };

        let self_closing = self.parse_attributes(node_index);
        if self_closing {
            return node_index;
        }

        loop {
            self.skip_space();
            if self.position >= self.bytes.len() {
                self.diagnostics.push(UiDiagnosticKind::UnexpectedEnd, self.position);
                return node_index;
            }
            if self.starts_with(b"</") {
                let close_offset = self.position;
                let close_name = self.parse_close_name();
                if close_name.as_ref().map(UiName::as_bytes) != Some(name.as_bytes()) {
                    self.diagnostics.push(UiDiagnosticKind::MismatchedClose, close_offset);
                }
                return node_index;
            }
            if self.peek() == Some(b'<') {
                self.parse_element(node_index.unwrap_or(parent));
                continue;
            }
            let text_start = self.position;
            while self.position < self.bytes.len() && self.peek() != Some(b'<') {
                self.position += 1;
            }
            let text = trim_space(&self.bytes[text_start..self.position]);
            if text.is_empty() {
                continue;
            }
            if let Some(index) = node_index {
                if let Some(node) = self.document.node_mut(index) {
                    if let Some(value) = UiText::from_bytes(text) {
                        node.text = value;
                    } else {
                        self.diagnostics.push(UiDiagnosticKind::InvalidValue, text_start);
                    }
                }
            } else {
                self.diagnostics.push(UiDiagnosticKind::TextNotAllowed, text_start);
            }
        }
    }

    fn parse_attributes(&mut self, node_index: Option<u16>) -> bool {
        loop {
            self.skip_space();
            if self.starts_with(b"/>") {
                self.position += 2;
                return true;
            }
            if self.consume_byte(b'>') {
                return false;
            }
            let offset = self.position;
            match self.peek() {
                Some(b'{') => self.parse_styles(node_index),
                Some(b'[') => self.parse_binding(node_index),
                Some(b'(') => self.parse_event(node_index),
                Some(b'#') => self.parse_node_name(node_index),
                Some(_) => self.parse_plain_attribute(node_index),
                None => {
                    self.diagnostics.push(UiDiagnosticKind::UnexpectedEnd, offset);
                    return true;
                }
            }
        }
    }

    fn parse_plain_attribute(&mut self, node_index: Option<u16>) {
        let offset = self.position;
        let Some(name) = self.read_name() else {
            self.diagnostics.push(UiDiagnosticKind::UnexpectedToken, offset);
            self.skip_until_attribute_end();
            return;
        };
        self.skip_space();
        if !self.consume_byte(b'=') {
            self.diagnostics.push(UiDiagnosticKind::MissingValue, offset);
            self.skip_until_attribute_end();
            return;
        }
        let Some(value) = self.read_quoted() else {
            self.diagnostics.push(UiDiagnosticKind::InvalidValue, offset);
            return;
        };
        if name.as_bytes() == b"tabIndex" {
            let Some(tab_index) = parse_tab_index(value) else {
                self.diagnostics.push(UiDiagnosticKind::InvalidValue, offset);
                return;
            };
            let Some(index) = node_index else { return };
            let Some(node) = self.document.node_mut(index) else { return };
            if !node.kind.is_interactive() {
                self.diagnostics.push(UiDiagnosticKind::UnknownAttribute, offset);
                return;
            }
            node.tab_index = tab_index;
            return;
        }
        if name.as_bytes() != b"id" {
            self.diagnostics.push(UiDiagnosticKind::UnknownAttribute, offset);
            return;
        }
        let Some(value) = UiName::from_bytes(value) else {
            self.diagnostics.push(UiDiagnosticKind::InvalidValue, offset);
            return;
        };
        if let Some(index) = node_index {
            if let Some(node) = self.document.node_mut(index) {
                node.key = value;
            }
        }
    }

    fn parse_node_name(&mut self, node_index: Option<u16>) {
        let offset = self.position;
        self.position += 1;
        let Some(name) = self.read_name() else {
            self.diagnostics.push(UiDiagnosticKind::UnexpectedToken, offset);
            self.skip_until_attribute_end();
            return;
        };
        if let Some(index) = node_index {
            if let Some(node) = self.document.node_mut(index) {
                node.key = name;
            }
        }
    }

    fn parse_binding(&mut self, node_index: Option<u16>) {
        let offset = self.position;
        self.position += 1;
        let two_way = self.consume_byte(b'(');
        let Some(name) = self.read_name() else {
            self.diagnostics.push(UiDiagnosticKind::UnknownBinding, offset);
            self.skip_until_attribute_end();
            return;
        };
        if (two_way && !self.starts_with(b")]")) || (!two_way && !self.consume_byte(b']')) {
            self.diagnostics.push(UiDiagnosticKind::UnknownBinding, offset);
            self.skip_until_attribute_end();
            return;
        }
        if two_way {
            self.position += 2;
        }
        self.skip_space();
        if !self.consume_byte(b'=') {
            self.diagnostics.push(UiDiagnosticKind::MissingValue, offset);
            return;
        }
        let Some(expression) = self.read_quoted().and_then(UiExpression::from_bytes) else {
            self.diagnostics.push(UiDiagnosticKind::InvalidValue, offset);
            return;
        };
        let Some(index) = node_index else { return };
        let Some(node) = self.document.node(usize::from(index)) else { return };
        let property = match name.as_bytes() {
            b"value" if two_way => Some(UiBindingProperty::Value),
            b"disabled" if !two_way => Some(UiBindingProperty::Disabled),
            b"form" if !two_way && node.kind == UiNodeKind::Form => Some(UiBindingProperty::Form),
            b"control" if !two_way && node.kind == UiNodeKind::TextInput => {
                Some(UiBindingProperty::Control)
            }
            b"canSubmit" if !two_way && node.kind == UiNodeKind::Form => {
                Some(UiBindingProperty::CanSubmit)
            }
            _ => None,
        };
        let Some(property) = property else {
            self.diagnostics.push(UiDiagnosticKind::UnknownBinding, offset);
            return;
        };
        self.add_binding(index, UiBinding { property, expression }, offset);
    }

    fn add_binding(&mut self, node_index: u16, binding: UiBinding, offset: usize) {
        if let Some(node) = self.document.node_mut(node_index) {
            if node.bindings.contains(binding.property) {
                self.diagnostics.push(UiDiagnosticKind::UnknownBinding, offset);
            } else if !node.bindings.push(binding) {
                self.diagnostics.push(UiDiagnosticKind::Capacity, offset);
            }
        }
    }

    fn parse_event(&mut self, node_index: Option<u16>) {
        let offset = self.position;
        self.position += 1;
        let Some(name) = self.read_name() else {
            self.diagnostics.push(UiDiagnosticKind::UnknownEvent, offset);
            self.skip_until_attribute_end();
            return;
        };
        if !self.consume_byte(b')') {
            self.diagnostics.push(UiDiagnosticKind::UnknownEvent, offset);
            self.skip_until_attribute_end();
            return;
        }
        self.skip_space();
        if !self.consume_byte(b'=') {
            self.diagnostics.push(UiDiagnosticKind::MissingValue, offset);
            return;
        }
        let Some(handler) = self.read_quoted().and_then(UiExpression::from_bytes) else {
            self.diagnostics.push(UiDiagnosticKind::InvalidValue, offset);
            return;
        };
        let kind = match name.as_bytes() {
            b"click" => UiEventKind::Click,
            b"submit" => UiEventKind::Submit,
            _ => {
                self.diagnostics.push(UiDiagnosticKind::UnknownEvent, offset);
                return;
            }
        };
        if let Some(index) = node_index {
            if let Some(node) = self.document.node_mut(index) {
                if node.event.is_present() {
                    self.diagnostics.push(UiDiagnosticKind::UnknownEvent, offset);
                } else {
                    node.event = UiEvent { kind, handler };
                }
            }
        }
    }

    fn parse_styles(&mut self, node_index: Option<u16>) {
        let offset = self.position;
        self.position += 1;
        let mut rules = [UiStyleRule::Base(UiStyle::FullHeight); MAX_UI_STYLE_TOKENS];
        let mut rule_count = 0;
        loop {
            self.skip_space();
            if self.consume_byte(b'}') {
                self.skip_space();
                if self.consume_byte(b'=') {
                    let Some(expression) = self.read_quoted().and_then(UiExpression::from_bytes)
                    else {
                        self.diagnostics.push(UiDiagnosticKind::InvalidValue, offset);
                        return;
                    };
                    let Some(UiStyleRule::Base(style)) = (rule_count == 1).then(|| rules[0]) else {
                        self.diagnostics.push(UiDiagnosticKind::InvalidValue, offset);
                        return;
                    };
                    if let Some(index) = node_index {
                        if let Some(node) = self.document.node_mut(index) {
                            if !node
                                .conditional_styles
                                .push(UiConditionalStyle { style, expression })
                            {
                                self.diagnostics.push(UiDiagnosticKind::Capacity, offset);
                            }
                        }
                    }
                    return;
                }
                if let Some(index) = node_index {
                    if let Some(node) = self.document.node_mut(index) {
                        for rule in rules.iter().take(rule_count) {
                            let accepted = match *rule {
                                UiStyleRule::Base(style) => node.styles.push(style),
                                UiStyleRule::Focus(style) => node
                                    .state_styles
                                    .push(UiStateStyle { state: UiStyleState::Focus, style }),
                            };
                            if !accepted {
                                self.diagnostics.push(UiDiagnosticKind::Capacity, offset);
                            }
                        }
                    }
                }
                return;
            }
            let Some(token) = self.read_name() else {
                self.diagnostics.push(UiDiagnosticKind::UnexpectedToken, offset);
                self.skip_until_attribute_end();
                return;
            };
            let Some(rule) = style_rule(token.as_bytes()) else {
                self.diagnostics.push(UiDiagnosticKind::UnknownStyle, offset);
                continue;
            };
            if rule_count == MAX_UI_STYLE_TOKENS {
                self.diagnostics.push(UiDiagnosticKind::Capacity, offset);
                continue;
            }
            rules[rule_count] = rule;
            rule_count += 1;
        }
    }

    fn parse_close_name(&mut self) -> Option<UiName> {
        self.position += 2;
        let name = self.read_name();
        self.skip_space();
        let _ = self.consume_byte(b'>');
        name
    }

    fn read_name(&mut self) -> Option<UiName> {
        let start = self.position;
        while let Some(byte) = self.peek() {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b':') {
                self.position += 1;
            } else {
                break;
            }
        }
        UiName::from_bytes(&self.bytes[start..self.position])
    }

    fn read_quoted(&mut self) -> Option<&[u8]> {
        if !self.consume_byte(b'"') {
            return None;
        }
        let start = self.position;
        while self.position < self.bytes.len() && self.peek() != Some(b'"') {
            self.position += 1;
        }
        let value = &self.bytes[start..self.position];
        let _ = self.consume_byte(b'"');
        Some(value)
    }

    fn skip_until_attribute_end(&mut self) {
        while self.position < self.bytes.len()
            && !matches!(self.peek(), Some(b'>') | Some(b'{') | Some(b'[') | Some(b'('))
        {
            self.position += 1;
        }
    }

    fn skip_space(&mut self) {
        while self.peek().is_some_and(|byte| byte.is_ascii_whitespace()) {
            self.position += 1;
        }
    }

    fn starts_with(&self, value: &[u8]) -> bool {
        self.bytes.get(self.position..).is_some_and(|tail| tail.starts_with(value))
    }

    fn consume_byte(&mut self, value: u8) -> bool {
        if self.peek() == Some(value) {
            self.position += 1;
            true
        } else {
            false
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.position).copied()
    }
}

fn element_kind(name: &[u8]) -> Option<UiNodeKind> {
    match name {
        b"ui.column" | b"ui.panel" => Some(UiNodeKind::Panel),
        b"ui.form" => Some(UiNodeKind::Form),
        b"ui.text" => Some(UiNodeKind::Label),
        b"ui.button" => Some(UiNodeKind::Button),
        b"ui.input" => Some(UiNodeKind::TextInput),
        _ => None,
    }
}

fn parse_tab_index(value: &[u8]) -> Option<i16> {
    if value.is_empty() {
        return None;
    }
    let (negative, digits) = if value[0] == b'-' { (true, &value[1..]) } else { (false, value) };
    if digits.is_empty() {
        return None;
    }
    let mut result = 0i16;
    for byte in digits {
        if !byte.is_ascii_digit() {
            return None;
        }
        result = result.checked_mul(10)?.checked_add(i16::from(byte - b'0'))?;
    }
    if negative { result.checked_neg() } else { Some(result) }
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum UiStyleRule {
    Base(UiStyle),
    Focus(UiStyle),
}

fn style_rule(token: &[u8]) -> Option<UiStyleRule> {
    if let Some(style) = token.strip_prefix(b"focus:").and_then(style_token) {
        return Some(UiStyleRule::Focus(style));
    }
    style_token(token).map(UiStyleRule::Base)
}

fn style_token(token: &[u8]) -> Option<UiStyle> {
    match token {
        b"h-full" => Some(UiStyle::FullHeight),
        b"w-full" => Some(UiStyle::FullWidth),
        b"flex-x" => Some(UiStyle::FlexX),
        b"flex-y" => Some(UiStyle::FlexY),
        b"items-center" => Some(UiStyle::ItemsCenter),
        b"justify-center" => Some(UiStyle::JustifyCenter),
        b"w-96" => Some(UiStyle::Width96),
        b"gap" => Some(UiStyle::Gap(4)),
        b"gap-x" => Some(UiStyle::GapX(4)),
        b"gap-y" => Some(UiStyle::GapY(4)),
        b"mt-4" => Some(UiStyle::MarginTop4),
        b"mb-2" => Some(UiStyle::MarginBottom2),
        b"px-6" => Some(UiStyle::PaddingX6),
        b"py-3" => Some(UiStyle::PaddingY3),
        b"rounded-lg" => Some(UiStyle::RoundedLarge),
        b"bg-accent" => Some(UiStyle::BackgroundAccent),
        b"text-4xl" => Some(UiStyle::Text4xl),
        b"font-light" => Some(UiStyle::FontLight),
        b"opacity-50" => Some(UiStyle::Opacity50),
        _ => spacing_style(token),
    }
}

fn spacing_style(token: &[u8]) -> Option<UiStyle> {
    if let Some(value) = token.strip_prefix(b"gap-x-").and_then(parse_spacing) {
        return Some(UiStyle::GapX(value));
    }
    if let Some(value) = token.strip_prefix(b"gap-y-").and_then(parse_spacing) {
        return Some(UiStyle::GapY(value));
    }
    token.strip_prefix(b"gap-").and_then(parse_spacing).map(UiStyle::Gap)
}

fn parse_spacing(value: &[u8]) -> Option<u8> {
    if value.is_empty() {
        return None;
    }
    let mut result = 0u8;
    for byte in value {
        if !byte.is_ascii_digit() {
            return None;
        }
        result = result.checked_mul(10)?.checked_add(byte - b'0')?;
    }
    (result <= 64).then_some(result)
}

fn trim_space(mut value: &[u8]) -> &[u8] {
    while value.first().is_some_and(u8::is_ascii_whitespace) {
        value = &value[1..];
    }
    while value.last().is_some_and(u8::is_ascii_whitespace) {
        value = &value[..value.len() - 1];
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::string::String;

    const LOGIN_PAGE: &str = include_str!("../examples/login.ui");

    #[test]
    fn lints_and_compiles_the_login_page() {
        let build = compile(LOGIN_PAGE);
        assert!(build.is_valid(), "diagnostics: {:?}", build.diagnostics);
        assert_eq!(build.document.node_count(), 6);
        assert_eq!(build.document.node(0).unwrap().kind, UiNodeKind::Panel);
        assert_eq!(build.document.node(1).unwrap().kind, UiNodeKind::Form);
        let form_bindings = &build.document.node(1).unwrap().bindings;
        assert_eq!(form_bindings.len, 2);
        assert_eq!(form_bindings.entries[0].property, UiBindingProperty::Form);
        assert_eq!(form_bindings.entries[1].property, UiBindingProperty::CanSubmit);
        assert_eq!(build.document.node(1).unwrap().event.kind, UiEventKind::Submit);
        let form_styles = &build.document.node(1).unwrap().styles;
        assert!(form_styles.tokens[..form_styles.len as usize].contains(&UiStyle::FlexY));
        assert!(form_styles.tokens[..form_styles.len as usize].contains(&UiStyle::GapY(4)));
        assert_eq!(build.document.node(2).unwrap().text.as_bytes(), b"LogOS");
        assert_eq!(build.document.node(3).unwrap().kind, UiNodeKind::TextInput);
        let username_states = &build.document.node(3).unwrap().state_styles;
        assert!(username_states.entries[..username_states.len as usize].contains(&UiStateStyle {
            state: UiStyleState::Focus,
            style: UiStyle::BackgroundAccent,
        }));
        assert_eq!(
            build.document.node(3).unwrap().bindings.entries[0].property,
            UiBindingProperty::Control
        );
        assert_eq!(
            build.document.node(4).unwrap().bindings.entries[0].property,
            UiBindingProperty::Control
        );
        assert_eq!(build.document.node(5).unwrap().event.kind, UiEventKind::Submit);
        let submit_conditions = &build.document.node(5).unwrap().conditional_styles;
        assert_eq!(submit_conditions.len, 1);
        assert_eq!(submit_conditions.entries[0].style, UiStyle::Opacity50);
        assert_eq!(submit_conditions.entries[0].expression.as_bytes(), b"failure");
        assert_eq!(lint(LOGIN_PAGE).len(), 0);
        let blueprint = build.document.to_blueprint().unwrap();
        assert_eq!(blueprint.len(), 6);
    }

    #[test]
    fn lints_and_compiles_the_register_page() {
        let build = compile_register_page();
        assert!(build.is_valid(), "diagnostics: {:?}", build.diagnostics);
        assert_eq!(build.document.node_count(), 8);
        assert_eq!(build.document.node(1).unwrap().kind, UiNodeKind::Form);
        assert_eq!(build.document.node(2).unwrap().text.as_bytes(), b"Create administrator");
        assert_eq!(build.document.node(6).unwrap().key.as_bytes(), b"confirmPassword");
        assert_eq!(build.document.node(7).unwrap().event.kind, UiEventKind::Submit);
        assert_eq!(lint(REGISTER_PAGE_SOURCE).len(), 0);
        assert_eq!(build.document.to_blueprint().unwrap().len(), 8);
    }

    #[test]
    fn rejects_unknown_elements_styles_and_bindings() {
        let build = compile(
            r#"<ui.column {not-a-style}>
                <ui.unknown />
                <ui.input [value]="name" />
              </ui.column>"#,
        );
        assert!(!build.is_valid());
        assert!(matches!(
            build.diagnostics.get(0),
            Some(UiDiagnostic { kind: UiDiagnosticKind::UnknownStyle, .. })
        ));
        assert!(matches!(
            build.diagnostics.get(1),
            Some(UiDiagnostic { kind: UiDiagnosticKind::UnknownElement, .. })
        ));
        assert!(matches!(
            build.diagnostics.get(2),
            Some(UiDiagnostic { kind: UiDiagnosticKind::UnknownBinding, .. })
        ));
    }

    #[test]
    fn conditional_styles_use_style_braces_not_property_bindings() {
        let build = compile(r#"<ui.button {opacity-50}="failure"/>"#);
        assert!(build.is_valid(), "diagnostics: {:?}", build.diagnostics);
        let conditionals = &build.document.node(0).unwrap().conditional_styles;
        assert_eq!(conditionals.len, 1);
        assert_eq!(conditionals.entries[0].style, UiStyle::Opacity50);
        assert_eq!(conditionals.entries[0].expression.as_bytes(), b"failure");

        let legacy = compile(r#"<ui.button [opacity-50]="failure"/>"#);
        assert!(!legacy.is_valid());
        assert!(matches!(
            legacy.diagnostics.get(0),
            Some(UiDiagnostic { kind: UiDiagnosticKind::UnknownBinding, .. })
        ));
    }

    #[test]
    fn interactive_components_expose_and_override_tab_index() {
        let build = compile(
            r#"<ui.column>
                <ui.input tabIndex="4" />
                <ui.button tabIndex="-1" />
            </ui.column>"#,
        );
        assert!(build.is_valid(), "diagnostics: {:?}", build.diagnostics);
        assert_eq!(build.document.node(1).unwrap().tab_index, 4);
        assert!(build.document.node(1).unwrap().kind.is_interactive());
        assert_eq!(build.document.node(2).unwrap().tab_index, -1);
        assert!(build.document.node(2).unwrap().kind.is_interactive());
    }

    #[test]
    fn tab_index_is_rejected_on_non_interactive_components() {
        let build = compile(r#"<ui.text tabIndex="1">label</ui.text>"#);
        assert!(!build.is_valid());
        assert!(
            build
                .diagnostics
                .entries
                .iter()
                .any(|diagnostic| diagnostic.kind == UiDiagnosticKind::UnknownAttribute)
        );
    }

    #[test]
    fn form_and_control_bindings_are_limited_to_their_component_kinds() {
        let build = compile(
            r#"<ui.form [form]="loginForm"><ui.input [control]="loginForm.controls.name" /></ui.form>"#,
        );
        assert!(build.is_valid(), "diagnostics: {:?}", build.diagnostics);
        assert_eq!(
            build.document.node(0).unwrap().bindings.entries[0].property,
            UiBindingProperty::Form
        );
        assert_eq!(
            build.document.node(1).unwrap().bindings.entries[0].property,
            UiBindingProperty::Control
        );

        let invalid = compile(r#"<ui.button [form]="loginForm" />"#);
        assert!(!invalid.is_valid());
        assert!(matches!(
            invalid.diagnostics.get(0),
            Some(UiDiagnostic { kind: UiDiagnosticKind::UnknownBinding, .. })
        ));
    }

    #[test]
    fn parses_hash_node_names_and_rejects_key_attributes() {
        let build = compile(r#"<ui.form #loginForm><ui.input #username /></ui.form>"#);
        assert!(build.is_valid(), "diagnostics: {:?}", build.diagnostics);
        assert_eq!(build.document.node(0).unwrap().key.as_bytes(), b"loginForm");
        assert_eq!(build.document.node(1).unwrap().key.as_bytes(), b"username");

        let legacy = compile(r#"<ui.form key="loginForm" />"#);
        assert!(!legacy.is_valid());
        assert!(matches!(
            legacy.diagnostics.get(0),
            Some(UiDiagnostic { kind: UiDiagnosticKind::UnknownAttribute, .. })
        ));
    }

    #[test]
    fn compiles_flex_and_axis_specific_gap_utilities() {
        let build = compile(r#"<ui.column {flex-x gap-2 gap-x-3 gap-y-4}/>"#);
        assert!(build.is_valid(), "diagnostics: {:?}", build.diagnostics);
        let styles = &build.document.node(0).unwrap().styles;
        let styles = &styles.tokens[..styles.len as usize];
        assert!(styles.contains(&UiStyle::FlexX));
        assert!(styles.contains(&UiStyle::Gap(2)));
        assert!(styles.contains(&UiStyle::GapX(3)));
        assert!(styles.contains(&UiStyle::GapY(4)));
    }

    #[test]
    fn enforces_source_and_node_bounds() {
        let too_large = "x".repeat(MAX_UI_SOURCE_BYTES + 1);
        let report = compile(&too_large);
        assert_eq!(report.diagnostics.get(0).unwrap().kind, UiDiagnosticKind::SourceTooLarge);

        let mut source = String::from("<ui.column>");
        for _ in 0..MAX_UI_NODES {
            source.push_str("<ui.text />");
        }
        source.push_str("</ui.column>");
        let report = compile(&source);
        assert!(
            report.diagnostics.get(0).is_some_and(|item| item.kind == UiDiagnosticKind::Capacity)
        );
    }
}
