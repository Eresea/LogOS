# LogOS UI Architecture

## Status

**Design direction / pre-implementation specification**

This document consolidates the current design decisions for the first LogOS graphical UI framework. The immediate consumer is the lock screen, but the architecture is intended to scale to system UI, desktop shell interfaces, and later application UI.

The goal is not to reproduce HTML/CSS/JavaScript or Angular. Instead, LogOS should adopt the strongest parts of modern declarative UI authoring while keeping the runtime small, typed, explicit, and native to Rust.

---

# 1. Goals

The LogOS UI system should make it easy to:

- build new interfaces quickly;
- compose reusable components;
- bind typed data into UI nodes;
- handle typed events;
- support writable/two-way bindings where appropriate;
- style interfaces without creating large numbers of custom style classes;
- share live state and services between components;
- directly control nodes/components when imperative interaction is genuinely required;
- expose safe native LogOS APIs such as terminal, filesystem, clipboard, session, power, and similar services;
- compile authoring conveniences away wherever possible;
- keep rendering, layout, event routing, invalidation, and component lifetime inside a native Rust runtime;
- maintain clear encapsulation and public/private component boundaries;
- remain compatible with LogOS's capability-oriented security model.

The resulting authoring model should feel closer to Angular templates plus Tailwind than to manually building widget trees in Rust.

---

# 2. High-Level Architecture

```text
Template
   │
   ├── component/node structure
   ├── inputs
   ├── outputs/events
   ├── writable bindings
   ├── interpolation
   ├── structural expressions
   ├── local references
   └── utility styling
   │
   ▼
Template compiler
   │
   ├── parse
   ├── resolve components/namespaces
   ├── type-check bindings
   ├── type-check events
   ├── validate refs/public APIs
   ├── resolve style utilities
   └── build dependency metadata
   │
   ▼
Typed UI representation
   │
   ▼
Rust UI runtime
   │
   ├── instantiate component tree
   ├── dependency injection
   ├── state invalidation
   ├── event routing
   ├── focus / hit testing
   ├── layout
   ├── paint invalidation
   └── lifecycle / cleanup
   │
   ▼
Renderer / compositor
```

The renderer should not know about template syntax such as `[value]`, `(click)`, `[(value)]`, or Tailwind-like utilities.

Those are authoring/compiler features.

---

# 3. Component Files

A component can initially consist of two files:

```text
lock-screen/
    lock.ui
    lock.rs
```

The template contains structure and styling.

The Rust file contains state, behavior, services, inputs, outputs, references, and public imperative APIs.

A separate style file may be introduced later for cases where utility styling is insufficient:

```text
lock-screen/
    lock.ui
    lock.rs
    lock.style
```

It should not be mandatory.

Utility-first styling should handle most normal UI.

---

# 4. Template Language

The template language should intentionally stay small.

Its core concepts are:

- nodes/components;
- inputs;
- events;
- writable bindings;
- text interpolation;
- utility styling;
- structural composition;
- optional local references.

Example:

```html
<ui.column {h-full items-center justify-center}>
    <ui.text {text-4xl font-light}>
        {{ time }}
    </ui.text>

    <input
        #passwordInput
        [(value)]="password"
        [disabled]="unlocking"
        (submit)="unlock()"
        {w-96 mt-8}
    />

    <button
        [disabled]="!canUnlock"
        (click)="unlock()"
        {mt-4 px-6 py-3 rounded-lg bg-accent}
    >
        Unlock
    </button>
</ui.column>
```

---

# 5. Node and Component Resolution

## 5.1 Nodes are unnamed by default

Normal nodes do not require IDs or names.

```html
<input [(value)]="password" />
```

Identity is only introduced when direct programmatic access is needed.

---

## 5.2 No implicit primitive-name fallback

LogOS should not maintain a hidden list where names such as `<input>`, `<button>`, or `<row>` may silently mean built-in primitives.

That creates ambiguity and makes typos dangerous.

