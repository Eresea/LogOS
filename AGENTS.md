# LogOS Agent Guide

LogOS vNext is a clean-slate `no_std` Rust UEFI kernel. The current milestone is the smallest
bootable slice: enter UEFI, emit one debug-console line, and remain alive.

## Rules

- Preserve `no_std`, fixed-size resource bounds, and QEMU debug-console output.
- Keep changes small and independently bootable; do not add runtime, allocator, or OS dependencies
  without a documented milestone.
- Run `cargo fmt --check` and `cargo clippy -- -D warnings` after Rust changes.
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

The single UEFI binary owns the current boot proof. Scheduling, memory, IPC, services, and
applications are deferred until a concrete acceptance test requires each one.

Plan before coding, verify the result, and report remaining issues. Do not bundle unrelated changes.
