# LogOS codebase review — completed portions — 2026-07-31

The remaining items stay in [`../review/20260731_codebase_review.md`](../review/20260731_codebase_review.md).

## Completed in this batch

### P0.1 — explicit build/test gate

- Added host, UEFI, and QEMU stages to [`scripts/check.ps1`](../../scripts/check.ps1) and [`scripts/verify.ps1`](../../scripts/verify.ps1).
- Host checks exclude `no_std` UEFI binaries; UEFI checks build `logos-uefi` and all three service payloads.
- Architecture and repository-relative Markdown-link checks are mandatory.
- UEFI artifacts use `target/logos-check/uefi/{debug,release}` with deterministic names and SHA-256 manifests.

### P2.16 — architecture validation

- Replaced the stale source-only analyzer with Cargo-metadata dependency validation in [`scripts/arch-deps.py`](../../scripts/arch-deps.py).
- Service implementation files are checked for assembly use outside `logos-service-rt`.
- The check fails CI on unapproved internal dependency edges.

### Completed portions of P0.2, P2.15, P2.19, and P2.20

- The authoritative workflow now has explicit permissions, cancellation, host/UEFI/QEMU stages, correct UEFI artifact names, and release payload hashes.
- Duplicate push/pull-request workflow triggers were reduced to manual or scheduled workflows.
- Workspace metadata, Rust version, and inherited Rust lints were added to the manifests.
- README, AGENTS, TODO, and roadmap milestone status now agree on Persistence v1.
- Stale lowercase README links and absolute `file:///` links in the archived review were repaired.

## Verification

- `cargo fmt --check --all`
- `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/check.ps1 -Stage host`
- `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/check.ps1 -Stage uefi -Release`
- `python scripts/arch-deps.py --check`
- `python scripts/docs-check.py`

QEMU was not available locally; the QEMU stage remains an active review requirement.
