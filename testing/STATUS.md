# vNext Core test status

Status: active preemptive SMP Core milestone.

## Verified in this tree

- `cargo test --lib`: scheduler transitions, publication-before-claim ordering, wake-pending,
  stale generations, bounded capacity, completion/reuse, and concurrent claims pass.
- `cargo fmt --check` and host clippy with warnings denied pass.
- UEFI debug build and `qemu-proof` build pass for `x86_64-unknown-uefi`.
- QEMU proof reaches `LogOS vNext: QEMU proof PASS` with `-smp 1`, `-smp 2`, and `-smp 8`.
  The proof exercises the root-task handoff, repeated cancellable timer waits, two non-yielding
  CPU-bound tasks, GPR/flags/XMM preservation, per-CPU timer ticks, repeated preemption, and
  blocked/wake (cross-CPU for SMP runs).
- The local-APIC period is measured against the calibrated TSC on the BSP and each AP before its
  timer is enabled.
- Non-proof UEFI boot reaches `LogOS vNext: core ready` and remains in the scheduler for 1, 2, and
  8 CPUs.

## Deliberate limits

The proof does not claim user mode, AVX/XSAVE, affinity, priorities, wake IPIs, dynamic stacks,
allocators, Runtime orchestration, IPC, capabilities, terminal, storage, or networking. AP startup
is limited to healthy xAPIC IDs, eight CPUs, fixed low-memory trampoline staging, and current CR3.

`v1_docs/` and reviewed v1 status records are historical reference only.
