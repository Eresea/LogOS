# ADR-0078: Bounded VirtIO-GPU Cursor Plane

- Status: Accepted
- Date: 2026-08-29

## Decision

Core owns the optional VirtIO-GPU hardware cursor resource and cursor
commands. Display remains the owner of cursor appearance and publishes fixed
cursor position, visibility, press state, and sequence metadata through the existing
`FramebufferPresentState` page. Core owns the fixed 24x24 BGRA dot-and-halo
bitmap in bounded DMA storage and attaches it to the cursor resource. The
bitmap is regenerated from local framebuffer luminance when the cursor moves
or the composed frame changes.

This keeps the present page small and stable while avoiding a per-frame bitmap
copy. Core sends `UPDATE_CURSOR` when the cursor becomes visible, moves, changes
press state, or the composed frame changes. GPU failure clears the
hardware-active flag and Display resumes its bounded software cursor.

## Limits

- VirtIO-GPU 2D cursor support is the only hardware cursor backend.
- The cursor is fixed-size, centered-hotspot, and limited to the existing
  LockScreen/Atrium dot-and-halo visual.
- Cursor motion does not transfer or flush framebuffer damage after hardware
  mode is active.
- General GPU primitive rendering remains outside this decision.

## Ownership

Display owns scene interpretation, cursor position/visibility, and software
fallback pixels. Core owns the fixed hardware cursor bitmap, PCI/MMIO access,
the cursor resource, DMA backing, queue commands, timeouts, and fallback
activation.
