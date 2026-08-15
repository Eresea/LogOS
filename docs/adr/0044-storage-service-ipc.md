# ADR-0044: Storage service IPC boundary

- Status: Accepted
- Date: 2026-08-15

## Decision

Storage service requests use fixed `repr(C)` messages from `logos-abi` and the
existing kernel-owned `IpcSend`/`IpcReceive` validation path. Each operation
uses one process-bound capability, one private writable staging page for the
request or returned block, and one bounded response. The service never receives
PCI configuration, MMIO, interrupt, queue, or arbitrary DMA addresses.

The ABI reserves commands for block read, block write, flush, format,
reopen/recover, and bounded transaction lifecycle operations. The current
mailbox activates read, write, flush, and reopen/recover; format and
transaction commands return `Unsupported` until their service-owned state
machine is exposed. Every request
contains a caller request ID, service epoch, capability generation, logical
block range, and operation-specific length. Responses always carry the same
request ID and a typed status; stale generations, wrong epochs, oversized
ranges, and unauthorized capabilities are rejected before storage code runs.

The storage service owns the format and journal state machine. Core owns the
block transport and completes device work behind the IPC boundary. The sixth
service now uses dedicated `StorageToCore` and `CoreToStorage` capabilities;
the kernel validates the request and response identity through the same
private staging path. Read, write, reopen, and flush requests now reach the
bounded VirtIO adapter; format, transaction orchestration, and namespace
policy remain service-owned.

## Consequences

- No storage service can program a device or retain a physical DMA address.
- Block data crosses the boundary through fixed private staging storage.
- The existing six trusted terminal queues remain unchanged.
- Storage IPC is process-bound without exposing device state or DMA addresses.
