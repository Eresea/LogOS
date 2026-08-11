# ADR-0019: Extend Remote Foundation into a persistent administration attachment

- Status: Accepted
- Date: 2026-08-03

## Context

ADR-0018 deliberately limited Remote Foundation to one Noise IK connection and one typed `ping`
request. Remote v1 now needs sequential administration commands and bounded diagnostic events while
preserving Core ownership of trust, persistence, capabilities, and device effects.

## Decision

- Keep one enrolled X25519 administrator and one IPv4 TCP attachment on port 7443.
- Version the encrypted application protocol at v2 and use bounded `Invoke`, `Subscribe`, `Credit`,
  `Cancel`, `Reply`, `Event`, and `Error` messages.
- Gateway remains a replaceable Ring-3 transport task. It receives no long-term key, Store, or audit
  capability; Core exposes only typed remote-gate operations.
- Authorize only inspection, service control, reboot, and power-off commands. Enrollment, key
  disclosure, recovery, layout, and unknown commands remain local-only.
- Persist the replay slot and rolling remote audit ring in one protected remote-control object so
  pending/complete journal transitions and their audit phase are atomic.
- Diagnostic events are cursor-based, fixed-buffer, and credit-limited. Overwritten cursors report a
  gap instead of silently claiming a complete history.

## Consequences

- Foundation protocol-v1 clients are intentionally incompatible with Remote v1 protocol v2; the
  Foundation wire was never released.
- A failed protected remote-control load disables remote administration while local operation stays
  available.
- Multi-client identity, delegated profiles, DNS, TLS/PKI, SSH, file transfer, and unbounded history
  remain outside this milestone.
