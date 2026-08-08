# Remote

> **Status:** Remote Foundation v1 behavior and ownership extraction remain in progress. Network
> readiness now belongs to NetworkRuntime and Gateway startup no longer probes through Terminal;
> the QEMU harness now uses structured Network readiness and host-side Remote authority; fixed-seed
> Network/Remote proof closure remains pending.

## Goal

Make remote operation the first complete LogOS user experience. SSH may be supported, but the native architecture uses the structured session and command contracts.

## Foundation v1

This is the capability-6 slice, not Remote v1 administration. It proves one locally enrolled
client can reconnect and invoke the existing typed `ping` command.

### Fixed decisions

- Use `Noise_IK_25519_ChaChaPoly_SHA256`, one X25519 client key, guest TCP port 7443, one
  connection, one request, and 1024-byte framed transport messages.
- `remote-key`, `enroll <64-hex-key>`, and `unenroll` are local-only commands. Successful
  enrollment returns `<machine-key-hex>:<generation>` for `logosctl invoke`; enrollment and
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

The following five IDs remain permanent catalog entries but are currently explicitly skipped as
unimplemented: `remote/enrollment-persistence`, `remote/reconnect-replay`,
`remote/pending-after-reset`, `remote/gateway-restart`, and `remote/protected-state-corrupt`.
They return only after their individual multi-boot/restart orchestration and semantic postconditions
are implemented.

### Implementation checkpoints

- [x] Bounded Noise IK, HKDF key separation, XChaCha protected-record primitive, fail-closed trust
  state, replay model, typed attach/invoke/reply codecs, enrollment/session record codecs, and
  partial/coalesced frame buffering.
- [x] Protocol-v2 message and canonical typed-invocation codecs, bounded request digests, and
  Core-owned remote-gate ABI operations (handshake, open, invoke, seal, subscription, credit,
  acknowledgement, and reset).
- [x] Native Gateway payload is present in the supervised manifest and remains independently
  optional while its Core relay is being completed.
- [x] Core carries each Network client owner out-of-band to the Network service; TCP listener,
  accepted stream, read, write, and close ownership is enforced there.
- [x] Protected Store enrollment, local trust commands, root-derived device/storage keys, and
  fail-closed corruption recovery.
- [ ] Gateway attachment and `logosctl` end-to-end invocation are wired through the typed Core
  remote gate; the harness runs the real bounded `logosctl` operation as the sole Remote proof
  authority and does not execute a second label-only Core scenario.
- [x] Host `logosctl keygen` and pinned Noise IK typed `invoke` client with bounded reconnects;
  `invoke` consumes the enrollment descriptor rather than a hard-coded generation.
- [x] `RemoteRuntime` coordinates `remote-key`, `enroll <64-hex-key>`, and `unenroll`; the machine
  key is available when the firmware root is present and enrollment persistence remains protected.
- [ ] QEMU typed-invocation proof after the Network/Gateway scheduling boundary is repaired.
- [ ] Reintroduce the five skipped proofs only with their documented clean-shutdown, reconnect,
  reset, restart, or protected-corruption orchestration.

The remote proof IDs are registered in `logos-test`. They remain explicit verification work rather
than environment-gated work. ABI v4 is not frozen until the full Network suite, all Remote proofs,
and the remaining ownership extraction pass together.

## Current ownership checkpoint

`RemoteRuntime` currently owns `RemoteState`, local trust commands, the enrollment gate, transport
start/reset state, protected control loading, and the Gateway start predicate. Both production and
test-driven Terminal input call `RemoteRuntime::local_command`; no second `remote-key`, `enroll`,
or `unenroll` implementation exists. External callers observe `state()` and narrow transport,
control, and request methods; mutable RemoteState is not exposed. The predicate consumes
`NetworkRuntime::configured()`, which is backed by Network's internal server `Status` transaction;
it does not depend on the Terminal client page. `platform::runtime` still owns Gateway endpoint
bindings, the large Remote request polling loop, deadline/reply lifecycle, protected persistence
context, and replacement composition. Extracting that polling loop remains deferred until this
consolidation is stable.
UEFI builds use target-scoped Fiat Curve25519 and software ChaCha20/Poly1305 backends so the full
payload set builds consistently on the supported host toolchains.

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
