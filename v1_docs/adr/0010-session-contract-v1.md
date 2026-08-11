# ADR-0010: Gate terminal syscalls with `foundation.session` v1

- Status: Accepted
- Date: 2026-07-29

## Context

The terminal submits typed syscalls through a Core-owned deferred gate. Input and Display now have
explicit session capabilities; syscalls still reach command dispatch without a Session gate.

## Decision

Grant the terminal session an explicit Session capability. Core checks it before parsing or
dispatching every syscall and replies with bounded `permission denied` when it is absent.

## Consequences

- Command-specific capabilities remain enforced by the command dispatcher.
- Core retains bootstrap command dispatch and privileged effects for this slice.
- No general session RPC or remote-session protocol is introduced.
