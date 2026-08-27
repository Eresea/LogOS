# ADR-0060: Bounded persistent program lifecycle

Status: Accepted

## Context

LogOS already owns bounded package storage, ELF admission, process address spaces, page tables, and
generation-safe scheduler tasks. A second transport or allocator for user programs would duplicate
those boundaries and make reclamation harder to prove.

## Decision

Program packages use the v2 package manifest with kind=Program and no service target. Storage
persists service and program records in one fixed catalog. Core extends the existing manager ABI with
a bounded program-name target and keeps one fixed program table in ServiceRuntime. program.start,
program.status, and program.stop use generation-safe records. A program receives only its ELF
mappings, fixed stack, and the bounded Atrium bootstrap/channels defined by ADR-0070; it receives no
ambient service capabilities.

Forced stop first requests scheduler termination. Process frames, page-table frames, loaded-image
frames, program IPC channels, queue frames, and bootstrap pages are reclaimed only after the
scheduler publishes task completion. Unknown ABI versions and malformed targets fail closed; legacy
service packages and storage snapshots remain readable.

## Consequences

Programs can remain running after the initiating Flow command returns, and reboot persistence is
provided by Storage. Filesystem/network access, capabilities, signatures, repository resolution,
automatic boot, and cooperative cancellation remain deferred; Atrium surface IPC is defined by
ADR-0070.

## Proof obligations

- package manifest and dependency rules reject invalid program targets;
- service/program targets, catalog records, generations, and legacy snapshots round-trip safely;
- start, status, stop, exit, fault, stale-handle rejection, slot reuse, and frame reclamation are bounded;
- the program proof covers install, persistent running state, stop/reclamation, relaunch, and reboot lookup.
