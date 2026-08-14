# ADR-0040: Hostile-peer IPC boundary

- Status: Accepted
- Date: 2026-08-14

## Decision

The five fixed services communicate through six kernel-owned bounded SPSC queues:
Input has capacity 32, Render has capacity 1, and the four stream edges have
capacity 8. Queue frames are never mapped into service address spaces.

Each service receives exactly one writable staging page and one read-only capability
page. `IpcSend` and `IpcReceive` accept only a capability slot and fixed message
metadata; the kernel copies through the private staging page and validates process
ownership, direction, generation, service epoch, exact message size, queue state,
and connection state.

Capabilities are fixed, process-bound grants with at most four slots per service.
Supervisor restart quiesces services, disconnects and reclaims queues, reclaims
staging and capability pages, and rebuilds the graph with a new identity.

## Consequences

- A service cannot read or corrupt another service's queue contents or queue state.
- Backpressure and empty-queue transitions still use the existing bounded event masks.
- The fixed graph and no-allocator resource model remain unchanged.
- Generic endpoint creation, capability transfer, persistence, networking, and
  dynamic grants remain outside this boundary.
