# ADR-0075: Retained Graphics v2

- Status: Accepted
- Date: 2026-08-27

## Decision

Graphics v2 replaces sequence-wide GUI batches with a fixed retained scene. Each
scene operation names a stable node ID and frame revision. Operations with
`GUI_DRAW_FLAG_MORE` update a private staging scene; the operation that clears
the flag atomically publishes that revision. Display damages only old and new
node bounds, including removed nodes.

Composition consumes a bounded dirty-tile queue, renders into a Display-owned
RAM backbuffer, and presents changed regions to GOP only after composition.
Static scene groups remain published while lockscreen field nodes update by
stable ID; retained glyph-run nodes therefore act as the glyph cache and the
fixed font atlas is never regenerated. Opaque retained nodes provide occlusion
to lower nodes. `GuiSurfaceRegistry::compose` consumes that retained scene
through a backend trait; the current backend rasterizes into RAM, while a
future GPU backend can replace only the command execution/presentation layer.

The first commit scope is per-surface: a surface revision is atomic, while
independent surfaces may publish independently. The terminal cell protocol
remains a compatibility input until its retained-node adapter is proven.

## Consequences

- Intermediate GUI fragments cannot reach the presented frame.
- Rendering work follows changed visible tiles rather than screen-height passes
  or total retained-node count.
- GOP is no longer an alpha-composition source.
- Fixed node, tile, cache, and backbuffer bounds remain part of the Display
  resource contract.
- GPU support requires no new scene protocol or producer migration.
