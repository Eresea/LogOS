# Remote

> **Status:** Remote v1 planned

## Goal

Make remote operation the first complete LogOS user experience. SSH may be supported, but the native architecture uses the structured session and command contracts.

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
