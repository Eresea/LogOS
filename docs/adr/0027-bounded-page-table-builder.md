# ADR-0027: Bounded page-table builder

## Status

Accepted

## Decision

User image pages are mapped through a fixed four-level x86-64 builder. The
builder allocates the root and intermediate table frames from `FramePool`,
uses a `PageTableMemory` access seam for architecture-specific writes, can
seed a root from the active kernel mappings, applies user/W^X/NX flags,
rejects duplicate leaves and huge-page conflicts, and reclaims all table
frames together. The UEFI adapter also implements the loader's `PageSink`
using the same reserved identity-mapped frames.

## Consequences

- Page-table construction is host-testable without dereferencing physical
  addresses.
- The architecture adapter remains the only owner of unsafe table memory
  access and CR3 activation.
- Kernel mappings are explicitly seeded by the UEFI adapter; service-specific
  device mappings remain separate capability-controlled inputs.
