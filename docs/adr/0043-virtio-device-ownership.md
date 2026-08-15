# ADR-0043: VirtIO device ownership and DMA boundary

- Status: Accepted
- Date: 2026-08-15

## Decision

Core owns VirtIO PCI discovery, feature negotiation, queue memory, DMA
addresses, interrupts, notification, timeout, reset, and flush completion.
Storage owns logical blocks, format, journaling, replay, recovery, and
durability. Storage code never reads PCI configuration or device registers.

The device adapter uses a fixed page-aligned DMA arena reserved by Core. Queue
descriptors and data buffers may reference only frames from that arena. Storage
requests are copied through bounded private staging buffers; a service cannot
provide an arbitrary physical address or map the device queue.

The initial adapter requires the VirtIO 1.0+ feature bit and negotiates only
the block commands represented by the bounded storage seam. Writable media
requires negotiated flush support. Queue depth, descriptor count, outstanding
requests, and timeout state are fixed at compile time.

## Consequences

- PCI/MMIO and unsafe DMA code remain outside `logos-storage`.
- Device reset invalidates all outstanding request generations.
- Completion handling is interrupt-driven on hardware and deterministic in
  host tests.
- The hardware path programs one Core-owned MSI-X table entry and routes its
  queue vector to the fixed storage interrupt gate; bounded polling remains a
  fallback for test environments.
- Multiqueue, discard, write-zeroes, secure erase, zoned I/O, and arbitrary
  user-page DMA remain deferred.
