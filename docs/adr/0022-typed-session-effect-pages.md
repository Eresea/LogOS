# ADR-0022: Use separate typed Session and Effect pages

## Status

Accepted

## Context

ABI v4 left Session requests, replies, and privileged effects in the generic
`ControlPage` payload. That mixed lifecycle notification with service protocol
data and made Terminal-to-Sessions mediation ambiguous during replacement.

## Decision

Terminal receives a `SessionClientPage`; Sessions receives independent
`SessionServerPage` and `EffectPage` mappings. Core copies validated bounded
values between them, checks capability authority, and is the sole effect
executor. Each page carries scalar state, request ID, service generation, and
endpoint generation; stale pages and replies are rejected.

`SessionsRuntime` is the concrete Core owner for this relay, the current
request association, unavailable state, and Terminal/Sessions rebinding.
It does not own allocation, scheduling, supervision, or privileged effects.

## Consequences

- `ControlPage` is lifecycle and gate metadata only for Session transport.
- Terminal and Sessions never share a writable Session protocol page.
- A replaced Terminal or Sessions task invalidates its own endpoint without
  granting the replacement access to an old request or effect result.
- Store and Block now use the persistence-specific page migration in ADR-0023.
  Network and Remote remain unchanged.
