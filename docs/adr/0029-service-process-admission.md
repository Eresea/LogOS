# ADR-0029: Service process admission

## Status

Accepted

## Decision

Each boot-loaded service is admitted to `ProcessTable` after its frames and
root are complete. Manifest capabilities are translated to the typed process
capability set, the root is bound, adjacent owned pages are coalesced into
bounded `VirtualMapping` runs, and `UserLaunch` is retained with the process
generation. A failure exits and reclaims the partially admitted process.

The scheduler does not run these launch records yet. The current context
bootstrap still enters kernel task functions, so arbitrary service RIP entry
is a separate ring-3 context slice.

## Consequences

- Service process identity, permissions, mappings, and launch metadata are
  real and generation-safe before scheduling.
- Mapping count stays bounded by coalescing contiguous pages with equal flags.
- No kernel trampoline is allowed to masquerade as a running service.