Instead, platform primitives are explicit through namespaces.

```html
<ui.column>
    <ui.text>Hello</ui.text>
</ui.column>
```

A plain name refers to an imported or locally declared component:

```html
<input />
<terminal />
<user-avatar />
```

If it cannot be resolved, compilation fails.

```text
error: component `input` is not in scope
```

There is no fallback to "generic node".

---

## 5.3 Namespaces

Namespaces explicitly identify primitive/platform families:

```html
<ui.row />
<ui.column />
<ui.text />
<ui.image />
```

Other namespaces may later exist where useful:

```html
<shell.avatar />
<media.image />
```

The important property is deterministic resolution.

Conceptually:

```text
<ui.input>    -> platform UI primitive
<input>       -> imported/local component
```

---

## 5.4 Imports and aliases

Rust-side imports can bring components into template scope.

Conceptually:

```rust
use logos_ui::components::Input as input;
use logos_ui::components::Button as button;
```

The template can then use:

```html
<input />
<button />
```

Aliases are useful when two component libraries expose similar names.

---

# 6. Inputs

Component inputs are explicit and typed.

Example component:

```rust
#[component]
pub struct Input {
    #[input]
    pub value: String,

    #[input]
    pub disabled: bool,
}
```

Template:

```html
<input
    [value]="password"
    [disabled]="unlocking"
/>
```

The compiler validates input names and types.

If:

```text
Input.value: String
password:    String
```

the binding is valid.

If incompatible values are supplied, compilation fails.

Inputs are part of the component's public declarative interface.

---

# 7. Events / Outputs

Components expose typed events.

Conceptually:

```rust
#[component]
pub struct Input {
    #[output]
    pub changed: Event<String>,

    #[output]
    pub submit: Event<()>,
}
```

Template:

```html
<input
    (changed)="passwordChanged($event)"
    (submit)="unlock()"
/>
```

The compiler knows the type of `$event`.

For example:

```rust
fn password_changed(value: String) {
    // ...
}
```

must receive the type declared by the component event.

This keeps event contracts strongly typed instead of relying on generic DOM-style event objects.

---

# 8. Writable / Two-Way Bindings

Angular-like writable bindings are supported:

```html
<input [(value)]="password" />
```

Conceptually, this is equivalent to:

```html
<input
    [value]="password"
    (value)="password = $event"
/>
```

but the compiler/runtime should implement the relationship directly rather than literally expanding it into strings or generic events.

The target must be readable and writable.

A useful conceptual model is:

```text
Readable<T>
Writable<T>

Signal<T>   = Readable<T> + Writable<T>
Computed<T> = Readable<T>
```

Therefore:

```html
<input [(value)]="password" />
```

works when `password` is writable.

A computed read-only expression does not:

```html
<input [(value)]="fullName" />
```

if `fullName` is `Computed<String>`.

Compilation should fail with a clear error such as:

```text
error: `fullName` is readable but not writable
```

---

# 9. Text Interpolation

Text values can be interpolated:

```html
<ui.text>
    {{ user.displayName }}
</ui.text>
```

Interpolation expressions are typed and dependency-tracked.

The template compiler should determine which reactive values each interpolation depends upon.

---

# 10. Signals and Reactive State

Signals are useful in LogOS UI specifically for:

- dependency tracking;
- targeted invalidation;
- computed values;
- writable bindings;
- efficient updates.

They should not become a system-wide hidden event architecture.

Example:

```rust
let password = Signal::new(String::new());

let can_unlock = Computed::new(|| {
    !password.read().is_empty()
});
```

Template:

```html
<button [disabled]="!canUnlock">
    Unlock
</button>
```

The runtime should know that this property depends on `canUnlock`, which itself depends on `password`.

A change can therefore target only affected bindings/nodes.

```text
password changed
      │
      ▼
invalidate dependent computations
      │
      ▼
update affected node properties
      │
      ├── layout invalidation only if required
      └── paint invalidation only if required
```

The UI should not need whole-screen rerenders for ordinary state changes.

