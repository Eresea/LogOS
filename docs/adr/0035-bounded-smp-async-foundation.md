# ADR-0035: Keep SMP scheduling and async execution bounded

- Status: Accepted
- Date: 2026-08-11

## Context

Core's production scheduler is a fixed cooperative registry of mutable `Runnable` references. It
has generation-tagged handles but assumes one scheduler owner, while ACPI exposes only APIC and
IO-APIC addresses. There is no safe AP entry path, per-CPU descriptor state, cross-CPU scheduler
notification, or scheduler-native `Future`/`Waker` path.

## Decision

Add a separate fixed-storage `SmpScheduler` with a shared registry of eight task slots and eight
async-capable slots. Each slot packs generation, scheduling state, and a wake-pending bit into one
atomic word. A claim changes `Runnable` to `Running` with compare-and-exchange; only the claimant
may poll or run the task. A pending transition first observes the wake bit and then rechecks after
publishing `Blocked`, so a wake racing with `Pending` cannot be lost. Completed slots advance their
generation before reuse.

Each of eight bounded CPU records owns an APIC identity, scan cursor, idle flag, and reschedule
hint. CPU workers scan the shared registry; work stealing and migration policy are deferred. The
architecture installs scheduler notification vector 49 and sends a fixed local-APIC IPI. The IPI
only records a reschedule hint and acknowledges the APIC.

Async entries borrow fixed pinned `Future<Output = ()> + Send` values. A fixed pool of sixteen
reference-counted waker tokens supplies generation-safe `RawWaker` values without `alloc`; stale
wakers fail the same generation check as stale handles. The existing cooperative `Scheduler` and
native-service scheduler remain unchanged and continue to own current services.

## Consequences

- Scheduler memory is bounded by 8 task slots, 8 async slots, 8 CPU records, and 16 waker tokens.
- Future storage is caller-owned and must remain at a stable scheduler address while wakers exist.
- Wakes are hints; task state remains authoritative and duplicate wakes do not enqueue work.
- The single-core path remains the only production execution path until AP startup is complete.
- Host tests cover the portable atomic state protocol; UEFI target checks cover integration.

## Verification boundary

The reviewed tests keep the portable atomic model separate from the production scheduler. The
model proves claim, wake-pending, duplicate-wake, generation, and capacity transitions, while the
production tests cover bounded task slots, `RawWaker` generation invalidation, retained-waker
capacity, and CPU guards. The model is not a proof of production `UnsafeCell` access or a true
cross-thread `Pending`/wake race.

Before AP release, add a host-runnable production-scheduler harness or a QEMU proof that exercises
the real `SmpScheduler` across CPUs; do not treat the model tests as a substitute for that proof.

## Deliberate deferrals

AP startup still requires a low-memory trampoline, release barrier, per-CPU stacks, and per-CPU
GDT/TSS/IDT state. The current `cpu` and `interrupts` modules use global descriptor and interrupt
state, so releasing an AP would violate the existing privilege and fault-return invariants. The
current local-APIC IPI primitive also targets xAPIC IDs representable in the fixed destination
field; x2APIC and interrupt-controller redesign are deferred.

There is no `-smp 2` QEMU proof in this increment. Adding one before APs execute the controlled Core
entry point would only prove BSP bookkeeping, not SMP scheduling. Timer preemption, task migration,
CPU affinity, FPU state, and async conversion of native services are likewise deferred.
