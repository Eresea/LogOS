# ADR-0086: Retained text-grid scene node and a 48-node budget

Status: Accepted

## Decision

Graphics v2 (ADR-0075) gains one new retained node kind, `GuiDrawKind::TextGrid`,
that draws a bounded columns x rows grid of monospace 8x16 cells inside the
node's own rectangle. The command carries only a descriptor (bounds, column
count, row count); actual cell content (codepoint, foreground, background,
attributes — the existing `Cell` type already used by the terminal
compatibility protocol) is delivered separately, one row at a time, through a
new `GuiTextGridRow` value addressed to the node's `(surface, node_id)`. Like
`RenderMessage`, `GuiTextGridRow` carries real payload and is exempt from
`MAX_IPC_BYTES`; it is not sent over the small scene-op ring. Sending a
changed row by itself — instead of re-publishing the whole grid — is the
"dirty-row" update the node is built around: only that row's rectangle is
damaged and repainted.

Display owns exactly one bound text-grid buffer (one grid at a time is all any
current or planned screen needs) and rasterizes it inside the owning surface's
clip through a new `GuiRenderBackend::draw_text_grid` method with a default
no-op body, so an eventual GPU backend can replace only that method, per
ADR-0075's backend trait split. Binding follows whichever node in the active
scene currently carries `GuiDrawKind::TextGrid`; a changed `(surface,
node_id)` blanks the buffer so stale content from a previous grid can never
show through a new one.

`MAX_GUI_NODES` rises from 24 to 48 so a Terminal surface can carry its own
chrome (title, future tab bar) alongside one grid node, with headroom left
over for the existing Home and Settings scenes. `MAX_UI_NODES` (the UI
component tree's own capacity, previously 32 and numerically unrelated to
`MAX_GUI_NODES`) rises to 48 too, so a screen's UI-tree size is never the
tighter budget now that the retained scene can emit more ops than before.
`MAX_UI_SCENE_PUBLISHER_BYTES` rises from 7_232 to 9_264, matching
`UiScenePublisher` holding two full `MAX_UI_SCENE_OPS`-sized frames.

## Fixed bounds and limits

- `MAX_GUI_TEXT_GRID_COLUMNS` reuses the terminal compatibility protocol's
  existing `MAX_COLUMNS` (160 = 1280 / 8), so a future adapter can hand the
  same `Cell` buffer to either path.
- `MAX_GUI_TEXT_GRID_ROWS` is 50 (800 / 16) — the 1280x800 Terminal viewport
  at 8x16 glyphs, not the terminal protocol's larger `MAX_ROWS` (100), which
  reserves scrollback headroom out of scope here. A row offset for
  scrollback is future work (#71's own boundary); this node has none yet.
- A `TextGrid` command's `width`/`height` must equal `columns * 8` and
  `rows * 16` exactly, `columns`/`rows` must be nonzero and within the two
  bounds above, and the command must use the identity transform (no
  rotation/scale support — a terminal-shaped grid has no use for it yet).
  Unknown/oversized/out-of-bounds shapes are rejected by
  `GuiDrawCommand::is_valid`.
- A `GuiTextGridRow` update is rejected if its row is out of the node's own
  row count, if it carries more cells than the node's own column count, or
  if any populated cell fails `Cell::is_valid` (unknown attribute bits,
  surrogate/out-of-range codepoints, zero or >2 cell width, or a nonzero
  reserved byte).
- `MAX_GUI_NODES` (48) bounds `RenderPlan`
  (`MAX_GUI_SURFACES * MAX_GUI_NODES`) and the retained scene op/publisher
  budgets together; SCENE-BUDGET coverage for the existing Home, Settings,
  and app scenes is unchanged (they sit well under the new ceiling, as
  before).

## Consequences

- Terminal migrating its content onto this node (#74/T1) is not done here:
  the raw `RenderCells`/`apply()` compatibility path is untouched, and the
  boundary from ADR-0075 ("the terminal cell protocol remains a
  compatibility input until its retained-node adapter is proven") still
  holds. This ADR proves the adapter node in isolation.
- Raising `MAX_UI_NODES`/`MAX_GUI_NODES` grows the stack frame of every
  function that holds a `UiTree`/`UiComponentTree`/`UiSceneFrame`/
  `UiScenePublisher` by value (roughly in proportion to 48/32 or 48/24).
  STACK-CHECK shows this growth concentrated in Atrium's and LockScreen's own
  `build_*`/`UiComponentTree`/`UiTree` construction functions (tens of KB,
  matching the raised tree/op capacity), not in Display: no Display function
  crosses the 2 KiB STACK-CHECK threshold from this change.
- No new runtime, allocator, or IPC transport is introduced: `GuiTextGridRow`
  reuses the existing "large compatibility struct passed directly, not over
  the small IPC ring" shape already established by `RenderMessage`.
