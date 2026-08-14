# ADR-0042: Bounded VirtIO block transport model

- Status: Accepted
- Date: 2026-08-15

## Decision

The first hardware-facing storage adapter targets the VirtIO block request
model, while keeping PCI/MMIO discovery and DMA mapping outside the format
crate. A request is represented as a fixed descriptor chain containing a
VirtIO block header, an optional data segment, and a one-byte device status.

The storage boundary exposes 4096-byte logical blocks. VirtIO sectors are
translated in 512-byte units, so one LogOS block maps to eight VirtIO sectors.
Read, write, and flush are the only initial commands. Queue depth and request
size remain fixed and bounded; multiqueue, discard, write-zeroes, secure erase,
and zoned commands are deferred until a real device adapter requires them.

The host-tested model validates request-chain shape and completion status but
does not claim hardware correctness. Actual PCI discovery, feature negotiation,
DMA addresses, interrupts, and queue notification belong to the future Core
adapter.

## Consequences

- VirtIO-specific wire rules are tested before unsafe MMIO code is added.
- The storage format remains independent of PCI and device memory addresses.
- Device completions can arrive out of order because request IDs remain
  generation-safe at the queue boundary.
- Unsupported device status is preserved as a typed completion outcome.
