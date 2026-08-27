# ADR-0070: Program-owned Atrium surface channels

- Status: Accepted
- Date: 2026-08-27

## Context

Program lifecycle admission previously provided only a private ELF, stack, and
Exit syscall. Atrium now owns application surfaces, so a launched program needs
a bounded way to request a surface, receive routed input, and submit rendering
without receiving Display or framebuffer authority.

## Decision

Core assigns each running program a generation-safe synthetic `ServiceHandle`
from the reserved program-client range. Core creates five private dynamic IPC
channels between that client and Atrium: surface request, surface response,
surface input, cell render, and GUI draw. Core maps only a read-only `ProgramBootstrapPage` and
one private staging page into the program address space; the page contains the
client identity and those five capabilities.

Atrium admits the surface only after validating the request client against the
capability peer and retains the client with the surface record. Input and
rendering are accepted only for a live surface owned by that client. Core
destroys the channels and private pages after scheduler completion; Atrium
retires any surface whose program client disappears.

The `logos-program` no-std library is the reference client for this contract;
it keeps request correlation and the admitted surface reference in bounded
state, and the demo program is its first client-shaped package source. Pointer
bytes remain private to Core and Input; Atrium receives semantic pointer events
through the existing GUI input contract and forwards surface-local coordinates
to the owning client.

## Consequences

- Programs can participate in Atrium without direct Display or framebuffer access.
- Program IPC remains bounded by the fixed program slots and per-channel queues.
- The current surface payload supports keyboard/text and pointer input, bounded
  cell renders, and bounded GUI draw batches; app discovery and richer program
  bootstrap APIs remain deferred.

## Proof obligations

- stale program generations cannot reuse a prior surface or capability;
- malformed, cross-client, and cross-surface requests are rejected;
- program stop, fault, and restart reclaim channels, queue frames, bootstrap, and staging;
- no program mapping includes the Display framebuffer or Display-owned control authority.
