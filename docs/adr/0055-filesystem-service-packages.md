# ADR-0055: Filesystem service packages

## Status

Accepted

## Decision

Storage format v3 adds a package arena between the bounded journal and checkpoint regions. The arena
is derived from the device size and is independent of ordinary namespace records. The catalog holds
at most eight generation-safe records keyed by `ServiceId`; each record has at most eight variable
length extents. An install writes only the required blocks, flushes package data, validates the
complete envelope, then commits one catalog record. Aborted or replaced extents are reusable, and
v1/v2 media remains readable without package operations or automatic migration.

`logos-package` defines the fixed envelope. It contains the package magic, format and ABI versions,
service identity, `Service` kind, payload length, package version, and CRC32C. Header validation is
bounded and reader-based; the only accepted payload is one service ELF.

Core and Storage use dedicated fixed `Lookup` and chunked `Read` messages. There is one outstanding
request and one reused 4 KiB transfer frame. Request IDs, package generations, offsets, lengths,
IPC generations, and service epochs are checked at both ends so stale or forged messages cannot
publish package bytes.

Activation validates the envelope and ELF headers before allocating. The loader streams pages through
one fixed scratch page into exact service-owned code/data/BSS/stack/page-table frames. Preparation is
isolated from the running graph; failures reclaim all prepared frames. Success quiesces the graph,
rebuilds its IPC generation and service epoch through the existing restart path, consumes the prepared
image only for the selected service, and reclaims old frames after task quiescence.

## Consequences

- Package size no longer inherits the 8 KiB ordinary-file limit or reserves a fixed 512 KiB object.
- The first activation is intentionally graph-wide; targeted hot replacement can be optimized later.
- Package installation is an internal Storage API for fixtures and future package-manager work.
- Package-manager UI, dependency resolution, signatures, boot preference, program packages, and
  program installation remain deferred.
