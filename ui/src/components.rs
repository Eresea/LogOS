use crate::events::{UiInputEvent, UiOutput, UiOutputError};
use crate::runtime::{UiInteraction, UiInteractive};
use crate::template::{MAX_UI_TEXT_BYTES, UiText};
use crate::{UiComponent, UiComponentContract, UiEventDisposition};

pub const UI_KEY_ENTER: u16 = 2;
pub const UI_KEY_BACKSPACE: u16 = 3;
pub const UI_KEY_UP: u16 = 12;
pub const UI_KEY_DOWN: u16 = 13;
pub const UI_KEY_LEFT: u16 = 14;
pub const UI_KEY_RIGHT: u16 = 15;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UiButtonEvent {
    Clicked,
}

#[derive(Clone, Copy)]
pub struct UiButton {
    interaction: UiInteraction,
}

impl UiButton {
    pub const fn new() -> Self {
        Self { interaction: UiInteraction::for_kind(crate::UiNodeKind::Button) }
    }
}

impl Default for UiButton {
    fn default() -> Self {
        Self::new()
    }
}

impl UiInteractive for UiButton {
    fn interaction(&self) -> &UiInteraction {
        &self.interaction
    }

    fn interaction_mut(&mut self) -> &mut UiInteraction {
        &mut self.interaction
    }
}

impl UiComponent for UiButton {
    type Output = UiButtonEvent;
    const CONTRACT: UiComponentContract = UiComponentContract::for_kind(crate::UiNodeKind::Button);

