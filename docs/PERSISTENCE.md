# Persistence

> **Status:** Persistence v1 next
>
> **Owner:** Foundation block service and System storage service

## Goal

Provide crash-safe, capability-scoped persistence without making a POSIX filesystem the native system model.

## V1 scope

### Block boundary

- [ ] VirtIO block discovery and stable device identity.
- [ ] Bounded asynchronous read, write, flush, timeout, cancellation, and reset.
- [ ] Completion, integrity, and recovery diagnostics.

### Storage boundary

- [ ] Versioned contract for service-owned namespaces, named objects, byte streams, and immutable versions.
- [ ] Atomic replacement and crash-safe metadata commits.
- [ ] Checksums, quotas, accounting, consistency checking, and recovery mode.
- [ ] Memory-backed temporary namespaces using the same outward contract.

### First consumers

- [ ] Service manifests and configuration.
- [ ] Terminal history through its existing bounded export/restore contract.
- [ ] Identity, trust, secret metadata, and audit export.

## Exit criteria

- State survives reset and controlled interruption at every write commit point.
- A service cannot access another service's namespace without an explicit capability.
- Corruption is detected and reported; nonessential indexes can be rebuilt.
- Configuration can atomically replace and roll back a prior version.
- Block or storage service failure is contained and recoverable without reboot.
- Permanent QEMU proofs cover recovery, denial, corruption, and resource reclamation.

## Non-goals

- POSIX as the native storage model.
- Snapshots, revision-retention policy, discard, partitions, or a file compatibility API unless V1 consumers require them.
- Application workspaces and agent-memory policy before Applications v1.
- Durable secret encryption design without a concrete key-protection boundary.

## Deferred scope

Retained from the original roadmap for later versions or when a V1 consumer proves the need:

- discard and partition discovery;
- snapshots or revision history and retention policy;
- a conventional directory/file compatibility API;
- persistent agent-memory namespaces with workspace visibility, retention, and redaction policy;
- durable content checksums where metadata checksums do not suffice.

Storage placement and the compatibility file view are described in [Architecture](ARCHITECTURE.md#11-storage-model). Security invariants remain governed by [Security](security.md); irreversible on-disk or cross-ring decisions require an [ADR](adr/README.md).
