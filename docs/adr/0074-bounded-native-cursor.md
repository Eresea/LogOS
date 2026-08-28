# ADR-0074: Bounded Native Cursor and LockScreen Pointer

- Status: Accepted
- Date: 2026-08-27

## Decision

Extend ADR-0073 with one Atrium-owned full-screen cursor surface. Atrium keeps
the signed 16-bit pointer position clamped to the fixed 640x400 GUI profile
and submits one existing bounded GuiSceneOp per update; Display remains the
sole framebuffer writer and rasterizes the cursor above retained GUI surfaces.

LockScreen consumes the existing semantic InputMessage pointer shape and
maps left-button down events to fixed username, password, confirmation, and
submit rectangles. It retains no hover, wheel, theme, or unbounded pointer
state.

The cursor surface is not an Atrium hit-test target. Surface generation,
queue capacity, and existing service restart cleanup remain authoritative;
no service ID, ELF image, ring, or payload ABI is added.

## Consequences

- A native pointer is visible when the GUI starts and remains visible across
  lockscreen/home transitions.
- Cursor movement damages only the old and new fixed cursor rectangles.
- LockScreen can focus fields and submit through mouse input while keyboard
  behavior remains unchanged.
- Cursor rendering and lockscreen pointer proof remain dependent on the live
  Display/Atrium service graph; QEMU restart-loop failures must not be hidden.
