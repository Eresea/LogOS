# ADR-0001: Fixed-stack preemptive SMP Core

- Status: Accepted
- Date: 2026-08-11

## Decision

Core uses eight fixed 16 KiB task stacks, fixed per-CPU scheduler stacks, and a bounded atomic
slot state word containing generation, lifecycle, and wake-pending state. Timer and voluntary
switch entry share one canonical saved frame. Assembly saves the complete outgoing frame on the
task stack, switches to the per-CPU scheduler stack, and only then publishes the task state for
another CPU to claim. Task bodies execute without a global scheduler lock.

UEFI discovers healthy processors, rejects unsupported x2APIC IDs and malformed capacity, stages a
low-memory trampoline, and starts APs sequentially with INIT/SIPI/SIPI using the current CR3. Each
CPU installs private GDT/IDT state, baseline x87/SSE configuration, GS-addressed local state, and a
periodic local-APIC timer.

## Consequences

The scheduler is SMP-native from its first implementation and can later expose task contexts to
Future/Waker integration without becoming an async executor. Fixed capacity and fixed stacks keep
the Core allocator-free and make publication ordering testable. AP startup remains intentionally
narrow and xAPIC-only.

## Deferred

Allocators, dynamic stacks, affinity, priorities, work stealing, wake IPIs, AVX/XSAVE, user mode,
Runtime orchestration, services, IPC, capabilities, terminal, storage, and networking are outside
Core. `v1_docs/` remains historical.
