# Persistence

> **Status:** Persistence v1 complete
>
> **Owner:** Foundation Block service and System Store service

## Goal

Prove that capability-scoped service state survives interrupted writes and resets without making
POSIX the native storage model.

## Delivery rules

This checklist is the implementation order. Start with the first unchecked phase item and do not
skip ahead.

- Read the whole current phase before editing code.
- Read every caller of code changed in the phase.
- Fix an in-scope prerequisite when found, run its smallest proof, and continue the phase.
- Record an unrelated, non-blocking problem under **Deferred** and continue the phase.
- Stop only for missing external access, missing hardware/tooling, or a new irreversible or
  cross-ring decision not settled by [ADR-0013](adr/0013-persistence-v1-boundary.md).
- Treat a failed build, test, boot, or recovery check as work to fix, not a reason to stop.
- Check an item only after its named proof passes.
- End every phase with `cargo fmt --check`, `cargo clippy -- -D warnings`, and the phase's smallest
  boot or test proof.
- Commit each independently bootable phase separately.

## Fixed v1 decisions

- Core owns VirtIO block DMA, interrupts, timeouts, reset, and physical-address translation.
- Store owns the disk format, object policy, recovery, and compaction.
- The native API is capability-scoped namespaces containing named byte objects.
- Replacing an object creates an immutable version.
- Reads expose only the current and immediately previous version.
- A successful replace means the final Block flush completed.
- A blank disk may be formatted; a non-blank corrupt disk must never be formatted automatically.
- One Store request per client and one Block request globally may be in flight in v1.
- Object names are at most `MAX_OBJECT_NAME` bytes and valid UTF-8.
- One v1 object payload is at most `PAGE_SIZE`; return `Invalid` for a larger object.
- Keep at most 32 live object names; return `Full` when that fixed index is exhausted.
- Keep production Store memory independent of disk size; never mirror the 16 MiB disk in RAM.
- Use the existing raw `target/logos-store.raw` device; do not add partitions or a filesystem.

Changing a fixed decision requires updating the architecture and, when irreversible or
cross-ring, adding or superseding an ADR before code changes.

## Completed foundation

- [x] Accept ADR-0013 for the Block/Store boundary and two-arena format.
- [x] Discover one dedicated raw VirtIO block device.
- [x] Bind the device before starting Store.
- [x] Provide bounded Block read, write, flush, timeout, cancellation, and reset operations.
- [x] Report Block completion and recovery diagnostics.
- [x] Define `BlockInfo`, `BlockRequest`, `StoreRequest`, namespace IDs, page handles, versions,
      operations, and persistence statuses in `logos-abi`.
- [x] Carry typed Store and Block requests through the native-service context.
- [x] Provide quota-bound, generation-tagged, owner-checked shared-page handles.
- [x] Start and restart a separate Ring-2 Store payload.
- [x] Implement and host-test the two-superblock, two-arena Store prototype.
- [x] Test old-or-new recovery at every prototype replace and compaction interruption point.
- [x] Keep current and previous object versions through prototype compaction.
- [x] Provide bounded terminal history export and restore bytes.

The prototype still mirrors the disk in an allocated `Vec`; it proves format behavior but cannot
be used by the 16 KiB bump-allocated Store payload. Phase 2 replaces that prototype-only runtime
shape before QEMU Store I/O.

## Ordered implementation checklist

### Phase 1: finish the wire contract

- [x] Add `BlockOperation::Info` so Store can discover sector count without a hard-coded disk size.
- [x] Add a bounded `BlockReply` containing request ID, status, and `BlockInfo` for `Info`.
- [x] Add a bounded `StoreReply` containing request ID, status, object version, and returned length.
- [x] Add checked `from_wire` conversion for every persistence enum.
- [x] Decode context bytes into integer wire fields before converting enums.
- [x] Never `read_unaligned` untrusted bytes directly into a struct containing Rust enums.
- [x] Reject unknown enum discriminants before constructing a request.
- [x] Define the valid field combinations for every Block operation in one validator.
- [x] Require a nonzero block count and an authorized page for `Read` and `Write`.
- [x] Require zero LBA, zero blocks, and no page for `Info`, `Flush`, `Cancel`, and `Reset`.
- [x] Reject `lba + blocks` overflow and requests past `BlockInfo.blocks`.
- [x] Define the valid field combinations for every Store operation in one validator.
- [x] Require `offset + length <= PAGE_SIZE` for `ReadChunk` and `WriteChunk`.
- [x] Require `BeginReplace.length <= PAGE_SIZE`.
- [x] Require a valid object name only on operations that identify an object.
- [x] Keep each request and reply small enough for the existing native-service context payload.
- [x] Encode and decode Block replies in `logos-core::native_service::Context`.
- [x] Encode and decode Store replies in `logos-core::native_service::Context`.
- [x] Make reply parsing verify the matching request ID.
- [x] Add one `logos-abi` test covering every accepted operation shape.
- [x] Add one `logos-abi` test covering unknown enums, overflow, invalid lengths, and invalid names.
- [x] Run `cargo test -p logos-abi -p logos-core`.
- [x] Commit the wire contract.

