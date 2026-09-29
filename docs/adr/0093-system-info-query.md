# ADR-0093: System-info query (CPU count and memory)

- Status: Accepted
- Date: 2026-09-29

## Decision

Settings' About page (S3b, #95) shows the online CPU count and physical memory total and used.
Services cannot see either (`CPU_COUNT` and the frame accounting are kernel-internal), so the
kernel gains one bounded, read-only query, mirroring `WALL_TIME_SYSCALL` (ADR-0086):

- **Syscall.** `SYSTEM_INFO_SYSCALL` (22), no arguments and no user pointer. The answer comes back
  packed in `rax`: `cpus:8 | total_mib:28 | used_mib:28` (`logos_abi::SystemInfo::pack`/`unpack`,
  host-tested; memory fields clamp at 2^28 - 1 MiB). No per-process or per-CPU detail.
- **Capability decision: open to every service.** Like `CURRENT_TICKS_SYSCALL` and
  `WALL_TIME_SYSCALL`, it only requires a live user launch (the same check the other bounded
  queries use). The values are three coarse aggregates of the machine, there is no
  handle to protect, and a capability would need a new grant path and manager plumbing for no gain.
  A later query that returns anything per-process should be capability-gated.
- **Kernel side.** CPU count is `arch::CPU_COUNT`. Memory is `memory::frame_totals`: capacity and
  free frames of the kernel frame allocator, converted to MiB. It takes the kernel heap lock with
  `try_lock` only (never blocks in the syscall path) and, if the lock is busy, returns the previous
  snapshot; the lock is not held across anything else. Frames cached per CPU count as used, so
  "used" can overstate by a small bounded amount.
- **ABI.** `ABI_VERSION` 12 -> 13.
- **Atrium.** `build_settings_scene` takes a `SystemInfo`; the About page renders it as
  `4 CPUs - 512 MiB, 38 MiB used`. The row reuses the idle select-chevron label node, so
  `MAX_GUI_NODES` stays 48. Under `qemu-proof` the service logs
  `Atrium about cpus=N mem_total=... mem_used=...` once, which the system proof compares to `-Cpus`.

Static totals and a one-shot used snapshot only: no live graphs, per-process breakdown or CPU load.
