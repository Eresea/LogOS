# ADR-0032: Service IPC pages

## Status

Accepted

## Decision

The terminal graph owns five fixed shared endpoint pages: Input→Terminal,
Terminal→Display, Terminal→Session, Session→Terminal, and Session→Commands.
Each page receives one bounded frame, generation `1`, and a stable user VA.
Only the producer and consumer roots receive that page, and each process gets
one matching data mapping.

The page allocation is complete before the startup barrier reaches
`LaunchReady`; rings and service loops will consume these pages in the next
slice.

## Consequences

- Shared data-plane ownership is explicit and capability-scoped by endpoint
  membership.
- A restarted service can invalidate the generation without reusing stale
  messages.
- IPC frames are included in the same fixed frame-pool exhaustion boundary as
  image and page-table frames.
