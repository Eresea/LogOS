# ADR-0065: Runtime allocation ownership

- Status: Accepted
- Date: 2026-08-20

## Decision

The vNext runtime uses one production physical allocator initialized from
UEFI-discovered memory. Before `ExitBootServices`, Core reserves metadata pages
large enough for the discovered frame set and excludes them from the usable map.

Core binds one process-wide `GlobalAlloc` adapter after the frame pool and
kernel heap are ready. Core allocations use the explicit `KernelHeap` boundary
and are charged to the kernel owner; warning and critical reclaim are retried
once, then unrecoverable UEFI exhaustion enters the fatal path. Services keep
private allocator state and private mapped pages. Service heap growth is
kernel-mediated and quota-controlled; allocator metadata is never writable
across service roots.

Task-context allocation may wait for bounded reclaim. Interrupt paths must use
explicit nonblocking allocation APIs and may not call `GlobalAlloc`.

## Consequences

- `FramePool` and `SmpFrameAllocator` converge on one production ownership path.
- Physical metadata capacity follows the boot memory map instead of a fixed frame count.
- Allocation failure remains observable through quota, exhaustion, reclaim, and fatal outcomes.
- Core collection storage now has an explicit heap adapter and fatal OOM boundary; dynamic
  runtime collections still require ownership and reclaim registration before use in new paths.
