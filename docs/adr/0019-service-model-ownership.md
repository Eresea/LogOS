# ADR-0019: Service model ownership

## Status

Accepted

## Decision

The Input, Terminal, and Session state machines are compiled as independent
`no_std` packages that depend only on `logos-abi`. The kernel keeps temporary
compatibility re-exports so existing host proofs and the current boot proof do
not change while service entrypoints and ELF packaging are implemented.

The packages own policy and state; the kernel owns only mechanisms and does
not become a dependency of these models.

## Consequences

- Service behavior can be tested and built without the UEFI target.
- Moving the models into ring-3 entrypoints no longer requires importing
  kernel modules.
- Display and command-service extraction remain separate because they still
  need their own framebuffer and process-control boundaries.
