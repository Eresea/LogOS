# ADR-0037: Bounded memory subsystem contracts

## Status

Accepted

## Decision

The memory boundary is layered behind fixed interfaces. UEFI descriptors are
normalized into sorted, disjoint physical runs with explicit exclusions.
Physical allocation uses compact `FrameId` values, hierarchical free-word
metadata, generation-stamped leases, batch operations, and reservation
accounting. The SMP facade owns disjoint shard locks, per-CPU magazines, and
remote-free queues; its async API uses preallocated wait nodes and never holds
a memory lock across wakeup.

Virtual memory, TLB coordination, kernel slab/page allocation, pressure, and
observability expose bounded host-testable contracts. The first boot path keeps
the existing `FramePool`/`FrameAddress` names and remains the architecture
owner of physical page-table access. A general `GlobalAlloc`, 2 MiB mappings,
and unbounded reclamation policy remain deferred until their proofs exist.

## Consequences

- Physical lookup and allocation do not scan a flat frame-address list.
- Stale leases and wrong owners are rejected across central, cached, and
  remote-free paths.
- Async and SMP callers have stable seams without making every allocation
  awaitable or introducing a runtime dependency.
- The fixed metadata cost is intentional and remains bounded by the existing
  frame/run/CPU limits.
