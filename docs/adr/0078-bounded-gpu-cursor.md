# ADR-0078: Bounded VirtIO-GPU Cursor Plane

- Status: Accepted
- Date: 2026-08-29

## Decision

Core owns the optional VirtIO-GPU hardware cursor resource and cursor
commands. Display remains the owner of cursor appearance and publishes fixed
cursor position, visibility, and sequence metadata through the existing
`FramebufferPresentState` page. Core owns the fixed 24x24 BGRA arrow bitmap in
bounded DMA storage and attaches it to the cursor resource.

This keeps the present page small and stable while avoiding a per-frame bitmap
copy. Core sends `UPDATE_CURSOR` only when the cursor becomes visible or hidden
and sends `MOVE_CURSOR` for position changes. GPU failure clears the
hardware-active flag and Display resumes its bounded software cursor.

## Limits

- VirtIO-GPU 2D cursor support is the only hardware cursor backend.
- The cursor is fixed-size, single-hotspot, and limited to the existing
  LockScreen/Atrium arrow.
- Cursor motion does not transfer or flush framebuffer damage after hardware
  mode is active.
- General GPU primitive rendering remains outside this decision.

## Ownership

Display owns scene interpretation, cursor position/visibility, and software
fallback pixels. Core owns the fixed hardware cursor bitmap, PCI/MMIO access,
the cursor resource, DMA backing, queue commands, timeouts, and fallback
activation.
