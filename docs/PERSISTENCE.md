# Persistence

> **Status:** Persistence v1 next
>
> **Owner:** Foundation block service and System storage service

## Goal

Prove that capability-scoped service state survives interrupted writes and resets without making POSIX the native storage model.

## V1 scope

### Block boundary

- [ ] Discover one dedicated raw VirtIO block device.
- [ ] Provide bounded asynchronous read, write, flush, timeout, cancellation, and reset.
- [ ] Report completion, integrity, and recovery diagnostics.

### Storage boundary

- [ ] Run one independently restartable System storage service.
- [ ] Expose service-owned namespaces containing named byte objects.
- [ ] Keep immutable object versions and atomically replace the current version.
- [ ] Recover checksummed records through a small crash-safe commit log.
- [ ] Require explicit namespace read and write capabilities.

### First consumer

- [ ] Persist terminal history through its existing bounded export/restore contract.

## Exit proof

For every commit interruption point, QEMU recovers either the complete old version or the complete new version, never partial data. State survives reset, cross-namespace access is denied, corruption is reported, and block/storage service failure is recoverable without reboot.

## Deferred

- General streams, partitions, discard, quotas, accounting, and file compatibility.
- Memory-backed namespaces and a general consistency-checking tool.
- Configuration manifests; the V1 primitive only keeps the prior object version readable.
- Identity, trust, secrets, audit retention, and durable secret encryption.
- Snapshots or retention policy beyond versions required for atomic replacement.
- Application workspaces and agent-memory policy.

The on-disk commit format and block/storage ownership boundary require an [ADR](adr/README.md) before implementation. See [Architecture](ARCHITECTURE.md#11-storage-model) and [Security](security.md).
