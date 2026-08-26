use crate::events::{
    MAX_UI_OUTPUT_EVENTS, UiEventError, UiEventRouter, UiInputEvent, UiOutput, UiOutputError,
    UiRoutedEvent,
};
use crate::runtime::{UiError, UiNodeHandle, UiNodeKind, UiTree};
use crate::{
    UiButton, UiButtonEvent, UiComponent, UiEventDisposition, UiInput, UiInputEventOutput,
    UiInteractive, UiPanel, UiText,
};

pub const MAX_UI_COMPONENTS: usize = crate::MAX_UI_NODES;

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
            self.tree.node_mut(old).map_err(map_tree_error)?.interaction.set_focused(false);
        }
        self.components[usize::from(handle.slot)].set_focused(true);
        self.tree.node_mut(handle).map_err(map_tree_error)?.interaction.set_focused(true);
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
        self.tree.node_mut(old).map_err(map_tree_error)?.interaction.set_focused(false);
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
        self.tree.node_mut(handle).map_err(map_tree_error)?.interaction.set_disabled(disabled);
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

    pub fn dispatch(
        &mut self,
        handle: UiNodeHandle,
        event: UiInputEvent,
        output: &mut UiOutput<UiComponentEvent>,
    ) -> Result<UiEventDisposition, UiComponentTreeError> {
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
        let Some(result) = result else { return Ok(disposition) };
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
        output.emit(event).map_err(|_| UiComponentTreeError::OutputFull)?;
        Ok(UiEventDisposition::Consumed)
    }

    pub fn dispatch_with_hooks(
        &mut self,
        handle: UiNodeHandle,
        event: UiInputEvent,
        router: &UiEventRouter,
        component_output: &mut UiOutput<UiComponentEvent>,
        routed_output: &mut UiOutput<UiRoutedEvent>,
    ) -> Result<UiEventDisposition, UiComponentTreeError> {
        if router.is_subscribed(handle, event.event_type())
            && routed_output.len() == MAX_UI_OUTPUT_EVENTS
        {
            return Err(UiComponentTreeError::OutputFull);
        }
        let disposition = self.dispatch(handle, event, component_output)?;
        router.dispatch(handle, event, routed_output).map_err(map_event_error)?;
        Ok(disposition)
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