### Phase 2: make `logos-store` safe for the real payload

- [x] Add an explicit on-disk format version to superblocks and records.
- [x] Reject an unsupported format version as `Corrupt`.
- [x] Keep the existing little-endian encoding and checksums.
- [x] Replace the production full-disk `Vec` with sector-by-sector backend access.
- [x] Replace the production `BTreeMap` with a fixed 32-entry object index.
- [x] Store namespace, name, current location, and previous location in each index entry.
- [x] Return `Full` when a new name needs a thirty-third index entry.
- [x] Read object bytes into a caller-provided bounded output buffer.
- [x] Write record sectors from a caller-provided payload slice.
- [x] Keep the record header and commit sector in fixed 512-byte buffers.
- [x] Keep object payloads at or below `PAGE_SIZE`.
- [x] Make backend reads take mutable backend access so request IDs and gate state need no interior
      mutability.
- [x] Preserve `Io`, `TimedOut`, `Corrupt`, `Full`, `Invalid`, and `NotFound` as distinct errors.
- [x] Make Store memory usage independent of `SectorBackend::sectors()`.
- [x] Keep an allocation-backed memory backend only for host tests.
- [x] Scan only the superblocks and selected arena during recovery.
- [x] Treat an absent record magic after the last commit as the arena tail.
- [x] Treat a torn header or missing commit after a valid record as `Incomplete`.
- [x] Treat a bad checksum, impossible length, or unsupported version as `Corrupt`.
- [x] Never expose an incomplete or corrupt record through the index.
- [x] Preserve both current and previous versions during compaction.
- [x] Copy compaction records sector by sector into the inactive arena.
- [x] Flush the inactive arena before writing its selecting superblock.
- [x] Flush the selecting superblock before changing the active generation in memory.
- [x] Preserve the old arena and superblock until the final flush succeeds.
- [x] Keep the existing replace interruption test at every sector write and flush.
- [x] Keep the existing compaction interruption test at every sector write and flush.
- [x] Add a test for a blank disk.
- [x] Add a test that a non-blank corrupt disk is not reformatted.
- [x] Add a test for the 32-name limit.
- [x] Add a test for the `PAGE_SIZE` payload limit.
- [x] Add a backend test proving reads do not allocate memory proportional to disk size.
- [x] Run `cargo test -p logos-store`.
- [x] Commit the bounded Store engine.

### Phase 3: dispatch Store-owned Block requests through Core

- [x] Give Store one owned transfer page for Block I/O.
- [x] Keep client transfer pages separate from Store's Block transfer page.
- [x] Extend the fixed address-space mapping only enough to map those two shared pages.
- [x] Register the Block page with Store's real principal; remove numeric owner literals.
- [x] Map the Block page writable and non-executable in Store.
- [x] Resolve every request page through `SharedPages::address` for Store's principal.
- [x] Deny stale, unowned, unloaned, or wrong-principal page handles.
- [x] Handle `Info` from `block_device.info()` without submitting a VirtIO descriptor.
- [x] Submit valid `Read`, `Write`, and `Flush` requests to the existing Block driver.
- [x] Return immediate validation, cancellation, reset, and allocation failures to Store.
- [x] Wait for the matching VirtIO completion without busy-spinning.
- [x] On deadline, call the existing timeout/reset path exactly once.
- [x] Return the matching request ID and final status to Store.
- [x] Wake Store only after its reply is complete in the context page.
- [x] Reject a second request while one request is pending.
- [x] Cancel or time out a pending request when Store exits.
- [x] Reclaim Store-owned pages when Store exits.
- [x] Keep the terminal, Sessions, and recovery console responsive after a failed Block request.
- [x] Add a Core self-check for an unauthorized Block page.
- [x] Add a Core self-check for a stale Block page generation.
- [x] Add a Core self-check for a mismatched Block reply ID.
- [x] Run `cargo test -p logos-core`.
- [x] Run `cargo run -p logos-test -- run persistence/block-read-flush`.
- [x] Commit Core Block dispatch.

### Phase 4: run Store on the raw disk

- [x] Implement a `SectorBackend` in `logos-storage-service` using the Block request/reply gate.
- [x] Issue `Info` first and reject invalid or smaller-than-minimum devices.
- [x] Translate backend sector reads to one-page bounded Block reads.
- [x] Translate backend sector writes to one-page bounded Block writes.
- [x] Translate backend flush directly to Block flush.
- [x] Use monotonically increasing nonzero request IDs.
- [x] Give every Block request a finite deadline.
- [x] Map Block failure statuses to `logos_store::Error` without collapsing `TimedOut`, `Corrupt`,
      `Full`, or `Io` into success.
