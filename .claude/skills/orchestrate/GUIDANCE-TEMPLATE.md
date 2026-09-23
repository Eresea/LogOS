# Guidance issue template

One pinned issue per initiative; every issue in the initiative links to it. Agents read it before starting, so it carries the exact commands and the rules — issue bodies stay short. Fill the placeholders, drop sections that do not apply.

```markdown
Shared proof commands and rules for <initiative> issues #<first>–#<last>.

## Harness
Use `docs/development.md` and the commands below. Run each command separately and stop on a non-zero `$LASTEXITCODE`; `scripts/check.ps1` does not stop on native failures. Ignore `skills/logos-*`: they describe a retired harness.

## BUILD
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --target x86_64-unknown-uefi
.\scripts\build-services.ps1 -Release
cargo clippy --target x86_64-unknown-none -p logos-service-images --bins -- -D warnings

## QEMU
- <proof name>: `.\scripts\run.ps1 -Proof <flags> -Cpus 1 -QmpPort 44<NN> -DiskImage target\<name>-<issue>-<run>.raw`
- `-LockScreenProof` needs a fresh disk path every run. NN = your issue number, so parallel agents never share a QMP port.
- If a proof fails, rerun the same command on your base commit before concluding your change caused it; report both.

## Evidence markers
<marker names the proofs must show; rejection-marker format and how to filter it per surface>

## Rules
- Start an issue only when every blocker in its "Depends on" section is merged, unless that section says it may start early. Branch from the current `main` tip in an isolated worktree.
- Each slice is proven and merged on its own. Issues that edit the same file run one after another: <chain>.
- Allowed dependency changes: <list>. No others.
- Visual checks are the maintainer's: attach screendumps to a PR comment (never commit them); do not mark the check complete yourself.
- Never commit proof artifacts (screendumps, logs, `evidence/` folders).
- State what your proof does not cover (for example, no QEMU path exists for X).
```
