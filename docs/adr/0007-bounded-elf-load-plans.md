# ADR-0007: Bounded ELF load plans

- Status: Accepted
- Date: 2026-08-12

## Decision

Validated ELF64 images produce a fixed plan containing at most 16 load segments and 512 KiB of
total memory. Segment file ranges, virtual ranges, W^X flags, and executable entry containment are
validated before admission. The plan exposes metadata and borrowed file bytes without allocation.

## Scope

This slice does not allocate frames, copy segments, bind hardware page tables, or switch CR3. Those
operations consume the plan in later bounded commits.
