# ADR-0002: Gate deterministic test control outside production

- Status: Accepted
- Date: 2026-07-27

## Context

QEMU proofs currently match human debug text and cannot drive deterministic failures or completion.

## Decision

Test builds expose the bounded `LOGOS/1` protocol over COM2 and terminate through QEMU debug-exit. Semantic fault points and virtual time are available only with `test-hooks`; production images retain debugcon diagnostics but no test control surface.

## Consequences

- Stable event IDs and semantic `RUN` proofs, not diagnostic prose or scenario labels, are the proof contract.
- The host harness owns timeouts, QMP, reports, and artifacts.
- One QEMU fixture may serve several scenarios after a synchronous `RESET`; a failed reset invalidates that fixture.
- Unsupported milestone scenarios are skipped rather than reported as passing.

## Alternatives considered

- Human log matching was rejected as a fragile ABI.
- Virtio-serial was deferred because a polling 16550 channel meets the current bounded need.
