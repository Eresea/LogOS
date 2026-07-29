# Console

> **Status:** Console v1 complete
> **Owner:** Sessions (normal terminal and command replies); Core (privileged effects and recovery console)

Console is LogOS's local textual interface. The normal terminal consumes bounded command results; the kernel recovery console is an independent fallback.

## Current implementation

`logos-terminal-service.efi` and `logos-sessions-service.efi` are staged from the boot payload and run in separate Core-owned Ring-3 address spaces. The terminal has no raw framebuffer, PS/2, kernel-memory, or device mapping. Sessions formats typed Core effect results and forwards bounded replies; `clear` remains terminal-local. The kernel recovery console remains independent and kernel-owned.

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
- [x] Bounded human reply formatting owned by Sessions.
- [x] Persistable history contract; Persistence v1 owns durable storage.
- [x] `health`, `ping` (Pong round trip), `tasks`, `services`, `drivers`, `trace`, `inspect`, `restart`, `cancel`, `clear`, `layout`, `reboot`, `poweroff`, `help`, and `commands`.

### Exit evidence

- [x] Normal operation does not require the recovery console.
- [x] Recovery remains available on startup/redraw failure and authorized live handoff.
- [x] Headless QEMU boots the separately loaded Ring-3 terminal, redraws it, and executes its command path.
- [x] Recovery remains the direct fallback on normal-startup failure and authorized handoff.
- [x] QEMU proves Terminal and Sessions service failure, restart, capability denial, and direct recovery availability without compromising Core.

## Next required boundary

- [x] Load `logos-terminal-service` as a native Sessions service rather than link it into `logos-uefi`.
- [x] Gate the bootstrap context's input, display, and typed syscall operations with explicit Input, Display, and Session capabilities.
- [x] Keep command/session dispatch outside Core and preserve kernel-only recovery input/output.

## Later — Unplanned

Record future console scope here before adding it to the roadmap. Do not infer V2 work from V1 implementation details.

### Demonstrated

- [x] Normal terminal startup, typed command sessions, UTF-8 editing, layouts, font rendering, history, scrollback, selection, search, and redraw recovery.
- [x] Discoverable typed commands with capability checks, cancellation/timeout handling, bounded output, pipelines, and Sessions-owned replies.
- [x] Local operational commands for health, tasks, services, drivers, trace, inspection, recovery, restart, cancellation, reset, and power-off.
- [x] Live transition to the recovery console when normal startup or redraw fails, or when authorized by the `recovery` command.
- [x] QEMU startup proofs for the normal-mode health gate and recovery fallback.
