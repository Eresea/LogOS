# ADR-0038: Event-driven blocking IPC

- Status: Accepted
- Date: 2026-08-14

## Decision

The five trusted ring-3 services keep direct access to their existing fixed SPSC
IPC pages. Empty-to-nonempty and full-to-not-full transitions produce bounded
event notifications; data does not move through the kernel.

Core exposes fixed `Wait(mask, timeout)` and `Notify(mask)` syscalls. Event masks
cover the six receive edges, six send-space edges, and the keyboard input edge.
The scheduler stores one wait mask per task and latches pending events so a
producer cannot lose a wake between a queue check and wait registration.

Waiters always recheck their queue after returning. A bounded timeout is only a
heartbeat/watchdog fallback for idle services; event notification is the normal
progress path. The existing generation and disconnect checks remain authoritative
during supervisor replacement.

## Consequences

- Empty receivers and full producers stop consuming CPU while idle or under
  backpressure.
- Keyboard IRQ1 can wake Input directly through the scheduler's IRQ-safe event
  signal path.
- The fixed `u64` mask supports bounded wait-any behavior for Terminal without
  dynamic wait sets or an allocator.
- Trusted-peer shared pages remain intentionally outside the hostile-peer
  isolation boundary; generic service control syscalls and capabilities remain
  deferred.
