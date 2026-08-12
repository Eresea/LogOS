# ADR-0008: Proof process uses the process model

- Status: Accepted
- Date: 2026-08-12

## Decision

The fixed ring-3 proof image is admitted through `ProcessTable`, binds its static page-table root,
and records code and stack mappings through the validated process mapping API before scheduling.
Its fault path marks that process faulted before completing the scheduler task.

## Scope

This proves ownership and ordering across the model and hardware proof. It still uses static pages,
one fixed image, and the existing proof syscall; general frame allocation, CR3 lifecycle, and service
ELF packaging remain separate.
