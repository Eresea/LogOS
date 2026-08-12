# ADR-0020: Service entry façades

## Status

Accepted

## Decision

Terminal and Session expose entry-ready façades that process one ABI message at
a time and retain all state inside their service package. Commands is a
separate bounded package for initial built-ins and command output.

These façades do not own endpoint validation, scheduling, or capabilities. The
future ring-3 entry loops will call them after the kernel has validated the
shared-ring message identity.

## Consequences

- Service loops have deterministic work units and no hidden queue ownership.
- Existing model tests remain valid while the real IPC graph is introduced.
- Session-to-Commands routing is carried by the fixed request and response
  rings; Session retains only line editing and prompt state while Commands owns
  live built-in execution.
