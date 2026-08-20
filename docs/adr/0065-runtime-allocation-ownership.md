# ADR-0065: Runtime allocation ownership

- Status: Accepted
- Date: 2026-08-20

## Decision

The vNext runtime uses one production physical allocator initialized from
UEFI-discovered memory. Before `ExitBootServices`, Core reserves metadata pages
large enough for the discovered frame set and excludes them from the usable map.

Core and every ring-3 service use `GlobalAlloc`. Core allocations are charged to
the kernel owner. Services use shared read-only allocator code with private heap
metadata and private mapped pages. Service heap growth is kernel-mediated and
quota-controlled; allocator metadata is never writable across service roots.

Task-context allocation may wait for bounded reclaim. Interrupt paths must use
explicit nonblocking allocation APIs and may not call `GlobalAlloc`.

## Consequences

- `FramePool` and `SmpFrameAllocator` converge on one production ownership path.
- Physical metadata capacity follows the boot memory map instead of a fixed frame count.
- Allocation failure remains observable through quota, exhaustion, reclaim, and fatal outcomes.
- A general allocator is now an explicit runtime dependency and must be proven before
  any service can depend on unbounded collection growth.
