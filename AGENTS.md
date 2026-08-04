# LogOS Agent Guide

## Scope

LogOS is an experimental Rust UEFI kernel. Persistence v1 and Network v1 are complete; the
current milestone is Remote Foundation v1 and the ABI-v4 maintainability migration recorded in
ADR-0020.

## Rules

- Keep kernel code `no_std`; do not add allocator, runtime, or OS dependencies without a concrete milestone.
- Prefer small, independently bootable changes. Preserve QEMU debug-console output.
- Run `cargo fmt --check` and `cargo clippy -- -D warnings` after Rust changes.
- Boot with `scripts/run.ps1` when touching boot, UEFI, or hardware-facing code.
- Document a new subsystem in `docs/ARCHITECTURE.md` before expanding it across files.
- Treat `docs/boot-sequence.md` and `docs/security.md` as architectural constraints, not implementation prompts.
- Create an ADR in `docs/adr/` for irreversible or cross-ring decisions; update its index and the affected roadmap/architecture docs. Skip ADRs for routine, reversible implementation details.
- Commit each independently bootable or documentation-only change separately; do not bundle unrelated work.

## Boundaries

The kernel owns hardware, memory, scheduling, IPC, and capabilities. Higher-level functionality belongs in replaceable services; applications are future WASM modules.

## Architecture

The onion rings define ownership and dependency direction, not runtime boundaries. Each ring exposes a small typed API, hides its implementation, and translates higher-level intent into lower-level operations. Use function calls, IPC, or syscalls according to the required isolation—not because of the ring boundary itself.

## Execution Policy

Plan before coding. Complete the requested task end-to-end. If a prerequisite issue is found, fix it, verify it, and continue. Stop only when blocked by missing information or an architectural decision outside the documented design. Report implemented changes, prerequisite fixes, tests run, and remaining issues.
