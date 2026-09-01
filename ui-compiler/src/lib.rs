#![no_std]

#[cfg(test)]
extern crate std;

mod codegen;

use logos_ui::{
    UiAnimationDirection, UiAnimationFill, UiAnimationPreset, UiComponentContract, UiComputedStyle,
    UiEasing, UiKeyframe, UiNodeKind, UiTransform, UiValueType,
};

pub use codegen::{UiCodegenError, write_rust};

pub use logos_ui::{
    MAX_UI_BINDINGS, MAX_UI_CONDITIONAL_STYLES, MAX_UI_EXPRESSION_BYTES, MAX_UI_NAME_BYTES,
    MAX_UI_STATE_STYLES, MAX_UI_STYLE_CONDITIONS, MAX_UI_STYLE_TOKENS, MAX_UI_TEXT_BYTES,
    UiBinding, UiBindingList, UiBindingProperty, UiConditionalStyle, UiConditionalStyleList,
    UiDocument, UiEvent, UiEventKind, UiExpression, UiName, UiNodeTemplate, UiStateStyle,
    UiStateStyleList, UiStyle, UiStyleConditions, UiStyleList, UiStyleResolveError, UiStyleState,
    UiText,
};

pub const MAX_UI_SOURCE_BYTES: usize = 4096;
pub const MAX_UI_DIAGNOSTICS: usize = 16;
pub const MAX_UI_ANIMATIONS: usize = 8;
pub const MAX_UI_HANDLERS: usize = 32;
pub const MAX_UI_COMPONENT_CONTRACTS: usize = 16;
pub const MAX_UI_VALUES: usize = 64;

pub const UI_COMPONENT_NAMES: [&str; 5] =
    ["ui.button", "ui.column", "ui.form", "ui.input", "ui.text"];
pub const UI_STYLE_NAMES: [&str; 39] = [
    "h-full",
    "w-full",
    "flex-x",
    "flex-y",
    "items-center",
    "justify-center",
    "w-96",
    "gap",
    "gap-x",
    "gap-y",
    "mt-4",
    "mb-2",
    "px-6",
    "py-3",
    "rounded-lg",
    "rounded-full",
    "bg-accent",
    "text-4xl",
    "font-light",
    "opacity-50",
    "transition-colors",
    "transition-opacity",
    "transition-transform",
    "transition-all",
    "duration-75",
    "duration-150",
    "duration-180",
    "duration-300",
    "duration-700",
    "duration-1000",
    "delay-75",
    "ease-linear",
    "ease-out",
    "ease-in-out",
    "ease-bezier-20-80-20-100",
    "animate-in",
    "fade",
    "animate-pulse",
    "animate-spin",
];
pub const UI_BINDING_NAMES: [&str; 5] = ["value", "disabled", "form", "control", "canSubmit"];
pub const UI_EVENT_NAMES: [&str; 3] = ["click", "submit", "changed"];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum UiDiagnosticKind {
    SourceTooLarge = 1,
    UnexpectedToken = 2,
    UnexpectedEnd = 3,
    UnknownElement = 4,
    UnknownAttribute = 5,
    UnknownBinding = 6,
    UnknownEvent = 7,
    UnknownStyle = 8,
    MissingValue = 9,
    InvalidValue = 10,
    MismatchedClose = 11,
    TextNotAllowed = 12,
    Capacity = 13,
    ReadOnlyBinding = 14,
    InvalidEventHandler = 15,
    EventPayloadMismatch = 16,
    UnknownEventHandler = 17,
    EventHandlerTypeMismatch = 18,
    UnknownBindingExpression = 19,
    BindingTypeMismatch = 20,
    StyleConditionTypeMismatch = 21,
    UnknownStyleExpression = 22,
    InvalidInterpolation = 23,
    UnknownTextExpression = 24,
    TextExpressionTypeMismatch = 25,
    DuplicateNodeName = 26,
}

