# ADR-0066: Dynamic IPC topology and runtime handles

- Status: Accepted
- Date: 2026-08-20

## Decision

ABI v5 replaces fixed IPC capability slots, the compile-time endpoint enum, and
the fixed service-count contract with generation-safe runtime handles. Services,
endpoints, capabilities, events, and event sets use distinct opaque handles
encoding an index and generation.

Core owns endpoint queues and capability records. Queue frames remain private to
Core, while message types and payload sizes remain bounded typed contracts. A
read-only service bootstrap page provides the service identity and bootstrap
control grants; a paginated directory operation discovers only capabilities
already granted by Core.

Dynamic service registration and endpoint creation are policy-controlled Core
operations. Services do not receive arbitrary capability delegation authority.
The ABI migration is versioned and intentionally does not support the previous
fixed ABI in the live service image set.

## Consequences

- Runtime topology can grow and shrink within physical-memory and owner quotas.
- Restart invalidates service, endpoint, capability, and event generations.
- Event waiting must move from endpoint-derived `u64` masks to runtime event sets.
- Fixed wire-size and queue backpressure validation remain part of the hostile-peer proof.
