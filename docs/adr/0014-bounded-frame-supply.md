# ADR-0014: Bounded frame supply

## Status

Accepted

## Decision

User address-space construction consumes physical pages from a fixed frame
pool populated from copied conventional-memory descriptors. The pool tracks at
most `MAX_MANAGED_FRAMES` page addresses and has no allocator dependency,
coalescing policy, or unbounded metadata.

Allocation, double-release, unknown-frame, and exhaustion outcomes are
explicit. Memory outside the cap is ignored until a later milestone extends
the bound.

## Consequences

- Page exhaustion can be tested and contained before a process is admitted.
- Frame ownership is separate from process metadata and can be reclaimed with
  an address space.
- The current implementation does not yet build hardware page tables or copy
  ELF segments; those are the next loader slice.
