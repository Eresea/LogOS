# ADR-0010: Scheduler CR3 selection

- Status: Accepted
- Date: 2026-08-12

## Decision

Before returning from an interrupt, the scheduler loads the kernel CR3 for kernel tasks and the
registered proof CR3 for the user task. The idle return path restores the kernel CR3. Kernel CR3 is
published before AP startup so every CPU has a valid fallback.

## Scope

This remains a bounded service-domain lifecycle. The scheduler carries the selected root needed for
the current process launch path; general address-space handles, replacement flush policy, and TLB
shootdown isolation remain outside this milestone.
