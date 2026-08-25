# LogOS Typed Forms

## Goal

Forms should be defined in the type-safe Rust layer and bound into UI templates.

The template should primarily describe presentation. Form state, validation, control types, and submission state should live in typed Rust structures.

---

## Typed Form Model

Example:

```rust
#[derive(FormModel)]
struct LoginForm {
    #[required]
    username: String,

    #[required]
    password: String,
}
```

A form instance exposes typed controls:

```rust
login_form.controls.username // Control<String>
login_form.controls.password // Control<String>
```

Each `Control<T>` is reactive and owns its form-related state.

Conceptually:

```rust
struct Control<T> {
    value: Signal<T>,
    disabled: Signal<bool>,
    readonly: Signal<bool>,

    valid: Readable<bool>,
    dirty: Signal<bool>,
    touched: Signal<bool>,
    focused: Signal<bool>,

    errors: Readable<ValidationErrors>,
}
```

---

## Binding a Form

The template binds the typed form to a `ui.form`:

```html
<ui.form
    #loginForm
    [form]="loginForm"
    (submit)="unlock"
>
    ...
</ui.form>
```

`ui.form` becomes the visual and interaction scope for that form.

---

## Binding Controls

Controls are explicitly passed to compatible UI components:

```html
<ui.input
    [control]="loginForm.controls.username"
/>

<ui.input
    [control]="loginForm.controls.password"
/>
```

The control supplies the component with its:

- value;
- required state;
- disabled state;
- readonly state;
- validation rules;
- validity;
- touched/dirty state;
- validation errors.

The template therefore does not need to duplicate declarations such as:

```html
[required]="true"
[(value)]="username"
```

when those concerns already belong to the form model.

---

## Type Safety

Controls carry their value type:

```rust
Control<String>
Control<bool>
Control<u32>
Control<PathBuf>
```

A UI component declares which control types it accepts.

For example:

```html
<ui.input [control]="loginForm.controls.username" />
```

is valid for `Control<String>`.

A checkbox may instead require:

```html
<ui.checkbox [control]="settingsForm.controls.rememberMe" />
```

where `rememberMe` is `Control<bool>`.

Incompatible bindings should fail during compilation.

---

## Validation

Validation belongs primarily in the typed form definition.

Example:

```rust
#[derive(FormModel)]
struct LoginForm {
    #[required]
    #[min_length(2)]
    username: String,

    #[required]
    #[min_length(8)]
    password: String,
}
```

Custom validators should also be possible.

Single-control validation belongs to the control.

Cross-control validation belongs to the form.

```text
Control validators
    -> field-level constraints

Form validators
    -> cross-field / whole-model constraints
```

---

## Form State

The form aggregates its controls and exposes reactive state:

```rust
login_form.valid
login_form.dirty
login_form.touched
login_form.submitting
login_form.errors
```

This can be consumed directly by the template:

```html
<ui.button
    [disabled]="!loginForm.valid || loginForm.submitting"
    (click)="loginForm.submit()"
>
    Unlock
</ui.button>
```

---

## Submission

Submission is owned by the form.

Anything that wants to submit calls:

```rust
login_form.submit();
```

or from the template:

```html
(click)="loginForm.submit()"
```

`ui.form` also establishes submission semantics for its controls.

For example, pressing Enter inside an eligible single-line input should submit the nearest bound form automatically.

A normal button does not need HTML-style `type="submit"` semantics.

Conceptually:

```text
form.submit()
    -> validate controls
    -> mark relevant invalid controls touched
    -> stop and expose errors if invalid
    -> emit submit if valid
```

Async submission may allow the form to track:

```rust
login_form.submitting
```

automatically while its submit handler is running.

---

## Example

```html
<ui.form
    #loginForm
    [form]="loginForm"
    (submit)="unlock"
    {flex-y gap-y-4}
>
    <ui.input
        [control]="loginForm.controls.username"
        {w-96 font-light focus:bg-accent}
    />

    <ui.input
        [control]="loginForm.controls.password"
        {w-96 font-light focus:bg-accent}
    />

    <ui.button
        [disabled]="!loginForm.valid || loginForm.submitting"
        (click)="loginForm.submit()"
        {mt-4 px-6 py-3 rounded-lg bg-accent}
    >
        Unlock
    </ui.button>
</ui.form>
```

---

## Core Principle

The form schema is owned by typed Rust.

The template decides how the form is presented.

```text
Rust Form<T>
    -> typed controls
    -> validation
    -> state
    -> submission

Template
    -> [form]
    -> [control]
    -> layout
    -> styling
    -> interaction
```

This avoids duplicating form rules in the UI markup while keeping the template concise, strongly typed, and reactive.
