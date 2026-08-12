# ADR-0003: Bounded terminal service contracts

- Status: Accepted
- Date: 2026-08-12

## Context

The terminal must not own keyboard hardware or pixels, and a terminal failure must not require a
kernel restart. The current Core has fixed tasks and no allocator, so the first useful milestone
needs deterministic contracts and proof workloads before architecture-specific service loading.

## Decision

- Input emits semantic key events and committed text; terminal code never receives PS/2 bytes.
- Terminal emits fixed cell diffs; display code never receives terminal parser state.
- Session owns line editing, command policy, pipelines, history, environment, and volatile files.
- Service messages use fixed typed shapes with explicit maximums and generation/epoch identity.
- Queue saturation, malformed pages, stale identities, and service restart are explicit outcomes.
- The service graph is modeled as Input, Display, Terminal, Session, and Commands with a fixed
  supervisor lifecycle and no automatic replay of old operations.
- QEMU proves terminal render, semantic input, and terminal rebind before AP startup; host tests
  prove the full Input → Terminal → Session → Display path and restart behavior.

## Consequences

Terminal policy is independently testable and has no direct hardware or scheduler dependency.
Display and input can be replaced without changing terminal semantics. The current `TerminalStack`
is a deterministic contract proof; ADR-0004 separately proves the Core's first fixed ring-3 task,
but the terminal service is not yet that task.

## Deferred

General address-space/page-table ownership, capability mapping, syscall dispatch, fixed ELF image
packaging, real hardware adapters, and supervisor-driven process replacement remain deferred. Until
then, no code may describe the host-tested process table or the fixed proof image as the terminal's
actual process isolation model.
