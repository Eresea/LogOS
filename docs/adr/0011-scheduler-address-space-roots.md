# ADR-0011: Scheduler address-space roots

- Status: Accepted
- Date: 2026-08-12

## Decision

Each scheduler slot publishes one page-aligned address-space root alongside its generation. Kernel
tasks use root zero, which selects the kernel CR3; the proof task publishes its fixed user root.
Publication happens before the slot becomes runnable and reclamation clears the root.

## Scope

The slot stores a raw root, not a process or address-space capability handle. Lifetime validation and
general process-to-task binding remain deferred.
