# ADR-0041: Storage ownership and durable format boundary

- Status: Accepted
- Date: 2026-08-15

## Decision

LogOS separates storage mechanics from durable state. Core owns block-device
mechanics: device discovery, DMA, queues, interrupts, reset, timeouts, and
flush/barrier completion. A future storage service owns the on-disk format,
allocation, journaling, replay, recovery, and durability policy.

The first storage implementation is a host-testable `logos-storage` format
crate. It remains `no_std`, fixed-size, and independent of the scheduler,
UEFI boot path, and hardware drivers.

Future storage-service IPC uses the hostile-peer boundary from ADR-0040:
kernel-owned queues, private staging pages, and process-bound capabilities.
Storage queue frames are never mapped directly into a service address space.

Paths, namespaces, filesystem syscalls, encryption, snapshots, and device
drivers are separate later milestones. The storage format must never
auto-format nonblank corrupt media.

## Consequences

- The format can be crash-tested without booting QEMU or owning a device.
- Core does not gain path parsing, filesystem policy, or journal state.
- Hardware backends can be added behind a stable block boundary.
- Durable completion is explicit: a commit is durable only after its commit
  marker and required flush succeed.
- Storage-service restart, replay, and idempotency remain independently
  testable before application persistence is introduced.
