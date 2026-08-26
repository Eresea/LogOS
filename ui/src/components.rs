use crate::events::{UiInputEvent, UiOutput, UiOutputError};
use crate::runtime::{UiInteraction, UiInteractive};
use crate::template::{MAX_UI_TEXT_BYTES, UiText};
use crate::{UiComponent, UiComponentContract, UiEventDisposition};

pub const UI_KEY_ENTER: u16 = 2;
pub const UI_KEY_BACKSPACE: u16 = 3;

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
            UiInputEvent::Click | UiInputEvent::Submit => {
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
}
