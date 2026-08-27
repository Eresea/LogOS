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
launcher selection, application window slots, focus, movement, close policy,
and delegated surface lifecycle. Atrium receives semantic keyboard input and
never receives framebuffer access.

Shell remains the authentication/session broker. LockScreen owns bounded
credential editing/rendering and sends typed authentication requests to Shell.
Shell continues to own User requests and Flow session-context handoff.
Authentication success, logout, restart, and stale generations clear Atrium's
volatile state.

Display remains the sole framebuffer writer. Atrium uses generation-safe typed
surface controls and bounded draw batches. The first app set is a functional
four-function calculator, the existing terminal, and a Files placeholder.
Pointer input, resizing, persistence, multiple workspaces, and package app
discovery remain outside this decision.

## Consequences

- The trusted built-in image set and dynamic IPC topology gain one service.
- Atrium state is discarded on logout, restart, and reboot; Storage is not involved.
- Shell and LockScreen can restart independently without granting GUI services
  framebuffer or filesystem authority.
- Existing terminal cell rendering remains supported while Atrium adds bounded
  GUI surface control and keyboard-first window management.
