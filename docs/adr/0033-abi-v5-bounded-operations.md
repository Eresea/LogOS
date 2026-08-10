# ADR-0033: ABI v5 bounded operation contract

- Status: Accepted
- Date: 2026-08-10

## Context

ABI v4 pages already carried generation and request identity, but Storage, Remote, Network,
and Gateway still had composition paths that synchronously drove dependent services. That made
completion ownership implicit and allowed stale work to survive a restart boundary.

## Decision

- Native service headers and typed service specifications use ABI v5 / protocol v2 atomically.
- `OperationToken` carries owner, service generation, request ID, deadline, and monotonic sequence.
- `OperationPhase` is authoritative; terminal phases are Complete, Failed, TimedOut, and Cancelled.
- `CompletionEnvelope` carries the token, phase, and subsystem status; notifications only schedule work.
- Every suspended operation remains in one fixed owner-held slot with explicit resource cleanup.
- v4 headers are rejected during payload validation; no compatibility adapter or second executor exists.

## Consequences

Storage, Remote, Network, and Gateway migrations must preserve fixed bounds and no-std behavior while
moving completion polling into Core-owned composition. Each changed boundary requires host transition
tests and the corresponding QEMU proof before ABI v5 is considered frozen.
