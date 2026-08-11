# ADR-0011: Run normal command dispatch in a Sessions service

- Status: Accepted
- Date: 2026-07-29

## Context

`foundation.session` v1 authorizes typed terminal syscalls, but Core still maps every command to
its reply and effect. That makes normal command/session dispatch part of the bootstrap gate rather
than replaceable Sessions functionality.

## Decision

Add a separately loaded Ring-3 Sessions service. The terminal submits bounded, versioned Session
requests to it; the Sessions service owns command dispatch and response formatting. Core only
brokers the bounded request, verifies the caller's Session capability, and exposes individually
capability-gated primitives for privileged effects.

The recovery console stays kernel-owned and does not use the Sessions service. This introduces no
general RPC framework: the initial transport carries only the terminal's existing bounded Session
request and reply values.

## Consequences

- The native loader and scheduler must support a second service before command dispatch moves.
- Core must not retain a normal-command registry once the Sessions service is active.
- QEMU must prove Sessions-service failure and restart leave recovery operational.
