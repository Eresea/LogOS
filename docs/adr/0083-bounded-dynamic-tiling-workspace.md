# ADR-0083: Bounded dynamic tiling workspace

- Status: Accepted
- Date: 2026-09-12

## Context

Atrium currently admits application surfaces with fullscreen bounds. Its
surface registry already owns focus, pointer capture, close policy, and surface
updates, but it does not provide a useful multi-surface desktop layout.

## Decision

Atrium owns one volatile workspace represented by a fixed-capacity binary split
tree. A focused leaf can be split vertically or horizontally. Each split keeps
an adjustable fixed-point ratio; dragging its divider recomputes the bounds of
all descendant surfaces. Closing a leaf promotes its sibling and collapses the
empty split.

The tree capacity is a safety bound, not a fixed application layout. Surface
clients remain unaware of the tree and continue to receive only their own
generation-safe surface reference. Atrium sends the existing surface `Update`
operation to Display after layout changes; no new service or ABI is introduced.

The first implementation supports tiled surfaces, focus, divider dragging,
keyboard-selected split direction, and bounded minimum pane sizes. Floating
windows, resize handles on application chrome, persistence, and multiple
workspaces remain deferred.

## Consequences

- Existing Calculator, Files, Terminal, System, and program surfaces can share
  the workspace without hard-coded four-window geometry.
- Display remains the sole framebuffer writer and still receives validated
  bounds through the existing control path.
- The layout tree remains allocation-free and bounded for `no_std` service
  execution.
- Workspace state is discarded on logout, restart, and reboot with the rest of
  Atrium's volatile state.
