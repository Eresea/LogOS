# ADR-0068: Runtime-owned lazy service stacks

- Status: Accepted
- Date: 2026-08-26

## Context

Service stack sizes were selected by individual service identity. That made a
Shell-specific increase look like a resource policy and committed memory for
call depth that might never be used. A service must still have a bounded stack,
but the runtime should own the backing frames and respond to demand.

## Decision

Every service starts with the common `USER_STACK_PAGES` window. Core retains a
per-service stack ledger and a single global `MAX_SERVICE_STACK_PAGES` ceiling.
When a running service faults below its committed stack but within the reserved
growth range, Core allocates the bounded missing span of owner-scoped frames,
clears and maps them into that service's address space, and retries the
interrupted instruction. A fault outside that range remains a contained user
fault.

The growth path is architecture-aware but service-agnostic. It uses the
existing page-table builder and frame-pool ownership, invalidates the mapping
through the normal page-table seam, and releases borrowed frames during service
restart or graph teardown. No service receives a frame allocator, page-table
authority, or a per-service stack budget.

## Consequences

- Shell, Flow, Storage, and other services no longer need separate stack page
  constants.
- Unused stack capacity consumes no physical frames beyond the common initial
  window.
- Stack growth is bounded and charged to the service's generation-scoped owner;
  large frame faults are admitted as a bounded contiguous virtual span.
- The runtime must keep the page-fault retry path correct; exhaustion or an
  invalid fault is contained as a service fault rather than becoming a kernel
  allocation policy.
