# ADR-0074: Bounded Native Cursor and LockScreen Pointer

- Status: Accepted
- Date: 2026-08-27

## Decision

Extend ADR-0073 with bounded native cursor surfaces at each GUI owner boundary.
Atrium keeps the signed 16-bit pointer position clamped to the fixed 640x400
GUI profile and maintains its Home cursor surface. While LockScreen is visible,
LockScreen owns a second cursor surface and updates it with the existing
bounded GuiDrawBatch adapter; Display remains the sole framebuffer writer and
rasterizes the cursor above retained GUI surfaces.

LockScreen consumes the existing semantic InputMessage pointer shape and
maps bounded pointer movement and left-button down events to fixed username,
password, confirmation, and submit rectangles. It retains only the hovered
target; wheel, theme, and unbounded pointer state remain out of scope.

The cursor surfaces are not Atrium hit-test targets. Surface generation, queue
capacity, and existing service restart cleanup remain authoritative; no service
ID, ELF image, ring, or payload ABI is added.

## Consequences

- A native pointer is visible when the GUI starts; the LockScreen surface is
  destroyed when LockScreen hides and Atrium's Home surface remains available.
- Cursor movement damages only the old and new fixed cursor rectangles.
- LockScreen can focus fields and submit through mouse input while keyboard
  behavior remains unchanged.
- Cursor rendering and lockscreen pointer proof remain dependent on the live
  Display/Atrium/LockScreen service graph; QEMU restart-loop failures must not
  be hidden.
