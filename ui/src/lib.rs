#![no_std]

#[cfg(test)]
extern crate std;

mod runtime;
mod template;

pub use runtime::{
    MAX_UI_NODES, TAB_INDEX_NONE, UiBlueprint, UiError, UiInteraction, UiInteractive, UiNode,
    UiNodeHandle, UiNodeKind, UiNodeSpec, UiRect, UiTree,
};
pub use template::{
    MAX_UI_BINDINGS, MAX_UI_CONDITIONAL_STYLES, MAX_UI_EXPRESSION_BYTES, MAX_UI_NAME_BYTES,
    MAX_UI_STATE_STYLES, MAX_UI_STYLE_TOKENS, MAX_UI_TEXT_BYTES, UiBinding, UiBindingList,
    UiBindingProperty, UiConditionalStyle, UiConditionalStyleList, UiDocument, UiEvent,
    UiEventKind, UiExpression, UiName, UiNodeTemplate, UiStateStyle, UiStateStyleList, UiStyle,
    UiStyleList, UiStyleState, UiText,
};
