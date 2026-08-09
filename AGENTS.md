# LogOS Agent Guide

LogOS is an experimental `no_std` Rust UEFI kernel. The current milestone is ABI v4
stabilization and migration closeout: Network bootstrap transport and the independent TCP
foundation are implemented, while scalable Network work, one Network suite proof, and five
explicitly skipped Remote proofs remain open.

## Rules

- Preserve `no_std`, fixed-size resource bounds, QEMU debug-console output, and public contracts.
- Keep changes small and independently bootable; do not add runtime, allocator, or OS dependencies
  without a documented milestone.
- Run `cargo fmt --check` and `cargo clippy -- -D warnings` after Rust changes.
- Run the smallest relevant host test; use `scripts/run.ps1` or `scripts/check.ps1` for boot, UEFI,
  or hardware-facing changes.
- Document new subsystem boundaries in `docs/architecture.md`; treat `docs/boot-sequence.md` and
  `docs/security.md` as constraints.
- Create and index an ADR for irreversible or cross-ring decisions. Routine reversible changes need
  no ADR.
- Keep historical evidence in reviewed documents or ADRs, not in active checklists.

## Ownership

Core owns hardware, memory, scheduling, IPC, and capabilities. Replaceable services own higher-level
policy and durable state; applications are future WASM modules. Onion rings describe dependency
direction, not mandatory runtime boundaries.

Plan before coding, verify the result, and report remaining issues. Do not bundle unrelated changes.