- [x] Read both superblocks on Store startup.
- [x] Complete the native-service handshake before issuing the first Block request.
- [x] Format only when both superblock sectors are entirely zero.
- [x] Flush the initial superblock before reporting Store ready.
- [x] Recover the highest valid superblock generation on later boots.
- [x] Report `Recovered` when an incomplete tail is ignored.
- [x] Report `Corrupt` and remain available for diagnostics when recovery finds corruption.
- [x] Do not overwrite a corrupt non-blank disk.
- [x] Mark Store healthy only after format or recovery reaches the idle request loop.
- [x] Remove the Store payload heap if the bounded engine no longer allocates.
- [x] Print one bounded debug marker for formatted, recovered, corrupt, and I/O-failed startup.
- [x] Add a headless boot marker proving Store read, write, flush, and recovery reached the real disk.
- [x] Run `cargo run -p logos-test -- run persistence/block-read-flush`.
- [x] Reboot the same raw disk and rerun the proof.
- [x] Commit the disk-backed Store startup.

### Phase 5: expose capability-scoped named objects

- [x] Keep the Store service in `READ_INPUT` while idle.
- [x] Relay Store requests only to the Store task.
- [x] Preserve one caller request ID through Store and back to the caller.
- [x] Track one bounded replace transaction containing caller, namespace, name, length, and bytes.
- [x] Reject `WriteChunk` without a matching `BeginReplace`.
- [x] Reject overlapping, skipped, repeated, or out-of-range chunks.
- [x] Reject `Commit` until exactly the declared bytes were written.
- [x] Make `Abort`, `Cancel`, caller exit, and Store restart discard the transaction.
- [x] Make `Commit` call the Store engine once and reply only after its final flush.
- [x] Make `OpenRead` select current or previous immutable version.
- [x] Make `ReadChunk` copy only the requested available bytes to the caller's loaned page.
- [x] Return the exact copied length and selected version in `StoreReply`.
- [x] Return `NotFound` for a missing object or missing previous version.
- [x] Return `Invalid` for malformed state transitions.
- [x] Return `Full`, `Corrupt`, `TimedOut`, and `Io` without losing the distinction.
- [x] Grant Terminal `StoreRead` scoped to `TERMINAL_NAMESPACE`.
- [x] Grant Terminal `StoreWrite` scoped to `TERMINAL_NAMESPACE`.
- [x] Check read capability before `OpenRead` and `ReadChunk` reach Store.
- [x] Check write capability before replace, commit, abort, or cancel reach Store.
- [x] Deny Terminal access to `TEXT_NAMESPACE`, `AUDIT_NAMESPACE`, and `SECRETS_NAMESPACE`.
- [x] Deny a read-only capability on every write operation.
- [x] Deny a revoked or stale capability.
- [x] Return `Denied` without waking Store or touching the disk.
- [x] Return every client page loan after the reply or failure.
- [x] Add a host test for the complete replace state machine.
- [x] Add a host test for current and previous reads.
- [x] Add a host test for abort and client-exit cleanup.
- [x] Add a QEMU test for allowed Terminal namespace access.
- [x] Promote `persistence/capability-denied` from future to implemented.
- [x] Run `cargo run -p logos-test -- run persistence/capability-denied`.
- [x] Commit the capability-scoped Store API.

### Phase 6: persist terminal history

- [x] Use the existing terminal-owned shared page; do not add another history buffer format.
- [x] Use `TERMINAL_NAMESPACE` and the fixed object name `history`.
- [x] On Terminal startup, request the current `history` object before accepting normal input.
- [x] Treat `NotFound` as empty history.
- [x] Validate the exact returned length before calling `restore_history_bytes`.
- [x] Treat invalid history bytes as reported corruption and continue with empty history.
- [x] After each non-empty submission, call the existing `export_history` contract.
- [x] Replace `history` with exactly `HISTORY_BYTES` bytes.
- [x] Keep the in-memory submission even when persistence fails.
- [x] Report one bounded diagnostic on persistence failure.
- [x] Keep the terminal interactive after Store failure or restart.
- [x] Retry only on the next history change; do not add a retry queue or timer in v1.
- [x] Add a test that missing history starts empty.
- [x] Add a test that valid persisted history restores Up/Down navigation.
- [x] Add a test that invalid persisted bytes do not replace live history.
- [x] Add a test that a failed save does not discard the submitted command.
- [x] Boot, enter two distinct commands, and stop QEMU.
- [x] Reboot the same disk and prove both commands are navigable in order.
- [x] Commit the first persistence consumer.

### Phase 7: prove interruption, corruption, and restart behavior

