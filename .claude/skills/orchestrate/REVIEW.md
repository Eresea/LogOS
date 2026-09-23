# PR review checklist

Run every item; each one is a question the review comment must answer. Evidence the agent reports is a claim until you reproduce it: CI does not boot QEMU (and may not run at all), so your runs are the only independent check.

## Checklist

1. **Base.** Is the branch on current `main` (`git merge-base --is-ancestor origin/main <sha>`)? If not, can it conflict with what merged since? Ask for a rebase when it can, or when its proofs predate a relevant fix.
2. **Scope.** Does the diff stay inside the issue's scope and "must not cross" list? Name every file outside it and whether it is justified. Flag new dependencies and widened public interfaces.
3. **Tests prove the change.** For each new test, would it fail on the old code? When unsure, run a **mutation check**: revert the key line in your exported copy and confirm the test goes red. A test that passes either way is tautological — ask for one bound to the production value.
4. **Host checks.** On an exported copy (below): `cargo fmt --check`, the touched packages' tests, and the target build/clippy the change affects (UEFI for Core, `x86_64-unknown-none` for service images and programs).
5. **QEMU.** Run the issue's proofs yourself with [scripts/qemu-runs.ps1](scripts/qemu-runs.ps1), fresh disks, your own QMP port. It reports pass/fail, the failing step, rejection-marker counts, and surface markers per run.
6. **Red proof triage.** Before blaming the PR, rerun the same command on its base commit. Same failure on base → pre-existing: say so and track it as its own issue. Failure only on the branch → the PR's. Failure that comes and goes → capture the log, find the mechanism (a race in the proof script is as likely as a product bug).
7. **Delivery evidence.** Required markers present; rejection markers zero for the surfaces the PR touches (filter by the logged surface handle when an owner is shared).
8. **Stack and memory.** When the PR moves state into or out of statics or changes a hot loop: run [scripts/stack-check.sh](scripts/stack-check.sh) on base and branch builds; no listed frame may grow without a stated reason.
9. **Visuals.** For any on-screen change, capture screendumps (the proof's own, or a late extra capture — a first frame can be mid-transition) and compare against `main`. Any difference goes to the maintainer; send the images with `SendUserFile`.
10. **Hygiene.** No proof artifacts committed (screendumps, logs, `evidence/` folders); the PR body states what was not proven (for example, no QEMU path for a component).

## Exported copies

Never build in the maintainer's checkout or an agent's worktree. Export the exact commit:

```bash
git archive <sha> | tar -x -C "<scratch>/pr<N>"
cd "<scratch>/pr<N>" && CARGO_TARGET_DIR="<scratch>/pr-target" cargo test -p <package>
```

`scripts/qemu-runs.ps1 -Ref <sha> -Name pr<N>` exports and runs in one step. Proof scripts write logs and screendumps under the export's own `target\`.

## Known proof pitfalls

- `scripts/check.ps1` and `verify.ps1 -Proof` hide native-command failures; run cargo commands individually and check exit codes.
- `-LockScreenProof` needs a **fresh** `-DiskImage` path every run.
- Parallel agents collide on the default QMP port 4444; use a distinct `-QmpPort` per issue and for your own runs.
- `qemu-proof-1.log` is overwritten by every run; copy it before the next run (the runner script does).
- `skills/logos-*` describe a retired harness; ignore their commands.

## Opus handoff

Dispatch with `Agent`, `model: "opus"`, and a self-contained prompt — the subagent starts cold:

```
Review PR #<N> (branch <branch>, head <sha>) on Eresea/LogOS for issue #<I>.
Read: the issue body, the guidance issue #<G>, and `.claude/skills/orchestrate/REVIEW.md`.
Focus: <why this was escalated — the shared module / state machine / failing proof>.
Verify on an exported copy in <scratch>; do not touch any other checkout; do not post to GitHub.
Return: a findings list (each with evidence: file:line, a failing test or log excerpt),
what you verified and how, and a verdict: ready to merge / changes needed.
```

Post the returned findings as your review comment, keeping the evidence.
