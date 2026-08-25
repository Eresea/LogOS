# ADR-0067: Event-driven graphical shell and compositor

- Status: Accepted
- Date: 2026-08-25

## Context

The Display service currently owns the only framebuffer mapping and rasterizes
terminal cell diffs. The v5 runtime now provides generation-safe service,
capability, endpoint, and event-set handles. A graphical shell must add
surfaces and input focus without introducing a periodic game loop, a second
framebuffer writer, or a second identity/session model.

## Decision

Display remains the sole framebuffer owner and becomes the compositor. It keeps
the legacy terminal-cell contract while accepting bounded GUI draw batches for
generation-safe surfaces. Shell owns surface policy, z-order, focus, session
state, and section lifecycle. LockScreen is the first independently isolated
section and receives only delegated surface and input capabilities.

Graphical work is invalidation-driven. Input, state changes, draw batches, and
explicit one-shot refresh requests wake the compositor through typed IPC and
event-set handles. There is no periodic idle render loop. Refresh requests are
coalesced and bounded by the compositor.

The first graphics vocabulary is limited to filled/stroked rectangles, lines,
clip rectangles, and fixed-glyph text runs. Clients never receive the
framebuffer or a pixel buffer. Surface and draw records use fixed capacities
and generation checks; malformed batches are rejected atomically.

The first graphical services are trusted built-ins. Future sections may be
activated lazily through the existing generation-safe service manager. Package
signatures, repository resolution, and automatic boot selection remain outside
this milestone.

Graphical login reuses the existing User session. Shell forwards the resulting
session, user, namespace capability, root, and rights to Flow through a typed
context handoff. Logout, restart, and stale generations clear that context.

## Consequences

- Display remains the only code that writes framebuffer pixels.
- Terminal rendering remains compatible while it is gradually migrated later.
- Surface lifecycle and focus are policy-owned by Shell rather than hidden in a
  generic event bus or compositor implementation.
- Idle systems do not consume render work; explicit refresh hooks remain
  available for future cursor or animation behavior.
- Adding Shell and LockScreen increases the trusted built-in image set and its
  bounded boot resource cost.
