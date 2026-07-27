# LogOS Agent Guide

## Scope

LogOS is an experimental Rust UEFI kernel. The current milestone is the Platform v1 service boundary.

## Rules

- Keep kernel code `no_std`; do not add allocator, runtime, or OS dependencies without a concrete milestone.
- Prefer small, independently bootable changes. Preserve QEMU debug-console output.
- Run `cargo fmt --check` and `cargo clippy -- -D warnings` after Rust changes.
- Boot with `scripts/run.ps1` when touching boot, UEFI, or hardware-facing code.
- Document a new subsystem in `docs/architecture.md` before expanding it across files.
- Treat `docs/boot-sequence.md` and `docs/security.md` as architectural constraints, not implementation prompts.
- Create an ADR in `docs/adr/` for irreversible or cross-ring decisions; update its index and the affected roadmap/architecture docs. Skip ADRs for routine, reversible implementation details.
- Commit each independently bootable or documentation-only change separately; do not bundle unrelated work.

## Boundaries

The kernel owns hardware, memory, scheduling, IPC, and capabilities. Higher-level functionality belongs in replaceable services; applications are future WASM modules.
