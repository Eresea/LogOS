# ADR-0034: Ring-3 BSP affinity during initial SMP boot

## Status

Superseded by ADR-0039

## Decision

Ring-3 service and proof tasks remain schedulable only on the bootstrap
processor until user-task migration has explicit per-CPU TSS entry stacks and
CR3/TLB shootdown ownership. Kernel and ring-0 tasks continue to use the full
SMP scheduler.

The service frame pool also reserves the active firmware page-table tree before
allocating isolated service roots. Those frames remain owned by the kernel
mapping until a later page-table teardown boundary exists.

## Consequences

- The bounded terminal graph is stable on 1, 2, and 8 CPU QEMU boots.
- Ring-3 parallelism is intentionally deferred rather than relying on unsafe
  address-space migration.
- A future migration slice must replace the affinity guard and add tests for
  per-CPU entry stacks, CR3 publication, and TLB invalidation.
