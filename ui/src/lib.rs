#![no_std]

#[cfg(test)]
extern crate std;

mod avatar;
mod component;
mod component_tree;
mod components;
mod events;
mod layout;
mod motion;
mod reactive;
mod runtime;
mod select;
mod template;

pub use avatar::{MAX_UI_AVATAR_TEXT_BYTES, UiAvatarContent, UiAvatarImage, UiAvatarText};
pub use component::{
    MAX_UI_COMPONENT_MEMBERS, UiComponent, UiComponentContract, UiComponentInput,
    UiComponentMethod, UiComponentOutput, UiEventDisposition, UiValueType,
};
pub use component_tree::{
    MAX_UI_BINDING_VALUES, MAX_UI_COMPONENTS, UiBindingStoreError, UiBindingValue,
    UiBindingValueStore, UiComponentEvent, UiComponentTree, UiComponentTreeError, UiRouteMount,
};
pub use components::{
    UI_KEY_BACKSPACE, UI_KEY_DOWN, UI_KEY_ENTER, UI_KEY_LEFT, UI_KEY_RIGHT, UI_KEY_UP, UiButton,
    UiButtonEvent, UiCommandMenu, UiCommandMenuEvent, UiInput, UiInputEventOutput, UiPanel,
};
pub use events::{
    MAX_UI_EVENT_ROUTES, MAX_UI_OUTPUT_EVENTS, UiEventError, UiEventRouter, UiEventType,
    UiHandlerId, UiInputEvent, UiOutput, UiOutputError, UiRoutedEvent,
};
pub use layout::{
    UiEdges, UiLayoutAlignment, UiLayoutDirection, UiLayoutEngine, UiLayoutError, UiLayoutStyle,
    UiOverflow, UiSize,
};
pub use motion::{
    MAX_UI_KEYFRAMES, MAX_UI_MOTION_DURATION_MS, UI_ANIMATION_INFINITE_REPEAT, UI_MOTION_FRAME_MS,
    UiAnimationDirection, UiAnimationFill, UiAnimationPreset, UiAnimationSpec, UiAnimator,
    UiComputedStyle, UiEasing, UiKeyframe, UiMotionStatus, UiTransform, UiTransitionProperty,
    UiTransitionSpec,
};
pub use reactive::{
    MAX_UI_DEPENDENCIES, MAX_UI_DEPENDENCY_RECORDS, MAX_UI_INVALIDATIONS, MAX_UI_TRACE_ENTRIES,
    UiBindingTarget, UiCommitCoordinator, UiCommitError, UiComputed, UiDebugTrace,
    UiDependencyGraph, UiDependencySet, UiInvalidation, UiInvalidationKind, UiInvalidationQueue,
    UiReactiveError, UiReadable, UiSignal, UiSignalChange, UiSignalId, UiTraceEntry, UiTraceKind,
    UiWritable,
};
pub use runtime::{
    MAX_UI_NODES, TAB_INDEX_NONE, UiBlueprint, UiError, UiIcon, UiInteraction, UiInteractive,
    UiNode, UiNodeHandle, UiNodeKind, UiNodeSpec, UiRect, UiTree,
};
pub use select::{
    MAX_UI_SELECT_OPTIONS, UI_KEY_ESCAPE, UI_SELECT_GAP, UI_SELECT_NO_SELECTION, UiPopover,
    UiPopoverLayout, UiPopoverPlacement, UiSelect, UiSelectEvent,
};
pub use template::{
    MAX_UI_BINDINGS, MAX_UI_CONDITIONAL_STYLES, MAX_UI_EXPRESSION_BYTES, MAX_UI_NAME_BYTES,
    MAX_UI_STATE_STYLES, MAX_UI_STYLE_CONDITIONS, MAX_UI_STYLE_TOKENS, MAX_UI_TEXT_BYTES,
    UiBinding, UiBindingList, UiBindingProperty, UiConditionalStyle, UiConditionalStyleList,
    UiDocument, UiDocumentError, UiEvent, UiEventKind, UiExpression, UiName, UiNodeTemplate,
    UiStateStyle, UiStateStyleList, UiStyle, UiStyleConditions, UiStyleList, UiStyleResolveError,
    UiStyleState, UiStyleStateFlags, UiText,
};
