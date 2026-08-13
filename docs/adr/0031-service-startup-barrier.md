# ADR-0031: Service startup barrier

## Status

Accepted

## Decision

Service startup is represented by a fixed five-entry state machine. Each
service advances through image, address-space, process, and launch-ready
states exactly once. A later started state requires dependencies in graph
order: Input and Display first, then Terminal, Session, and Commands.

The current boot reaches launch-ready for all five services and starts them
through the normal scheduler path only after the graph is complete. This keeps
a partially wired IPC graph from appearing healthy to the supervisor.

## Consequences

- Startup ordering and incomplete prerequisites are explicit, bounded errors.
- Process admission can be tested independently from service execution.
- IPC-page/device mapping is part of the live launch contract. Service
  heartbeats and supervisor-driven restart can be added without changing the
  boot ordering contract.
