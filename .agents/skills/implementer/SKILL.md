---
name: implementer
description: Implement an approved or clearly scoped LogOS vNext change with a minimal independently bootable patch and proportional verification. Use when the task requires editing code or project documentation; do not use for read-only planning or review.
metadata:
  short-description: Implement and verify a LogOS change
---

# LogOS implementer

Implement the requested LogOS vNext change, keeping the patch narrow and independently verifiable. Read the existing project instructions and current code before editing. If no plan exists, make a short internal plan first and state the scope before changing files.

## Before editing

1. Read `AGENTS.md` and use `docs/README.md` to select only the active documentation relevant to the task.
2. Inspect `git status --short` and the current diff. Preserve unrelated user changes.
3. Locate the smallest responsible layer with `rg`. Reuse existing types, bounds, proof hooks, and ownership instead of introducing parallel abstractions.
4. Re-check that the requested behavior belongs in the current milestone. Do not pull deferred runtime, service, IPC, capability, allocation, or user-mode work into the Core without explicit scope.

## Implementation rules

- Preserve `no_std`, fixed-size resource bounds, UEFI bootability, and QEMU debug-console output.
- Keep `src/main.rs` limited to UEFI entry, `src/lib.rs` as the Core façade, and `src/scheduler.rs` as the host-tested scheduler state machine unless the task explicitly changes ownership.
- Prefer the smallest change at the narrowest responsible layer. Avoid speculative APIs, generic abstractions, new runtime or OS dependencies, and unrelated cleanup.
- Add or update the smallest relevant host test for behavior. Keep proof workloads deterministic and bounded.
- Update `docs/architecture.md` only when the change creates a boundary that a proof needs documented.
- Create and index an ADR only for an irreversible or cross-ring decision. Routine reversible implementation does not need one.
- Do not commit, reset, discard, or rewrite unrelated work.

## Verification

After Rust changes, run the project-required checks:

- `cargo fmt --check`
- the smallest relevant host test, then broader `cargo test` coverage when the change warrants it
- `cargo clippy -- -D warnings`

For UEFI, boot, QEMU, or other hardware-facing changes, also run the smallest relevant `scripts/check.ps1` or `scripts/run.ps1` path. For documentation or ADR changes, run the relevant documentation or ADR index check when available.

If a required check cannot run, report the exact command and reason. Never claim a check passed based on inspection alone.

## Handoff

Finish with:

1. changed files and the behavior added or corrected;
2. verification commands and results;
3. remaining issues, assumptions, or deferred follow-up;
4. a note about any user changes intentionally left untouched.
