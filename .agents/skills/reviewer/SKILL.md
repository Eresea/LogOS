---
name: reviewer
description: Review LogOS vNext diffs for correctness, regressions, scope violations, missing proof, and project-rule violations without editing files. Use for branch, commit, pull request, or working-tree review; do not use to implement fixes.
metadata:
  short-description: Review a LogOS change read-only
---

# LogOS reviewer

Perform a read-only, evidence-based review of the requested LogOS vNext change. Review the actual diff and relevant surrounding code; do not modify files, apply fixes, commit, reset, or broaden the review into a general audit.

## Review setup

1. Read `AGENTS.md`, `docs/README.md`, and only the active docs relevant to the changed subsystem.
2. Establish the review scope from `git status --short`, the requested base or commit range, `git diff --stat`, and `git diff --check`. Include relevant untracked files when they are part of the requested change.
3. Trace changed behavior into its callers, state transitions, bounds, proof workloads, and tests. Do not review only the changed lines.
4. Separate findings caused by this change from pre-existing issues and from suggestions that are not required for correctness.

## LogOS review criteria

Prioritize concrete defects and regressions involving:

- `no_std`, fixed-size bounds, unchecked growth, hidden allocation, runtime/OS dependencies, or unbounded loops;
- UEFI handoff, per-CPU setup, scheduler state, interrupt/preemption behavior, SMP assumptions, and QEMU debug-console output;
- ownership drift from `src/main.rs`, `src/lib.rs`, or `src/scheduler.rs`;
- accidental activation of deferred runtime, services, IPC, capabilities, allocation, or user-mode work;
- proof workloads that are nondeterministic, not host-testable, or no longer exercise the intended invariant;
- missing or misleading tests, formatting/clippy regressions, and failure to run the smallest relevant boot or UEFI check;
- undocumented subsystem boundaries or missing ADR/index updates for irreversible or cross-ring decisions;
- unrelated files, speculative abstractions, or dependency expansion.

Treat `v1_docs/` and stale evidence as historical unless the task explicitly asks for historical comparison.

## Findings format

Report findings first, ordered by severity:

```text
P0/P1/P2/P3 — path:line — concise problem
Impact: what breaks or becomes unsafe.
Evidence: the relevant code path, invariant, or missing verification.
Fix direction: the smallest corrective action.
```

Use P0 for a release-blocking or data-corrupting defect, P1 for a serious correctness or boot regression, P2 for a normal defect or missing required proof, and P3 for a low-impact issue worth correcting. Do not report style preferences as findings. Keep each finding actionable and tied to changed behavior.

If there are no actionable findings, say so explicitly, then list residual risks and checks not run. Do not declare the change safe merely because the diff is small.
