# ADR-0026: ELF page population boundary

## Status

Accepted

## Decision

`loader::LoadedImage` populates owned frames through a fixed `PageSink` trait.
The loader validates every segment's file range before touching the sink,
clears all segment and stack pages, and copies only page-local file chunks.
The sink implementation owns architecture-specific physical memory access and
maps sink failures to the bounded loader error set.

## Consequences

- BSS and user-stack contents are deterministic before a process can run.
- Host tests verify segment offsets, unaligned segments, zero fill, and frame
  ownership without unsafe physical dereferences.
- Page-table construction and scheduler binding remain architecture-owned adapters; no
  architecture-specific memory access enters the loader model.
