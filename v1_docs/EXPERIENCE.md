# Experience

> **Status:** Experience v1 planned

## Goal

Build a local graphical experience entirely on existing system, session, and runtime contracts.

## V1 scope

- [ ] Modern keyboard/pointer and USB HID support.
- [ ] Display modes, scanout ownership, shared surfaces, buffer lifecycle, damage, and presentation timing.
- [ ] A software-first compositor with surfaces, input/focus routing, isolation, and independent restart.
- [ ] A basic accessibility metadata contract.
- [ ] A minimal shell and graphical terminal using the existing Session protocol.

## Exit criteria

Multiple isolated applications render without arbitrary input or surface access. The compositor and shell restart independently, remote operation survives their failure, and all clients share underlying system contracts.

## V2 — Complete desktop experience

- Launcher, settings, notifications, clipboard, drag-and-drop, and workspaces.
- Accessibility services, capture permissions, application integration, and polished lifecycle UX.

## V3 — Advanced presentation

- Multiple displays, high-refresh scheduling, touch, richer text input, and remote visual streaming.
- GPU acceleration only when measurements justify it.

See Experience placement in [Architecture](architecture.md#rings).
