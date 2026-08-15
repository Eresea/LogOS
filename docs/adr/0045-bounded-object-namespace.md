# ADR-0045: Bounded durable object namespace

- Status: Accepted
- Date: 2026-08-15

## Decision

The storage service exposes one default volume containing generation-safe
opaque object IDs. The initial namespace has a fixed object table, directory
entries represented by parent/object/name tuples, 255-byte components, and a
32-component path-resolution limit. Core stores no path, directory, or file
metadata.

Create, rename, unlink, metadata updates, and file writes are journal
transactions. File contents are bounded to four 4 KiB logical blocks and are
stored as journal records for this proof milestone. Recovery rebuilds the
object table by replaying only complete committed transactions. Symlinks,
hard links, snapshots, encryption, compression, replication, repair, and a
page cache are deferred.

## Consequences

- Object IDs remain safe across deletion and slot reuse through generations.
- Namespace mutations are atomic at the journal transaction boundary.
- The first file API is bounded and service-owned; no path syscall is added.
- A future extent allocator and page cache can replace journal-resident file
  data behind the same object and transaction contracts.
