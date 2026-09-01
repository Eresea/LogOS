# ADR-0082: Bounded UI motion

Status: Accepted

## Decision

`logos-ui` owns a fixed-point motion model with bounded transition utilities, eight keyframes per
animation, eight custom animations per document, bounded delays/repeats, named easing curves, and
validated cubic-bezier control points. Framework-approved looping presets use an explicit infinite
repeat sentinel; inline document animations may only use finite repeats. `UiComponentTree` owns the
per-node animator and exposes its next deadline; Atrium and LockScreen remain responsible for
waiting and re-emitting only while motion is active. Nodes are removed immediately, so unmount
animation does not retain resources.

The graphics ABI is versioned for identity-preserving transform metadata. Display computes
conservative transformed damage, clips transformed output to the owning surface/framebuffer, and
does not use transformed commands as occluders. Identity commands retain the existing raster fast
path.

## Rationale

CSS-like utility names keep common transitions easy to author while inline keyframes cover richer
motion without a stylesheet, allocator, float math, or permanent render loop. Fixed bounds preserve
the kernel/service resource model and make refresh work event-driven.

## Consequences

Rotation applies around each command's center and uses a bounded integer trigonometry table. Hit
testing applies the inverse transform. Color interpolation is represented by the UI motion model;
the current scene adapter applies animated opacity and transforms while theme color ownership remains
with `logos-ui-graphics`.