---

# 11. Styling

## 11.1 Utility-first styling

LogOS should adopt a Tailwind-like styling model.

Example:

```html
<ui.column {
    h-full
    items-center
    justify-center
    gap-4
    px-8
}>
```

The braces represent style utilities rather than arbitrary CSS class names.

Utilities compile into native style values.

Conceptually:

```text
items-center
gap-4
px-8
```

becomes something similar to:

```rust
Style {
    align_items: Center,
    gap: Spacing::S4,
    padding_left: Spacing::S8,
    padding_right: Spacing::S8,
    ..
}
```

There should be no runtime CSS selector matching or general cascade.

---

## 11.2 State variants

Useful state variants are represented as bounded state-style records:

```html
<button {
    px-4
    py-2
    rounded-md
    bg-accent
    focus:bg-accent
}>
```

Likely states include:

- hover;
- focus;
- pressed;
- disabled;
- selected;
- checked where applicable.

---

## 11.3 Conditional styles

Dynamic utility application should be supported.

Exact syntax remains open, but conceptually:

```html
<ui.row {
    items-center
    [opacity-50]="hasError"
}>
```

The first supported state is `focus`. Unsupported state names are compile
diagnostics rather than runtime selector lookups.

The compiler stores conditional utilities separately from ordinary bindings.
The expression is a bounded boolean name resolved by the owning component; it
is not a general runtime expression evaluator. A false value removes the
utility from the resolved style set, and a true value activates it. Each node
has fixed limits for state and conditional style records.

---

## 11.4 Optional local styles

A separate `.style` file may later support:

- complex animations;
- reusable style compositions;
- advanced component-local states;
- cases that are awkward with utility classes.

It should remain scoped and avoid recreating the unrestricted CSS cascade.

---

# 12. Structural Composition

The system will eventually need first-class conditional and iterative tree construction.

Prefer explicit template syntax rather than Angular-style directives.

Example:

```html
@if authenticated {
    <desktop />
} @else {
    <login />
}
```

Iteration:

```html
@for user in users {
    <user-row [user]="user" />
}
```

These are compiler-understood structural operations, not arbitrary behaviors attached to nodes.

---

# 13. Directives

Angular-style arbitrary directives should not be implemented.

Avoid mechanisms like:

```html
<div someBehavior />
```

where hidden behavior can attach to arbitrary nodes.

They make behavior less explicit and complicate reasoning, compilation, and runtime behavior.

If LogOS needs a common structural capability, it should generally become:

- first-class template syntax;
- a normal component;
- a style utility;
- or an injected service.

---

# 14. Pipes

Pipes are currently not planned.

Angular-style:

```html
{{ user.name | uppercase }}
```

does not provide enough value over:

```html
{{ uppercase(user.name) }}
```

or a computed property:

```rust
let display_name = Computed::new(|| format_user_name(user.read()));
```

```html
{{ displayName }}
```

Pipes may be reconsidered only if real UI authoring reveals a recurring ergonomic need.

---

# 15. Lifecycle Hooks

Avoid a large lifecycle API.

LogOS should not reproduce callback sets such as:

```text
OnInit
OnChanges
DoCheck
AfterContentInit
AfterContentChecked
AfterViewInit
AfterViewChecked
OnDestroy
...
```

Most normal state behavior should be handled by signals, effects, bindings, services, and explicit events.

If necessary, a minimal lifecycle may eventually include operations similar to:

```rust
fn mounted(&mut self) {}
fn unmounted(&mut self) {}
```

Effects should preferably clean themselves up when their component scope is destroyed.

Lifecycle APIs should be added only in response to concrete needs.

---

# 16. Zones

Zone-like automatic async/change-detection interception should not be implemented.

State updates should be explicit through LogOS's own reactive and event mechanisms.

The runtime should know what changed because state dependencies are known, not because asynchronous operations are globally monkey-patched or intercepted.

---

# 17. Programmatic References

