# ADR-0028: Async-first subsystem state and scheduling boundary

- Status: Accepted
- Date: 2026-08-09

## Context

LogOS coordinates hardware, timers, Network, persistence, services, Remote operations, and user
interaction across bounded native tasks. A long-lived operation must remain correct when its caller is
dormant, replaced, or resumed later. Nested `wake`/`run` choreography makes that lifetime depend on
an execution stack instead of the subsystem that owns the operation.

The existing Network scalable stream slice demonstrates the desired direction: connection state,
accepted and acknowledged byte watermarks, readiness, completion records, and generations are owned
by NetworkRuntime and its bounded endpoint state.

## Decision

LogOS adopts the following system-wide rule:

> Long-lived or externally-driven work is represented as bounded, explicitly owned state. State
> transitions advance from commands, device completions, events, and timers; subsystem code reports
> readiness or completion, while Core/platform composition owns task scheduling.

The rule has these requirements:

1. Waiting is state: packet RX, ACK, NIC completion, Block I/O, Store completion, timers, service
   requests, Remote peers, user input, restart, and rebind are represented by bounded owner state.
2. Subsystems do not directly schedule another subsystem's task as normal protocol progress. They
   publish a bounded runnable/completion notification; scheduler/composition code decides execution.
3. Accepted work and terminal completion are separate where the contract permits. A Network
   `SubmitWrite` accepts bytes into connection-owned storage; later TX/ACK transitions publish
   acknowledged progress and readiness.
4. Readiness is reconstructable from authoritative state. Notifications may be coalesced or dropped
   when the client can resynchronize, as with authoritative `PollStream`.
5. Deadlines and cancellation are explicit transitions, and generation/identity checks reject stale
   completion, cancellation, replacement, and rebind state.
6. Blocking APIs may remain as convenience wrappers over these primitives. Durable Store completion
   still means the existing durability invariant is true; acceptance is not reported as commit.
7. Async structures remain bounded, capability-scoped, allocation-free where required, explicitly
   owned, and deterministic under exhaustion (`Busy`/`Full` rather than hidden allocation).
8. No generic async runtime, futures/promises, executor, universal command/completion ABI, or global
   event bus is introduced by this decision.

The generic device shape is:

```text
Submitted -> InFlight -> Complete | TimedOut | Reset
```

The current Network shape is:

```text
SubmitWrite -> accepted_bytes -> buffered connection state
             -> later TX/ACK -> acknowledged_bytes/readiness
```

Remote operations should eventually own a bounded record containing identity, generation, request
ID, phase, deadline, persisted state, pending Session/Storage identity, and terminal result. Waiting
for Storage or Sessions is Remote state, not a suspended Gateway -> Core -> service execution chain.

For future APIs, observable state should expose a monotonic generation or sequence alongside the
state itself so a client can say it observed version `N` and resynchronize when the version changes.
This is guidance, not a new universal Signals API.

## Consequences

- NetworkRuntime and `PollStream` remain the reference implementation for readiness and bounded wake
  reporting.
- Top-level platform composition may drain bounded wake notifications and run tasks; protocol owners
  must not force dependent task execution as part of ordinary progress.
- Existing bootstrap wrappers may remain when their synchronous public contract or durability
  boundary requires them, but they are recorded as transitional and must not spread.
- Remote, Sessions relay, and Storage relay conversions are separate bounded milestones; this ADR
  does not redesign their ABI or cryptography.
- Tests should assert state transitions, stale-generation rejection, timeout, cancellation, and
  resynchronization rather than rely on arbitrary delays.

## Alternatives considered

- Rust `async`/`await` and a kernel executor - rejected because LogOS needs explicit ownership,
  bounded storage, and small typed ABI pages, not a generic continuation runtime.
- A universal event bus or Future/Promise ABI - rejected because it hides ownership and introduces
  unbounded routing pressure.
- A wholesale scheduler rewrite - rejected because the current scheduler already owns task
  execution; only concrete protocol coupling should move.
