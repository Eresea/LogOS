# ADR-0081: Bounded 1280×800 UI profile and framework command menu

- Status: Accepted
- Date: 2026-09-01

## Context

The initial graphical shell used a fixed 640×400 coordinate profile. Its
command menu was hand-painted in service code, and glyph runs did not carry a
display scale or reliable vertical alignment. Surface fills could also be
treated as unbounded occluders when a command used the surface sentinel.

## Decision

The default UI profile is 1280×800. ABI constants define the profile and GOP
admission requires a mode at least that large; Atrium, LockScreen, Input, and
Display use the same bounds and centered pointer origin. The fixed framebuffer
cap remains authoritative.

The command menu uses the portable `UiCommandMenu` behavior component and a
bounded `UiComponentTree` rendered through `logos-ui-graphics`. `Text4xl`
maps to a fixed 2× glyph raster, `text-muted` selects a theme color, and glyph
placement is vertically centered within its node. Display clips occluders to
the owning surface before subtracting them from lower layers.

## Consequences

- The shell has a larger, consistent coordinate space without changing IPC
  payload sizes or adding allocation to the UI framework.
- Menu selection and submission are typed, bounded, and host-testable.
- Surface-local full fills cannot hide pixels outside their surface bounds.
- Existing 8×16 terminal cells remain unchanged; richer font and animation
  systems remain deferred.
