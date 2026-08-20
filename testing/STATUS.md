# vNext Core test status

Status: active preemptive SMP Core milestone.

## Verified in this tree

- `cargo test --lib`: scheduler transitions, publication-before-claim ordering, wake-pending,
  event wait/signal races, timeout cleanup, stale generations, bounded capacity, completion/reuse,
  concurrent claims, ABI edge notifications, and the host-only service restart contract pass.
- `cargo fmt --check`, workspace tests, and host clippy with warnings denied pass.
- UEFI debug build and `qemu-proof` build pass for `x86_64-unknown-uefi`.
- QEMU proof reaches `LogOS vNext: QEMU proof PASS` with debug `-smp 1` and `-smp 8`, and
  release `-smp 2`. Debug `-smp 2` remains blocked: repeated runs exit after `boot resources
  ready`, before `service address spaces ready`. A bounded QEMU `-d
  int,cpu_reset,guest_errors` trace from the baseline handoff shows the current GS points to BSP
  CPU 0 while the firmware GDT/IDT and firmware stack are still active; the BSP then takes a
  protection fault (`vector 0e`, error `0003`) followed by a double fault and triple fault. CPU 1
  reset records are firmware MP state, not evidence that the AP scheduler is executing. Early AP
  table setup and parked-AP experiments did not pass and were reverted. Moving BSP tables before
  runtime startup did not clear the service-start page fault. Pivoting to the existing scheduler
  stack reached runtime but produced an invalid-instruction loop in Rust startup and was also
  reverted. The remaining issue is an unresolved UEFI runtime-startup stack/handoff fault, not a
  committed feature change.
  The proof exercises the root-task handoff, repeated cancellable timer waits, two non-yielding
  CPU-bound tasks, GPR/flags/XMM preservation, per-CPU timer ticks, repeated preemption, a bounded
  Runtime timeout/completion/cancel lifecycle with generation-safe slot reuse, event-driven service
  waits and IPC backpressure notifications, keyboard wakeup, a real completed
  task reclaimed and replaced in the same scheduler slot with stale-handle rejection, the typed
  in-process Runtime command/response lifecycle and mailbox backpressure, and the typed
  in-process Health Ping command/response and restart/retry path, the loaded ring-3 service graph,
  framebuffer/keyboard mappings, semantic input, rendering, and supervisor restart, plus
  blocked/wake (cross-CPU for SMP runs), post-CR3 ring-3 execution on an AP, and bounded reschedule
  IPI delivery. The service-manager boundary proof confirms Commands-only manager capability
  mapping, list/status, unauthorized caller rejection, and manager-driven restart completion.
  Hostile-peer coverage rejects forged process access, wrong-direction capabilities, stale and
  oversized operations, disconnected queues, and legacy endpoint mappings.
- The fresh-disk v5 storage proof reaches `Storage persistent-disk proof PASS` after proving the
  live Storage image formats the v5 root, materializes the `LOGUSR01` User catalog inside the
  reserved system pool, reopens it after reboot, and recovers from a torn inactive root. Host
  tests cover the versioned API, malformed requests, transaction ownership, bounded namespace
  operations, command parsing, committed/staged visibility, and prepared checkpoint recovery.
- Network v2 host coverage passes for bounded configuration parsing and fail-closed Disabled mode,
  inline payload validation, multi-client request routing, smoltcp packet-device copying, IPv4 checksums, static-to-DHCP fallback,
  fixed socket/listener capacities, listener accept pairing, stale socket generations, modern VirtIO
  capability parsing, and the bounded VirtIO queue model.
- Storage v2 staged writes and the host-tested Fetch protocol pass bounded creation, replacement,
  abort, chunk ordering, HTTP framing, split responses, and body limits. The Fetch image is wired
  into the fixed service graph and terminal/session path; the dedicated real-peer Fetch persistence
  proof is wired but currently blocked in the QMP harness: keyboard interrupts arrive, while the
  long command is not yet reaching the guest parser reliably.
- The local-APIC period is measured against the calibrated TSC on the BSP and each AP before its
  timer is enabled.
- Non-proof UEFI boot reaches `LogOS vNext: core ready` and remains in the scheduler for 1, 2, and
  8 CPUs.

## Deliberate limits

The proof does not claim AVX/XSAVE, affinity, priorities, dynamic stacks, allocators,
Runtime orchestration beyond the first bounded operation table, multi-client storage transactions,
permissions, directories, or real-peer Network packet/TCP behavior. AP startup
is limited to healthy xAPIC IDs, eight CPUs, fixed low-memory trampoline staging, and current CR3.

The enabled-profile Network and dedicated Fetch QEMU proofs are not yet PR gates; the Network
  listener path and Fetch terminal-input path still need a stable real-peer proof. DHCP fallback
  QEMU proof is also deferred post-merge. No massive-traffic claim is made; smoltcp 0.12 TCP
  congestion-control/high-throughput work remains outside this milestone.

- Storage v3 package host coverage passes variable-sized extent allocation/reuse, atomic replacement,
  abort, incomplete-write non-publication after reopen, stale handles, ordinary-file limits, and v1/v2
  package incompatibility. Package-format tests cover header, kind, service, ABI, size, truncation, and
  CRC failures. Reader-based ELF tests cover cross-page streaming, zeroing, read failure, exact service
  ownership, exhaustion, and reclamation. The dedicated `scripts/package-proof.ps1` gate seeds a real
  service ELF, activates it through Core↔Storage, rejects a corrupt package without disturbing the graph,
  and reopens the package after reboot under a ten-second per-boot non-network timeout.

`v1_docs/` and reviewed v1 status records are historical reference only.
