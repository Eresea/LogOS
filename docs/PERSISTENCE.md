# Persistence

> **Status:** Persistence v1 complete; v2 deferred
> **Owner:** Ring-2 Store service, with Core-owned Block access

Persistence provides bounded capability-scoped named byte objects. It is not a filesystem and does
not make disk size proportional to RAM usage.

## Fixed v1 contract

- One dedicated raw VirtIO block device; no partitions or filesystem.
- One independently restartable Store payload and one Core-mediated Block boundary.
- One object payload is at most `PAGE_SIZE`; at most 32 live object names.
- Store memory is fixed and independent of disk size; production code is allocation-free after startup.
- Objects have current and previous immutable versions. Replace is atomic or absent.
- `Busy`, `Full`, `Invalid`, `NotFound`, `Corrupt`, `TimedOut`, and `Io` remain distinct outcomes.
- Terminal may access only its scoped namespace; read-only, stale, revoked, or cross-namespace writes
  are denied before Store wake-up or disk access.

Changing these decisions requires updating [Architecture](architecture.md) and, for an irreversible or
cross-ring change, an [ADR](adr/README.md).

## Ownership and wire boundary

Core owns VirtIO negotiation, DMA, queues, interrupts, timeout/reset, page loans, capability checks,
and address mappings. Store owns object names, versions, commit/recovery, compaction, and namespace
policy. The Block and Store pages carry bounded scalar requests/replies with request IDs, generations,
owners, lengths, and page handles; untrusted enum and field combinations are rejected before state
mutation.

The active crates are:

- `logos-abi`: bounded Block and Store wire contracts;
- `logos-store`: `no_std`, host-testable format, versions, commit, recovery, and compaction;
- `logos-storage-service`: restartable Store payload and Block-backed sector adapter;
- existing Core driver code: VirtIO block ownership until a separate driver boundary is justified.

## Recovery invariants

The format uses two superblocks and two arenas with checksummed records and a commit marker. Recovery
selects the highest valid superblock and ignores an incomplete tail. A torn record, bad checksum,
impossible length, or unsupported format is `Corrupt`; a non-blank corrupt disk is never reformatted.
Compaction writes the inactive arena, flushes it, writes and flushes the selecting superblock, then
publishes the new generation. Until the final step, the old arena remains recoverable.

Every replace and compaction interruption must expose exactly the complete old or complete new value,
never a prefix, mixed value, or zero-filled record. Reset, timeout, cancellation, and Store restart
must reclaim in-flight pages and preserve completed commits.

## Current proof

Persistence v1 is complete when the QEMU suite proves the raw-disk path, interruption recovery,
corruption handling, capability denial, timeout/reset progress, Store restart, and terminal-history
survival. Host tests cover codecs, bounds, the object state machine, and format recovery. Current proof
IDs and logs are in [testing status](../testing/STATUS.md); historical phase evidence is in
`docs/reviewed/` and git history.

## Deferred

Multi-page or streamed objects, more than 32 names, concurrent requests, quotas/accounting, snapshots,
repair tooling, general filesystems, application workspaces, and durable secret/audit policy remain
future slices. Remote protected state currently uses bounded Store records and fail-closed behavior;
it does not expand the v1 contract.

See [ADR-0013](adr/0013-persistence-v1-boundary.md), [Boot](boot-sequence.md), and [Security](security.md).
