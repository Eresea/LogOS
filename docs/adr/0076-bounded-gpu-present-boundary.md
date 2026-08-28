# ADR-0076: Bounded GPU Present Boundary

- Status: Accepted
- Date: 2026-08-28

## Decision

Display remains the sole owner of retained scenes, damage policy, and the
software fallback. An optional GPU path is split at the existing Core/Display
boundary:

- Core owns PCI discovery, feature negotiation, device MMIO, fixed DMA memory,
  command queues, interrupts, reset, and bounded fence timeouts.
- Display submits only validated fixed-size frame resources and dirty-rectangle
  updates through a typed backend. It never receives PCI, MMIO, queue, or raw
  physical-address authority.
- The first hardware slice is GPU-backed resource upload and scanout of dirty
  regions, with a bounded software fallback when the device is absent, busy, or
  reset. Existing retained scene producers and the GUI scene protocol remain
  unchanged.
- VirtIO-GPU 2D resource transfer is treated as presentation acceleration, not
  arbitrary primitive rasterization. Primitive execution requires a separate
  backend capability and proof; it is not inferred from scanout support.

## Consequences

- GPU failures can fall back to the current RAM backbuffer without changing
  producer behavior or losing scene ownership.
- Device-facing memory and queue lifetimes remain Core-owned and fixed-bound.
- The first GPU proof can measure reduced GOP copy work independently from
  primitive-raster correctness.
- A later primitive backend must add an explicit capability, bounded command
  encoding, and device-specific proof rather than widening the Display ABI.

## Rejected

- Direct GPU device access from the Display service.
- Replacing retained scenes with producer-side GPU command streams.
- Treating a VirtIO-GPU scanout/upload device as a general 2D primitive engine.
