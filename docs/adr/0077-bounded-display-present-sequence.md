# ADR-0077: Bounded Display Present Sequence

- Status: Accepted
- Date: 2026-08-28

## Decision

Core allocates one page containing an atomic present sequence, a full-frame
flag, and at most eight damage rectangles. It maps the page writable only into
Display at `DISPLAY_PRESENT_BASE`. Display writes the bounded metadata and
increments the sequence after it copies a completed frame or damage tile into
the mapped GOP framebuffer. Core reads the sequence with acquire ordering and
submits either one full transfer or the validated changed rectangles only when
the value changed.

The page carries no pixels, handles, or device authority. A missing sequence
page keeps the existing full-transfer fallback.

## Consequences

- Idle supervision no longer submits repeated full-frame GPU transfers.
- Small updates no longer require a full-frame GPU transfer; overflowing or
  malformed damage falls back to one full transfer.
- Display remains the sole framebuffer writer and owns present timing.
- The sequence wraps naturally; equality is the only suppression decision, so
  a wrapped value still causes a transfer when the producer publishes again.
- The bounded software renderer and no-GPU path remain unchanged.

## Rejected

- Mapping PCI, MMIO, queue, or GPU command authority into Display.
- Letting Core infer damage by scanning framebuffer pixels.
- Using an unbounded event or allocation-backed present queue.
