# ADR-0079: Asynchronous GPU Frame Submission

- Status: Accepted
- Date: 2026-08-29

## Decision

Core owns two fixed VirtIO-GPU scanout resources and their page-aligned DMA
backing. Display remains the owner of retained scene composition and publishes
completed frame sequence/damage metadata through `FramebufferPresentState`.

When a new sequence arrives, Core copies the completed full frame or dirty
rectangles from the Display mapping into a free GPU frame slot, submits bounded
`TRANSFER_TO_HOST_2D`, `RESOURCE_FLUSH`, and `SET_SCANOUT` commands, and polls
the existing fence queue on later supervision passes. The active slot remains
read-only to the producer until the replacement slot is presented; a full
queue reports busy and preserves the software path.

## Consequences

- Display can continue rendering into its RAM backbuffer while a prior GPU
  present is in flight, without modifying a DMA source still being read.
- Present work is bounded by published damage and fixed queue/slot limits; no
  blocking completion spin is used in the steady-state frame path.
- VirtIO-GPU 2D remains an upload/scanout accelerator, not a primitive or
  compute engine. GPU raster, textures, shaders, and compute require a later
  explicit 3D/virgl capability and command ABI.

## Rejected

- Reusing the Display framebuffer as an in-flight GPU source.
- Allocator-backed or unbounded frame queues.
- Exposing GPU resources, PCI/MMIO, or command encoding to Display.
