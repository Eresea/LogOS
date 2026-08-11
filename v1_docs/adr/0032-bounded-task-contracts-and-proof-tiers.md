# ADR-0032: Bounded task contracts and proof tiers

- Status: Accepted
- Date: 2026-08-10

## Context

ABI-v4 work spans typed endpoint pages, Core scheduler composition, service-local state, and QEMU
hardware contracts. A crate boundary alone does not identify the isolation seam, and host tests
cannot establish boot, device, replacement, or fault-containment behavior.

## Decision

- Every task crossing a service, device, or privilege boundary records the fields in
  [`task-contract-template.md`](../task-contract-template.md).
- A fixed owner-held slot is the canonical async form. `OperationIdentity` supplies the common
  owner/generation/request identity check; typed subsystem phase state retains capability-approved
  resources, deadlines, cancellation, and terminal cleanup.
- Fast host tests are required for changed bounded state, validation, ownership/generation,
  timeout, cancellation, and resource-exhaustion behavior.
- QEMU proofs are additionally required for changed boot, target-specific, device, IPC/page,
  scheduler, replacement, recovery, or fault-containment behavior. They remain milestone evidence,
  not a substitute for the host acceptance test.

## Consequences

- Storage, Sessions, and Network retain their typed states; this does not introduce an executor,
  universal completion ABI, or a second scheduler.
- A vertical slice may cross crates, while an isolated crate change can still require a QEMU proof.
- Existing proof IDs remain frozen regression contracts until their public contract is deprecated.
