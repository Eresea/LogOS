# ADR-0087: Retained text-grid scene node and a 48-node budget

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

Display owns a fixed array of `MAX_GUI_TEXT_GRIDS` (4) bound text-grid
buffers, one per surface, and rasterizes each inside its owning surface's clip
through a new `GuiRenderBackend::draw_text_grid` method with a default no-op
body, so an eventual GPU backend can replace only that method, per ADR-0075's
backend trait split. A store binds to whichever node in its surface's active
scene currently carries `GuiDrawKind::TextGrid`; a changed `node_id` on an
already-bound store blanks its cells so stale content from a previous grid on
that surface can never show through a new one. A new surface's grid claims a
free store, or is rejected outright with `GuiRegistryError::Capacity` — before
any of that frame's node-table state is committed — if all four are already
bound elsewhere; it never evicts another surface's grid to make room. One
store per surface (never a single shared slot) is the fix for the same bug
class that broke Terminal originally, where Display's single-surface
`terminal_bounds()` first-match let one surface's state stand in for another's.

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
- `MAX_GUI_TEXT_GRIDS` is 4: one store per Terminal session, matching the up
  to 4 sessions planned in #76 (T3). A 5th concurrent grid is rejected, not
  silently evicted; host tests cover independent content across two bound
  surfaces, a released store being reused by a new grid, and a 5th grid
  being rejected without disturbing the first four.

## Memory cost

Each store is `MAX_GUI_TEXT_GRID_COLUMNS * MAX_GUI_TEXT_GRID_ROWS` (160 x 50 =
8,000) `Cell`s at `size_of::<Cell>()` = 16 bytes: 128,000 bytes. Four stores
are 512,000 bytes (500 KiB), always resident in `GuiSurfaceRegistry`
regardless of how many grids are actually bound (fixed-size arrays, no
allocator). Measured against ADR-0080's 2048 KiB (2,097,152 byte) service
image ceiling, built release `logos-display`:

| | Base (before this ADR) | With `MAX_GUI_TEXT_GRIDS = 4` | Delta |
| --- | ---: | ---: | ---: |
| ELF file size | 614,336 B | 1,313,320 B | +698,984 B |
| `text+data+bss` | 560,674 B | 1,259,428 B | +698,754 B |

1,313,320 bytes is 62.6% of the 2,097,152-byte budget, leaving roughly 784 KiB
of headroom — it fits. The four-store delta (roughly 682 KiB) is larger than
the raw 500 KiB of cell data alone because `Cell::EMPTY` isn't the all-zero
pattern (space codepoint, opaque foreground), so the initialized arrays land
in `.data` rather than zero-filled `.bss`, plus `RenderPlan`'s own growth from
`MAX_GUI_NODES` (48, ADR-0087) is included in both columns.

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
  matching the raised tree/op capacity).
- `GuiSurfaceRegistry`/`RenderPlan`/`Display` must never be reconstructed by
  value at runtime once `MAX_GUI_TEXT_GRIDS` stores are part of the struct:
  `GuiSurfaceRegistry::new()`/`Display::new()` stay `const fn` (the one
  `static mut DISPLAY: Display = Display::new(1)` use is compile-time
  const-evaluated into static data — zero runtime cost), but a *runtime*
  call that rebuilt the whole value from scratch, such as
  `Display::replace_generation`'s prior `self.gui =
  GuiSurfaceRegistry::new();` or `ensure_plan`'s prior `self.plan =
  RenderPlan::new();`, forced the compiler to materialize the entire
  ~600 KiB struct (`MAX_GUI_SURFACES * MAX_GUI_NODES` render-plan entries
  plus four 128,000-byte text-grid buffers) as a stack value before the
  move-assignment — measured at 616,104 bytes for `replace_generation` and
  43,416 for `ensure_plan` in the release `x86_64-unknown-none` binary,
  against a 1,048,576-byte (`MAX_SERVICE_STACK_PAGES`, 256 pages) total
  per-service stack ceiling. `ensure_plan` runs on every scene change, not a
  rare path. Both now call a `reset()` method that clears each field in
  place (array elements and cells via `.fill()`/loops, never a fresh
  struct literal mixing runtime fields with a large const array), dropping
  `replace_generation` to 4,360 bytes and `ensure_plan` to 4,040 bytes —
  both now below their own *pre-ADR-0087* baselines (52,520 and 20,480).
  No Display function crosses the 2 KiB STACK-CHECK threshold from this ADR
  once `reset()` is used; `GuiSurfaceRegistry::create` (unrelated to text
  grids) grows from 7,336 to 14,632 bytes, proportional to `MAX_GUI_NODES`
  doubling the per-surface node arrays it initializes, and stays well under
  the per-service ceiling.
- No new runtime, allocator, or IPC transport is introduced: `GuiTextGridRow`
  reuses the existing "large compatibility struct passed directly, not over
  the small IPC ring" shape already established by `RenderMessage`.
