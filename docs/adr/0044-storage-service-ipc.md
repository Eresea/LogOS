# ADR-0044: Storage service IPC boundary

- Status: Accepted
- Date: 2026-08-15

## Decision

Storage service requests use fixed `repr(C)` messages from `logos-abi` and the
existing kernel-owned `IpcSend`/`IpcReceive` validation path. Each operation
uses one process-bound capability, one private writable staging page for the
request or returned block, and one bounded response. The service never receives
PCI configuration, MMIO, interrupt, queue, or arbitrary DMA addresses.

The initial contract supports block read, block write, flush, format,
reopen/recover, and bounded transaction lifecycle commands. Every request
contains a caller request ID, service epoch, capability generation, logical
block range, and operation-specific length. Responses always carry the same
request ID and a typed status; stale generations, wrong epochs, oversized
ranges, and unauthorized capabilities are rejected before storage code runs.

The storage service owns the format and journal state machine. Core owns the
block transport and completes device work behind the IPC boundary. A
host-tested adapter proves the same request validation and staging ownership
without requiring a running service image.

## Consequences

- No storage service can program a device or retain a physical DMA address.
- Block data crosses the boundary through fixed private staging storage.
- The existing six trusted terminal queues remain unchanged.
- Service startup and a seventh boot image require a separate lifecycle proof.
