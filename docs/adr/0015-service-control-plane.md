# ADR-0015: Service control plane

## Status

Superseded

## Decision

The generic service syscall request/response ABI is deferred and removed from
the active milestone. The implemented ring-3 boundary contains only the proof
workload's Yield and Heartbeat paths; service lifecycle, process, endpoint, and
resource access are established by kernel-side admission and fixed mappings.

Terminal streams continue to use shared bounded IPC rings. A future control
plane may add scalar operations, but it must not become a second message
transport.

## Consequences

- The old request/response shapes remain historical evidence only.
- A future syscall ABI must be reintroduced with a concrete dispatcher and
  capability proof, rather than an authorization-only facade.
