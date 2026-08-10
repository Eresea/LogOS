# Flow

> **Status:** Optional, deferred language and automation charter

The [full Flow specification](optional/FLOW.md) remains available for design work; this page is the
project-level summary only.

Flow is a future typed language and automation surface, not a current kernel or ABI contract. Do not
use this document to justify new runtime, parser, compiler, package, or AI infrastructure. Current
implementation work follows [the roadmap](roadmap.md), [TODO](TODO.md), and the relevant ADR.

## Charter

- Compose typed operations and service contracts rather than shell text.
- Make capabilities, resource bounds, cancellation, and side effects visible.
- Keep automation deterministic where the host and target contracts permit it.
- Treat remote and AI actions as ordinary audited, capability-scoped clients.
- Preserve a small Core; Flow must not become a privileged execution path.
- Prefer a WASM-compatible execution target and bounded host interfaces.

## Explicit non-goals for the current milestone

No Flow parser, compiler, bytecode VM, package registry, language server, general async runtime, or
AI-specific authority is required for ABI v4 stabilization. The Phase 0 charter remains deferred until
real stable system/application operations exist.

## Future gates

When scheduled, define grammar, type/effect rules, capability analysis, cancellation, persistence,
diagnostics, compatibility, and proof strategy as separate small decisions. Update the roadmap and
create an ADR before adding a runtime boundary.
