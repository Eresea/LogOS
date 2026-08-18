---
name: debugger
description: Diagnose and, when explicitly requested, narrowly fix LogOS vNext build, test, boot, UEFI, QEMU, scheduler, or proof failures using reproduction and evidence. Use for a specific failure or regression; do not use for broad feature planning or general code review.
metadata:
  short-description: Diagnose a LogOS failure
---

# LogOS debugger

Find the smallest evidence-backed root cause of a specific LogOS failure. Diagnose before editing. Apply a fix only when the request explicitly asks for one or the debugging task clearly includes remediation; otherwise stop after the diagnosis and recommended patch.

## Establish the failure

1. Read `AGENTS.md`, `docs/README.md`, and the active docs for the affected subsystem.
2. Capture the exact command or boot path, expected result, actual result, environment, repeatability, and the last known-good revision or diff when available.
3. Inspect `git status --short` and the current diff before changing anything. Preserve unrelated user work.
4. Reproduce with the smallest relevant command. Use host tests for pure state logic; use `scripts/check.ps1` or `scripts/run.ps1` for UEFI, QEMU, boot, and hardware-facing failures.
5. For boot failures, preserve and inspect QEMU debug-console output. Do not hide a failure by weakening assertions, suppressing output, increasing arbitrary timeouts, or changing proof workloads without evidence.

## Narrow the cause

- Classify the failure as build/dependency, host-test logic, UEFI handoff, per-CPU/SMP, scheduler/preemption, memory/bounds, proof integration, or documentation/tooling.
- Form a small set of falsifiable hypotheses, then test them with targeted inspection or one focused experiment at a time.
- Trace the failing value or state transition to its owner. Check invariants at the boundary where the bad state is introduced, not only where it becomes visible.
- Prefer existing bounded types, scheduler transitions, proof hooks, and diagnostics. Do not introduce a new subsystem or dependency to make a diagnosis easier.
- For concurrency or boot failures, distinguish a deterministic logic error from timing sensitivity and record the evidence for that distinction.

## If remediation is in scope

- Make the smallest patch at the responsible layer.
- Preserve `no_std`, fixed-size resource bounds, independently bootable increments, and QEMU debug-console output.
- Add a focused regression test or proof assertion when practical.
- Follow the project verification rules after Rust changes: `cargo fmt --check`, the smallest relevant host test, and `cargo clippy -- -D warnings`. Run the relevant boot or UEFI script as well when applicable.
- Do not bundle refactors, speculative hardening, or unrelated cleanup.

## Deliverable

Return:

1. **Reproduction** — exact command/path and observed failure.
2. **Root cause** — the responsible file, symbol, state transition, and violated invariant.
3. **Evidence** — checks or experiments that ruled out competing explanations.
4. **Fix** — changed files and why the patch is sufficient, if remediation was requested.
5. **Verification** — commands and exact results, including anything blocked.
6. **Remaining risk** — only concrete unresolved behavior or follow-up.