Bindings and events remain the default interaction model, but some UI operations are inherently imperative.

Nodes/components can therefore receive an optional local reference.

Template:

```html
<input
    #passwordInput
    [(value)]="password"
/>
```

Rust:

```rust
#[ref("passwordInput")]
password_input: UiRef<Input>;
```

Usage:

```rust
self.password_input.focus();
self.password_input.select_all();
```

Custom components work similarly:

```html
<terminal #terminal />
```

```rust
#[ref("terminal")]
terminal: ComponentRef<Terminal>;
```

```rust
self.terminal.clear();
self.terminal.scroll_to_bottom();
```

The compiler should verify:

- that the named reference exists;
- that the referenced node/component type is correct;
- that invoked APIs are publicly exposed.

---

# 18. Component Encapsulation

Component internals are private by default.

The public component contract consists of explicit categories:

```text
#[input]   parent -> component declarative data
#[output]  component -> parent events
#[expose]  public imperative API
private    internal implementation
```

Example:

```rust
#[component]
pub struct Terminal {
    #[input]
    pub cwd: Signal<Path>,

    #[output]
    pub command_executed: Event<CommandResult>,

    internal_buffer: Buffer,
}
```

Public imperative methods:

```rust
#[expose]
pub fn clear(&mut self) {
    // ...
}

#[expose]
pub fn focus(&mut self) {
    // ...
}

fn rebuild_layout(&mut self) {
    // private
}
```

A parent with a reference can call:

```rust
terminal.clear();          // valid
terminal.focus();          // valid
terminal.rebuild_layout(); // compile error
```

This preserves normal Rust-style encapsulation across template/component boundaries.

---

# 19. Preferred Interaction Order

Inter-component communication should follow a clear preference hierarchy.

## 19.1 Bindings

Use bindings for parent-to-child data flow.

```html
<user-card [user]="selectedUser" />
```

---

## 19.2 Events

Use events for child-to-parent communication.

```html
<user-card (selected)="selectUser($event)" />
```

---

## 19.3 Injected services

Use shared services when multiple components/screens legitimately operate on shared state or shared system functionality.

```text
SessionService
   ├── LockScreen
   ├── UserMenu
   └── DesktopShell
```

---

## 19.4 References

Use refs for genuinely imperative UI operations.

Good examples:

```rust
input.focus();
terminal.scroll_to_bottom();
canvas.capture_pointer();
list.scroll_to(index);
```

Directly mutating another component's ordinary business state through refs should usually be avoided.

---

# 20. Parent, Child, and Sibling Interaction

## Child -> Parent

Use events.

```html
<password-input (submit)="unlock()" />
```

A child should not normally retrieve and mutate its parent directly.

---

## Parent -> Child

Use:

- inputs for normal state/data;
- refs for imperative operations.

---

## Sibling -> Sibling

Prefer parent-mediated state:

```text
Component A
    │ event
    ▼
Parent state
    │ binding
    ▼
Component B
```

For broader collaboration, use an injected shared service.

Arbitrary sibling lookup should not be part of the normal programming model.

---

# 21. Dependency Injection

Dependency injection is a first-class planned feature.

It is valuable for:

- shared service logic;
- shared live state;
- system APIs;
- application-wide state;
- screen/subtree-local services;
- testability;
- future capability-aware authority injection.

Example:

```rust
#[component]
struct LockScreen {
    #[inject]
    session: SessionService,

    #[inject]
    clock: ClockService,

    #[inject]
    power: PowerService,
}
```

Multiple components can consume the same reactive service.

---

# 22. Hierarchical DI

DI should be hierarchical.

Conceptually:

```text
System/App scope
│
├── SessionService
├── ThemeService
├── InputService
│
└── LockScreen scope
    │
    ├── LockScreen-specific services
    └── child component scopes
```

Dependency resolution walks outward through the scope hierarchy.

A subtree can override a service without replacing it globally.

Example concept:

```rust
provide::<ThemeService>(LockScreenTheme::new());
```

