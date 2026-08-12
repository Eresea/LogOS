# ADR-0005: Process address-space ownership

- Status: Accepted
- Date: 2026-08-12

## Decision

Every admitted process reserves exactly one generation-safe address-space identity. The process owns
that identity until exit or fault reclamation. A page-aligned root may be bound once; stale process
or address-space handles cannot affect a replacement slot.

## Scope

This slice records ownership only. It does not switch CR3, allocate page tables, load ELF segments,
map capabilities, or make the existing fixed ring-3 proof image a general process.
