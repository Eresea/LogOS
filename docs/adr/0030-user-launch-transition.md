# ADR-0030: User launch transition

## Status

Accepted

## Decision

The scheduler selects a task CR3 before restoring its saved context. Kernel
tasks use the boot CR3; a user task uses its bound process root. A bounded
kernel trampoline can enter a validated `UserLaunch` by constructing the
ring-3 `iretq` frame with the fixed user selectors and aligned stack top.

Service tasks are not scheduled by this commit. The existing proof task keeps
the only live ring-3 entry until the service startup barrier and service loop
protocol are integrated.

## Consequences

- Address-space selection is no longer conditional on the proof feature.
- The transition primitive is centralized and does not expose arbitrary CR3 or
  selector values to service code.
- Service process launch records remain inert until their dependencies and
  restart ownership are ready.
