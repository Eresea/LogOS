# Update

> **Status:** Update v1 planned

## Goal

Evolve kernel, services, drivers, and applications independently without making failed updates fatal.

## V1 scope

- [ ] Signed packages/manifests, reproducible metadata, dependency and compatibility resolution, and signing-identity revocation.
- [ ] Staging, atomic activation, health-gated commit, automatic rollback, and recovery boot profile.
- [ ] Persistent update journal and rollback-safe configuration migration.
- [ ] Remote inspect, apply, cancel, and rollback operations.
- [ ] Separate policy for kernel, trusted services, drivers, and WASM applications.

## Exit criteria

- Power loss during staging or activation leaves a bootable system.
- Failed health checks roll back deterministically.
- Independently updated kernel and services negotiate compatibility.
- Diagnostics explain what changed and why rollback occurred.
- QEMU interrupts every phase and proves recovery.

See the update state machine in [Architecture](ARCHITECTURE.md#13-update-model).
