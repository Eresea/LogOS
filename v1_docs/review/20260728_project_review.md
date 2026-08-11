# Architecture Review Follow-ups

The 2026-07-28 review is archived in `docs/reviewed/20260728_project_review.md`.

Keep these decisions as entry criteria for their future milestones:

- Before System-layer delegation, define capability ownership, attenuation, expiry, provenance, and revocation semantics.
- Before Persistence v1, define durable ownership and cross-service consistency around idempotent operations and reconciliation; do not promise distributed transactions.
- Before untrusted DMA-capable drivers, define the IOMMU/security model.
- Before broad restart or update support, classify services and operations as restartable, resettable, fatal, reversible, or irreversible.
- Keep QEMU plus one physical reference platform until Core needs another target.

Everything else in the archived review remains research, not a current implementation promise.
