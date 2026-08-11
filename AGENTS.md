# LogOS Agent Guide

LogOS vNext is a clean-slate `no_std` Rust UEFI kernel. The active milestone is the bounded
preemptive SMP Core: UEFI handoff, per-CPU setup, fixed-stack scheduler, and proof workloads.

## Rules

- Preserve `no_std`, fixed-size resource bounds, and QEMU debug-console output.
- Keep changes small and independently bootable; do not add runtime, allocator, or OS dependencies
  without a documented milestone.
- Run `cargo fmt --check`, host tests, and `cargo clippy -- -D warnings` after Rust changes.
- Run the smallest relevant host test; use `scripts/run.ps1` or `scripts/check.ps1` for boot, UEFI,
  or hardware-facing changes.
- Document new subsystem boundaries in `docs/architecture.md`; add them only when a proof requires
  them.
- Use `docs/README.md` to select the smallest relevant documentation set; optional annexes are not
  default project context.
- Create and index an ADR for irreversible or cross-ring decisions. Routine reversible changes need
  no ADR.
- Keep historical evidence in reviewed documents or ADRs, not in active checklists.

## Ownership

`src/main.rs` owns only the UEFI entry. `src/lib.rs` owns Core mechanisms and the host-tested
scheduler state machine. Runtime, services, IPC, capabilities, allocation, and user mode remain
deferred. `v1_docs/` and stale v1 evidence are historical reference only.

Plan before coding, verify the result, and report remaining issues. Do not bundle unrelated changes.
