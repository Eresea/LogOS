# ADR-0047: Bounded Commands-to-Storage API

- Status: Accepted
- Date: 2026-08-15

## Context

The storage service can format, journal, recover, and expose a bounded namespace, but terminal file
commands need a direct kernel-mediated service boundary. The boundary must preserve fixed capacities,
private staging pages, and Storage ownership of paths and durability without adding a syscall or
allocator.

## Decision

- Add `CommandsToStorage` and `StorageToCommands` while preserving the existing low-level Storage
  endpoint IDs. Commands and Storage each use exactly four capability slots.
- Encode versioned `Begin`, `Commit`, `Abort`, `List`, `CreateFile`, `Read`, `Write`, `Remove`, and
  `Rename` messages in the existing `IpcBytes` transport. Lengths, versions, operations, flags,
  transaction IDs, paths, and payloads are validated at the API boundary.
- Storage owns one active transaction. It keeps changes in a fixed shadow namespace, accepts at most
  `MAX_RECORDS_PER_TRANSACTION` journal records, publishes only after one durable journal transaction
  and flush, and drops staged state on abort or service restart.
- Transactional reads use staged state; transaction ID zero reads committed state. The Commands shell
  keeps transaction controls private and implements mutating commands as Begin → operation → Commit,
  aborting when the operation fails. `write` replaces file contents atomically within that transaction.
- Replies remain bounded and may be chunked through the existing IPC flags; terminal rendering remains
  capped at 512 bytes.

## Consequences

Storage remains the only owner of namespace state, journaling, recovery, and durability. Commands is a
single bounded client, so there is no multi-client transaction arbitration, directory creation,
permissions model, networking, or generalized service topology. Committed files survive reboot;
uncommitted changes do not.

The host suite covers endpoint authorization, protocol rejection, transaction semantics, command
parsing, and namespace recovery. The fresh-disk QEMU proof covers command commit, reopen, aborted and
removed files, flush, and torn-journal recovery.
