# ADR-0027: Bounded page-table builder

## Status

Accepted

## Decision

User image pages are mapped through a fixed four-level x86-64 builder. The
builder allocates the root and intermediate table frames from `FramePool`,
uses a `PageTableMemory` access seam for architecture-specific writes, applies
user/W^X/NX flags, rejects duplicate leaves and huge-page conflicts, and
reclaims all table frames together.

## Consequences

- Page-table construction is host-testable without dereferencing physical
  addresses.
- The architecture adapter remains the only owner of unsafe table memory
  access and CR3 activation.
- Kernel mappings and service-specific device mappings remain explicit inputs
  to the later UEFI adapter; the builder does not silently inherit them.
