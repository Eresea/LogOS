# ADR-0065: Runtime allocation ownership

- Status: Accepted
- Date: 2026-08-20

## Decision

The vNext runtime uses one production physical allocator initialized from
UEFI-discovered memory. Before `ExitBootServices`, Core reserves metadata pages
large enough for the discovered frame set and excludes them from the usable map.

Core does not install a process-wide `GlobalAlloc`. Core allocations use the
explicit `KernelHeap` handle boundary and are charged to the kernel owner.
The heap returns generation-safe ownership records and identity-mapped frame
addresses; turning those addresses into typed runtime storage requires an
explicit mapping contract in a later slice. Services keep private allocator
state and private mapped pages. Service heap growth is kernel-mediated and
quota-controlled; allocator metadata is never writable across service roots.

Task-context allocation may wait for bounded reclaim. Interrupt paths must use
explicit nonblocking allocation APIs and may not call `GlobalAlloc`.

## Consequences

- `FramePool` and `SmpFrameAllocator` converge on one production ownership path.
- Physical metadata capacity follows the boot memory map instead of a fixed frame count.
- Allocation failure remains observable through quota, exhaustion, reclaim, and fatal outcomes.
- A general allocator is not silently introduced into Core; dynamic runtime collections
  require a separately proven mapped-storage adapter before live services depend on them.
