# Update

> **Status:** Update v1 planned

## Goal

Replace a signed system bundle without making failed updates fatal.

## V1 scope

- [ ] Signed boot/system bundle containing the kernel and trusted native services.
- [ ] Staging, A/B-style atomic activation, health-gated commit, automatic rollback, and recovery profile.
- [ ] Persistent update journal.
- [ ] Remote inspect, apply, cancel, and rollback operations.

## Exit criteria

- Power loss during staging or activation leaves a bootable system.
- Failed health checks roll back deterministically.
- Diagnostics explain what changed and why rollback occurred.
- QEMU interrupts every phase and proves recovery.

## V2 — Independent components

- Independent service and driver packages, dependency resolution, and configuration migrations.
- Signing-key revocation, application package integration, delta downloads, and storage cleanup.

## V3 — Continuous evolution

- Live service handoff, snapshot-integrated rollback, and compatibility windows.
- Staged deployment policy and multi-machine rollout.

Application updates are not part of V1.

See the update state machine in [Architecture](architecture.md#13-update-model).
