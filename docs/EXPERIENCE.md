# Experience

> **Status:** Experience v1 planned

## Goal

Build a local graphical experience entirely on existing system, session, and runtime contracts.

## V1 scope

- [ ] Modern keyboard/pointer and USB HID support.
- [ ] Display modes, scanout ownership, shared surfaces, buffer lifecycle, damage, and presentation timing.
- [ ] A software-first compositor with surfaces, window/layer policy, input/focus, clipboard/drag-and-drop, accessibility, capture permissions, remote streaming, and independent restart.
- [ ] A graphical shell with login/unlock, launcher, notifications, settings, workspaces, health/remote management, and a terminal using the existing session protocol.

## Exit criteria

Multiple isolated applications render without arbitrary input or surface access. The compositor and shell restart independently, remote operation survives their failure, and all clients share underlying system contracts.

GPU acceleration is deferred until the software compositor proves it necessary.

See Experience placement in [Architecture](architecture.md#ring-5--experience).
