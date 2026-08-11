# ADR-0017: Reload native services after contained faults

- Status: Accepted
- Date: 2026-08-02

## Context

Native services already execute in separate address spaces, but the bootstrap restart path only
resets the shared context and saved frame. It cannot safely reuse memory after a service fault, and
an exception from Ring 3 currently halts Core with every other exception.

## Decision

Core treats Terminal, Sessions, Store, and Network as restartable native services. A contained
Ring-3 fault or explicit panic stops only the active service. Core cancels its pending work,
returns page loans, invalidates generation-tagged endpoints, releases its address space, and starts
a fresh instance from the immutable payload staged at boot.

Terminal is required for normal mode and gets one immediate restart before Core enters recovery.
Sessions, Store, and Network use bounded 2/4/8 tick retries and then remain degraded; local
terminal operations and recovery stay available. Core and its supervisor remain fatal boundaries.
Core-owned drivers stay resettable, not independently fault-isolated, until DMA isolation exists.

Only faults and explicit panics are contained in this decision. Timer preemption and recovery from
an uncooperative infinite loop remain Core v2 work.

## Consequences

- Native task replacement must allocate and validate a new address space before releasing the old
  one.
- User exception stubs distinguish CPL 3 from Core faults and return structured fault metadata to
  the native-service scheduler.
- Service payload protocol compatibility is checked before each fresh start; incompatible payloads
  are rejected according to the service's availability policy.
- The recovery console remains the authority for restarting Terminal or Sessions after normal
  control is unavailable.

## Alternatives considered

- Reset only the shared service context -- rejected because code, stack, heap, and mappings may
  have been corrupted.
- Recover every CPU exception -- rejected because a Core invariant failure must not continue.
- Add timer preemption now -- deferred because this milestone does not require a starvation
  boundary.
