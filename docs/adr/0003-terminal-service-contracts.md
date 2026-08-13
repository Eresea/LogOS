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
- QEMU proves service image loading, ring-3 entry, terminal render, and semantic input; host tests
  prove the full Input → Terminal → Session → Display path and restart behavior.

## Consequences

Terminal policy is independently testable and has no direct hardware or scheduler dependency.
Display and input can be replaced without changing terminal semantics. `TerminalStack` remains a
deterministic host reference; the live service graph is loaded and scheduled through the bounded
ELF/process path.

## Deferred

Complex shell features, dynamic fonts, and additional hardware backends remain deferred. The live
supervisor now owns heartbeat-driven replacement and safe page-table teardown; the host reference
model remains a test oracle rather than the runtime service implementation.
