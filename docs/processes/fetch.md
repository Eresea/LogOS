# Durable Fetch

Fetch is a fixed service reached through Flow:

```text
Terminal → Session → Flow → Fetch → Network
                           ↘ Storage
Storage/Network → Fetch → Flow → Session → Terminal
```

`net.fetch(url, destination)` owns one staged Begin → chunk → Commit publication. A failed or
cancelled operation aborts staging, so a partial destination is never visible. `net.fetch(url)`
uses the same Fetch ownership but returns a typed `Response`; Fetch emits correlated bounded body
chunks (`request_id`, `offset`, `len`) before its terminal progress message. Flow rejects stale,
reordered, malformed, or oversized chunks.

Flow assigns request IDs, retains at most four promise slots, and forwards cancellation to Fetch.
The numeric-IPv4 HTTP limitations and `MAX_FILE_BYTES` response bound remain unchanged.
