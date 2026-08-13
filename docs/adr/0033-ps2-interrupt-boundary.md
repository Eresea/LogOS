# ADR-0033: PS/2 interrupt boundary

## Status

Accepted

## Decision

The kernel owns only the PS/2 transport boundary. It remaps the legacy PIC,
keeps all legacy IRQs masked until the Input service ring is published, then
unmasks IRQ1 and copies each byte from port `0x60` into the fixed
`KeyboardByteRing`. The Input service owns Set-2 decoding and semantic message
construction.

## Consequences

- Keyboard bytes cannot enter the service graph before the Input mapping exists.
- A full ring is an explicit bounded outcome and increments a kernel drop
  counter; it does not block the interrupt path.
- The kernel does not own layout, modifiers, text composition, or key meaning.
