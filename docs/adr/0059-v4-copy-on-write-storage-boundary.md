# ADR-0059: v4 copy-on-write storage boundary

Status: Accepted

## Decision

The replacement on-disk format is v4. It has dual checksummed superblocks, immutable 4 KiB data
pages, a persistent allocation bitmap, a bounded retired-extent record, and a two-slot commit
record. A mutation publishes a new metadata extent only after page writes, allocation metadata,
the commit record, and the alternate root have each been flushed.

Recovery selects the newest valid root and may complete a valid flushed commit record. Torn or
unknown formats fail closed; v1/v2/v3 media is reported as `UnsupportedVersion` and is never
reformatted.

The Storage service persists its namespace/catalog snapshot through this store. Snapshot pages are
streamed through one 4 KiB staging page and package payload extents are excluded from the metadata
allocation prefix. The existing path API remains compatible, and `Fsync` is an explicit idempotent
block flush. The versioned service extension now provides bounded generation-safe file handles,
compact `Stat`, handle reads/writes, `Mkdir`, and close-time stale-handle rejection. Core validates
private read-only map requests and page-table unmap invalidates translations. Storage exposes a
32-page physical cache arena, and Core grants bounded read-only windows to authorized Flow/Fetch
processes and clears them on restart. The Filesystem API now pins block-aligned ranges within one
extent, returns generation-safe map handles, and still requires its caller to submit the returned
cache-slot descriptor through the private Core mapping channel.

## Bounds and follow-up

The current host proof keeps one active writer, bounded paths, bounded handles, up to
`MAX_FILE_EXTENTS` COW extents per streamed file, and bounded package extents. Storage now owns a
fixed 32-page physical cache arena, with Core mapping individual cache frames read-only into
bounded Flow/Fetch windows. Mapping ranges larger than one bounded window remain follow-up work;
they must not mutate a v4 root in place.
