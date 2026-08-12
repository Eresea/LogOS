# ADR-0010: Scheduler CR3 selection

- Status: Accepted
- Date: 2026-08-12

## Decision

Before returning from an interrupt, the scheduler loads the kernel CR3 for kernel tasks and the
registered proof CR3 for the user task. The idle return path restores the kernel CR3. Kernel CR3 is
published before AP startup so every CPU has a valid fallback.

## Scope

This is the proof-domain lifecycle only. The scheduler does not yet carry a general process address-
space handle, flush policy, or TLB isolation contract.
