use crate::events::{
    MAX_UI_OUTPUT_EVENTS, UiEventError, UiEventRouter, UiInputEvent, UiOutput, UiOutputError,
    UiRoutedEvent,
};
use crate::runtime::{UiError, UiNodeHandle, UiNodeKind, UiTree};
use crate::{
    UiBinding, UiBindingProperty, UiButton, UiButtonEvent, UiComponent, UiEventDisposition,
    UiExpression, UiInput, UiInputEventOutput, UiInteractive, UiPanel, UiStyleConditions,
    UiStyleList, UiText,
};

pub const MAX_UI_COMPONENTS: usize = crate::MAX_UI_NODES;
pub const MAX_UI_BINDING_VALUES: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UiBindingValue {
    Text(UiText),
    Bool(bool),
    Styles(UiStyleList),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UiBindingStoreError {
    InvalidExpression,
    Capacity,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct UiBindingValueEntry {
    expression: UiExpression,
    value: UiBindingValue,
}

impl UiBindingValueEntry {
    const EMPTY: Self =
        Self { expression: UiExpression::EMPTY, value: UiBindingValue::Text(UiText::EMPTY) };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiBindingValueStore {
    entries: [UiBindingValueEntry; MAX_UI_BINDING_VALUES],
    count: usize,
}

impl UiBindingValueStore {
    pub const fn new() -> Self {
        Self { entries: [UiBindingValueEntry::EMPTY; MAX_UI_BINDING_VALUES], count: 0 }
    }

    pub const fn len(&self) -> usize {
        self.count
    }

    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }

    pub fn set(
        &mut self,
        expression: UiExpression,
        value: UiBindingValue,
    ) -> Result<bool, UiBindingStoreError> {
        if expression.as_bytes().is_empty() {
            return Err(UiBindingStoreError::InvalidExpression);
        }
        if let Some(entry) =
            self.entries[..self.count].iter_mut().find(|entry| entry.expression == expression)
        {
            if entry.value == value {
                return Ok(false);
            }
            entry.value = value;
            return Ok(true);
        }
        if self.count == MAX_UI_BINDING_VALUES {
            return Err(UiBindingStoreError::Capacity);
        }
        self.entries[self.count] = UiBindingValueEntry { expression, value };
        self.count += 1;
        Ok(true)
    }

    pub fn resolve(&self, expression: UiExpression) -> Option<UiBindingValue> {
        self.entries[..self.count]
            .iter()
            .find(|entry| entry.expression == expression)
            .map(|entry| entry.value)
    }
}

impl Default for UiBindingValueStore {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UiComponentEvent {
    Clicked { target: UiNodeHandle },
    Changed { target: UiNodeHandle, value: UiText },
    Submitted { target: UiNodeHandle },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UiComponentTreeError {
    Capacity,
    Stale,
    NotComponent,
    NotFocusable,
    OutputFull,
    TypeMismatch,
    UnsupportedBinding,
}

#[derive(Clone, Copy)]
enum UiComponentSlot {
    Empty,
    Panel(UiPanel),
    Button(UiButton),
    Input(UiInput),
}

enum UiComponentOutput {
    Button(UiButtonEvent),
    Input(UiInputEventOutput),
}

impl UiComponentSlot {
    const EMPTY: Self = Self::Empty;

    fn from_kind(kind: UiNodeKind) -> Self {
        match kind {
            UiNodeKind::Panel => Self::Panel(UiPanel::new()),
            UiNodeKind::Button => Self::Button(UiButton::new()),
            UiNodeKind::TextInput => Self::Input(UiInput::new()),
            UiNodeKind::Root | UiNodeKind::Label | UiNodeKind::Form => Self::Empty,
        }
    }

    fn handle(
        &mut self,
        event: UiInputEvent,
    ) -> Result<(UiEventDisposition, Option<UiComponentOutput>), UiOutputError> {
        match self {
            Self::Empty => Ok((UiEventDisposition::Ignored, None)),
            Self::Panel(component) => {
                let mut output = UiOutput::new();
                let disposition = component.handle_event(event, &mut output)?;
                Ok((disposition, None))
            }
            Self::Button(component) => {
                let mut output = UiOutput::new();
                let disposition = component.handle_event(event, &mut output)?;
                Ok((disposition, output.pop().map(UiComponentOutput::Button)))
            }
            Self::Input(component) => {
                let mut output = UiOutput::new();
                let disposition = component.handle_event(event, &mut output)?;
                Ok((disposition, output.pop().map(UiComponentOutput::Input)))
            }
        }
    }

    fn set_disabled(&mut self, disabled: bool) -> bool {
        match self {
            Self::Button(component) => component.set_disabled(disabled),
            Self::Input(component) => component.set_disabled(disabled),
            Self::Empty | Self::Panel(_) => return false,
        }
        true
    }

    fn set_focused(&mut self, focused: bool) -> bool {
        match self {
            Self::Button(component) => component.set_focused(focused),
            Self::Input(component) => component.set_focused(focused),
            Self::Empty | Self::Panel(_) => return false,
        }
        true
    }

    fn set_value(&mut self, value: UiText) -> bool {
        match self {
            Self::Input(component) => component.set_value(value),
            Self::Empty | Self::Panel(_) | Self::Button(_) => false,
        }
    }

    fn value(&self) -> Option<UiText> {
        match self {
            Self::Input(component) => Some(component.value()),
            Self::Empty | Self::Panel(_) | Self::Button(_) => None,
        }
    }

    fn is_focusable(&self) -> bool {
        match self {
            Self::Button(component) => component.is_focusable(),
            Self::Input(component) => component.is_focusable(),
            Self::Empty | Self::Panel(_) => false,
        }
    }
}

pub struct UiComponentTree {
    tree: UiTree,
    components: [UiComponentSlot; MAX_UI_COMPONENTS],
    focused: UiNodeHandle,
}

impl UiComponentTree {
    pub fn from_document(
        document: &crate::UiDocument,
        router: &mut UiEventRouter,
    ) -> Result<Self, UiComponentTreeError> {
        let blueprint = document.to_blueprint().map_err(map_tree_error)?;
        let host = Self::from_blueprint(&blueprint)?;
        host.install_document_hooks(document, router)?;
        Ok(host)
    }

    pub fn from_blueprint(blueprint: &crate::UiBlueprint) -> Result<Self, UiComponentTreeError> {
        let tree = UiTree::from_blueprint(blueprint).map_err(map_tree_error)?;
        let mut components = [UiComponentSlot::EMPTY; MAX_UI_COMPONENTS];
        for (index, component) in components.iter_mut().enumerate().take(blueprint.len()) {
            let spec = blueprint.spec(index).ok_or(UiComponentTreeError::Stale)?;
            *component = UiComponentSlot::from_kind(spec.kind);
        }
        Ok(Self { tree, components, focused: UiNodeHandle::EMPTY })
    }

    pub fn new() -> Self {
        Self {
            tree: UiTree::new(),
            components: [UiComponentSlot::EMPTY; MAX_UI_COMPONENTS],
            focused: UiNodeHandle::EMPTY,
        }
    }

    pub fn insert(
        &mut self,
        kind: UiNodeKind,
        parent: UiNodeHandle,
        key: u16,
    ) -> Result<UiNodeHandle, UiComponentTreeError> {
        let handle = self.tree.insert(kind, parent, key).map_err(map_tree_error)?;
        self.components[usize::from(handle.slot)] = UiComponentSlot::from_kind(kind);
        Ok(handle)
    }

    pub fn destroy(&mut self, handle: UiNodeHandle) -> Result<(), UiComponentTreeError> {
        self.tree.destroy(handle).map_err(map_tree_error)?;
        if self.tree.node(self.focused).is_err() {
            self.focused = UiNodeHandle::EMPTY;
        }
        Ok(())
    }

    pub fn destroy_with_router(
        &mut self,
        handle: UiNodeHandle,
        router: &mut UiEventRouter,
    ) -> Result<usize, UiComponentTreeError> {
        self.destroy(handle)?;
        Ok(router.unsubscribe_target(handle))
    }

    pub const fn tree(&self) -> &UiTree {
        &self.tree
    }

    pub fn tree_mut(&mut self) -> &mut UiTree {
        &mut self.tree
    }

    pub fn hit_test(&self, x: i32, y: i32) -> Option<UiNodeHandle> {
        self.tree.hit_test(x, y)
    }

    pub fn dispatch_at(
        &mut self,
        event: UiInputEvent,
        output: &mut UiOutput<UiComponentEvent>,
    ) -> Result<Option<UiNodeHandle>, UiComponentTreeError> {
        let (x, y) = match event {
            UiInputEvent::PointerDown { x, y }
            | UiInputEvent::PointerUp { x, y }
            | UiInputEvent::PointerMove { x, y } => (x, y),
            _ => return Err(UiComponentTreeError::NotComponent),
        };
        let Some(target) = self.hit_test(x, y) else { return Ok(None) };
        if matches!(event, UiInputEvent::PointerDown { .. }) {
            self.focus(target)?;
        }
        self.dispatch(target, event, output)?;
        Ok(Some(target))
    }

    pub fn dispatch_at_with_hooks(
        &mut self,
        event: UiInputEvent,
        router: &UiEventRouter,
        component_output: &mut UiOutput<UiComponentEvent>,
        routed_output: &mut UiOutput<UiRoutedEvent>,
    ) -> Result<Option<UiNodeHandle>, UiComponentTreeError> {
        let (x, y) = match event {
            UiInputEvent::PointerDown { x, y }
            | UiInputEvent::PointerUp { x, y }
            | UiInputEvent::PointerMove { x, y } => (x, y),
            _ => return Err(UiComponentTreeError::NotComponent),
        };
        let Some(target) = self.hit_test(x, y) else { return Ok(None) };
        if matches!(event, UiInputEvent::PointerDown { .. }) {
            self.focus(target)?;
        }
        self.dispatch_with_hooks(target, event, router, component_output, routed_output)?;
        Ok(Some(target))
    }

    pub fn handle_for_name(
        &self,
        document: &crate::UiDocument,
        name: &[u8],
    ) -> Option<UiNodeHandle> {
        let index = document.node_index_by_name(name)?;
        self.tree.handle_at(usize::from(index)).ok()
    }

    pub const fn focused(&self) -> UiNodeHandle {
        self.focused
    }

    pub fn focus(&mut self, handle: UiNodeHandle) -> Result<(), UiComponentTreeError> {
        let node = self.tree.node(handle).map_err(map_tree_error)?;
        if !node.interaction.is_focusable()
            || !self.components[usize::from(handle.slot)].is_focusable()
        {
            return Err(UiComponentTreeError::NotFocusable);
        }
        if self.focused == handle {
            return Ok(());
        }
        if self.focused.is_valid() {
            let old = self.focused;
            self.tree.node(old).map_err(map_tree_error)?;
            self.components[usize::from(old.slot)].set_focused(false);
            self.tree.set_focused(old, false).map_err(map_tree_error)?;
        }
        self.components[usize::from(handle.slot)].set_focused(true);
        self.tree.set_focused(handle, true).map_err(map_tree_error)?;
        self.focused = handle;
        Ok(())
    }

    pub fn clear_focus(&mut self) -> Result<(), UiComponentTreeError> {
        if !self.focused.is_valid() {
            return Ok(());
        }
        let old = self.focused;
        self.tree.node(old).map_err(map_tree_error)?;
        self.components[usize::from(old.slot)].set_focused(false);
        self.tree.set_focused(old, false).map_err(map_tree_error)?;
        self.focused = UiNodeHandle::EMPTY;
        Ok(())
    }

    pub fn focus_next(&mut self, forward: bool) -> Result<UiNodeHandle, UiComponentTreeError> {
        let mut candidates = [UiNodeHandle::EMPTY; MAX_UI_COMPONENTS];
        let mut count = 0;
        for index in 0..MAX_UI_COMPONENTS {
            let Ok(handle) = self.tree.handle_at(index) else { continue };
            let node = self.tree.node(handle).map_err(map_tree_error)?;
            if node.interaction.is_focusable()
                && self.components[index].is_focusable()
                && count < candidates.len()
            {
                let mut position = count;
                while position != 0 {
                    let previous = candidates[position - 1];
                    let previous_node = self.tree.node(previous).map_err(map_tree_error)?;
                    if (previous_node.interaction.tab_index(), previous.slot)
                        <= (node.interaction.tab_index(), handle.slot)
                    {
                        break;
                    }
                    candidates[position] = previous;
                    position -= 1;
                }
                candidates[position] = handle;
                count += 1;
            }
        }
        if count == 0 {
            return Err(UiComponentTreeError::NotFocusable);
        }
        let current = candidates[..count].iter().position(|candidate| *candidate == self.focused);
        let index = match (current, forward) {
            (Some(index), true) => (index + 1).min(count - 1),
            (Some(index), false) => index.saturating_sub(1),
            (None, true) => 0,
            (None, false) => count - 1,
        };
        let handle = candidates[index];
        self.focus(handle)?;
        Ok(handle)
    }

    pub fn set_disabled(
        &mut self,
        handle: UiNodeHandle,
        disabled: bool,
    ) -> Result<(), UiComponentTreeError> {
        self.tree.node(handle).map_err(map_tree_error)?;
        if !self.components[usize::from(handle.slot)].set_disabled(disabled) {
            return Err(UiComponentTreeError::NotComponent);
        }
        self.tree.set_disabled(handle, disabled).map_err(map_tree_error)?;
        if disabled && self.focused == handle {
            self.focused = UiNodeHandle::EMPTY;
        }
        Ok(())
    }

    pub fn set_value(
        &mut self,
        handle: UiNodeHandle,
        value: UiText,
    ) -> Result<bool, UiComponentTreeError> {
        self.tree.node(handle).map_err(map_tree_error)?;
        if !matches!(self.components[usize::from(handle.slot)], UiComponentSlot::Input(_)) {
            return Err(UiComponentTreeError::NotComponent);
        }
        Ok(self.components[usize::from(handle.slot)].set_value(value))
    }

    pub fn value(&self, handle: UiNodeHandle) -> Result<UiText, UiComponentTreeError> {
        self.tree.node(handle).map_err(map_tree_error)?;
        self.components[usize::from(handle.slot)].value().ok_or(UiComponentTreeError::NotComponent)
    }

    pub fn set_text(
        &mut self,
        handle: UiNodeHandle,
        text: UiText,
    ) -> Result<bool, UiComponentTreeError> {
        self.tree.set_text(handle, text).map_err(map_tree_error)
    }

    pub fn set_styles(
        &mut self,
        handle: UiNodeHandle,
        styles: UiStyleList,
    ) -> Result<bool, UiComponentTreeError> {
        self.tree.set_styles(handle, styles).map_err(map_tree_error)
    }

    pub fn apply_document_styles(
        &mut self,
        document: &crate::UiDocument,
        node_index: u16,
        conditions: &UiStyleConditions,
    ) -> Result<bool, UiComponentTreeError> {
        let node = document.node(usize::from(node_index)).ok_or(UiComponentTreeError::Stale)?;
        let handle = self.tree.handle_at(usize::from(node_index)).map_err(map_tree_error)?;
        let focused = self.tree.node(handle).map_err(map_tree_error)?.interaction.is_focused();
        let styles =
            node.resolve_styles(focused, conditions).map_err(|_| UiComponentTreeError::Capacity)?;
        self.set_styles(handle, styles)
    }

    pub fn apply_binding(
        &mut self,
        handle: UiNodeHandle,
        binding: UiBinding,
        value: UiBindingValue,
    ) -> Result<bool, UiComponentTreeError> {
        match (binding.property, value) {
            (UiBindingProperty::Text, UiBindingValue::Text(value)) => self.set_text(handle, value),
            (UiBindingProperty::Styles, UiBindingValue::Styles(value)) => {
                self.set_styles(handle, value)
            }
            (UiBindingProperty::Value, UiBindingValue::Text(value)) => {
                self.set_value(handle, value)
            }
            (UiBindingProperty::Disabled, UiBindingValue::Bool(value)) => {
                let changed =
                    self.tree.node(handle).map_err(map_tree_error)?.interaction.is_disabled()
                        != value;
                self.set_disabled(handle, value)?;
                Ok(changed)
            }
            (
                UiBindingProperty::Form | UiBindingProperty::Control | UiBindingProperty::CanSubmit,
                _,
            ) => Err(UiComponentTreeError::UnsupportedBinding),
            _ => Err(UiComponentTreeError::TypeMismatch),
        }
    }

    pub fn apply_document_bindings(
        &mut self,
        document: &crate::UiDocument,
        values: &UiBindingValueStore,
    ) -> Result<usize, UiComponentTreeError> {
        let mut changed = 0;
        for index in 0..document.node_count() {
            let node = document.node(index).ok_or(UiComponentTreeError::Stale)?;
            let handle = self.tree.handle_at(index).map_err(map_tree_error)?;
            if !node.text_binding.as_bytes().is_empty() {
                if let Some(value) = values.resolve(node.text_binding) {
                    changed += usize::from(self.apply_binding(
                        handle,
                        UiBinding {
                            property: UiBindingProperty::Text,
                            expression: node.text_binding,
                        },
                        value,
                    )?);
                }
            }
            for binding in node.bindings.entries.iter().take(usize::from(node.bindings.len)) {
                let Some(value) = values.resolve(binding.expression) else { continue };
                changed += usize::from(self.apply_binding(handle, *binding, value)?);
            }
        }
        Ok(changed)
    }

    pub fn dispatch(
        &mut self,
        handle: UiNodeHandle,
        event: UiInputEvent,
        output: &mut UiOutput<UiComponentEvent>,
    ) -> Result<UiEventDisposition, UiComponentTreeError> {
        let (disposition, component_event) = self.dispatch_component(handle, event, output)?;
        if let Some(component_event) = component_event {
            output.emit(component_event).map_err(|_| UiComponentTreeError::OutputFull)?;
            return Ok(UiEventDisposition::Consumed);
        }
        Ok(disposition)
    }

    fn dispatch_component(
        &mut self,
        handle: UiNodeHandle,
        event: UiInputEvent,
        output: &UiOutput<UiComponentEvent>,
    ) -> Result<(UiEventDisposition, Option<UiComponentEvent>), UiComponentTreeError> {
        self.tree.node(handle).map_err(map_tree_error)?;
        if matches!(self.components[usize::from(handle.slot)], UiComponentSlot::Empty) {
            return Err(UiComponentTreeError::NotComponent);
        }
        if output.len() == MAX_UI_OUTPUT_EVENTS {
            return Err(UiComponentTreeError::OutputFull);
        }
        let (disposition, result) = self.components[usize::from(handle.slot)]
            .handle(event)
            .map_err(|_| UiComponentTreeError::OutputFull)?;
        let Some(result) = result else { return Ok((disposition, None)) };
        let event = match result {
            UiComponentOutput::Button(UiButtonEvent::Clicked) => {
                UiComponentEvent::Clicked { target: handle }
            }
            UiComponentOutput::Input(UiInputEventOutput::Changed(value)) => {
                UiComponentEvent::Changed { target: handle, value }
            }
            UiComponentOutput::Input(UiInputEventOutput::Submitted) => {
                UiComponentEvent::Submitted { target: handle }
            }
        };
        Ok((disposition, Some(event)))
    }

    pub fn dispatch_with_hooks(
        &mut self,
        handle: UiNodeHandle,
        event: UiInputEvent,
        router: &UiEventRouter,
        component_output: &mut UiOutput<UiComponentEvent>,
        routed_output: &mut UiOutput<UiRoutedEvent>,
    ) -> Result<UiEventDisposition, UiComponentTreeError> {
        self.tree.node(handle).map_err(map_tree_error)?;
        let slot = &self.components[usize::from(handle.slot)];
        let mut routed_count = usize::from(router.is_subscribed(handle, event.event_type()));
        if let Some(generated_type) = possible_component_event_type(slot, event) {
            if generated_type != event.event_type() && router.is_subscribed(handle, generated_type)
            {
                routed_count += 1;
            }
        }
        if routed_output.len() + routed_count > MAX_UI_OUTPUT_EVENTS {
            return Err(UiComponentTreeError::OutputFull);
        }
        let (disposition, component_event) = if matches!(slot, UiComponentSlot::Empty) {
            if !router.is_subscribed(handle, event.event_type()) {
                return Err(UiComponentTreeError::NotComponent);
            }
            (UiEventDisposition::Ignored, None)
        } else {
            self.dispatch_component(handle, event, component_output)?
        };
        if let Some(component_event) = component_event {
            component_output.emit(component_event).map_err(|_| UiComponentTreeError::OutputFull)?;
        }
        router.dispatch(handle, event, routed_output).map_err(map_event_error)?;
        if let Some(component_event) = component_event {
            let generated = component_event.as_input_event();
            if generated.event_type() != event.event_type() {
                router.dispatch(handle, generated, routed_output).map_err(map_event_error)?;
            }
        }
        Ok(disposition)
    }

    fn install_document_hooks(
        &self,
        document: &crate::UiDocument,
        router: &mut UiEventRouter,
    ) -> Result<usize, UiComponentTreeError> {
        let mut hooks = [None; crate::MAX_UI_NODES];
        let mut count = 0;
        for index in 0..document.node_count() {
            let node = document.node(index).ok_or(UiComponentTreeError::Stale)?;
            if !node.event.is_present() {
                continue;
            }
            let target = self.tree.handle_at(index).map_err(map_tree_error)?;
            hooks[count] = Some((target, node.event.kind.event_type(), index as u16));
            count += 1;
        }

        let new_routes = hooks[..count]
            .iter()
            .filter(|hook| {
                let (target, event_type, _) = hook.expect("document hook initialized");
                !router.is_subscribed(target, event_type)
            })
            .count();
        if router.len() + new_routes > crate::MAX_UI_EVENT_ROUTES {
            return Err(UiComponentTreeError::Capacity);
        }

        for hook in hooks[..count].iter().copied() {
            let (target, event_type, node_index) = hook.expect("document hook initialized");
            router
                .subscribe(target, event_type, crate::UiHandlerId::new(node_index))
                .map_err(map_event_error)?;
        }
        Ok(count)
    }
}

impl UiComponentEvent {
    fn as_input_event(self) -> UiInputEvent {
        match self {
            Self::Clicked { .. } => UiInputEvent::Click,
            Self::Changed { value, .. } => UiInputEvent::Changed { value },
            Self::Submitted { .. } => UiInputEvent::Submit,
        }
    }
}

fn possible_component_event_type(
    component: &UiComponentSlot,
    event: UiInputEvent,
) -> Option<crate::UiEventType> {
    match component {
        UiComponentSlot::Button(_)
            if matches!(
                event,
                UiInputEvent::Click
                    | UiInputEvent::PointerUp { .. }
                    | UiInputEvent::Submit
                    | UiInputEvent::KeyDown { code: crate::UI_KEY_ENTER, .. }
            ) =>
        {
            Some(crate::UiEventType::Click)
        }
        UiComponentSlot::Input(_) if matches!(event, UiInputEvent::TextInput { .. }) => {
            Some(crate::UiEventType::Changed)
        }
        UiComponentSlot::Input(_)
            if matches!(
                event,
                UiInputEvent::Submit | UiInputEvent::KeyDown { code: crate::UI_KEY_ENTER, .. }
            ) =>
        {
            Some(crate::UiEventType::Submit)
        }
        UiComponentSlot::Empty
        | UiComponentSlot::Panel(_)
        | UiComponentSlot::Button(_)
        | UiComponentSlot::Input(_) => None,
    }
}

impl Default for UiComponentTree {
    fn default() -> Self {
        Self::new()
    }
}

fn map_tree_error(error: UiError) -> UiComponentTreeError {
    match error {
        UiError::Capacity => UiComponentTreeError::Capacity,
        UiError::InvalidParent | UiError::RootExists | UiError::Stale | UiError::NotFound => {
            UiComponentTreeError::Stale
        }
    }
}

fn map_event_error(error: UiEventError) -> UiComponentTreeError {
    match error {
        UiEventError::OutputFull => UiComponentTreeError::OutputFull,
        UiEventError::Capacity => UiComponentTreeError::Capacity,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_retains_typed_input_and_button_instances() {
        let mut blueprint = crate::UiBlueprint::new();
        let root = blueprint.push_root(UiNodeKind::Root, 1).unwrap();
        let input = blueprint.push_child(UiNodeKind::TextInput, root, 2).unwrap();
        let button = blueprint.push_child(UiNodeKind::Button, root, 3).unwrap();
        let mut host = UiComponentTree::from_blueprint(&blueprint).unwrap();
        let root_handle = host.tree().handle_at(root as usize).unwrap();
        let input_handle = host.tree().handle_at(input as usize).unwrap();
        let button_handle = host.tree().handle_at(button as usize).unwrap();
        assert_eq!(
            host.dispatch(root_handle, UiInputEvent::Click, &mut UiOutput::new()),
            Err(UiComponentTreeError::NotComponent)
        );
        host.focus(input_handle).unwrap();
        let mut output = UiOutput::new();
        host.dispatch(
            input_handle,
            UiInputEvent::TextInput { scalar: u32::from(b'a') },
            &mut output,
        )
        .unwrap();
        assert_eq!(host.value(input_handle).unwrap().as_bytes(), b"a");
        assert_eq!(
            output.pop(),
            Some(UiComponentEvent::Changed {
                target: input_handle,
                value: UiText::from_bytes(b"a").unwrap(),
            })
        );

        host.focus(button_handle).unwrap();
        host.dispatch(button_handle, UiInputEvent::Submit, &mut output).unwrap();
        assert_eq!(output.pop(), Some(UiComponentEvent::Clicked { target: button_handle }));
        assert!(!host.tree().node(input_handle).unwrap().interaction.is_focused());
    }

    #[test]
    fn coordinate_dispatch_focuses_and_activates_the_hit_target() {
        let mut host = UiComponentTree::new();
        let root = host.insert(UiNodeKind::Root, UiNodeHandle::EMPTY, 1).unwrap();
        let button = host.insert(UiNodeKind::Button, root, 2).unwrap();
        host.tree_mut().set_bounds(button, crate::UiRect::new(4, 5, 20, 10)).unwrap();

        let mut output = UiOutput::new();
        assert_eq!(
            host.dispatch_at(UiInputEvent::PointerDown { x: 8, y: 7 }, &mut output),
            Ok(Some(button))
        );
        assert_eq!(host.focused(), button);
        assert!(output.is_empty());
        assert_eq!(
            host.dispatch_at(UiInputEvent::PointerUp { x: 8, y: 7 }, &mut output),
            Ok(Some(button))
        );
        assert_eq!(output.pop(), Some(UiComponentEvent::Clicked { target: button }));
        assert_eq!(host.dispatch_at(UiInputEvent::PointerUp { x: 0, y: 0 }, &mut output), Ok(None));
        assert!(output.is_empty());
    }

    #[test]
    fn coordinate_dispatch_routes_pointer_activation_to_click_hooks() {
        let mut host = UiComponentTree::new();
        let root = host.insert(UiNodeKind::Root, UiNodeHandle::EMPTY, 1).unwrap();
        let button = host.insert(UiNodeKind::Button, root, 2).unwrap();
        host.tree_mut().set_bounds(button, crate::UiRect::new(0, 0, 10, 10)).unwrap();
        let mut router = crate::UiEventRouter::new();
        router.subscribe(button, crate::UiEventType::Click, crate::UiHandlerId::new(4)).unwrap();
        let mut component_output = UiOutput::new();
        let mut routed_output = UiOutput::new();

        assert_eq!(
            host.dispatch_at_with_hooks(
                UiInputEvent::PointerUp { x: 5, y: 5 },
                &router,
                &mut component_output,
                &mut routed_output,
            ),
            Ok(Some(button))
        );
        assert_eq!(component_output.pop(), Some(UiComponentEvent::Clicked { target: button }));
        assert_eq!(
            routed_output.pop(),
            Some(UiRoutedEvent {
                target: button,
                handler: crate::UiHandlerId::new(4),
                event: UiInputEvent::Click,
            })
        );
    }

    #[test]
    fn host_applies_document_styles_using_focus_and_conditions() {
        let mut styles = crate::UiStyleList::EMPTY;
        assert!(styles.push(crate::UiStyle::BackgroundAccent));
        let mut document = crate::UiDocument::EMPTY;
        document
            .push_node(crate::UiNodeTemplate {
                kind: UiNodeKind::Button,
                tab_index: 0,
                styles,
                state_styles: crate::UiStateStyleList {
                    entries: [crate::UiStateStyle {
                        state: crate::UiStyleState::Focus,
                        style: crate::UiStyle::RoundedLarge,
                    }; crate::MAX_UI_STATE_STYLES],
                    len: 1,
                },
                conditional_styles: crate::UiConditionalStyleList {
                    entries: [crate::UiConditionalStyle {
                        style: crate::UiStyle::Opacity50,
                        expression: crate::UiExpression::from_bytes(b"failure").unwrap(),
                    }; crate::MAX_UI_CONDITIONAL_STYLES],
                    len: 1,
                },
                ..crate::UiNodeTemplate::EMPTY
            })
            .unwrap();
        let mut router = crate::UiEventRouter::new();
        let mut host = UiComponentTree::from_document(&document, &mut router).unwrap();
        let handle = host.tree().handle_at(0).unwrap();
        host.focus(handle).unwrap();
        let mut conditions = crate::UiStyleConditions::EMPTY;
        assert!(conditions.set(crate::UiExpression::from_bytes(b"failure").unwrap(), true));

        assert_eq!(host.apply_document_styles(&document, 0, &conditions), Ok(true));
        assert!(host.tree().has_style(handle, crate::UiStyle::RoundedLarge).unwrap());
        assert!(host.tree().has_style(handle, crate::UiStyle::Opacity50).unwrap());
        assert_eq!(host.apply_document_styles(&document, 0, &conditions), Ok(false));

        assert!(conditions.set(crate::UiExpression::from_bytes(b"failure").unwrap(), false));
        assert_eq!(host.apply_document_styles(&document, 0, &conditions), Ok(true));
        assert!(!host.tree().has_style(handle, crate::UiStyle::Opacity50).unwrap());
        assert!(host.set_text(handle, UiText::from_bytes(b"Unlock").unwrap()).unwrap());
        assert_eq!(host.tree().node(handle).unwrap().text.as_bytes(), b"Unlock");
    }

    #[test]
    fn named_handles_are_generation_safe_after_destroy_and_reuse() {
        let mut document = crate::UiDocument::EMPTY;
        let root = document
            .push_node(crate::UiNodeTemplate {
                kind: UiNodeKind::Root,
                key: crate::UiName::from_bytes(b"root").unwrap(),
                ..crate::UiNodeTemplate::EMPTY
            })
            .unwrap();
        document
            .push_node(crate::UiNodeTemplate {
                kind: UiNodeKind::TextInput,
                parent: root,
                key: crate::UiName::from_bytes(b"username").unwrap(),
                tab_index: 0,
                ..crate::UiNodeTemplate::EMPTY
            })
            .unwrap();
        let mut router = crate::UiEventRouter::new();
        let mut host = UiComponentTree::from_document(&document, &mut router).unwrap();
        let username = host.handle_for_name(&document, b"username").unwrap();
        assert_eq!(host.handle_for_name(&document, b"missing"), None);

        host.destroy(username).unwrap();
        assert_eq!(host.tree().node(username), Err(UiError::Stale));
        let replacement =
            host.insert(UiNodeKind::TextInput, host.tree().handle_at(0).unwrap(), 9).unwrap();
        assert_eq!(replacement.slot, username.slot);
        assert_ne!(replacement.generation, username.generation);
        assert_eq!(host.tree().node(username), Err(UiError::Stale));
    }

    #[test]
    fn host_applies_typed_document_bindings_and_suppresses_noops() {
        let title = crate::UiExpression::from_bytes(b"title").unwrap();
        let input_value = crate::UiExpression::from_bytes(b"username").unwrap();
        let input_disabled = crate::UiExpression::from_bytes(b"usernameDisabled").unwrap();
        let mut input_bindings = crate::UiBindingList::EMPTY;
        assert!(input_bindings.push(crate::UiBinding {
            property: crate::UiBindingProperty::Value,
            expression: input_value,
        }));
        assert!(input_bindings.push(crate::UiBinding {
            property: crate::UiBindingProperty::Disabled,
            expression: input_disabled,
        }));

        let mut document = crate::UiDocument::EMPTY;
        let root = document
            .push_node(crate::UiNodeTemplate {
                kind: UiNodeKind::Root,
                ..crate::UiNodeTemplate::EMPTY
            })
            .unwrap();
        document
            .push_node(crate::UiNodeTemplate {
                kind: UiNodeKind::Label,
                parent: root,
                text_binding: title,
                ..crate::UiNodeTemplate::EMPTY
            })
            .unwrap();
        document
            .push_node(crate::UiNodeTemplate {
                kind: UiNodeKind::TextInput,
                parent: root,
                tab_index: 0,
                bindings: input_bindings,
                ..crate::UiNodeTemplate::EMPTY
            })
            .unwrap();

        let mut values = UiBindingValueStore::new();
        assert!(
            values
                .set(title, UiBindingValue::Text(UiText::from_bytes(b"Welcome").unwrap()))
                .unwrap()
        );
        assert!(
            values
                .set(input_value, UiBindingValue::Text(UiText::from_bytes(b"admin").unwrap()))
                .unwrap()
        );
        assert!(values.set(input_disabled, UiBindingValue::Bool(true)).unwrap());

        let mut router = crate::UiEventRouter::new();
        let mut host = UiComponentTree::from_document(&document, &mut router).unwrap();
        assert_eq!(host.apply_document_bindings(&document, &values), Ok(3));
        let label = host.tree().handle_at(1).unwrap();
        let input = host.tree().handle_at(2).unwrap();
        assert_eq!(host.tree().node(label).unwrap().text.as_bytes(), b"Welcome");
        assert_eq!(host.value(input).unwrap().as_bytes(), b"admin");
        assert!(host.tree().node(input).unwrap().interaction.is_disabled());
        assert_eq!(host.apply_document_bindings(&document, &values), Ok(0));

        assert_eq!(
            host.apply_binding(
                input,
                crate::UiBinding {
                    property: crate::UiBindingProperty::Value,
                    expression: input_value
                },
                UiBindingValue::Bool(true),
            ),
            Err(UiComponentTreeError::TypeMismatch)
        );
        assert_eq!(
            host.apply_binding(
                input,
                crate::UiBinding {
                    property: crate::UiBindingProperty::Control,
                    expression: input_value
                },
                UiBindingValue::Text(UiText::EMPTY),
            ),
            Err(UiComponentTreeError::UnsupportedBinding)
        );
    }

    #[test]
    fn binding_value_store_is_bounded_and_generation_independent() {
        let mut values = UiBindingValueStore::new();
        for index in 0..MAX_UI_BINDING_VALUES {
            let expression = crate::UiExpression::from_bytes(&[b'x', index as u8]).unwrap();
            assert_eq!(values.set(expression, UiBindingValue::Bool(index % 2 == 0)), Ok(true));
        }
        let extra = crate::UiExpression::from_bytes(b"overflow").unwrap();
        assert_eq!(
            values.set(extra, UiBindingValue::Bool(true)),
            Err(UiBindingStoreError::Capacity)
        );
        assert_eq!(
            values.set(crate::UiExpression::EMPTY, UiBindingValue::Bool(true)),
            Err(UiBindingStoreError::InvalidExpression)
        );
    }

    #[test]
    fn focus_traversal_orders_tab_indices_and_supports_reverse_navigation() {
        let mut host = UiComponentTree::new();
        let root = host.insert(UiNodeKind::Root, UiNodeHandle::EMPTY, 1).unwrap();
        let first = host.insert(UiNodeKind::TextInput, root, 2).unwrap();
        let second = host.insert(UiNodeKind::Button, root, 3).unwrap();
        host.tree.node_mut(first).unwrap().interaction.set_tab_index(2);
        host.tree.node_mut(second).unwrap().interaction.set_tab_index(1);

        assert_eq!(host.focus_next(true), Ok(second));
        assert_eq!(host.focus_next(true), Ok(first));
        assert_eq!(host.focus_next(true), Ok(first));
        assert_eq!(host.focus_next(false), Ok(second));
        assert_eq!(host.focus_next(false), Ok(second));
    }

    #[test]
    fn host_rejects_stale_handles_and_reuses_slots_with_new_generations() {
        let mut host = UiComponentTree::new();
        let root = host.insert(UiNodeKind::Root, UiNodeHandle::EMPTY, 1).unwrap();
        let input = host.insert(UiNodeKind::TextInput, root, 2).unwrap();
        host.destroy(input).unwrap();
        let mut output = UiOutput::new();
        assert_eq!(
            host.dispatch(input, UiInputEvent::Submit, &mut output),
            Err(UiComponentTreeError::Stale)
        );
        let replacement = host.insert(UiNodeKind::TextInput, root, 3).unwrap();
        assert_eq!(replacement.slot, input.slot);
        assert_ne!(replacement.generation, input.generation);
        assert_eq!(host.value(input), Err(UiComponentTreeError::Stale));
    }

    #[test]
    fn routed_dispatch_keeps_component_and_handler_outputs_typed() {
        let mut host = UiComponentTree::new();
        let root = host.insert(UiNodeKind::Root, UiNodeHandle::EMPTY, 1).unwrap();
        let button = host.insert(UiNodeKind::Button, root, 2).unwrap();
        let mut router = crate::UiEventRouter::new();
        router.subscribe(button, crate::UiEventType::Click, crate::UiHandlerId::new(9)).unwrap();
        let mut component_output = UiOutput::new();
        let mut routed_output = UiOutput::new();

        host.dispatch_with_hooks(
            button,
            UiInputEvent::Click,
            &router,
            &mut component_output,
            &mut routed_output,
        )
        .unwrap();

        assert_eq!(component_output.pop(), Some(UiComponentEvent::Clicked { target: button }));
        assert_eq!(
            routed_output.pop(),
            Some(UiRoutedEvent {
                target: button,
                handler: crate::UiHandlerId::new(9),
                event: UiInputEvent::Click,
            })
        );
    }

    #[test]
    fn changed_output_routes_the_new_bounded_value() {
        let mut host = UiComponentTree::new();
        let root = host.insert(UiNodeKind::Root, UiNodeHandle::EMPTY, 1).unwrap();
        let input = host.insert(UiNodeKind::TextInput, root, 2).unwrap();
        host.focus(input).unwrap();
        let mut router = crate::UiEventRouter::new();
        router.subscribe(input, crate::UiEventType::Changed, crate::UiHandlerId::new(7)).unwrap();
        let mut component_output = UiOutput::new();
        let mut routed_output = UiOutput::new();

        host.dispatch_with_hooks(
            input,
            UiInputEvent::TextInput { scalar: u32::from(b'a') },
            &router,
            &mut component_output,
            &mut routed_output,
        )
        .unwrap();

        assert_eq!(
            component_output.pop(),
            Some(UiComponentEvent::Changed {
                target: input,
                value: UiText::from_bytes(b"a").unwrap(),
            })
        );
        assert_eq!(
            routed_output.pop().unwrap().event,
            UiInputEvent::Changed { value: UiText::from_bytes(b"a").unwrap() }
        );
    }

    #[test]
    fn compiled_document_installs_generation_safe_node_index_hooks() {
        let mut document = crate::UiDocument::EMPTY;
        let root = crate::UiNodeTemplate {
            kind: UiNodeKind::Root,
            parent: u16::MAX,
            event: crate::UiEvent {
                kind: crate::UiEventKind::Submit,
                handler: crate::UiExpression::from_bytes(b"submit").unwrap(),
            },
            ..crate::UiNodeTemplate::EMPTY
        };
        document.push_node(root).unwrap();
        let mut router = crate::UiEventRouter::new();
        let mut host = UiComponentTree::from_document(&document, &mut router).unwrap();
        let target = host.tree().handle_at(0).unwrap();
        let mut component_output = UiOutput::new();
        let mut routed_output = UiOutput::new();

        assert_eq!(
            host.dispatch_with_hooks(
                target,
                UiInputEvent::Submit,
                &router,
                &mut component_output,
                &mut routed_output,
            ),
            Ok(UiEventDisposition::Ignored)
        );
        assert_eq!(routed_output.pop().unwrap().handler, crate::UiHandlerId::new(0));
    }

    #[test]
    fn destroying_a_component_cleans_its_routes_before_slot_reuse() {
        let mut host = UiComponentTree::new();
        let root = host.insert(UiNodeKind::Root, UiNodeHandle::EMPTY, 1).unwrap();
        let button = host.insert(UiNodeKind::Button, root, 2).unwrap();
        let mut router = crate::UiEventRouter::new();
        router.subscribe(button, crate::UiEventType::Click, crate::UiHandlerId::new(3)).unwrap();

        assert_eq!(host.destroy_with_router(button, &mut router), Ok(1));
        let replacement = host.insert(UiNodeKind::Button, root, 4).unwrap();
        let mut component_output = UiOutput::new();
        let mut routed_output = UiOutput::new();
        host.dispatch_with_hooks(
            replacement,
            UiInputEvent::Click,
            &router,
            &mut component_output,
            &mut routed_output,
        )
        .unwrap();
        assert!(routed_output.is_empty());
    }

    #[test]
    fn routed_backpressure_does_not_mutate_component_state() {
        let mut host = UiComponentTree::new();
        let root = host.insert(UiNodeKind::Root, UiNodeHandle::EMPTY, 1).unwrap();
        let button = host.insert(UiNodeKind::Button, root, 2).unwrap();
        let mut router = crate::UiEventRouter::new();
        router.subscribe(button, crate::UiEventType::Click, crate::UiHandlerId::new(1)).unwrap();
        let mut component_output = UiOutput::new();
        let mut routed_output = UiOutput::new();
        for _ in 0..MAX_UI_OUTPUT_EVENTS {
            routed_output
                .emit(UiRoutedEvent {
                    target: button,
                    handler: crate::UiHandlerId::new(0),
                    event: UiInputEvent::Click,
                })
                .unwrap();
        }

        assert_eq!(
            host.dispatch_with_hooks(
                button,
                UiInputEvent::Click,
                &router,
                &mut component_output,
                &mut routed_output,
            ),
            Err(UiComponentTreeError::OutputFull)
        );
        assert!(component_output.is_empty());
    }
}
