# Remote

> **Status:** Remote Foundation v1 integration in progress

## Goal

Make remote operation the first complete LogOS user experience. SSH may be supported, but the native architecture uses the structured session and command contracts.

## Foundation v1

This is the capability-6 slice, not Remote v1 administration. It proves one locally enrolled
client can reconnect and invoke the existing typed `ping` command.

### Fixed decisions

- Use `Noise_IK_25519_ChaChaPoly_SHA256`, one X25519 client key, guest TCP port 7443, one
  connection, one request, and 1024-byte framed transport messages.
- `remote-key`, `enroll <64-hex-key>`, and `unenroll` are local-only commands. Enrollment and
  replay state are encrypted in the protected Store namespace.
- A completed request is replayed after reconnect or reset. A journalled-but-incomplete request is
  `Indeterminate` and is not executed again.
- Only `ping` receives remote authority. Missing entropy, root key, Store, Network, Sessions, or
  Gateway leaves local operation available and remote unavailable.

### Exit proofs

- `network/tcp-stream`
- `remote/enrollment-persistence`
- `remote/auth-denied`
- `remote/typed-invoke`
- `remote/reconnect-replay`
- `remote/pending-after-reset`
- `remote/gateway-restart`
- `remote/protected-state-corrupt`

### Implementation checkpoints

- [x] Bounded Noise IK, HKDF key separation, XChaCha protected-record primitive, fail-closed trust
  state, replay model, typed attach/invoke/reply codecs, enrollment/session record codecs, and
  partial/coalesced frame buffering.
- [ ] Passive TCP owner multiplexing for Terminal and Gateway clients.
- [ ] Protected Store enrollment, local trust commands, Gateway attachment, and `logosctl`.
- [ ] QEMU restart, corruption, and typed-invocation proofs.

## V1 scope

- [ ] Authenticated machine enrollment and public/device-key authentication.
- [ ] One minimal client using existing typed Session contracts.
- [ ] Structured commands/results plus health, log, and trace streaming.
- [ ] Service control, reset, and power-off.
- [ ] Bounded reconnect, explicit session capabilities, version negotiation, flow control, and audit.

## Exit criteria

Without local display or keyboard, an authenticated user can inspect the machine, execute commands, reconnect, follow diagnostics, control services, reset, and power off.

## V2 — Complete remote environment

- Multiplexed persistent sessions and robust resume across service and connection restart.
- Large object/package transfer and Update inspect/apply/cancel/rollback.
- Delegated and expiring capability grants; web, desktop, and mobile clients.

## V3 — Fleet and compatibility

- Multi-machine administration, relays, discovery, and coordinated rollout policy.
- Remote graphical streaming and an SSH gateway.

Update execution and general large-file transfer are not part of V1.

See session placement in [Architecture](architecture.md#3-dependency-rules) and constraints in [Security](security.md).
