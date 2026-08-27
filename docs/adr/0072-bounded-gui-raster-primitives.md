# ADR-0072: Bounded GUI Raster Primitives

- Status: Accepted
- Date: 2026-08-27

## Decision

Extend the fixed graphics ABI with rounded fills, rounded strokes, thick lines,
and shadows. The `GuiDrawCommand` wire footprint and three-command packet limit
remain unchanged; retained batches may use five bounded fragments.

Display remains the sole framebuffer writer. It rasterizes rounded geometry with
fixed four-sample coverage and shadows with fixed weighted ring kernels (blur
radius 0–4), without a heap or scratch framebuffer. Corner radius is capped at 32 px,
stroke and line width at 8 px, and shadow offsets at ±32 px. Colors retain the
legacy `0xRRGGBB` opaque form and may use `0xAARRGGBB` for explicit alpha.
GUI damage is composed in fixed 64-row passes so a large redraw yields back to
the service loop and continues heartbeats. Rounded fills use scanline spans and
strokes only rasterize their border bands; plotting does not rescan damage rects
that were already clipped by the caller.

Shell and LockScreen use the primitives directly. General curves, paths, CSS
cascades, and theme files remain deferred until a bounded authoring contract is
proven.

## Consequences

- Modern panels and controls share one validated, allocation-free raster path.
- Damage bounds include line width and shadow extent, preserving incremental redraw.
- The ABI stays fixed-size and compatible with existing command storage.
- The fixed ring kernels provide bounded soft shadows without runtime memory growth.
- Large GUI updates cannot starve the Display heartbeat while being composed.
- Display uses a package-local size-optimized release profile to remain inside the fixed image cap.
