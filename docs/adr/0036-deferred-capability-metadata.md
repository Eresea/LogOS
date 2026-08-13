# ADR-0036: Deferred capability metadata

- Status: Accepted
- Date: 2026-08-13

## Decision

Remove process kinds and capability grants from live process and service-image
metadata. The current kernel has no syscall or hostile-peer boundary that
enforces these labels, so retaining them creates security-shaped bookkeeping
without security effect. Authorization returns with the enforcing boundary.

## Consequences

Process admission remains bounded by image validation, address-space ownership,
mapping rules, and generation-safe handles. Shared IPC remains a trusted-peer
data plane; endpoint identity and generation checks do not provide sandboxing.
