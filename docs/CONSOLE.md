# Console

> **Status:** Terminal model complete; native terminal handoff bootstrapped, service fan-out pending Platform v1
> **Owner:** UEFI bootstrap (normal terminal); Core (recovery console); Sessions after extraction

Console is LogOS's local textual interface. The normal terminal consumes typed command results; the kernel recovery console is an independent fallback.

## Implemented bootstrap

The normal terminal is currently linked into the UEFI image. It is not yet an independently loaded Sessions service; its direct framebuffer and PS/2 access must move behind capability-scoped input and display contracts. The kernel recovery console remains independent and kernel-owned.

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
- [x] `health`, `ping` (Pong round trip), `tasks`, `services`, `drivers`, `trace`, `inspect`, `restart`, `cancel`, `clear`, `layout`, `reboot`, `poweroff`, `help`, and `commands`.

### Exit evidence

- [x] Normal operation does not require the recovery console.
- [x] Recovery remains available on startup/redraw failure and authorized live handoff.
- [x] Headless QEMU verifies startup self-checks and normal/recovery mode selection.
- [ ] QEMU proves a separately loaded terminal service can start, redraw, fail, and hand off to recovery.

## Next required boundary

- [x] Load `logos-terminal` as a native Sessions service rather than link it into `logos-uefi`.
- [ ] Replace raw framebuffer and PS/2 access with capability-only input and display client contracts.
- [ ] Keep command/session dispatch outside Core and preserve kernel-only recovery input/output.

## Later — Unplanned

Record future console scope here before adding it to the roadmap. Do not infer V2 work from V1 implementation details.

### Demonstrated

- [x] Normal terminal startup, typed command sessions, UTF-8 editing, layouts, font rendering, history, scrollback, selection, search, and redraw recovery.
- [x] Discoverable typed commands with capability checks, cancellation/timeout handling, bounded output, pipelines, and human/table/tree/JSON formatting.
- [x] Local operational commands for health, tasks, services, drivers, trace, inspection, recovery, restart, cancellation, reset, and power-off.
- [x] Live transition to the recovery console when normal startup or redraw fails, or when authorized by the `recovery` command.
- [x] QEMU startup proofs for the normal-mode health gate and recovery fallback.