impl UiDiagnosticKind {
    pub const fn code(self) -> &'static str {
        match self {
            Self::SourceTooLarge => "UI001",
            Self::UnexpectedToken => "UI002",
            Self::UnexpectedEnd => "UI003",
            Self::UnknownElement => "UI004",
            Self::UnknownAttribute => "UI005",
            Self::UnknownBinding => "UI006",
            Self::UnknownEvent => "UI007",
            Self::UnknownStyle => "UI008",
            Self::MissingValue => "UI009",
            Self::InvalidValue => "UI010",
            Self::MismatchedClose => "UI011",
            Self::TextNotAllowed => "UI012",
            Self::Capacity => "UI013",
            Self::ReadOnlyBinding => "UI014",
            Self::InvalidEventHandler => "UI015",
            Self::EventPayloadMismatch => "UI016",
            Self::UnknownEventHandler => "UI017",
            Self::EventHandlerTypeMismatch => "UI018",
            Self::UnknownBindingExpression => "UI019",
            Self::BindingTypeMismatch => "UI020",
            Self::StyleConditionTypeMismatch => "UI021",
            Self::UnknownStyleExpression => "UI022",
            Self::InvalidInterpolation => "UI023",
            Self::UnknownTextExpression => "UI024",
            Self::TextExpressionTypeMismatch => "UI025",
            Self::DuplicateNodeName => "UI026",
        }
    }

    pub const fn message(self) -> &'static str {
        match self {
            Self::SourceTooLarge => "UI source exceeds the bounded source limit",
            Self::UnexpectedToken => "unexpected token",
            Self::UnexpectedEnd => "unexpected end of UI source",
            Self::UnknownElement => "element is not in scope",
            Self::UnknownAttribute => "attribute is not supported",
            Self::UnknownBinding => "binding is not supported for this component",
            Self::UnknownEvent => "event is not supported for this component",
            Self::UnknownStyle => "style utility is not recognized",
            Self::MissingValue => "attribute or binding requires a value",
            Self::InvalidValue => "value is invalid for this component",
            Self::MismatchedClose => "closing element does not match the open element",
            Self::TextNotAllowed => "text is not allowed in this component",
            Self::Capacity => "UI document exceeds a bounded compiler limit",
            Self::ReadOnlyBinding => "two-way binding requires a writable component input",
            Self::InvalidEventHandler => {
                "event handler must be a bounded method or call expression"
            }
            Self::EventPayloadMismatch => "$event is only available on changed handlers",
            Self::UnknownEventHandler => "event handler is not registered for this component",
            Self::EventHandlerTypeMismatch => {
                "event handler is registered for a different event payload"
            }
            Self::UnknownBindingExpression => "binding expression is not registered",
            Self::BindingTypeMismatch => "binding expression has the wrong value type",
            Self::StyleConditionTypeMismatch => "conditional style expression must be boolean",
            Self::UnknownStyleExpression => "conditional style expression is not registered",
            Self::InvalidInterpolation => "text interpolation must be a bounded whole expression",
            Self::UnknownTextExpression => "text interpolation expression is not registered",
            Self::TextExpressionTypeMismatch => "text interpolation expression must be text",
            Self::DuplicateNodeName => "node name is already used in this document",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiHandlerSpec {
    pub expression: UiExpression,
    pub event: UiEventKind,
}

impl UiHandlerSpec {
    pub const fn new(expression: UiExpression, event: UiEventKind) -> Self {
        Self { expression, event }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UiHandlerRegistryError {
    InvalidExpression,
    Capacity,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiHandlerRegistry {
    entries: [UiHandlerSpec; MAX_UI_HANDLERS],
    count: usize,
}

impl UiHandlerRegistry {
    pub const fn new() -> Self {
        Self {
            entries: [UiHandlerSpec::new(UiExpression::EMPTY, UiEventKind::Click); MAX_UI_HANDLERS],
            count: 0,
        }
    }

    pub const fn len(&self) -> usize {
        self.count
    }

    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }

    pub fn register_bytes(
        &mut self,
        expression: &[u8],
        event: UiEventKind,
    ) -> Result<(), UiHandlerRegistryError> {
        let expression = UiExpression::from_bytes(expression)
            .ok_or(UiHandlerRegistryError::InvalidExpression)?;
        self.register(expression, event)
    }

    pub fn register(
        &mut self,
        expression: UiExpression,
        event: UiEventKind,
    ) -> Result<(), UiHandlerRegistryError> {
        if expression.as_bytes().is_empty() {
            return Err(UiHandlerRegistryError::InvalidExpression);
        }
        let spec = UiHandlerSpec::new(expression, event);
        if self.entries[..self.count].contains(&spec) {
            return Ok(());
        }
        if self.count == MAX_UI_HANDLERS {
            return Err(UiHandlerRegistryError::Capacity);
        }
        self.entries[self.count] = spec;
        self.count += 1;
        Ok(())
    }

    fn contains_expression(&self, expression: UiExpression) -> bool {
        self.entries[..self.count].iter().any(|spec| spec.expression == expression)
    }

    fn accepts(&self, expression: UiExpression, event: UiEventKind) -> bool {
        self.entries[..self.count].contains(&UiHandlerSpec::new(expression, event))
    }
}

impl Default for UiHandlerRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UiComponentRegistryError {
    InvalidName,
    DuplicateName,
    Capacity,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiComponentRegistry {
    entries: [UiComponentContract; MAX_UI_COMPONENT_CONTRACTS],
    count: usize,
}

impl UiComponentRegistry {
    pub const fn new() -> Self {
        Self { entries: [UiComponentContract::EMPTY; MAX_UI_COMPONENT_CONTRACTS], count: 0 }
    }

    pub const fn builtins() -> Self {
        let mut registry = Self::new();
        registry.entries[0] = UiComponentContract::for_kind(UiNodeKind::Button);
        registry.entries[1] = UiComponentContract::for_kind(UiNodeKind::TextInput);
        registry.entries[2] = UiComponentContract::for_kind(UiNodeKind::Form);
        registry.entries[3] = UiComponentContract::for_kind(UiNodeKind::Root);
        registry.entries[4] = UiComponentContract::for_kind(UiNodeKind::Panel);
        registry.entries[5] = UiComponentContract::for_kind(UiNodeKind::Label);
        registry.entries[6] = UiComponentContract::new("ui.column", UiNodeKind::Panel, false);
        registry.count = 7;
        registry
    }

    pub const fn len(&self) -> usize {
        self.count
    }

    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }

    pub fn register(
        &mut self,
        contract: UiComponentContract,
    ) -> Result<(), UiComponentRegistryError> {
        let name = contract.name.as_bytes();
        if !valid_component_name(name) {
            return Err(UiComponentRegistryError::InvalidName);
        }
        if self.entries[..self.count].iter().any(|entry| entry.name == contract.name) {
            return Err(UiComponentRegistryError::DuplicateName);
        }
        if self.count == MAX_UI_COMPONENT_CONTRACTS {
            return Err(UiComponentRegistryError::Capacity);
        }
        self.entries[self.count] = contract;
        self.count += 1;
        Ok(())
    }

    pub fn resolve(&self, name: &[u8]) -> Option<UiComponentContract> {
        self.entries[..self.count].iter().copied().find(|contract| contract.name.as_bytes() == name)
    }
}

impl Default for UiComponentRegistry {
    fn default() -> Self {
        Self::builtins()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiValueSpec {
    pub expression: UiExpression,
    pub value_type: UiValueType,
    pub writable: bool,
}

impl UiValueSpec {
    pub const fn new(expression: UiExpression, value_type: UiValueType, writable: bool) -> Self {
        Self { expression, value_type, writable }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UiValueRegistryError {
    InvalidExpression,
    DuplicateExpression,
    Capacity,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiValueRegistry {
    entries: [UiValueSpec; MAX_UI_VALUES],
    count: usize,
}

impl UiValueRegistry {
    pub const fn new() -> Self {
        Self {
            entries: [UiValueSpec::new(UiExpression::EMPTY, UiValueType::Unit, false);
                MAX_UI_VALUES],
            count: 0,
        }
    }

    pub const fn len(&self) -> usize {
        self.count
    }

    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }

    pub fn register_bytes(
        &mut self,
        expression: &[u8],
        value_type: UiValueType,
        writable: bool,
    ) -> Result<(), UiValueRegistryError> {
        let expression =
            UiExpression::from_bytes(expression).ok_or(UiValueRegistryError::InvalidExpression)?;
        self.register(expression, value_type, writable)
    }

    pub fn register(
        &mut self,
        expression: UiExpression,
        value_type: UiValueType,
        writable: bool,
    ) -> Result<(), UiValueRegistryError> {
        if expression.as_bytes().is_empty() {
            return Err(UiValueRegistryError::InvalidExpression);
        }
        if self.entries[..self.count].iter().any(|entry| entry.expression == expression) {
            return Err(UiValueRegistryError::DuplicateExpression);
        }
        if self.count == MAX_UI_VALUES {
            return Err(UiValueRegistryError::Capacity);
        }
        self.entries[self.count] = UiValueSpec::new(expression, value_type, writable);
        self.count += 1;
        Ok(())
    }

    pub fn resolve(&self, expression: UiExpression) -> Option<UiValueSpec> {
        self.entries[..self.count].iter().copied().find(|entry| entry.expression == expression)
    }
}

impl Default for UiValueRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy)]
pub struct UiCompileContext<'a> {
    component_registry: &'a UiComponentRegistry,
    handler_registry: Option<&'a UiHandlerRegistry>,
    value_registry: Option<&'a UiValueRegistry>,
}

impl<'a> UiCompileContext<'a> {
    pub const fn new(component_registry: &'a UiComponentRegistry) -> Self {
        Self { component_registry, handler_registry: None, value_registry: None }
    }

    pub const fn with_handlers(mut self, handler_registry: &'a UiHandlerRegistry) -> Self {
        self.handler_registry = Some(handler_registry);
        self
    }

    pub const fn with_values(mut self, value_registry: &'a UiValueRegistry) -> Self {
        self.value_registry = Some(value_registry);
        self
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiSourceSpan {
    pub start: u16,
    pub length: u16,
}

impl UiSourceSpan {
    pub const fn point(start: usize) -> Self {
        let start = if start > u16::MAX as usize { u16::MAX } else { start as u16 };
        Self { start, length: 1 }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiDiagnostic {
    pub kind: UiDiagnosticKind,
    pub span: UiSourceSpan,
}

impl UiDiagnostic {
    pub const fn code(self) -> &'static str {
        self.kind.code()
    }

    pub const fn message(self) -> &'static str {
        self.kind.message()
    }

    pub const fn offset(self) -> usize {
        self.span.start as usize
    }

    pub fn line_column(self, source: &str) -> (usize, usize) {
        let offset = self.offset().min(source.len());
        let prefix = &source.as_bytes()[..offset];
        let line = prefix.iter().filter(|byte| **byte == b'\n').count() + 1;
        let column = prefix.iter().rev().take_while(|byte| **byte != b'\n').count() + 1;
        (line, column)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiDiagnostics {
    entries: [UiDiagnostic; MAX_UI_DIAGNOSTICS],
    count: usize,
}

impl UiDiagnostics {
    const fn new() -> Self {
        Self {
            entries: [UiDiagnostic {
                kind: UiDiagnosticKind::UnexpectedToken,
                span: UiSourceSpan::point(0),
            }; MAX_UI_DIAGNOSTICS],
            count: 0,
        }
    }

    const EMPTY: Self = Self {
        entries: [UiDiagnostic {
            kind: UiDiagnosticKind::UnexpectedToken,
            span: UiSourceSpan::point(0),
        }; MAX_UI_DIAGNOSTICS],
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
        self.entries[self.count] = UiDiagnostic { kind, span: UiSourceSpan::point(offset) };
        self.count += 1;
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiBuild {
    pub document: UiDocument,
    pub diagnostics: UiDiagnostics,
}

impl UiBuild {
    pub const fn from_document(document: UiDocument) -> Self {
        Self { document, diagnostics: UiDiagnostics::new() }
    }

    pub const fn is_valid(&self) -> bool {
        self.diagnostics.is_empty() && self.document.node_count() != 0
    }
}

pub fn lint(source: &str) -> UiDiagnostics {
    compile(source).diagnostics
}

pub fn lint_with_handlers(source: &str, handlers: &UiHandlerRegistry) -> UiDiagnostics {
    compile_with_handlers(source, handlers).diagnostics
}

pub fn lint_with_context(source: &str, context: &UiCompileContext<'_>) -> UiDiagnostics {
    compile_with_context(source, context).diagnostics
}

pub fn lint_with_values(source: &str, values: &UiValueRegistry) -> UiDiagnostics {
    compile_with_values(source, values).diagnostics
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
    let components = UiComponentRegistry::builtins();
    let context = UiCompileContext::new(&components);
    compile_with_context(source, &context)
}

pub fn compile_with_handlers(source: &str, handlers: &UiHandlerRegistry) -> UiBuild {
    let components = UiComponentRegistry::builtins();
    let context = UiCompileContext::new(&components).with_handlers(handlers);
    compile_with_context(source, &context)
}

pub fn compile_with_components(source: &str, components: &UiComponentRegistry) -> UiBuild {
    let context = UiCompileContext::new(components);
    compile_with_context(source, &context)
}

pub fn compile_with_values(source: &str, values: &UiValueRegistry) -> UiBuild {
    let components = UiComponentRegistry::builtins();
    let context = UiCompileContext::new(&components).with_values(values);
    compile_with_context(source, &context)
}

pub fn compile_with_context(source: &str, context: &UiCompileContext<'_>) -> UiBuild {
    if source.len() > MAX_UI_SOURCE_BYTES {
        let mut diagnostics = UiDiagnostics::EMPTY;
        diagnostics.push(UiDiagnosticKind::SourceTooLarge, MAX_UI_SOURCE_BYTES);
        return UiBuild { document: UiDocument::EMPTY, diagnostics };
    }
    let mut parser = Parser {
        bytes: source.as_bytes(),
        component_registry: context.component_registry,
        handler_registry: context.handler_registry,
        value_registry: context.value_registry,
        position: 0,
        document: UiDocument::EMPTY,
        diagnostics: UiDiagnostics::EMPTY,
        animation_count: 0,
    };
    parser.parse_document();
    UiBuild { document: parser.document, diagnostics: parser.diagnostics }
}

struct Parser<'a> {
    bytes: &'a [u8],
    component_registry: &'a UiComponentRegistry,
    handler_registry: Option<&'a UiHandlerRegistry>,
    value_registry: Option<&'a UiValueRegistry>,
    position: usize,
    document: UiDocument,
    diagnostics: UiDiagnostics,
    animation_count: usize,
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
        let contract = self.component_registry.resolve(name.as_bytes());
        if contract.is_none() {
            self.diagnostics.push(UiDiagnosticKind::UnknownElement, start);
        }
        let node_index = if let Some(contract) = contract {
            let kind = contract.kind;
            let node = UiNodeTemplate {
                kind,
                parent,
                tab_index: if kind.is_interactive() { 0 } else { logos_ui::TAB_INDEX_NONE },
                ..UiNodeTemplate::EMPTY
            };
            let index = self.document.push_node(node);
            if index.is_none() {
                self.diagnostics.push(UiDiagnosticKind::Capacity, start);
            }
            index
        } else {
            None
        };

        let self_closing = self.parse_attributes(node_index, contract);
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
            self.apply_text(node_index, text, text_start);
        }
    }

    fn apply_text(&mut self, node_index: Option<u16>, text: &[u8], offset: usize) {
        if has_interpolation_marker(text) {
            if !text.starts_with(b"{{") || !text.ends_with(b"}}") {
                self.diagnostics.push(UiDiagnosticKind::InvalidInterpolation, offset);
                return;
            }
            let expression_bytes = trim_space(&text[2..text.len() - 2]);
            let Some(expression) = UiExpression::from_bytes(expression_bytes) else {
                self.diagnostics.push(UiDiagnosticKind::InvalidInterpolation, offset);
                return;
            };
            if let Some(registry) = self.value_registry {
                let Some(value) = registry.resolve(expression) else {
                    self.diagnostics.push(UiDiagnosticKind::UnknownTextExpression, offset);
                    return;
                };
                if value.value_type != UiValueType::Text {
                    self.diagnostics.push(UiDiagnosticKind::TextExpressionTypeMismatch, offset);
                    return;
                }
            }
            let Some(index) = node_index else {
                self.diagnostics.push(UiDiagnosticKind::TextNotAllowed, offset);
                return;
            };
            if let Some(node) = self.document.node_mut(index) {
                node.text_binding = expression;
            }
            return;
        }
        if let Some(index) = node_index {
            if let Some(node) = self.document.node_mut(index) {
                if let Some(value) = UiText::from_bytes(text) {
                    node.text = value;
                } else {
                    self.diagnostics.push(UiDiagnosticKind::InvalidValue, offset);
                }
            }
        } else {
            self.diagnostics.push(UiDiagnosticKind::TextNotAllowed, offset);
        }
    }

    fn parse_attributes(
        &mut self,
        node_index: Option<u16>,
        contract: Option<UiComponentContract>,
    ) -> bool {
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
                Some(b'a') if self.starts_with(b"animation") => self.parse_animation(node_index),
                Some(b'[') => self.parse_binding(node_index, contract),
                Some(b'(') => self.parse_event(node_index, contract),
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
            self.assign_node_name(index, value, offset);
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
            self.assign_node_name(index, name, offset);
        }
    }

    fn assign_node_name(&mut self, index: u16, name: UiName, offset: usize) {
        let duplicate = (0..self.document.node_count()).any(|candidate| {
            candidate != usize::from(index)
                && self.document.node(candidate).is_some_and(|node| node.key == name)
        });
        if duplicate {
            self.diagnostics.push(UiDiagnosticKind::DuplicateNodeName, offset);
            return;
        }
        if let Some(node) = self.document.node_mut(index) {
            node.key = name;
        }
    }

    fn parse_binding(&mut self, node_index: Option<u16>, contract: Option<UiComponentContract>) {
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
        let Some(contract) = contract else { return };
        let Some(input) = contract.input(name.as_bytes()) else {
            self.diagnostics.push(UiDiagnosticKind::UnknownBinding, offset);
            return;
        };
        if two_way && !input.writable {
            self.diagnostics.push(UiDiagnosticKind::ReadOnlyBinding, offset);
            return;
        }
        if let Some(registry) = self.value_registry {
            let Some(value) = registry.resolve(expression) else {
                self.diagnostics.push(UiDiagnosticKind::UnknownBindingExpression, offset);
                return;
            };
            if value.value_type != input.value_type {
                self.diagnostics.push(UiDiagnosticKind::BindingTypeMismatch, offset);
                return;
            }
            if two_way && !value.writable {
                self.diagnostics.push(UiDiagnosticKind::ReadOnlyBinding, offset);
                return;
            }
        }
        let property = match name.as_bytes() {
            b"value" if node.kind == UiNodeKind::TextInput => Some(UiBindingProperty::Value),
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

    fn parse_event(&mut self, node_index: Option<u16>, contract: Option<UiComponentContract>) {
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
        let Some(contract) = contract else { return };
        if contract.output(name.as_bytes()).is_none() {
            self.diagnostics.push(UiDiagnosticKind::UnknownEvent, offset);
            return;
        }
        let kind = match name.as_bytes() {
            b"click" => UiEventKind::Click,
            b"submit" => UiEventKind::Submit,
            b"changed" => UiEventKind::Changed,
            _ => {
                self.diagnostics.push(UiDiagnosticKind::UnknownEvent, offset);
                return;
            }
        };
        if let Err(diagnostic) = validate_event_handler(kind, handler.as_bytes()) {
            self.diagnostics.push(diagnostic, offset);
            return;
        }
        if let Some(registry) = self.handler_registry {
            if !registry.accepts(handler, kind) {
                let diagnostic = if registry.contains_expression(handler) {
                    UiDiagnosticKind::EventHandlerTypeMismatch
                } else {
                    UiDiagnosticKind::UnknownEventHandler
                };
                self.diagnostics.push(diagnostic, offset);
                return;
            }
        }
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
                    if let Some(registry) = self.value_registry {
                        let Some(value) = registry.resolve(expression) else {
                            self.diagnostics.push(UiDiagnosticKind::UnknownStyleExpression, offset);
                            return;
                        };
                        if value.value_type != UiValueType::Bool {
                            self.diagnostics
                                .push(UiDiagnosticKind::StyleConditionTypeMismatch, offset);
                            return;
                        }
                    }
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
                                UiStyleRule::State(state, style) => {
                                    node.state_styles.push(UiStateStyle { state, style })
                                }
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

    fn parse_animation(&mut self, node_index: Option<u16>) {
        let offset = self.position;
        self.position += b"animation".len();
        self.skip_space();
        if !self.consume_byte(b'{') {
            self.diagnostics.push(UiDiagnosticKind::InvalidValue, offset);
            return;
        }
        if self.animation_count == MAX_UI_ANIMATIONS {
            self.diagnostics.push(UiDiagnosticKind::Capacity, offset);
            self.skip_balanced_block();
            return;
        }
        let mut spec = logos_ui::UiAnimationSpec::EMPTY;
        loop {
            self.skip_space();
            if self.consume_byte(b'}') {
                break;
            }
            let mut name_buffer = [0u8; 32];
            let name_len = {
                let Some(name) = self.read_until(b'{', b'}').map(trim_space) else {
                    self.diagnostics.push(UiDiagnosticKind::InvalidValue, offset);
                    self.skip_balanced_block();
                    return;
                };
                if name.len() > name_buffer.len() {
                    self.diagnostics.push(UiDiagnosticKind::InvalidValue, offset);
                    self.skip_balanced_block();
                    return;
                }
                name_buffer[..name.len()].copy_from_slice(name);
                name.len()
            };
            let name = &name_buffer[..name_len];
            let keyframe_offset = parse_keyframe_offset(name);
            let keyframe = self.peek() == Some(b'{');
            let property = match name {
                b"duration" => 0,
                b"delay" => 1,
                b"repeat" => 2,
                b"direction" => 3,
                b"fill" => 4,
                b"ease" => 5,
                _ => 6,
            };
            self.skip_space();
            if keyframe {
                self.position += 1;
                let Some(offset_q16) = keyframe_offset else {
                    self.diagnostics.push(UiDiagnosticKind::InvalidValue, offset);
                    self.skip_balanced_block();
                    return;
                };
                let mut style = UiComputedStyle::DEFAULT;
                let mut properties = 0;
                let mut easing = UiEasing::EaseOut;
                if !self.parse_keyframe_body(&mut style, &mut properties, &mut easing, offset) {
                    return;
                }
                let keyframe = UiKeyframe { offset_q16, properties, style, easing };
                if !spec.push(keyframe) {
                    self.diagnostics.push(UiDiagnosticKind::Capacity, offset);
                    return;
                }
                continue;
            }
            if !self.consume_byte(b':') {
                self.diagnostics.push(UiDiagnosticKind::InvalidValue, offset);
                self.skip_balanced_block();
                return;
            }
            let Some(value) = self.read_until(b';', b'}').map(trim_space) else {
                self.diagnostics.push(UiDiagnosticKind::InvalidValue, offset);
                return;
            };
            let valid = match property {
                0 => parse_ms(value).map(|value| spec.duration_ms = value).is_some(),
                1 => parse_ms(value).map(|value| spec.delay_ms = value).is_some(),
                2 => parse_bounded_repeat(value).map(|value| spec.repeat = value).is_some(),
                3 => parse_direction(value).map(|value| spec.direction = value).is_some(),
                4 => parse_fill(value).map(|value| spec.fill = value).is_some(),
                5 => parse_easing(value).map(|value| spec.easing = value).is_some(),
                _ => false,
            };
            if !valid {
                self.diagnostics.push(UiDiagnosticKind::InvalidValue, offset);
                return;
            }
            if self.peek() == Some(b';') {
                self.position += 1;
            } else if self.peek() == Some(b'}') {
                self.position += 1;
                break;
            }
        }
        if !spec.is_valid() {
            self.diagnostics.push(UiDiagnosticKind::InvalidValue, offset);
            return;
        }
        self.animation_count += 1;
        if let Some(index) = node_index {
            if let Some(node) = self.document.node_mut(index) {
                node.animation = spec;
            }
        }
    }

    fn parse_keyframe_body(
        &mut self,
        style: &mut UiComputedStyle,
        properties: &mut u8,
        easing: &mut UiEasing,
        offset: usize,
    ) -> bool {
        loop {
            self.skip_space();
            if self.consume_byte(b'}') {
                return true;
            }
            let Some(name) = self.read_name() else {
                self.diagnostics.push(UiDiagnosticKind::InvalidValue, offset);
                self.skip_balanced_block();
                return false;
            };
            let name_bytes = name.as_bytes().strip_suffix(b":").unwrap_or(name.as_bytes());
            self.skip_space();
            if name_bytes == name.as_bytes() && !self.consume_byte(b':') {
                self.diagnostics.push(UiDiagnosticKind::InvalidValue, offset);
                self.skip_balanced_block();
                return false;
            }
            let Some(value) = self.read_until(b';', b'}').map(trim_space) else {
                self.diagnostics.push(UiDiagnosticKind::InvalidValue, offset);
                return false;
            };
            let valid = match name_bytes {
                b"opacity" => parse_ratio(value)
                    .map(|value| {
                        style.opacity_q16 = value;
                        *properties |= 1 << 1;
                    })
                    .is_some(),
                b"transform" => parse_transform(value)
                    .map(|value| {
                        style.transform = value;
                        *properties |= 1 << 2;
                    })
                    .is_some(),
                b"ease" => parse_easing(value).map(|value| *easing = value).is_some(),
                _ => false,
            };
            if !valid {
                self.diagnostics.push(UiDiagnosticKind::InvalidValue, offset);
                return false;
            }
            if self.peek() == Some(b';') {
                self.position += 1;
            } else if self.peek() == Some(b'}') {
                self.position += 1;
                return true;
            }
        }
    }

    fn read_until(&mut self, first: u8, second: u8) -> Option<&[u8]> {
        let start = self.position;
        while self.position < self.bytes.len()
            && self.peek() != Some(first)
            && self.peek() != Some(second)
            && self.peek() != Some(b':')
            && self.peek() != Some(b';')
        {
            self.position += 1;
        }
        if self.position == self.bytes.len() {
            return None;
        }
        let value = &self.bytes[start..self.position];
        Some(value)
    }

    fn skip_balanced_block(&mut self) {
        let mut depth = 0usize;
        while let Some(byte) = self.peek() {
            self.position += 1;
            match byte {
                b'{' => depth += 1,
                b'}' if depth == 0 => break,
                b'}' => depth -= 1,
                _ => {}
            }
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

fn validate_event_handler(kind: UiEventKind, expression: &[u8]) -> Result<(), UiDiagnosticKind> {
    if kind != UiEventKind::Changed && contains_event_placeholder(expression) {
        return Err(UiDiagnosticKind::EventPayloadMismatch);
    }

    let mut parser = HandlerParser { expression, position: 0 };
    parser.skip_space();
    if !parser.read_identifier() {
        return Err(UiDiagnosticKind::InvalidEventHandler);
    }
    while parser.consume(b'.') {
        if !parser.read_identifier() {
            return Err(UiDiagnosticKind::InvalidEventHandler);
        }
    }
    parser.skip_space();
    if parser.consume(b'(') {
        parser.skip_space();
        if parser.starts_with(b"$event") {
            if kind != UiEventKind::Changed {
                return Err(UiDiagnosticKind::EventPayloadMismatch);
            }
            parser.position += b"$event".len();
            parser.skip_space();
        }
        if !parser.consume(b')') {
            return Err(UiDiagnosticKind::InvalidEventHandler);
        }
        parser.skip_space();
    }
    if parser.position != expression.len() {
        return Err(UiDiagnosticKind::InvalidEventHandler);
    }
    Ok(())
}

fn contains_event_placeholder(expression: &[u8]) -> bool {
    expression.windows(b"$event".len()).any(|window| window == b"$event")
}

struct HandlerParser<'a> {
    expression: &'a [u8],
    position: usize,
}

impl HandlerParser<'_> {
    fn read_identifier(&mut self) -> bool {
        let Some(first) = self.expression.get(self.position).copied() else { return false };
        if !first.is_ascii_alphabetic() && first != b'_' {
            return false;
        }
        self.position += 1;
        while self
            .expression
            .get(self.position)
            .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
        {
            self.position += 1;
        }
        true
    }

    fn skip_space(&mut self) {
        while self.expression.get(self.position).is_some_and(u8::is_ascii_whitespace) {
            self.position += 1;
        }
    }

    fn starts_with(&self, value: &[u8]) -> bool {
        self.expression.get(self.position..).is_some_and(|tail| tail.starts_with(value))
    }

    fn consume(&mut self, value: u8) -> bool {
        if self.expression.get(self.position).copied() == Some(value) {
            self.position += 1;
            true
        } else {
            false
        }
    }
}

fn valid_component_name(name: &[u8]) -> bool {
    !name.is_empty()
        && name.len() <= MAX_UI_NAME_BYTES
        && name
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(*byte, b'.' | b'_' | b'-' | b':'))
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

fn parse_keyframe_offset(value: &[u8]) -> Option<u16> {
    if value == b"from" {
        return Some(0);
    }
    if value == b"to" {
        return Some(u16::MAX);
    }
    let value = value.strip_suffix(b"%")?;
    let percent =
        if value.contains(&b'.') { parse_decimal(value, 100)? } else { parse_unsigned(value)? };
    (percent <= 100).then_some((percent * 65_535 / 100) as u16)
}

fn parse_ms(value: &[u8]) -> Option<u16> {
    let value = value.strip_suffix(b"ms")?;
    let parsed = parse_unsigned(value)?;
    (parsed <= u32::from(logos_ui::MAX_UI_MOTION_DURATION_MS)).then_some(parsed as u16)
}

fn parse_u8(value: &[u8]) -> Option<u8> {
    let value = parse_unsigned(value)?;
    (value <= u32::from(u8::MAX)).then_some(value as u8)
}

fn parse_bounded_repeat(value: &[u8]) -> Option<u8> {
    let value = parse_u8(value)?;
    (value <= 8).then_some(value)
}

fn parse_unsigned(value: &[u8]) -> Option<u32> {
    if value.is_empty() {
        return None;
    }
    let mut result = 0u32;
    for byte in value {
        if !byte.is_ascii_digit() {
            return None;
        }
        result = result.checked_mul(10)?.checked_add(u32::from(byte - b'0'))?;
    }
    Some(result)
}

fn parse_decimal(value: &[u8], scale: u32) -> Option<u32> {
    let value = value.strip_prefix(b"+").unwrap_or(value);
    let dot = value.iter().position(|byte| *byte == b'.');
    let (whole, fraction) = dot.map_or((value, &[][..]), |dot| (&value[..dot], &value[dot + 1..]));
    let whole = if whole.is_empty() { 0 } else { parse_unsigned(whole)? };
    if fraction.len() > 4 || !fraction.iter().all(u8::is_ascii_digit) {
        return None;
    }
    if fraction.is_empty() {
        return whole.checked_mul(scale);
    }
    let fraction_value = parse_unsigned(fraction)?;
    let divisor = 10u32.pow(fraction.len() as u32);
    whole.checked_mul(scale)?.checked_add(fraction_value * scale / divisor)
}

fn parse_ratio(value: &[u8]) -> Option<u16> {
    let scaled = if value.contains(&b'.') {
        parse_decimal(value, 65_535)?
    } else {
        parse_unsigned(value)?.checked_mul(65_535)?
    };
    (scaled <= 65_535).then_some(scaled as u16)
}

fn parse_signed_unit(value: &[u8], suffix: &[u8], scale: u32) -> Option<i16> {
    let value = value.strip_suffix(suffix)?;
    let (negative, value) = value.strip_prefix(b"-").map_or((false, value), |value| (true, value));
    let parsed = if value.contains(&b'.') {
        parse_decimal(value, scale)?
    } else {
        parse_unsigned(value)?.checked_mul(scale)?
    };
    let signed = i32::try_from(parsed).ok()?.checked_mul(if negative { -1 } else { 1 })?;
    i16::try_from(signed).ok()
}

fn parse_transform(value: &[u8]) -> Option<UiTransform> {
    let mut transform = UiTransform::IDENTITY;
    let mut found = false;
    if let Some(inner) = function_argument(value, b"translateX(") {
        transform.translate_x = parse_signed_unit(inner, b"px", 1)?;
        found = true;
    }
    if let Some(inner) = function_argument(value, b"translateY(") {
        transform.translate_y = parse_signed_unit(inner, b"px", 1)?;
        found = true;
    }
    if let Some(inner) = function_argument(value, b"scale(") {
        transform.scale_q8_8 = u16::try_from(parse_decimal(inner, 256)?).ok()?;
        found = true;
    }
    if let Some(inner) = function_argument(value, b"rotate(") {
        transform.rotation_degrees = parse_signed_unit(inner, b"deg", 1)?;
        found = true;
    }
    found.then_some(transform)
}

fn function_argument<'a>(value: &'a [u8], name: &[u8]) -> Option<&'a [u8]> {
    let start = value.windows(name.len()).position(|window| window == name)? + name.len();
    let end = value[start..].iter().position(|byte| *byte == b')')? + start;
    Some(trim_space(&value[start..end]))
}

fn parse_direction(value: &[u8]) -> Option<UiAnimationDirection> {
    match value {
        b"normal" => Some(UiAnimationDirection::Normal),
        b"reverse" => Some(UiAnimationDirection::Reverse),
        b"alternate" => Some(UiAnimationDirection::Alternate),
        b"alternate-reverse" => Some(UiAnimationDirection::AlternateReverse),
        _ => None,
    }
}

fn parse_fill(value: &[u8]) -> Option<UiAnimationFill> {
    match value {
        b"none" => Some(UiAnimationFill::None),
        b"forwards" => Some(UiAnimationFill::Forwards),
        b"backwards" => Some(UiAnimationFill::Backwards),
        b"both" => Some(UiAnimationFill::Both),
        _ => None,
    }
}

fn parse_easing(value: &[u8]) -> Option<UiEasing> {
    match value {
        b"linear" => Some(UiEasing::Linear),
        b"ease-in" => Some(UiEasing::EaseIn),
        b"ease-out" => Some(UiEasing::EaseOut),
        b"ease-in-out" => Some(UiEasing::EaseInOut),
        _ => {
            let args = value.strip_prefix(b"cubic-bezier(")?.strip_suffix(b")")?;
            let mut values = [0i16; 4];
            let mut count = 0;
            for value in args.split(|byte| *byte == b',') {
                if count == values.len() {
                    return None;
                }
                let value = trim_space(value);
                let negative = value.first() == Some(&b'-');
                let value = value.strip_prefix(b"-").unwrap_or(value);
                let percent = i16::try_from(parse_decimal(value, 100)?).ok()?;
                values[count] = if negative { -percent } else { percent };
                count += 1;
            }
            (count == 4).then_some(UiEasing::CubicBezier {
                x1: values[0],
                y1: values[1],
                x2: values[2],
                y2: values[3],
            })
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum UiStyleRule {
    Base(UiStyle),
    State(UiStyleState, UiStyle),
}

fn style_rule(token: &[u8]) -> Option<UiStyleRule> {
    for (prefix, state) in [
        (b"hover:".as_slice(), UiStyleState::Hover),
        (b"focus:".as_slice(), UiStyleState::Focus),
        (b"pressed:".as_slice(), UiStyleState::Pressed),
        (b"disabled:".as_slice(), UiStyleState::Disabled),
    ] {
        if let Some(style) = token.strip_prefix(prefix).and_then(style_token) {
            return Some(UiStyleRule::State(state, style));
        }
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
        b"rounded-full" => Some(UiStyle::RoundedFull),
        b"bg-accent" => Some(UiStyle::BackgroundAccent),
        b"text-muted" => Some(UiStyle::TextMuted),
        b"text-4xl" => Some(UiStyle::Text4xl),
        b"font-light" => Some(UiStyle::FontLight),
        b"opacity-50" => Some(UiStyle::Opacity50),
        b"transition-colors" => Some(UiStyle::TransitionColors),
        b"transition-opacity" => Some(UiStyle::TransitionOpacity),
        b"transition-transform" => Some(UiStyle::TransitionTransform),
        b"transition-all" => Some(UiStyle::TransitionAll),
        b"duration-75" => Some(UiStyle::Duration(75)),
        b"duration-150" => Some(UiStyle::Duration(150)),
        b"duration-180" => Some(UiStyle::Duration(180)),
        b"duration-300" => Some(UiStyle::Duration(300)),
        b"duration-700" => Some(UiStyle::Duration(700)),
        b"duration-1000" => Some(UiStyle::Duration(1_000)),
        b"delay-75" => Some(UiStyle::Delay(75)),
        b"ease-linear" => Some(UiStyle::Ease(UiEasing::Linear)),
        b"ease-out" => Some(UiStyle::Ease(UiEasing::EaseOut)),
        b"ease-in-out" => Some(UiStyle::Ease(UiEasing::EaseInOut)),
        b"ease-bezier-20-80-20-100" => {
            Some(UiStyle::Ease(UiEasing::CubicBezier { x1: 20, y1: 80, x2: 20, y2: 100 }))
        }
        b"animate-in" | b"fade" => Some(UiStyle::Animation(UiAnimationPreset::In)),
        b"animate-pulse" => Some(UiStyle::Animation(UiAnimationPreset::Pulse)),
        b"animate-spin" => Some(UiStyle::Animation(UiAnimationPreset::Spin)),
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

fn has_interpolation_marker(value: &[u8]) -> bool {
    value.windows(2).any(|window| window == b"{{" || window == b"}}")
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
    fn compiles_bounded_motion_utilities_and_inline_keyframes() {
        let build = compile(
            r#"<ui.button {transition-colors transition-opacity duration-150 ease-out hover:bg-accent pressed:opacity-50}
                animation {
                    duration: 240ms;
                    delay: 0ms;
                    repeat: 1;
                    direction: normal;
                    fill: both;
                    ease: cubic-bezier(.2, .8, .2, 1);
                    from { opacity: 0; transform: translateY(-8px) scale(.98) rotate(0deg); }
                    60% { opacity: .92; transform: translateY(2px) scale(1.01) rotate(0deg); ease: ease-out; }
                    to { opacity: 1; transform: translateY(0px) scale(1) rotate(0deg); }
                }
            />"#,
        );
        assert!(build.is_valid(), "diagnostics: {:?}", build.diagnostics);
        let node = build.document.node(0).unwrap();
        assert!(node.styles.contains(UiStyle::TransitionColors));
        assert!(node.styles.contains(UiStyle::Duration(150)));
        assert_eq!(node.animation.keyframe_count, 3);
        assert_eq!(node.animation.duration_ms, 240);
        assert_eq!(node.animation.keyframes[1].offset_q16, 39_321);
        assert!(
            node.state_styles.entries[..node.state_styles.len as usize].contains(&UiStateStyle {
                state: UiStyleState::Pressed,
                style: UiStyle::Opacity50,
            })
        );
    }

    #[test]
    fn rejects_invalid_motion_bounds() {
        let too_long = compile(
            r#"<ui.panel animation { duration: 2401ms; from { opacity: 0; } to { opacity: 1; } }/>"#,
        );
        assert!(!too_long.is_valid());
        let unbounded_custom = compile(
            r#"<ui.panel animation { repeat: 9; from { opacity: 0; } to { opacity: 1; } }/>"#,
        );
        assert!(!unbounded_custom.is_valid());
        let duplicate = compile(
            r#"<ui.panel animation { from { opacity: 0; } 20% { opacity: .5; } 20% { opacity: 1; } to { opacity: 1; } }/>"#,
        );
        assert!(!duplicate.is_valid());
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
    fn diagnostics_expose_stable_codes_and_source_locations() {
        let source = "<ui.column>\n  <ui.unknown />\n</ui.column>";
        let diagnostics = lint(source);
        let diagnostic = diagnostics.get(0).unwrap();
        assert_eq!(diagnostic.kind, UiDiagnosticKind::UnknownElement);
        assert_eq!(diagnostic.code(), "UI004");
        assert_eq!(diagnostic.message(), "element is not in scope");
        assert_eq!(diagnostic.span, UiSourceSpan { start: 14, length: 1 });
        assert_eq!(diagnostic.line_column(source), (2, 3));
    }

    #[test]
    fn rejects_unknown_elements_styles_and_bindings() {
        let build = compile(
            r#"<ui.column {not-a-style}>
                <ui.unknown />
                <ui.input [not-a-binding]="name" />
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
    fn event_handlers_use_a_bounded_typed_shape() {
        let valid = compile(r#"<ui.input (changed)="passwordChanged($event)"/>"#);
        assert!(valid.is_valid(), "diagnostics: {:?}", valid.diagnostics);

        let unit_payload = compile(r#"<ui.button (click)="unlock($event)"/>"#);
        assert!(!unit_payload.is_valid());
        assert!(matches!(
            unit_payload.diagnostics.get(0),
            Some(UiDiagnostic { kind: UiDiagnosticKind::EventPayloadMismatch, .. })
        ));

        let arbitrary_argument = compile(r#"<ui.input (changed)="passwordChanged(value)"/>"#);
        assert!(!arbitrary_argument.is_valid());
        assert!(matches!(
            arbitrary_argument.diagnostics.get(0),
            Some(UiDiagnostic { kind: UiDiagnosticKind::InvalidEventHandler, .. })
        ));

        let unbounded = compile(r#"<ui.button (click)="unlock + 1"/>"#);
        assert!(!unbounded.is_valid());
        assert!(matches!(
            unbounded.diagnostics.get(0),
            Some(UiDiagnostic { kind: UiDiagnosticKind::InvalidEventHandler, .. })
        ));
    }

    #[test]
    fn strict_handler_compilation_checks_registered_event_contracts() {
        let mut handlers = UiHandlerRegistry::new();
        handlers.register_bytes(b"login", UiEventKind::Submit).unwrap();
        handlers.register_bytes(b"passwordChanged($event)", UiEventKind::Changed).unwrap();
        handlers.register_bytes(b"login", UiEventKind::Submit).unwrap();
        assert_eq!(handlers.len(), 2);

        let valid = compile_with_handlers(r#"<ui.form (submit)="login"/>"#, &handlers);
        assert!(valid.is_valid(), "diagnostics: {:?}", valid.diagnostics);

        let unknown = compile_with_handlers(r#"<ui.form (submit)="register"/>"#, &handlers);
        assert!(matches!(
            unknown.diagnostics.get(0),
            Some(UiDiagnostic { kind: UiDiagnosticKind::UnknownEventHandler, .. })
        ));

        let mismatch = compile_with_handlers(r#"<ui.button (click)="login"/>"#, &handlers);
        assert!(matches!(
            mismatch.diagnostics.get(0),
            Some(UiDiagnostic { kind: UiDiagnosticKind::EventHandlerTypeMismatch, .. })
        ));
    }

    #[test]
    fn component_registry_compiles_custom_native_aliases() {
        let mut components = UiComponentRegistry::builtins();
        components
            .register(
                UiComponentContract::new("controls.text-field", UiNodeKind::TextInput, true)
                    .with_input(logos_ui::UiComponentInput::new(
                        "value",
                        logos_ui::UiValueType::Text,
                        true,
                    ))
                    .with_output(logos_ui::UiComponentOutput::new(
                        "changed",
                        logos_ui::UiValueType::Text,
                    )),
            )
            .unwrap();

        let mut handlers = UiHandlerRegistry::new();
        handlers.register_bytes(b"nameChanged($event)", UiEventKind::Changed).unwrap();
        let context = UiCompileContext::new(&components).with_handlers(&handlers);
        let build = compile_with_context(
            r#"<controls.text-field [(value)]="name" (changed)="nameChanged($event)"/>"#,
            &context,
        );
        assert!(build.is_valid(), "diagnostics: {:?}", build.diagnostics);
        assert_eq!(build.document.node(0).unwrap().kind, UiNodeKind::TextInput);
        assert_eq!(components.len(), 8);
    }

    #[test]
    fn component_registry_rejects_invalid_duplicate_and_excess_contracts() {
        let mut components = UiComponentRegistry::builtins();
        assert_eq!(
            components.register(UiComponentContract::new("", UiNodeKind::Panel, false)),
            Err(UiComponentRegistryError::InvalidName)
        );
        assert_eq!(
            components.register(UiComponentContract::new("ui.button", UiNodeKind::Button, true)),
            Err(UiComponentRegistryError::DuplicateName)
        );

        let names = [
            "custom.a", "custom.b", "custom.c", "custom.d", "custom.e", "custom.f", "custom.g",
            "custom.h", "custom.i", "custom.j",
        ];
        for name in names.iter().take(MAX_UI_COMPONENT_CONTRACTS - components.len()) {
            components.register(UiComponentContract::new(name, UiNodeKind::Panel, false)).unwrap();
        }
        assert_eq!(components.len(), MAX_UI_COMPONENT_CONTRACTS);
        assert_eq!(
            components.register(UiComponentContract::new(
                "custom.overflow",
                UiNodeKind::Panel,
                false
            )),
            Err(UiComponentRegistryError::Capacity)
        );
    }

    #[test]
    fn value_registry_checks_binding_types_and_writable_targets() {
        let mut values = UiValueRegistry::new();
        values.register_bytes(b"username", UiValueType::Text, true).unwrap();
        values.register_bytes(b"failure", UiValueType::Bool, false).unwrap();
        values.register_bytes(b"computedName", UiValueType::Text, false).unwrap();

        let components = UiComponentRegistry::builtins();
        let context = UiCompileContext::new(&components).with_values(&values);
        let valid = compile_with_context(
            r#"<ui.input [(value)]="username" {opacity-50}="failure"/>"#,
            &context,
        );
        assert!(valid.is_valid(), "diagnostics: {:?}", valid.diagnostics);

        let unknown = compile_with_context(r#"<ui.input [value]="missing"/>"#, &context);
        assert!(matches!(
            unknown.diagnostics.get(0),
            Some(UiDiagnostic { kind: UiDiagnosticKind::UnknownBindingExpression, .. })
        ));

        let mismatch = compile_with_context(r#"<ui.input [value]="failure"/>"#, &context);
        assert!(matches!(
            mismatch.diagnostics.get(0),
            Some(UiDiagnostic { kind: UiDiagnosticKind::BindingTypeMismatch, .. })
        ));

        let readonly = compile_with_context(r#"<ui.input [(value)]="computedName"/>"#, &context);
        assert!(matches!(
            readonly.diagnostics.get(0),
            Some(UiDiagnostic { kind: UiDiagnosticKind::ReadOnlyBinding, .. })
        ));

        let style_mismatch =
            compile_with_context(r#"<ui.button {opacity-50}="username"/>"#, &context);
        assert!(matches!(
            style_mismatch.diagnostics.get(0),
            Some(UiDiagnostic { kind: UiDiagnosticKind::StyleConditionTypeMismatch, .. })
        ));

        let style_unknown =
            compile_with_context(r#"<ui.button {opacity-50}="missing"/>"#, &context);
        assert!(matches!(
            style_unknown.diagnostics.get(0),
            Some(UiDiagnostic { kind: UiDiagnosticKind::UnknownStyleExpression, .. })
        ));
    }

    #[test]
    fn value_registry_rejects_invalid_and_duplicate_expressions() {
        let mut values = UiValueRegistry::new();
        assert_eq!(
            values.register(UiExpression::EMPTY, UiValueType::Text, true),
            Err(UiValueRegistryError::InvalidExpression)
        );
        assert_eq!(
            values.register_bytes(b"", UiValueType::Text, true),
            Err(UiValueRegistryError::InvalidExpression)
        );
        values.register_bytes(b"name", UiValueType::Text, true).unwrap();
        assert_eq!(
            values.register_bytes(b"name", UiValueType::Text, true),
            Err(UiValueRegistryError::DuplicateExpression)
        );
    }

    #[test]
    fn text_interpolation_is_bounded_typed_and_codegen_safe() {
        let mut values = UiValueRegistry::new();
        values.register_bytes(b"displayName", UiValueType::Text, false).unwrap();
        values.register_bytes(b"failure", UiValueType::Bool, false).unwrap();
        let components = UiComponentRegistry::builtins();
        let context = UiCompileContext::new(&components).with_values(&values);

        let valid = compile_with_context(r#"<ui.text>{{ displayName }}</ui.text>"#, &context);
        assert!(valid.is_valid(), "diagnostics: {:?}", valid.diagnostics);
        let node = valid.document.node(0).unwrap();
        assert!(node.text.as_bytes().is_empty());
        assert_eq!(node.text_binding.as_bytes(), b"displayName");

        let mut generated = String::new();
        write_rust(&valid, &mut generated).unwrap();
        assert!(generated.contains("text_binding: logos_ui::UiExpression::from_bytes"));
        assert!(generated.contains("displayName"));

        let unknown = compile_with_context(r#"<ui.text>{{ missing }}</ui.text>"#, &context);
        assert!(matches!(
            unknown.diagnostics.get(0),
            Some(UiDiagnostic { kind: UiDiagnosticKind::UnknownTextExpression, .. })
        ));

        let mismatch = compile_with_context(r#"<ui.text>{{ failure }}</ui.text>"#, &context);
        assert!(matches!(
            mismatch.diagnostics.get(0),
            Some(UiDiagnostic { kind: UiDiagnosticKind::TextExpressionTypeMismatch, .. })
        ));

        let mixed = compile(r#"<ui.text>Hello {{ displayName }}</ui.text>"#);
        assert!(matches!(
            mixed.diagnostics.get(0),
            Some(UiDiagnostic { kind: UiDiagnosticKind::InvalidInterpolation, .. })
        ));
    }

    #[test]
    fn handler_registry_rejects_invalid_expressions() {
        let mut handlers = UiHandlerRegistry::new();
        assert_eq!(
            handlers.register_bytes(b"", UiEventKind::Click),
            Err(UiHandlerRegistryError::InvalidExpression)
        );
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
    fn rejects_duplicate_hash_node_names() {
        let build = compile(r#"<ui.column #root><ui.text #root>duplicate</ui.text></ui.column>"#);
        assert!(!build.is_valid());
        assert!(matches!(
            build.diagnostics.get(0),
            Some(UiDiagnostic { kind: UiDiagnosticKind::DuplicateNodeName, .. })
        ));
        assert_eq!(build.diagnostics.get(0).unwrap().code(), "UI026");
    }

    #[test]
    fn compiles_flex_and_axis_specific_gap_utilities() {
        let build = compile(r#"<ui.column {flex-x gap-2 gap-x-3 gap-y-4 rounded-full}/>"#);
        assert!(build.is_valid(), "diagnostics: {:?}", build.diagnostics);
        let styles = &build.document.node(0).unwrap().styles;
        let styles = &styles.tokens[..styles.len as usize];
        assert!(styles.contains(&UiStyle::FlexX));
        assert!(styles.contains(&UiStyle::Gap(2)));
        assert!(styles.contains(&UiStyle::GapX(3)));
        assert!(styles.contains(&UiStyle::GapY(4)));
        assert!(styles.contains(&UiStyle::RoundedFull));
    }

    #[test]
    fn enforces_source_and_node_bounds() {
        let too_large = "x".repeat(MAX_UI_SOURCE_BYTES + 1);
        let report = compile(&too_large);
        assert_eq!(report.diagnostics.get(0).unwrap().kind, UiDiagnosticKind::SourceTooLarge);

        let mut source = String::from("<ui.column>");
        for _ in 0..logos_ui::MAX_UI_NODES {
            source.push_str("<ui.text />");
        }
        source.push_str("</ui.column>");
        let report = compile(&source);
        assert!(
            report.diagnostics.get(0).is_some_and(|item| item.kind == UiDiagnosticKind::Capacity)
        );
    }
}
