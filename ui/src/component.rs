use crate::{UiInputEvent, UiOutput, UiOutputError};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UiEventDisposition {
    Ignored,
    Consumed,
}

pub trait UiComponent {
    type Output: Copy;

    fn handle_event(
        &mut self,
        event: UiInputEvent,
        output: &mut UiOutput<Self::Output>,
    ) -> Result<UiEventDisposition, UiOutputError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum ToggleOutput {
        Changed(bool),
    }

    struct Toggle {
        enabled: bool,
    }

    impl UiComponent for Toggle {
        type Output = ToggleOutput;

        fn handle_event(
            &mut self,
            event: UiInputEvent,
            output: &mut UiOutput<Self::Output>,
        ) -> Result<UiEventDisposition, UiOutputError> {
            if event != UiInputEvent::Click {
                return Ok(UiEventDisposition::Ignored);
            }
            self.enabled = !self.enabled;
            output.emit(ToggleOutput::Changed(self.enabled))?;
            Ok(UiEventDisposition::Consumed)
        }
    }

    #[test]
    fn component_contract_dispatches_typed_outputs_without_runtime_reflection() {
        let mut toggle = Toggle { enabled: false };
        let mut output = UiOutput::new();
        assert_eq!(
            toggle.handle_event(UiInputEvent::KeyDown { code: 9, modifiers: 0 }, &mut output),
            Ok(UiEventDisposition::Ignored)
        );
        assert_eq!(
            toggle.handle_event(UiInputEvent::Click, &mut output),
            Ok(UiEventDisposition::Consumed)
        );
        assert_eq!(output.pop(), Some(ToggleOutput::Changed(true)));
    }
}
