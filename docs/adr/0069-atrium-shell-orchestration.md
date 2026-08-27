# ADR-0069: Atrium GUI shell orchestration

- Status: Accepted
- Date: 2026-08-26

## Context

The graphical shell needs a stable root for boot, lock, home, and application
surface routing. Shell currently combines authentication flow with the first
lock-screen renderer, while Display already owns the retained surface registry
and framebuffer. Adding window policy to Display or duplicating authentication
in a new service would violate those ownership boundaries.

## Decision

Atrium is a separate trusted built-in service and the GUI orchestration root.
It owns the bounded Boot/Locked/Home state machine, one volatile workspace,
launcher selection, application surface slots, focus, movement, close policy,
and delegated surface lifecycle. Atrium is authoritative for surface creation,
retained focus order, and topmost hit testing: a point resolves to at most one
live surface before interaction is routed. Atrium is also authoritative for
surface retention: an application or process requests a surface with its
generation-safe client identity and receives only the resulting generation-safe
surface reference; it cannot create, retain, or render an unregistered surface.
Atrium receives semantic keyboard input and never receives framebuffer access.

The implementation separates request from admission. Atrium validates the
current phase and capacity, returns a one-use bounded surface request containing
the app's placement policy, and admits the surface only after Display returns a
valid reference. Reserved root/lock surfaces and duplicate references are
rejected before the record is retained.

The first process-facing contract is the fixed Terminal↔Atrium surface request
and response path. Atrium may defer a valid request until Home and Display are
ready, then returns the admitted reference or queues a revoke event when the
surface is closed or the session ends. The request carries the caller's
generation-safe built-in service identity, and Atrium retains it with the surface
record so a restarted client cannot inherit the old surface. Terminal now holds the admitted reference
and stamps it onto its bounded cell updates. Those render messages cross the
Terminal→Atrium route; Atrium validates the exact live reference and forwards
them over the Atrium→Display route. Atrium also wraps Terminal input in the same
reference, so the client cannot consume input for another surface. Display maps
the validated updates into the Atrium-marked Terminal surface; future pointer
events and richer Terminal rendering can reuse the envelope without changing
ownership.

Shell remains the authentication/session broker. LockScreen owns bounded
credential editing/rendering and sends typed authentication requests to Shell.
Shell continues to own User requests and Flow session-context handoff.
Authentication success, logout, restart, and stale generations clear Atrium's
volatile state.

Display remains the sole framebuffer writer. Atrium uses generation-safe typed
surface controls and bounded draw batches. The first app set is a functional
four-function calculator, the existing terminal, and a Files placeholder.
Pointer input, resizing, persistence, multiple workspaces, and package app
discovery remain outside this decision. A future system-management process may
inspect process and device state through a separate typed service contract; it
does not own surface lifecycle.

## Consequences

- The trusted built-in image set and dynamic IPC topology gain one service.
- Atrium state is discarded on logout, restart, and reboot; Storage is not involved.
- A surface reference is valid only while its Atrium-owned record and backing
  Display surface remain live; teardown is initiated by Atrium.
- Shell and LockScreen can restart independently without granting GUI services
  framebuffer or filesystem authority.
- Existing terminal cell rendering remains supported as a bounded compatibility
  path inside the Atrium-owned Terminal surface.
