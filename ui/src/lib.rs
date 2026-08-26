#![no_std]

#[cfg(test)]
extern crate std;

mod component;
mod component_tree;
mod components;
mod events;
mod layout;
mod reactive;
mod runtime;
mod template;

pub use component::{
    MAX_UI_COMPONENT_MEMBERS, UiComponent, UiComponentContract, UiComponentInput,
    UiComponentMethod, UiComponentOutput, UiEventDisposition, UiValueType,
};
pub use component_tree::{
    MAX_UI_COMPONENTS, UiComponentEvent, UiComponentTree, UiComponentTreeError,
};
pub use components::{
    UI_KEY_BACKSPACE, UI_KEY_ENTER, UiButton, UiButtonEvent, UiInput, UiInputEventOutput, UiPanel,
};
pub use events::{
    MAX_UI_EVENT_ROUTES, MAX_UI_OUTPUT_EVENTS, UiEventError, UiEventRouter, UiEventType,
    UiHandlerId, UiInputEvent, UiOutput, UiOutputError, UiRoutedEvent,
};
pub use layout::{
    UiEdges, UiLayoutAlignment, UiLayoutDirection, UiLayoutEngine, UiLayoutError, UiLayoutStyle,
    UiOverflow, UiSize,
};
pub use reactive::{
    MAX_UI_DEPENDENCIES, MAX_UI_DEPENDENCY_RECORDS, MAX_UI_INVALIDATIONS, MAX_UI_TRACE_ENTRIES,
    UiBindingTarget, UiCommitCoordinator, UiCommitError, UiComputed, UiDebugTrace,
    UiDependencyGraph, UiDependencySet, UiInvalidation, UiInvalidationKind, UiInvalidationQueue,
    UiReactiveError, UiReadable, UiSignal, UiSignalChange, UiSignalId, UiTraceEntry, UiTraceKind,
    UiWritable,
};
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
