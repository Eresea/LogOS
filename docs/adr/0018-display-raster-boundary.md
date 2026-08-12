# ADR-0018: Display raster boundary

## Status

Accepted

## Decision

The Display component owns the transition from validated cell diffs to
framebuffer pixels. It validates the framebuffer geometry, rasterizes only
dirty cells through the fixed glyph cache, and supports the copied GOP RGB/BGR
formats. Terminal owns no pixel buffer and the kernel performs no drawing.

## Consequences

- Framebuffer writes are isolated to one service-owned component.
- Repeated renders with no cell changes perform no pixel work.
- Stride, dimensions, and buffer-size failures are explicit before writes.
