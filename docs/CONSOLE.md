# Console

> **Status:** Console v1 complete  
> **Owner:** Sessions (normal terminal); Core (recovery console)

Console is LogOS's local textual interface. The normal terminal consumes typed command results; the kernel recovery console is an independent fallback.

## V1 — Complete

### Foundation

- [x] Input service: physical/logical keys, modifiers, press/release, repeat, QWERTY and AZERTY.
- [x] Display service and font-backed text rendering.
- [x] Kernel-owned recovery input/output independent of normal services.

### Terminal

- [x] UTF-8 editing, cursor/caret, insert/delete, navigation, wrapping, and resize-aware redraw.
- [x] Bounded output, scrollback, command history, selection, clipboard-ready bytes, and search.
- [x] Rendering is separate from the editor/output model and redraws after display-service replacement.

### Sessions and commands

- [x] Session identity and explicit capability context.
- [x] Discoverable descriptors, typed arguments/results/errors, cancellation, timeout, backpressure, variables, and structured pipelines.
- [x] Human, table, tree, and JSON formatting.
- [x] Persistable history contract; Persistence v1 owns durable storage.
- [x] `health`, `tasks`, `services`, `drivers`, `trace`, `inspect`, `restart`, `cancel`, `clear`, `layout`, `reboot`, `poweroff`, `help`, and `commands`.

### Exit evidence

- [x] Normal operation does not require the recovery console.
- [x] Recovery remains available on startup/redraw failure and authorized live handoff.
- [x] Headless QEMU verifies editing, history, commands, cancellation, and input/display recovery.

## V2 — Unplanned

Record future console scope here before adding it to the roadmap. Do not infer V2 work from V1 implementation details.
