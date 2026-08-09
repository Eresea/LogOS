# ADR-0027: Network scalable stream slice

- Status: Accepted - current scalable Network architecture baseline
- Date: 2026-08-09

## Context

The bootstrap Network endpoint already owns capability checks, generation-bound handles, transfer
page validation, replacement, and client plumbing. A second asynchronous ABI would duplicate those
invariants. The bootstrap TCP state also used one listener, one connection, and a global readiness
path that could not scale or preserve byte-stream semantics.

## Decision

Evolve `NetworkRequest`/`NetworkReply` with `SubmitWrite` and `PollStream`. Add an auxiliary,
generation-bound `StreamPage` for coalesced `Readable`, `Writable`, and `Closed` state plus bounded
completion records, sequence numbers, and overflow/loss indication. `SubmitWrite` accepts bytes
into connection-owned stream storage and reports cumulative accepted progress through the stream
record; acknowledgement progress is reported separately. Application writes have no TCP message
semantics, and TCP sequence numbers are assigned only when a wire range is armed.

Represent listeners and connections with separate bounded tables. The initial capacities are one
listener and eight connections. Each connection isolates ownership, generation, tuple, TCP phase,
RX/TX storage, retransmission, close/reset state, and progress watermarks. Four 1 KiB TX chunks are
storage units only.

NetworkRuntime drains service stream records, publishes them into the owning client's StreamPage,
and reports notifications. `PollStream` queries the owning connection and returns authoritative
readiness plus accepted/acknowledged watermarks. Readiness is level/coalesced and queues are bounded;
on overflow, clients poll each owned endpoint and clear the overflow flag. The scheduler owns task
execution and NetworkRuntime never runs Gateway, Remote, or client tasks as protocol work.

## Consequences

- Existing bootstrap TCP operations remain available during migration.
- Client/server ABI, capability plumbing, endpoint replacement, and ownership remain shared.
- Multiple connections cannot consume a single global eight-event ring.
- Accepted bytes are distinct from acknowledged bytes and are never treated as message completions.
- Multiple-listener policy and full congestion control remain later work.
