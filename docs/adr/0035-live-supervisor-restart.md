# ADR-0035: Live supervisor restart ownership

- Status: Accepted
- Date: 2026-08-13

## Decision

The supervisor owns graph-wide service replacement. A missed heartbeat or contained service fault
first quiesces every service task at a scheduler boundary. Only after all service tasks are completed
are process records, address-space mappings, page-table frames, image frames, device mappings, and IPC
pages reclaimed. The replacement graph receives a new IPC generation and service epoch; old messages
are rejected by the shared ABI identity check.

Restart is bounded by the existing service, process, mapping, image, frame, and scheduler limits.
The UEFI restart workload runs on the fixed 256 KiB task stack, a deliberate bound required by the nested
ELF/page-table teardown and rebuild path. Kernel tasks never expose a user address-space root, even if
stale metadata is observed during handoff.

This is live in-memory replacement only. No terminal state, operation journal, or resumable work is
stored for a later reboot. A full reboot starts a fresh service graph from the boot image.

## Consequences

An isolated service failure does not require a kernel reboot and cannot leave its old address space
schedulable. Restart temporarily stops the terminal graph, so the terminal redraws/reissues its prompt
after the new generation is published. Restart attempts are capped; repeated failure becomes policy
error. Durable recovery is a separate future task requiring a proto-filesystem, storage ownership,
and explicit journal/idempotency contracts.

The deterministic QEMU proof suppresses one service heartbeat, verifies restart and stale-message
rejection, and continues to the terminal/input/display assertions on one, two, and eight CPUs.
