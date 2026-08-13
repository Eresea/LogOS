# ADR-0021: User launch contract

## Status

Accepted

## Decision

The process table emits `UserLaunch` metadata only for a running process with a
bound, page-aligned address-space root. The scheduler accepts that metadata
with a generation-safe process handle and publishes the user entry, stack top,
root, and process identity before making the task runnable.

The scheduler continues to execute its kernel trampoline until the hardware
ring-3 context path consumes this metadata. No loader or service code is
allowed to infer launch registers from independent globals.

## Consequences

- Loader, process, and scheduler responsibilities have one testable handoff.
- A missing root, invalid user address, or null process handle fails before a
  task becomes runnable.
- Reclaim clears the published user metadata with the task generation.
- Real page-table construction and ring-3 entry remain a later hardware slice.