This also supports tests:

```rust
provide::<SessionService>(FakeSessionService::new());
```

---

# 23. DI Is Not a String-Based Service Locator

Dependencies should be statically declared.

Prefer:

```rust
#[inject]
session: SessionService
```

over:

```rust
services.get("SessionService")
```

This lets the compiler/runtime know component requirements ahead of time.

Conceptually:

```text
LockScreen
├ requires SessionService
├ requires PowerService
└ requires ClockService
```

This improves:

- static checking;
- dependency analysis;
- testing;
- security analysis;
- future capability integration.

---

# 24. Service Lifetimes

The initial DI model likely only needs a small number of lifetimes.

Potential examples:

```rust
#[singleton]
struct ThemeService;

#[scoped]
struct LockScreenState;
```

The most important distinctions are:

- system/application singleton;
- screen/subtree scoped.

Transient services can be introduced later if an actual use case requires them.

---

# 25. Native LogOS Rust APIs

LogOS should expose easy, strongly typed native Rust APIs for operating system facilities.

Examples:

```rust
use logos::fs::Filesystem;
use logos::terminal::Terminal;
use logos::notifications::Notifications;
use logos::session::Session;
use logos::clipboard::Clipboard;
use logos::power::Power;
```

These are OS/application APIs, not UI widgets.

They should be usable independently of the graphical UI framework.

---

# 26. Native APIs Through DI

UI components can consume native APIs through dependency injection.

Example:

```rust
#[component]
struct FilePicker {
    #[inject]
    filesystem: Filesystem,

    #[inject]
    clipboard: Clipboard,
}
```

Usage:

```rust
let files = self.filesystem.read_dir(path).await?;
```

```rust
self.clipboard.write_text(path.to_string()).await?;
```

This makes native system functionality easy to use while keeping dependencies explicit.

---

# 27. Terminal API vs Terminal Component

System functionality and visual components must remain distinct.

```text
logos::terminal
     │
     ▼
native terminal/session API
```

versus:

```text
<terminal>
     │
     ▼
visual terminal component
```

The native API handles concepts such as:

- terminal sessions;
- process/shell attachment;
- input/output streams;
- terminal state;
- session lifetime.

The visual component displays and interacts with one of those sessions.

Conceptual usage:

```rust
let session = terminal.spawn(TerminalOptions {
    cwd: project_path,
    shell: Shell::Flow,
}).await?;
```

Template:

```html
<terminal [session]="session" />
```

The same separation applies to other features.

---

# 28. Filesystem API vs File Browser

Similarly:

```text
Filesystem API
    │
    ├── enumerate
    ├── read
    ├── write
    ├── move
    └── metadata
```

is distinct from:

```text
<FileBrowser>
    │
    └── graphical file browsing component
```

The component consumes the native API rather than implementing filesystem functionality itself.

---

# 29. DI and LogOS Capabilities

A future LogOS-specific advantage is integrating DI with the capability/authority model.

A component may request something conceptually like:

```rust
#[inject]
filesystem: Filesystem<ReadOnly>;
```

rather than:

```rust
#[inject]
filesystem: Filesystem<ReadWrite>;
```

The application/component should only receive services and authority that its environment actually permits.

This allows DI to become more than convenience:

```text
dependency resolution
        +
service lifetime
        +
authority/capability distribution
```

The exact capability API remains a future design task.

---

# 30. Runtime Event Model

The Rust runtime should own standard typed UI events.

Conceptually:

```rust
enum UiEvent {
    PointerDown(PointerEvent),
    PointerUp(PointerEvent),
    PointerMove(PointerEvent),
    Click(PointerEvent),

    KeyDown(KeyEvent),
    KeyUp(KeyEvent),
    TextInput(TextEvent),

    Focus,
    Blur,

    Scroll(ScrollEvent),
}
```

The runtime is responsible for:

```text
device-independent input
        │
        ▼
compositor / surface
        │
        ▼
hit testing
        │
        ▼
UI node
        │
        ▼
component event/output
        │
        ▼
Rust handler
```

