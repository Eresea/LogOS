# ADR-0054: Flow interpreter service migration

## Status

Accepted

## Decision

Flow replaces the Commands service in the existing service slot. The raw service identity, endpoint
identities, capability slots, and wire values remain unchanged so the graph migration does not
require an ABI-generation reset. The image and ownership names are Flow; there is no ninth service.

Flow owns a fixed lexer, parser, source spans, type checker, evaluator, typed operation registry,
completion provider, eight persistent variable slots, and four promise slots. Source is capped at
256 bytes; response/body values are capped at `MAX_FILE_BYTES`; callback nesting is capped at four.
Variables and promises are volatile and reset when Session or Flow restarts.

`net.fetch(url, destination)` remains an atomic Fetch→Storage publication. `net.fetch(url)` is a
typed `Promise<Response>` and Fetch sends bounded `FetchBodyChunk` messages to Flow. Each chunk is
correlated by request ID and offset; Flow rejects malformed, stale, reordered, or oversized chunks.
Network ownership is not bypassed.

Assignments keep a promise active in its fixed slot. `await` promotes the foreground evaluation and
Ctrl-C forwards cancellation to the owning operation. `promise.cancel()` uses the same cancellation
path. A failed expression produces a bounded source-aware diagnostic and does not invalidate
unrelated promise slots. `try`/`catch`, global `fetch`, dynamic objects, loops, and imports remain
deferred.

The old standalone command parser and aliases are not part of the Flow contract. Canonical object
syntax (`fs.open(...).read()`, `fs.touch(...).write(...)`, `net.fetch(...)`, and typed service
members) is the supported terminal language. Completion candidates and argument scaffolds come
from the typed registry and retain request-ID/line-revision stale handling.

## Consequences

- Session remains an editor/history/prompt boundary and does not interpret system operations.
- Storage, Network, Supervisor, and Fetch retain state ownership behind bounded IPC.
- Promise response bodies add a bounded transport message without changing existing numeric IDs.
- Restarting Flow intentionally abandons volatile evaluation state; durable Storage state survives.
