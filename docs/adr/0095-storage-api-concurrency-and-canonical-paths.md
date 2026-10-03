# ADR-0095: Storage API Concurrency and Canonical Paths

- Status: Accepted
- Date: 2026-10-03

## Context

ADR-0047 assumed a single Storage client, but Flow and Fetch now share the API, and direct operations
(`Mkdir`, handle writes, staged writes) persist outside the active transaction. A transaction
publishes whole record copies, so a commit after such a change silently dropped it; when both
claimed the same free slot, the volume no longer reopened. Path handling also accepted `.`/`..`
names and trailing slashes inconsistently, which made scenario replays depend on the client.

## Decision

- A namespace transaction records the volume generation at `Begin`. `Commit` returns `Stale` and
  publishes nothing if any other change persisted in between; clients restart the transaction.
- `Mkdir` and `Stat` honour a non-zero transaction ID (staged state); `Fsync` rejects one as
  `Invalid`. Transaction ID zero keeps its committed-state meaning.
- `StageWriteBegin` returns `Busy` while a staged write is open, matching `Begin`; the owner must
  commit or abort it.
- Storage paths are canonical: absolute, no empty components, and no trailing slash except `/`.
  New names may not be `.` or `..`; existing records keep the older rule so current volumes open.
  The shell trims trailing slashes before sending paths.
- Snapshot bounds failures (`TooLarge`, `Capacity`) that publish nothing report their cause rather
  than `Recovery`.
- Extent-backed handle writes copy only touched blocks and fall back to a compacting rewrite when
  the layout would exceed `MAX_FILE_EXTENTS`.

## Consequences

Concurrent clients get optimistic concurrency instead of lost updates. A client holding a
transaction across another client's write sees `Stale` and must retry. A crashed stage owner leaves
staging `Busy` until Storage restarts, as an abandoned transaction already does.
