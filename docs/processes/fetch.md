# Durable Fetch

`net.fetch(url, destination)` is a single-operation ring-3 workflow:

```text
Terminal → Session → Commands → Fetch → Network
                                      ↘ Storage
Storage/Network → Fetch → Commands → Session → Terminal
```

Fetch parses numeric-IPv4 `http://` URLs, builds one bounded HTTP/1.1 request, and incrementally
parses split Network responses. Only successful 2xx responses with Content-Length or strict
chunked framing are accepted. The body is held in a fixed Storage-sized buffer.

Storage receives staged begin/chunk/commit/abort requests from Fetch. Staged bytes are invisible to
normal reads; commit uses one internal durable transaction, so a failed or restarted Fetch cannot
publish a partial destination. Storage restart clears the volatile staging slot.

Commands assigns the request ID and forwards best-effort progress. Session accepts only Ctrl-C while
the command is active, forwards cancellation with that ID, and restores the prompt after reliable
completion, failure, or cancellation.
