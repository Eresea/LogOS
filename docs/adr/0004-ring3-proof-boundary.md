# ADR-0004: Fixed ring-3 proof boundary

- Status: Accepted
- Date: 2026-08-12

## Context

The terminal contracts are bounded, but the Core still needs a hardware-backed proof that a
service-shaped task can leave ring 0 and fail without taking the machine down. The scheduler has
fixed task stacks, no allocator, and no general process loader.

## Decision

- Add ring-3 code/data descriptors, one packed TSS per CPU, and a DPL-3 software interrupt gate.
- Keep the active task stack as that task's TSS RSP0 so interrupt and exception frames stay with the
  task being stopped; use the per-CPU entry stack only while no task is active.
- Build one static proof address space by cloning the current kernel root and replacing one user
  PML4 slot with aligned code and stack pages.
- Run a fixed user image that invokes vector 49 once and then executes `ud2`.
- Treat #UD, #GP, and #PF as contained only for the registered proof handle; all other kernel faults
  remain fatal. A contained fault publishes `Completed` to the existing scheduler.

## Consequences

QEMU now proves ring-3 entry, a user-originated software interrupt, and task-local fault recovery
on 1, 2, and 8 CPUs. The proof uses no allocator, ELF loader, or general syscall payload ABI.

## Deferred

Per-process address-space objects and CR3 ownership, capability-backed mappings, ELF image loading,
syscall dispatch beyond the proof vector, process reaping, and terminal service packaging remain
follow-on work. The proof address space must not be treated as the general process model.