    fn handle_event(
        &mut self,
        event: UiInputEvent,
        output: &mut UiOutput<Self::Output>,
    ) -> Result<UiEventDisposition, UiOutputError> {
        if self.is_disabled() {
            return Ok(UiEventDisposition::Ignored);
        }
        match event {
            UiInputEvent::Focus => {
                self.set_focused(true);
                Ok(UiEventDisposition::Consumed)
            }
            UiInputEvent::Blur => {
                self.set_focused(false);
                Ok(UiEventDisposition::Consumed)
            }
            UiInputEvent::Click | UiInputEvent::PointerUp { .. } | UiInputEvent::Submit => {
                output.emit(UiButtonEvent::Clicked)?;
                Ok(UiEventDisposition::Consumed)
            }
            UiInputEvent::KeyDown { code: UI_KEY_ENTER, .. } => {
                output.emit(UiButtonEvent::Clicked)?;
                Ok(UiEventDisposition::Consumed)
            }
            _ => Ok(UiEventDisposition::Ignored),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UiInputEventOutput {
    Changed(UiText),
    Submitted,
}

#[derive(Clone, Copy)]
pub struct UiInput {
    interaction: UiInteraction,
    value: [u8; MAX_UI_TEXT_BYTES],
    len: usize,
    readonly: bool,
    masked: bool,
}

impl UiInput {
    pub const fn new() -> Self {
        Self {
            interaction: UiInteraction::for_kind(crate::UiNodeKind::TextInput),
            value: [0; MAX_UI_TEXT_BYTES],
            len: 0,
            readonly: false,
            masked: false,
        }
    }

    pub fn value(&self) -> UiText {
        UiText::from_bytes(&self.value[..self.len]).unwrap_or(UiText::EMPTY)
    }

    pub fn set_value(&mut self, value: UiText) -> bool {
        if self.value() == value {
            return false;
        }
        self.value[..value.as_bytes().len()].copy_from_slice(value.as_bytes());
        self.len = value.as_bytes().len();
        true
    }

    pub fn clear_value(&mut self) -> bool {
        if self.len == 0 {
            return false;
        }
        self.value.fill(0);
        self.len = 0;
        true
    }

    pub const fn is_readonly(&self) -> bool {
        self.readonly
    }

    pub const fn is_masked(&self) -> bool {
        self.masked
    }

    pub fn set_readonly(&mut self, readonly: bool) {
        self.readonly = readonly;
    }

    pub fn set_masked(&mut self, masked: bool) {
        self.masked = masked;
    }

    fn append_scalar(&mut self, scalar: u32) -> bool {
        let mut encoded = [0; 4];
        let width = encode_scalar(scalar, &mut encoded);
        if width == 0 || self.len + width > MAX_UI_TEXT_BYTES {
            return false;
        }
        self.value[self.len..self.len + width].copy_from_slice(&encoded[..width]);
        self.len += width;
        true
    }

    fn pop_scalar(&mut self) -> bool {
        if self.len == 0 {
            return false;
        }
        self.len -= 1;
        while self.len > 0 && self.value[self.len] & 0xc0 == 0x80 {
            self.len -= 1;
        }
        true
    }
}

impl Default for UiInput {
    fn default() -> Self {
        Self::new()
    }
}

impl UiInteractive for UiInput {
    fn interaction(&self) -> &UiInteraction {
        &self.interaction
    }

    fn interaction_mut(&mut self) -> &mut UiInteraction {
        &mut self.interaction
    }
}

impl UiComponent for UiInput {
    type Output = UiInputEventOutput;
    const CONTRACT: UiComponentContract =
        UiComponentContract::for_kind(crate::UiNodeKind::TextInput);

    fn handle_event(
        &mut self,
        event: UiInputEvent,
        output: &mut UiOutput<Self::Output>,
    ) -> Result<UiEventDisposition, UiOutputError> {
        if self.is_disabled() {
            return Ok(UiEventDisposition::Ignored);
        }
        match event {
            UiInputEvent::Focus => {
                self.set_focused(true);
                Ok(UiEventDisposition::Consumed)
            }
            UiInputEvent::Blur => {
                self.set_focused(false);
                Ok(UiEventDisposition::Consumed)
            }
            UiInputEvent::TextInput { scalar }
                if self.is_focused() && !self.readonly && self.append_scalar(scalar) =>
            {
                output.emit(UiInputEventOutput::Changed(self.value()))?;
                Ok(UiEventDisposition::Consumed)
            }
            UiInputEvent::KeyDown { code: UI_KEY_BACKSPACE, .. }
                if self.is_focused() && !self.readonly && self.pop_scalar() =>
            {
                output.emit(UiInputEventOutput::Changed(self.value()))?;
                Ok(UiEventDisposition::Consumed)
            }
            UiInputEvent::KeyDown { code: UI_KEY_ENTER, .. } | UiInputEvent::Submit
                if self.is_focused() =>
            {
                output.emit(UiInputEventOutput::Submitted)?;
                Ok(UiEventDisposition::Consumed)
            }
            _ => Ok(UiEventDisposition::Ignored),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiPanel;

impl UiPanel {
    pub const fn new() -> Self {
        Self
    }
}

impl Default for UiPanel {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UiCommandMenuEvent {
    SelectionChanged { index: u8 },
    QueryChanged { value: UiText },
    Submitted { index: u8 },
}

/// Bounded keyboard behavior for a command menu. Layout and painting stay in
/// the host tree, while selection policy remains reusable and testable.
#[derive(Clone, Copy)]
pub struct UiCommandMenu {
    item_count: u8,
    selected: u8,
    query: [u8; MAX_UI_TEXT_BYTES],
    query_len: u8,
}

impl UiCommandMenu {
    pub const fn new(item_count: u8) -> Self {
        let item_count = if item_count == 0 {
            1
        } else if item_count > 8 {
            8
        } else {
            item_count
        };
        Self { item_count, selected: 0, query: [0; MAX_UI_TEXT_BYTES], query_len: 0 }
    }

    pub const fn item_count(self) -> u8 {
        self.item_count
    }

    pub const fn selected(self) -> u8 {
        self.selected
    }

    pub fn query(self) -> UiText {
        UiText::from_bytes(&self.query[..usize::from(self.query_len)]).unwrap_or(UiText::EMPTY)
    }

    pub fn set_item_count(&mut self, item_count: u8) -> bool {
        let item_count = item_count.min(8);
        let changed = self.item_count != item_count;
        self.item_count = item_count;
        if item_count == 0 {
            self.selected = 0;
        } else if self.selected >= item_count {
            self.selected = item_count - 1;
        }
        changed
    }

    pub fn append_text(&mut self, text: &[u8]) -> bool {
        let available = MAX_UI_TEXT_BYTES.saturating_sub(usize::from(self.query_len));
        let count = text.len().min(available);
        if count == 0 {
            return false;
        }
        let start = usize::from(self.query_len);
        self.query[start..start + count].copy_from_slice(&text[..count]);
        self.query_len += count as u8;
        true
    }

    pub fn clear_query(&mut self) -> bool {
        if self.query_len == 0 {
            return false;
        }
        self.query.fill(0);
        self.query_len = 0;
        true
    }

    pub fn pop_query_scalar(&mut self) -> bool {
        if self.query_len == 0 {
            return false;
        }
        self.query_len -= 1;
        while self.query_len > 0 && self.query[usize::from(self.query_len)] & 0xc0 == 0x80 {
            self.query_len -= 1;
        }
        true
    }

    pub fn set_selected(&mut self, index: u8) -> bool {
        if self.item_count == 0 {
            return false;
        }
        let index = index.min(self.item_count - 1);
        if self.selected == index {
            return false;
        }
        self.selected = index;
        true
    }

    pub fn handle_event(
        &mut self,
        event: UiInputEvent,
        output: &mut UiOutput<UiCommandMenuEvent>,
    ) -> Result<UiEventDisposition, UiOutputError> {
        let UiInputEvent::KeyDown { code, modifiers: 0 } = event else {
            if let UiInputEvent::TextInput { scalar } = event {
                let mut encoded = [0; 4];
                let width = encode_scalar(scalar, &mut encoded);
                if width != 0 && self.append_text(&encoded[..width]) {
                    output.emit(UiCommandMenuEvent::QueryChanged { value: self.query() })?;
                    return Ok(UiEventDisposition::Consumed);
                }
            }
            return Ok(UiEventDisposition::Ignored);
        };
        let next = match code {
            UI_KEY_UP | UI_KEY_LEFT => self.selected.saturating_sub(1),
            UI_KEY_DOWN | UI_KEY_RIGHT => (self.selected + 1).min(self.item_count - 1),
            UI_KEY_ENTER => {
                if self.item_count == 0 {
                    return Ok(UiEventDisposition::Consumed);
                }
                output.emit(UiCommandMenuEvent::Submitted { index: self.selected })?;
                return Ok(UiEventDisposition::Consumed);
            }
            UI_KEY_BACKSPACE => {
                if self.pop_query_scalar() {
                    output.emit(UiCommandMenuEvent::QueryChanged { value: self.query() })?;
                    return Ok(UiEventDisposition::Consumed);
                }
                return Ok(UiEventDisposition::Ignored);
            }
            _ => return Ok(UiEventDisposition::Ignored),
        };
        if self.set_selected(next) {
            output.emit(UiCommandMenuEvent::SelectionChanged { index: self.selected })?;
        }
        Ok(UiEventDisposition::Consumed)
    }
}

impl Default for UiCommandMenu {
    fn default() -> Self {
        Self::new(1)
    }
}

impl UiComponent for UiPanel {
    type Output = ();
    const CONTRACT: UiComponentContract = UiComponentContract::for_kind(crate::UiNodeKind::Panel);

    fn handle_event(
        &mut self,
        _event: UiInputEvent,
        _output: &mut UiOutput<Self::Output>,
    ) -> Result<UiEventDisposition, UiOutputError> {
        Ok(UiEventDisposition::Ignored)
    }
}

fn encode_scalar(scalar: u32, output: &mut [u8; 4]) -> usize {
    match scalar {
        0..=0x7f => {
            output[0] = scalar as u8;
            1
        }
        0x80..=0x7ff => {
            output[0] = 0xc0 | (scalar >> 6) as u8;
            output[1] = 0x80 | (scalar & 0x3f) as u8;
            2
        }
        0x800..=0xd7ff | 0xe000..=0xffff => {
            output[0] = 0xe0 | (scalar >> 12) as u8;
            output[1] = 0x80 | ((scalar >> 6) & 0x3f) as u8;
            output[2] = 0x80 | (scalar & 0x3f) as u8;
            3
        }
        0x10000..=0x10ffff => {
            output[0] = 0xf0 | (scalar >> 18) as u8;
            output[1] = 0x80 | ((scalar >> 12) & 0x3f) as u8;
            output[2] = 0x80 | ((scalar >> 6) & 0x3f) as u8;
            output[3] = 0x80 | (scalar & 0x3f) as u8;
            4
        }
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn button_emits_typed_clicks_and_respects_disabled_state() {
        let mut button = UiButton::new();
        let mut output = UiOutput::new();
        assert_eq!(
            button.handle_event(UiInputEvent::Focus, &mut output),
            Ok(UiEventDisposition::Consumed)
        );
        assert!(button.is_focused());
        assert_eq!(
            button.handle_event(
                UiInputEvent::KeyDown { code: UI_KEY_ENTER, modifiers: 0 },
                &mut output
            ),
            Ok(UiEventDisposition::Consumed)
        );
        assert_eq!(output.pop(), Some(UiButtonEvent::Clicked));
        button.set_disabled(true);
        assert_eq!(
            button.handle_event(UiInputEvent::Click, &mut output),
            Ok(UiEventDisposition::Ignored)
        );
        assert!(output.is_empty());
    }

    #[test]
    fn button_treats_pointer_up_as_a_typed_click() {
        let mut button = UiButton::new();
        let mut output = UiOutput::new();
        assert_eq!(
            button.handle_event(UiInputEvent::PointerUp { x: 1, y: 2 }, &mut output),
            Ok(UiEventDisposition::Consumed)
        );
        assert_eq!(output.pop(), Some(UiButtonEvent::Clicked));
    }

    #[test]
    fn input_emits_changes_for_focused_text_and_backspace() {
        let mut input = UiInput::new();
        let mut output = UiOutput::new();
        assert_eq!(
            input.handle_event(UiInputEvent::TextInput { scalar: u32::from(b'x') }, &mut output),
            Ok(UiEventDisposition::Ignored)
        );
        input.handle_event(UiInputEvent::Focus, &mut output).unwrap();
        assert_eq!(
            input.handle_event(UiInputEvent::TextInput { scalar: u32::from('é') }, &mut output),
            Ok(UiEventDisposition::Consumed)
        );
        assert_eq!(
            output.pop(),
            Some(UiInputEventOutput::Changed(UiText::from_bytes("é".as_bytes()).unwrap()))
        );
        assert_eq!(
            input.handle_event(UiInputEvent::TextInput { scalar: u32::from(b'x') }, &mut output),
            Ok(UiEventDisposition::Consumed)
        );
        assert_eq!(
            input.handle_event(
                UiInputEvent::KeyDown { code: UI_KEY_BACKSPACE, modifiers: 0 },
                &mut output
            ),
            Ok(UiEventDisposition::Consumed)
        );
        assert_eq!(input.value(), UiText::from_bytes("é".as_bytes()).unwrap());
    }

    #[test]
    fn input_submit_readonly_and_overflow_are_bounded() {
        let mut input = UiInput::new();
        let mut output = UiOutput::new();
        input.handle_event(UiInputEvent::Focus, &mut output).unwrap();
        input.set_readonly(true);
        assert_eq!(
            input.handle_event(UiInputEvent::TextInput { scalar: u32::from(b'a') }, &mut output),
            Ok(UiEventDisposition::Ignored)
        );
        assert_eq!(
            input.handle_event(
                UiInputEvent::KeyDown { code: UI_KEY_ENTER, modifiers: 0 },
                &mut output
            ),
            Ok(UiEventDisposition::Consumed)
        );
        assert_eq!(output.pop(), Some(UiInputEventOutput::Submitted));
        input.set_readonly(false);
        for _ in 0..MAX_UI_TEXT_BYTES {
            assert_eq!(
                input
                    .handle_event(UiInputEvent::TextInput { scalar: u32::from(b'a') }, &mut output),
                Ok(UiEventDisposition::Consumed)
            );
            let _ = output.pop();
        }
        assert_eq!(
            input.handle_event(UiInputEvent::TextInput { scalar: u32::from(b'b') }, &mut output),
            Ok(UiEventDisposition::Ignored)
        );
    }

    #[test]
    fn programmatic_values_do_not_emit_component_events() {
        let mut input = UiInput::new();
        let value = UiText::from_bytes(b"admin").unwrap();
        assert!(input.set_value(value));
        assert_eq!(input.value(), value);
        assert!(!input.set_value(value));
    }

    #[test]
    fn clearing_a_value_zeroes_the_bounded_buffer() {
        let mut input = UiInput::new();
        input.set_value(UiText::from_bytes(b"secret").unwrap());
        assert!(input.clear_value());
        assert_eq!(input.value(), UiText::EMPTY);
        assert!(!input.clear_value());
    }

    #[test]
    fn command_menu_selection_is_bounded_and_typed() {
        let mut menu = UiCommandMenu::new(4);
        let mut output = UiOutput::new();
        menu.handle_event(UiInputEvent::KeyDown { code: UI_KEY_DOWN, modifiers: 0 }, &mut output)
            .unwrap();
        assert_eq!(menu.selected(), 1);
        assert_eq!(output.pop(), Some(UiCommandMenuEvent::SelectionChanged { index: 1 }));
        for _ in 0..8 {
            menu.handle_event(
                UiInputEvent::KeyDown { code: UI_KEY_DOWN, modifiers: 0 },
                &mut output,
            )
            .unwrap();
        }
        assert_eq!(menu.selected(), 3);
        output.clear();
        menu.handle_event(UiInputEvent::KeyDown { code: UI_KEY_ENTER, modifiers: 0 }, &mut output)
            .unwrap();
        assert_eq!(output.pop(), Some(UiCommandMenuEvent::Submitted { index: 3 }));
    }

    #[test]
    fn command_menu_query_is_bounded_and_editable() {
        let mut menu = UiCommandMenu::new(4);
        let mut output = UiOutput::new();
        menu.handle_event(UiInputEvent::TextInput { scalar: u32::from('x') }, &mut output).unwrap();
        assert_eq!(menu.query(), UiText::from_bytes(b"x").unwrap());
        assert_eq!(
            output.pop(),
            Some(UiCommandMenuEvent::QueryChanged { value: UiText::from_bytes(b"x").unwrap() })
        );
        menu.set_item_count(1);
        menu.handle_event(
            UiInputEvent::KeyDown { code: UI_KEY_BACKSPACE, modifiers: 0 },
            &mut output,
        )
        .unwrap();
        assert_eq!(menu.query(), UiText::EMPTY);
        assert_eq!(output.pop(), Some(UiCommandMenuEvent::QueryChanged { value: UiText::EMPTY }));
    }
}