- [x] Promote `persistence/write-interruption` from future to implemented.
- [x] Create one baseline disk containing an old complete `history` value.
- [x] Copy the baseline disk before every interruption case.
- [x] Interrupt after every record sector write and every flush boundary.
- [x] Reboot the interrupted disk.
- [x] Assert the recovered value is exactly old or exactly new.
- [x] Assert no prefix, suffix, zero-filled, or mixed value is exposed.
- [x] Repeat the interruption matrix for compaction.
- [x] Interrupt before and after the inactive-arena flush.
- [x] Interrupt before and after the selecting-superblock write.
- [x] Interrupt before and after the selecting-superblock flush.
- [x] Promote `persistence/recovery` from future to implemented.
- [x] Prove reset preserves a completed value on the same raw disk.
- [x] Prove an incomplete tail reports recovery and remains writable.
- [x] Promote `persistence/corruption-detected` from future to implemented.
- [x] Flip one committed payload byte on a copied disk.
- [x] Reboot and assert `Corrupt` is reported.
- [x] Assert the corrupt object is not returned.
- [x] Assert Store does not reformat the disk.
- [x] Restart Store with no request in flight and prove reads still work.
- [x] Restart Store during an uncommitted replace and prove the old value remains current.
- [x] Restart Store after commit completion and prove the new value remains current.
- [x] Force a Block timeout and prove reset permits a later read without reboot.
- [x] Preserve the raw disk, debug log, control log, and QMP log for every failed case.
- [x] Run `cargo run -p logos-test -- suite persistence`.
- [x] Run `cargo fmt --check`.
- [x] Run `cargo clippy -- -D warnings`.
- [x] Run `scripts/run.ps1` and verify an interactive boot.
- [x] Run `scripts/check.ps1`.
- [x] Commit the Persistence v1 exit proof.

### Phase 8: close the milestone

- [x] Check every v1 scope item below against a passing proof.
- [x] Confirm `docs/ARCHITECTURE.md` still matches the Store model; no change is needed.
- [x] Update `docs/boot-sequence.md` with Store dependencies, failure mode, and recovery path.
- [x] Update `docs/security.md` with namespace read/write capability enforcement.
- [x] Update `docs/ROADMAP.md` to mark Persistence v1 complete.
- [x] Change this document's status to complete.
- [x] Commit the documentation-only milestone close separately.

## V1 scope

### Block boundary

- [x] Discover one dedicated raw VirtIO block device.
- [x] Provide bounded read, write, flush, timeout, cancellation, and reset in Core.
- [x] Report Block completion and recovery diagnostics.
- [x] Dispatch Store Block requests through the Core gate and Store-owned transfer page.

### Store boundary

- [x] Run one independently restartable System Store payload.
- [x] Expose capability-scoped namespaces containing named byte objects.
- [x] Keep immutable versions and atomic replacement in the host-tested Store engine.
- [x] Recover checksummed records through the host-tested crash-safe commit log.
- [x] Run the bounded Store engine against the real VirtIO disk.
- [x] Require explicit namespace read and write capabilities.

### First consumer

- [x] Persist terminal history through its existing bounded export/restore contract.

## Exit proof

Persistence v1 is complete only when the persistence test suite proves all of the following on the
same QEMU raw disk:

- Every replace and compaction interruption recovers the complete old or complete new value.
- Completed state survives reset.
- Cross-namespace and read-only writes are denied before disk access.
- Corruption is reported and never silently reformatted.
- Store restart loses no completed state.
- Block timeout/reset permits later progress without reboot.
- Terminal history survives reboot and remains usable.

## Crate boundary

- `logos-abi`: bounded Block and Store wire contracts.
- `logos-store`: `no_std`, host-testable disk format, object versions, commit, recovery, and
  compaction.
- `logos-storage-service`: independently restartable Ring-2 Store payload and Block-backed sector
  adapter.
- Existing kernel driver code: VirtIO block DMA and interrupt ownership until Ring-1 driver
  isolation is enforceable.

Do not split journals, objects, compaction, or other Store internals into more crates unless a real
dependency boundary requires it.

## Deferred

- Workspace-wide all-target clippy still mixes host tests with no-std UEFI binaries; targeted UEFI and host checks pass.
- Objects larger than one transfer page.
- More than 32 live object names.
- Multiple concurrent Store or Block requests.
- General streams, partitions, discard, quotas, accounting, and file compatibility.
- Memory-backed namespaces and a general consistency-checking tool.
- Configuration manifests.
- Identity, trust, secrets, audit retention, and durable secret encryption.
- Snapshots or retention beyond current and previous versions.
- Application workspaces and agent-memory policy.
- Any unrelated issue discovered while executing a phase; record the issue here in one line and
  continue when it does not invalidate the current phase's proof.

See [ADR-0013](adr/0013-persistence-v1-boundary.md),
[Architecture](architecture.md#11-storage-model), and [Security](security.md).
