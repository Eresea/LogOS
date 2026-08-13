# ADR-0032: Service IPC pages

## Status

Accepted

## Decision

The terminal graph owns six fixed shared endpoint pages: Input→Terminal,
Terminal→Display, Terminal→Session, Session→Terminal, Session→Commands, and
Commands→Session.
Each page receives one bounded frame, generation `1`, and a stable user VA.
Only the producer and consumer roots receive that page, and each process gets
one matching data mapping. Service loops reread the page identity at bounded
iteration boundaries so a future supervisor can invalidate old work without
rebuilding the protocol.

The page allocation is complete before the startup barrier reaches
`LaunchReady`; the live service loops consume these pages after the scheduler
starts them.

## Consequences

- Shared data-plane ownership is explicit and capability-scoped by endpoint
  membership.
- A restarted graph receives a new generation and service epoch; stale messages
  are rejected before they can reach a replacement service.
- IPC frames are included in the same fixed frame-pool exhaustion boundary as
  image and page-table frames.
