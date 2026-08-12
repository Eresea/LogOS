# ADR-0031: Service startup barrier

## Status

Accepted

## Decision

Service startup is represented by a fixed five-entry state machine. Each
service advances through image, address-space, process, and launch-ready
states exactly once. A later started state requires dependencies in graph
order: Input and Display first, then Terminal, Session, and Commands.

The current boot reaches launch-ready for all five services but does not mark
them started. This prevents a partially wired IPC graph from appearing
healthy to the supervisor.

## Consequences

- Startup ordering and incomplete prerequisites are explicit, bounded errors.
- Process admission can be tested independently from service execution.
- The next slice can add IPC-page/device mapping and service heartbeats
  without changing the boot ordering contract.
