# ADR-0039: Ring-3 SMP migration

- Status: Accepted
- Date: 2026-08-14

## Decision

Generation-safe ring-3 tasks may be claimed by any online CPU after their
context has been published as `Runnable`. The existing global fixed-slot
scheduler remains the run-queue mechanism; no per-CPU queues, work stealing,
affinity, or priorities are introduced.

The target CPU loads the task's published CR3, updates its TSS `RSP0` to the
task-owned fixed stack, and only then restores the saved context. Loaded user
address spaces are immutable while runnable. A CR3 reload refreshes the
target CPU's local translations; general live page-table mutation and TLB
shootdown integration remain deferred until that boundary exists.

Vector 49 remains the user syscall path. A dedicated APIC reschedule vector
causes a CPU to save its current context and claim runnable work. Event,
timeout, and proof wakeups send at most one bounded reschedule IPI to another
online CPU, preferring an idle target.

## Consequences

- Ring-3 service and proof tasks can execute on non-BSP CPUs without changing
  the fixed scheduler storage or task-stack ownership.
- Idle CPUs receive a bounded prompt when a wakeup makes work runnable.
- Supervisor restart remains safe because it quiesces every service task before
  reclaiming process roots, mappings, and frames.
- QEMU proof now requires post-CR3 ring-3 execution on an AP and receipt of a
  reschedule IPI for SMP runs.
