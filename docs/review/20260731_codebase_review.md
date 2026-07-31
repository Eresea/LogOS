# LogOS codebase review — remaining backlog — 2026-07-31

Completed portions are archived in [`../reviewed/20260731_codebase_review.md`](../reviewed/20260731_codebase_review.md).

This file keeps only work not completed by the 2026-07-31 gate/contract pass.

## P0

### CI follow-up

No active workflow uses unpinned third-party actions or swallows failures. Documentation link, spelling, and performance instrumentation remain outside the current gate where no corresponding workflow exists.

### Persistence vertical slice

Finish malformed-input, full-storage, restart/recovery, power-loss, and QEMU proofs for the storage service. Keep `logos-store` as the host-testable format engine.

## P1 — ownership and contracts

- Thin the UEFI root and kernel orchestrator into typed boot, startup-check, lifecycle, and terminal-bootstrap seams.
- Remove direct terminal implementation knowledge from kernel code.
- Separate policy from mechanism in `src/platform` and `src/drivers`.
- Finish the safe native-service runtime boundary and split protocol contracts/versioning where consumers require it.
- Specify enforceable capability ownership, attenuation, provenance, expiry/revocation, restart, reclaim, denial, and audit behavior.
- Contain remaining assembly/unsafe code behind documented HAL wrappers.
- Make memory allocation, mapping, payload staging, and service layout failures transactional and visible.
- Replace externally influenced invariant panics with structured denial, health, or recovery failures.
- Define secret/identity lifecycle and IPC concurrency/request-generation rules.

## P2 — development loop

- Finish the workspace toolchain/target policy and package-role documentation.
- Add the remaining boundary negative tests and QEMU proofs.

Do not start the deferred crate-per-ring, IPC redesign, allocator rewrite, distributed transactions, graphics framework, higher-half, or SMP work without a concrete milestone.
