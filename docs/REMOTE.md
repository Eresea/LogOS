# Remote

> **Status:** Remote v1 planned

## Goal

Make remote operation the first complete LogOS user experience. SSH may be supported, but the native architecture uses the structured session and command contracts.

## V1 scope

- [ ] Authenticated machine enrollment and public/device-key authentication.
- [ ] Multiplexed, resumable sessions with explicit capability grants.
- [ ] Structured commands/results; file/object transfer; health, log, and trace streaming.
- [ ] Service control, updates, reset, and power-off.
- [ ] Version negotiation, compression, bounded flow control, and complete audit trails.
- [ ] One minimal client using the same contracts as later web, desktop, mobile, and SSH surfaces.

## Exit criteria

Without local display or keyboard, an authenticated user can inspect the machine, execute commands, reconnect, transfer data, follow diagnostics, control services, update, reset, and power off.

## Deferred clients

- Web, desktop, and mobile clients.
- SSH compatibility gateway.

See session placement in [Architecture](ARCHITECTURE.md#3-dependency-rules) and constraints in [Security](security.md).
