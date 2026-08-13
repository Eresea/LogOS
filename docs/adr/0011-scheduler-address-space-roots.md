# ADR-0011: Scheduler address-space roots

- Status: Accepted
- Date: 2026-08-12

## Decision

Each scheduler slot publishes one page-aligned address-space root alongside its generation. Kernel
tasks use root zero, which selects the kernel CR3; the proof task publishes its fixed user root.
Publication happens before the slot becomes runnable and reclamation clears the root. Service
processes use this same binding when they enter ring 3.

## Scope

The slot stores a raw root, not a process or address-space capability handle. The live supervisor
validates task generations before teardown and clears the root before reuse; replacement reclaims
process mappings and page-table frames before publishing a new service task.
