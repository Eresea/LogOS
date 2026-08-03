# ADR-0018: Keep Remote Foundation behind bounded TCP, trust, and session gates

- Status: Accepted
- Date: 2026-08-03

## Context

Remote Foundation crosses Network v2, Platform v2, Persistence v2, and Sessions v2. It needs
mutual authentication, encrypted transport, durable enrollment, and reconnect without granting a
network client ambient command authority.

## Decision

- `system.network` owns passive TCP state; Core retains NIC DMA, queues, interrupts, reset, and
  page ownership.
- `system.secrets` derives device and protected-store keys from the existing UEFI root, owns Noise
  IK handshake and record protection, and exposes only typed operations.
- `system.store` persists encrypted `enrollment` and `remote-session` records in a protected
  namespace; an authentication failure disables remote access and never selects an older record.
- `session.remote` owns one bounded TCP attachment and forwards only authenticated typed session
  requests. Sessions owns sequence/replay policy and journals pending work before execution.
- The v1 profile is `Noise_IK_25519_ChaChaPoly_SHA256`, one enrolled X25519 client key, guest TCP
  port 7443, one connection, one request, and `ping` as the sole remotely authorized command.

## Consequences

- Remote access is unavailable when the UEFI root key or firmware entropy is unavailable.
- Reconnect after a completed command replays its persisted reply; a recovered pending command is
  `Indeterminate` and is never re-executed.
- TCP connect, DNS, TLS/PKI, multiple clients, streams/jobs, remote administration, and rollback
  protection remain later work.
- The implementation adds the Remote Foundation proof suite and updates Architecture, Security,
  boot ordering, Naming, and the roadmap.

## Alternatives considered

- TLS 1.3: rejected for this slice because the available server providers do not support the UEFI
  target without a separate provider port.
- Signed plaintext commands: rejected because authentication alone does not protect command
  confidentiality or integrity after login.
