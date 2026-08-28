# ADR-0073: Bounded Pointer Input Path

- Status: Accepted
- Date: 2026-08-27

## Decision

Keep PS/2 pointer delivery inside the existing `Input` service. Core owns IRQ12,
the fixed pointer-byte ring, and a dedicated hardware pointer event. `Input`
owns bounded three-byte decoding and sends the existing `InputMessage` to
Atrium. Atrium owns hit testing, button capture, and conversion to
surface-local coordinates before routing `AtriumSurfaceInput`.

Keyboard and pointer wakeups use distinct directory flags and event sources;
Input waits on both in one fixed two-entry event set. Restart and failure
cleanup clears both events and the existing pointer mapping.

This decision supports only the existing signed 16-bit accumulated coordinates,
three-byte PS/2 packets, and current button states. Hover, wheel input, new
service IDs/images, new payload ABI, and unbounded state remain out of scope.
Cursor rendering is covered separately by ADR-0074.

## Consequences

- IRQ12 cannot be mistaken for a keyboard wakeup.
- Pointer delivery remains allocation-free in the interrupt path and bounded at
  every ring, event, and IPC boundary.
- Atrium remains the sole owner of pointer target and capture policy.
- QEMU proof can inject relative motion and left-button down/up through QMP.