DOM-style capture/bubble behavior should not be copied automatically. It can be considered later only if concrete interaction requirements justify it.

---

# 31. Template Compilation

Templates should be compiled, not interpreted as markup at runtime.

Pipeline:

```text
component.ui
    │
    ▼
parser
    │
    ▼
template AST
    │
    ├── resolve namespaces/components
    ├── type-check inputs
    ├── type-check outputs
    ├── validate writable bindings
    ├── validate refs/exposed methods
    ├── resolve structural expressions
    ├── resolve style utilities
    └── build reactive dependencies
    │
    ▼
typed UI IR
    │
    ▼
generated representation
    │
    ▼
native Rust UI runtime
```

The exact generated format remains open.

Possible options include:

- generated Rust;
- static typed descriptors;
- compact binary metadata;
- a combination of code and metadata.

The important requirement is that normal template authoring should not require runtime parsing.

---

# 32. Runtime Responsibilities

The Rust UI runtime should handle:

- component tree instantiation;
- component scope creation/destruction;
- DI resolution;
- local reference resolution;
- event registration and routing;
- signal subscriptions;
- dependency invalidation;
- focus;
- hit testing;
- layout;
- paint invalidation;
- renderer interaction;
- cleanup.

The template compiler tells the runtime what exists and how it is connected.

The runtime executes it efficiently.

---

# 33. Update Model

The framework should prefer targeted updates rather than generic "rerender the component" behavior.

Example dependency metadata:

```text
Text #4
    depends on clock.time

Input #7.value
    depends on password

Button #9.disabled
    depends on canUnlock
```

If `password` changes, only affected properties need reevaluation.

The runtime can then determine whether the resulting property changes require:

- no visual work;
- repaint;
- relayout;
- subtree reconstruction.

This should make high-refresh-rate interfaces efficient without requiring developers to manually suppress ordinary UI updates.

---

# 34. Example Lock Screen

Template:

```html
<ui.column {
    h-full
    items-center
    justify-center
    bg-surface
}>
    <ui.text {text-5xl font-light}>
        {{ clock.time }}
    </ui.text>

    <user-avatar [user]="session.currentUser" />

    <password-input
        #passwordInput
        [(value)]="password"
        [disabled]="unlocking"
        (submit)="unlock()"
        {w-96 mt-8}
    />

    <button
        [disabled]="!canUnlock"
        (click)="unlock()"
        {mt-4 px-6 py-3 rounded-lg bg-accent disabled:opacity-50}
    >
        Unlock
    </button>
</ui.column>
```

Rust:

```rust
#[component]
pub struct LockScreen {
    password: Signal<String>,
    unlocking: Signal<bool>,

    #[ref("passwordInput")]
    password_input: ComponentRef<PasswordInput>,

    #[inject]
    session: Session,

    #[inject]
    clock: Clock,

    #[inject]
    power: Power,
}

impl LockScreen {
    fn can_unlock(&self) -> bool {
        !self.password.read().is_empty()
            && !self.unlocking.read()
    }

    async fn unlock(&mut self) {
        self.unlocking.write(true);

        let result = self
            .session
            .unlock(self.password.read())
            .await;

        self.unlocking.write(false);

        if result.is_err() {
            self.password.write(String::new());
            self.password_input.focus();
        }
    }
}
```

The lock screen owns presentation and interaction.

Authentication authority remains in the appropriate native LogOS service.

---

# 35. Component Contract Summary

A LogOS UI component can be summarized as:

```text
Component
│
├── Template
│   ├── primitive/component nodes
│   ├── [inputs]
│   ├── (outputs/events)
│   ├── [(writable bindings)]
│   ├── {{ interpolation }}
│   ├── @if / @for
│   ├── #local references
│   └── {utility styling}
│
└── Rust
    ├── private state
    ├── signals/computed values
    ├── #[input]
    ├── #[output]
    ├── #[expose]
    ├── #[ref]
    ├── #[inject]
    └── behavior
```

