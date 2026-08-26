use crate::{UiInputEvent, UiNodeKind, UiOutput, UiOutputError};

pub const MAX_UI_COMPONENT_MEMBERS: usize = 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum UiValueType {
    Unit = 1,
    Text = 2,
    Bool = 3,
    Form = 4,
    Control = 5,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiComponentInput {
    pub name: &'static str,
    pub value_type: UiValueType,
    pub writable: bool,
}

impl UiComponentInput {
    const EMPTY: Self = Self { name: "", value_type: UiValueType::Unit, writable: false };

    pub const fn new(name: &'static str, value_type: UiValueType, writable: bool) -> Self {
        Self { name, value_type, writable }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiComponentOutput {
    pub name: &'static str,
    pub value_type: UiValueType,
}

impl UiComponentOutput {
    const EMPTY: Self = Self { name: "", value_type: UiValueType::Unit };

    pub const fn new(name: &'static str, value_type: UiValueType) -> Self {
        Self { name, value_type }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiComponentMethod {
    pub name: &'static str,
}

impl UiComponentMethod {
    const EMPTY: Self = Self { name: "" };

    pub const fn new(name: &'static str) -> Self {
        Self { name }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiComponentContract {
    pub name: &'static str,
    pub kind: UiNodeKind,
    pub interactive: bool,
    pub inputs: [UiComponentInput; MAX_UI_COMPONENT_MEMBERS],
    pub input_count: u8,
    pub outputs: [UiComponentOutput; MAX_UI_COMPONENT_MEMBERS],
    pub output_count: u8,
    pub methods: [UiComponentMethod; MAX_UI_COMPONENT_MEMBERS],
    pub method_count: u8,
}

impl UiComponentContract {
    pub const EMPTY: Self = Self {
        name: "",
        kind: UiNodeKind::Panel,
        interactive: false,
        inputs: [UiComponentInput::EMPTY; MAX_UI_COMPONENT_MEMBERS],
        input_count: 0,
        outputs: [UiComponentOutput::EMPTY; MAX_UI_COMPONENT_MEMBERS],
        output_count: 0,
        methods: [UiComponentMethod::EMPTY; MAX_UI_COMPONENT_MEMBERS],
        method_count: 0,
    };

    pub const fn for_kind(kind: UiNodeKind) -> Self {
        match kind {
            UiNodeKind::Button => Self::new("ui.button", kind, true)
                .with_input(UiComponentInput::new("disabled", UiValueType::Bool, false))
                .with_output(UiComponentOutput::new("click", UiValueType::Unit))
                .with_output(UiComponentOutput::new("submit", UiValueType::Unit))
                .with_method(UiComponentMethod::new("focus")),
            UiNodeKind::TextInput => Self::new("ui.input", kind, true)
                .with_input(UiComponentInput::new("value", UiValueType::Text, true))
                .with_input(UiComponentInput::new("disabled", UiValueType::Bool, false))
                .with_input(UiComponentInput::new("control", UiValueType::Control, false))
                .with_output(UiComponentOutput::new("changed", UiValueType::Text))
                .with_output(UiComponentOutput::new("submit", UiValueType::Unit))
                .with_method(UiComponentMethod::new("focus")),
            UiNodeKind::Form => Self::new("ui.form", kind, false)
                .with_input(UiComponentInput::new("form", UiValueType::Form, false))
                .with_input(UiComponentInput::new("canSubmit", UiValueType::Bool, false))
                .with_output(UiComponentOutput::new("submit", UiValueType::Unit)),
            UiNodeKind::Root => Self::new("ui.root", kind, false),
            UiNodeKind::Panel => Self::new("ui.panel", kind, false),
            UiNodeKind::Label => Self::new("ui.text", kind, false),
        }
    }

    pub fn input(&self, name: &[u8]) -> Option<UiComponentInput> {
        self.inputs[..usize::from(self.input_count)]
            .iter()
            .copied()
            .find(|input| input.name.as_bytes() == name)
    }

    pub fn output(&self, name: &[u8]) -> Option<UiComponentOutput> {
        self.outputs[..usize::from(self.output_count)]
            .iter()
            .copied()
            .find(|output| output.name.as_bytes() == name)
    }

    pub fn method(&self, name: &[u8]) -> bool {
        self.methods[..usize::from(self.method_count)]
            .iter()
            .any(|method| method.name.as_bytes() == name)
    }

    pub const fn new(name: &'static str, kind: UiNodeKind, interactive: bool) -> Self {
        Self { name, kind, interactive, ..Self::EMPTY }
    }

    pub const fn with_input(mut self, input: UiComponentInput) -> Self {
        self.inputs[self.input_count as usize] = input;
        self.input_count += 1;
        self
    }

    pub const fn with_output(mut self, output: UiComponentOutput) -> Self {
        self.outputs[self.output_count as usize] = output;
        self.output_count += 1;
        self
    }

    pub const fn with_method(mut self, method: UiComponentMethod) -> Self {
        self.methods[self.method_count as usize] = method;
        self.method_count += 1;
        self
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UiEventDisposition {
    Ignored,
    Consumed,
}

pub trait UiComponent {
    type Output: Copy;

    const CONTRACT: UiComponentContract = UiComponentContract::EMPTY;

    fn handle_event(
        &mut self,
        event: UiInputEvent,
        output: &mut UiOutput<Self::Output>,
    ) -> Result<UiEventDisposition, UiOutputError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOGGLE_CONTRACT: UiComponentContract =
        UiComponentContract::new("controls.toggle", UiNodeKind::Button, true)
            .with_input(UiComponentInput::new("disabled", UiValueType::Bool, false))
            .with_output(UiComponentOutput::new("changed", UiValueType::Bool))
            .with_method(UiComponentMethod::new("focus"));
    const _: () = assert!(TOGGLE_CONTRACT.interactive);

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

    #[test]
    fn built_in_contracts_describe_typed_members_and_methods() {
        let input = UiComponentContract::for_kind(UiNodeKind::TextInput);
        assert_eq!(input.input(b"value").unwrap().value_type, UiValueType::Text);
        assert!(input.input(b"value").unwrap().writable);
        assert_eq!(input.output(b"changed").unwrap().value_type, UiValueType::Text);
        assert!(input.method(b"focus"));
        assert!(!input.method(b"rebuild_layout"));

        let button = UiComponentContract::for_kind(UiNodeKind::Button);
        assert!(button.input(b"value").is_none());
        assert_eq!(button.output(b"click").unwrap().value_type, UiValueType::Unit);
    }

    #[test]
    fn rust_components_can_declare_bounded_contracts() {
        assert_eq!(TOGGLE_CONTRACT.name, "controls.toggle");
        assert_eq!(TOGGLE_CONTRACT.kind, UiNodeKind::Button);
        assert_eq!(TOGGLE_CONTRACT.input(b"disabled").unwrap().value_type, UiValueType::Bool);
        assert_eq!(TOGGLE_CONTRACT.output(b"changed").unwrap().value_type, UiValueType::Bool);
        assert!(TOGGLE_CONTRACT.method(b"focus"));
    }
}
