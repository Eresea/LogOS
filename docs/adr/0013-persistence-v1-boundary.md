# ADR-0013: Persistence v1 block and store boundary

- Status: Accepted
- Date: 2026-07-30

## Context

Persistence v1 needs crash-safe service state without making files or POSIX paths the native
storage model. Block DMA remains Core-owned until isolated Ring-1 drivers can safely own it, while
storage policy belongs in a restartable Ring-2 service. Native services also need bounded bulk
buffers that do not expose physical memory.

## Decision

Core exposes versioned Block operations and generation-tagged, owner-checked shared pages. Pages
are quota-bound, non-executable, temporarily lendable, and reclaimed with their owning service.
The Ring-2 Storage service exposes capability-scoped namespaces containing named byte objects.

The disk contains two checksummed superblocks and two equal append-only arenas. An object replace
writes its header and payload, flushes, writes a checksummed commit sector, then flushes again.
Recovery accepts only complete committed records. Compaction copies each object's current and
previous version to the inactive arena before an alternating superblock selects it.

## Consequences

- A successful Store commit is durable through the Block flush boundary.
- Store reads expose only the current and immediately previous immutable versions.
- Interrupted commits and compaction select either the old or new complete state.
- Files, directories, streams, arbitrary retention, and cross-service transactions remain out of
  scope.
- `logos-store` owns portable format and recovery logic; internal journal details remain in that
  crate.

## Alternatives considered

- One append-only arena without compaction — rejected because device capacity would eventually be
  unusable.
- In-place metadata updates — rejected because torn writes could expose partial state.
- A filesystem — rejected because it would make a compatibility view the native contract.