---

# 36. Web/Angular Analogy

| Existing concept | LogOS equivalent |
|---|---|
| HTML | `.ui` declarative template |
| DOM elements | namespaced native primitives |
| Angular components | typed LogOS components |
| Angular `[input]` | `[input]` |
| Angular `(output)` | `(event)` |
| Angular `[(value)]` | typed writable binding |
| Angular interpolation | `{{ expression }}` |
| Angular Signals | small native UI signals/computed values |
| Angular services | native/scoped injectable Rust services |
| Angular DI | hierarchical typed LogOS DI |
| `ViewChild`/template refs | `#ref` + typed `#[ref]` |
| public component methods | explicit `#[expose]` methods |
| TailwindCSS | compiled native style utilities |
| CSS engine | native style/layout representation |
| JS/TypeScript logic | Rust component logic |
| Browser runtime | LogOS Rust UI runtime |
| browser renderer | LogOS renderer/compositor |

---

# 37. Explicit Non-Goals

The initial system should not reproduce:

- a browser DOM;
- runtime HTML parsing;
- runtime CSS selector matching;
- unrestricted CSS cascading;
- arbitrary Angular-style directives;
- zones;
- a large lifecycle-hook API;
- generic string-based service lookup;
- implicit component-to-parent mutation;
- arbitrary sibling lookup;
- silently recognized primitive tag names;
- whole-screen rerendering as the normal state update model.

Pipes are intentionally deferred rather than permanently rejected.

---

# 38. Initial Implementation Scope

The lock screen should drive the first implementation.

A reasonable first milestone is:

```text
logos-ui
├── tree
│   ├── nodes
│   ├── component instances
│   └── references
│
├── reactive
│   ├── Signal<T>
│   └── Computed<T>
│
├── component
│   ├── inputs
│   ├── outputs
│   ├── exposed methods
│   └── scopes
│
├── di
│   ├── providers
│   ├── hierarchical scopes
│   └── singleton/scoped lifetime
│
├── layout
│   ├── row
│   ├── column
│   ├── stack
│   └── constraints
│
├── style
│   ├── native Style
│   ├── theme values
│   └── state variants
│
├── input
│   ├── event
│   ├── focus
│   └── hit testing
│
└── renderer bridge
```

And:

```text
logos-ui-compiler
├── template parser
├── component/name resolution
├── expression binding
├── type checking
├── style utility compiler
├── ref validation
└── typed UI IR generation
```

The first lock screen does not need every eventual framework feature.

It should establish the architecture while implementing only the capabilities actually required.

---

# 39. Decisions Still Open

The following details remain intentionally unresolved:

1. exact `.ui` grammar;
2. exact import/alias syntax between Rust and templates;
3. exact syntax for conditional style utilities;
4. exact primitive namespace naming;
5. exact signal API;
6. exact DI provider declaration syntax;
7. whether exposed public fields beyond inputs should exist at all;
8. exact typed UI IR format;
9. whether templates generate Rust code, static descriptors, or both;
10. exact semantics for async event handlers;
11. whether event bubbling/capture is ever needed;
12. exact animation model;
13. exact local `.style` syntax if introduced;
14. exact capability-aware service API;
15. whether pipes ever justify becoming language syntax.

These should be resolved through concrete lock-screen and early desktop-shell implementation rather than abstract framework expansion.

---

# 40. Core Principle

The central design principle is:

> Rust implements the UI platform and behavior, while a small declarative language makes describing interfaces fast and expressive.

The author should think in:

```text
components
+ typed data
+ events
+ writable state
+ services
+ utility styling
```

rather than:

```text
drawing calls
+ renderer internals
+ manual tree mutation
```

Most template-level abstractions should compile into a typed representation before runtime.

The result should preserve Angular-like productivity and Tailwind-like styling speed while remaining native, deterministic, strongly typed, and suitable for an operating system UI.
