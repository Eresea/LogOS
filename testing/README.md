# vNext Core testing

The active milestone is the fixed-stack preemptive SMP Core. Host tests exercise only the
allocator-free scheduler state machine; QEMU is required for the assembly boundary.

## Host

`cargo test --lib` covers bounded capacity, generation-safe stale handles, runnable/running/
blocked/completed transitions, wake-pending races, completion reuse, context-publication ordering,
and simultaneous CPU claims. `cargo clippy --lib -- -D warnings` is required.

## QEMU proof

Run independent bounded proofs for `-smp 1`, `-smp 2`, and `-smp 8`:

```text
.\scripts\run.ps1 -Proof -Cpus 1 -TimeoutSeconds 60
.\scripts\run.ps1 -Proof -Cpus 2 -TimeoutSeconds 60
.\scripts\run.ps1 -Proof -Cpus 8 -TimeoutSeconds 60
```

Each proof requires UEFI entry, the Core-ready marker, a root-task handoff through the normal
scheduler path, repeated cancellable timer waits by the root task, and two non-yielding assembly
tasks with preserved GPR/flags/XMM canaries and sustained progress, timer ticks on every online
CPU, repeated preemptive switches, and a blocked task woken by another CPU for SMP (same CPU for
`-smp 1`). The runner captures debugcon output and fails on timeout or fatal output.

`v1_docs/` and old v1 evidence are historical; they are not active proof criteria.
