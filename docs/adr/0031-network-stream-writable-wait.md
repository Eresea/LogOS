# ADR-0031: Network-owned writable stream wait

- Status: Accepted
- Date: 2026-08-10

## Context

Gateway has a Network client page but deliberately has no Network event endpoint. Waiting for TCP
send capacity through `network_wait` therefore cannot receive an event and can strand the Gateway
task. Repeated `PollStream` requests would avoid that invalid wait but wastes bounded scheduler
work while the stream remains full.

## Decision

Append `AwaitWritable` to the typed Network operation enum. Network records at most one deferred
writable request, waits on its owned event endpoint, and replies with the authoritative stream
readiness and watermarks when the stream becomes writable, closes, or reaches its deadline.
Gateway uses that request after a non-writable poll or busy write submission. It never calls
`network_wait`.

## Consequences

- TCP backpressure is event-driven without a Gateway busy loop or an invalid cross-service wait.
- The single active Network client transaction remains bounded and preserves the existing ABI page.
- Per-connection request queues and independent concurrent waiters remain future scalable work.
