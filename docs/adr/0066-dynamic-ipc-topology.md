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
already granted by Core. Each capability directory record carries a stable typed
contract ID, exact payload metadata, and the event handle for its direction.

Dynamic service registration and endpoint creation are policy-controlled Core
operations. Services do not receive arbitrary capability delegation authority.
The ABI migration is versioned and intentionally does not support the previous
fixed ABI in the live service image set.

## Consequences

- Runtime topology can grow and shrink within physical-memory and owner quotas.
- Restart invalidates service, endpoint, capability, and event generations.
- Event waiting must move from endpoint-derived `u64` masks to runtime event sets.
- The staged event syscall supports explicit wait cancellation; cancellation clears
  the published waiter and wakes the scheduler object without consuming an event.
- Hardware producers bind pre-existing event handles and signal their pending state
  through an allocation-free source router; no endpoint-derived event mask is used
  for the network IRQ path.
- Contract IDs describe typed message agreements and never identify runtime
  endpoint slots; endpoint and event generations remain independently stale-safe.
- Fixed wire-size and queue backpressure validation remain part of the hostile-peer proof.

## Implementation status

The Core registries, v5 directory records, generation checks, and event-set syscall operations are
live. Built-in service traffic resolves capabilities through the paginated directory and runtime
endpoint handles; route-specific device, storage, package, and network adapters remain Core-owned
policy behind those records. Service lifecycle requests validate dynamic handles before using the
internal built-in image router, while program lifecycle remains a separately bounded contract.
