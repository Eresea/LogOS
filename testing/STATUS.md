# vNext Core test status

Status: active preemptive SMP Core milestone.

## Verified in this tree

- `cargo test --lib`: scheduler transitions, publication-before-claim ordering, wake-pending,
  event wait/signal races, timeout cleanup, stale generations, bounded capacity, completion/reuse,
  concurrent claims, ABI edge notifications, and the host-only service restart contract pass.
- `cargo fmt --check`, workspace tests, and host clippy with warnings denied pass.
- UEFI debug build and `qemu-proof` build pass for `x86_64-unknown-uefi`.
- QEMU proof reaches `LogOS vNext: QEMU proof PASS` with `-smp 1`, `-smp 2`, and `-smp 8`.
  The proof exercises the root-task handoff, repeated cancellable timer waits, two non-yielding
  CPU-bound tasks, GPR/flags/XMM preservation, per-CPU timer ticks, repeated preemption, a bounded
  Runtime timeout/completion/cancel lifecycle with generation-safe slot reuse, event-driven service
  waits and IPC backpressure notifications, keyboard wakeup, a real completed
  task reclaimed and replaced in the same scheduler slot with stale-handle rejection, the typed
  in-process Runtime command/response lifecycle and mailbox backpressure, and the typed
  in-process Health Ping command/response and restart/retry path, the loaded ring-3 service graph,
  framebuffer/keyboard mappings, semantic input, rendering, and supervisor restart, plus
  blocked/wake (cross-CPU for SMP runs), post-CR3 ring-3 execution on an AP, and bounded reschedule
  IPI delivery. Hostile-peer coverage rejects forged process access, wrong-direction capabilities,
  stale and oversized operations, disconnected queues, and legacy endpoint mappings.
- The fresh-disk storage proof reaches `Storage persistent-disk proof PASS` after proving durable
  command API writes, reboot reopen, aborted and removed files, and torn-journal recovery. Host tests
  cover the versioned API, malformed requests, transaction ownership, bounded namespace operations,
  command parsing, and committed/staged visibility.
- The local-APIC period is measured against the calibrated TSC on the BSP and each AP before its
  timer is enabled.
- Non-proof UEFI boot reaches `LogOS vNext: core ready` and remains in the scheduler for 1, 2, and
  8 CPUs.

## Deliberate limits

The proof does not claim AVX/XSAVE, affinity, priorities, dynamic stacks, allocators,
Runtime orchestration beyond the first bounded operation table, multi-client storage transactions,
permissions, directories, or networking. AP startup
is limited to healthy xAPIC IDs, eight CPUs, fixed low-memory trampoline staging, and current CR3.

`v1_docs/` and reviewed v1 status records are historical reference only.
