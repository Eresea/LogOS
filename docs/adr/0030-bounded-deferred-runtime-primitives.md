# ADR-0030: Bounded deferred runtime primitives

- Status: Accepted
- Date: 2026-08-10

## Decision

The deferred endpoint evolution is now represented by bounded, allocation-free primitives:

- `logos_abi::endpoint_v5` defines a generation-bound endpoint table without changing ABI v4.
- `logos_core::manifest` validates a bounded binary service manifest.
- `logos_core::resource` provides owner- and generation-scoped resource leases.
- `logos_core::event` provides a bounded FIFO with explicit overflow accounting.
- `logos_core::poll_runtime` provides a cooperative bounded polling runtime.

These primitives are independent of the active ABI-v4 boot path. They do not introduce an implicit
allocator, wake dependent tasks, or replace existing typed pages. Production adoption requires a
separate compatibility milestone and proof for each consumer.

## Consequences

- ABI-v5 experimentation no longer requires modifying the ABI-v4 structures.
- Manifest and resource bounds are explicit and deterministic under exhaustion.
- Event loss is observable instead of silently allocating or blocking.
- The polling runtime is opt-in; Core's existing scheduler remains authoritative.
