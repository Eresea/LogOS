# ADR-0094: Terminal split panes attach their own session

- Status: Accepted
- Date: 2026-09-29

## Context

T3 (#76) gave one Terminal surface up to four sessions with a tab bar. Atrium's tiling workspace
(ADR-0083) can show several surfaces at once, but Atrium allowed one Terminal surface
(`request_surface` rejected a second one from the same client, `AtriumAction::Launch` refocused the
existing one), and the Terminal process, Atrium's Terminal plumbing (grid node id, tab state,
bounds updates, revokes) and the wire request all assumed exactly one surface. #97 asks that
opening Terminal into a split pane shows its own session.

Before this change the Terminal process asked for its one surface with an `AtriumSurfaceRequest`
(request id 1) and kept `terminal_surface`/`terminal_bounds`. It re-sent the same request whenever
its tab bar changed, packing the open-tab bitmap and active slot into `tab_state`; Atrium
recognised "surface already exists for this client", stored `tab_state` in one global and
answered with a reconfirmation.

## Decision

**One Terminal process serves several surfaces ("panes"); the four session slots stay one shared
pool.** Chosen over spawning a Terminal process per pane (a new endpoint/capability set per pane,
out of scope) and over an Atrium-owned pane table.

- **Panes own sessions.** `TerminalService` gains `MAX_TERMINAL_PANES` (= `MAX_TERMINAL_SESSIONS`
  = 4) panes. Every open session belongs to exactly one pane (`Session.owner`); a pane's tab strip
  is the set of sessions it owns and its `active` slot is the one whose grid it shows. Opening a
  tab takes any free slot of the shared four (`open_session_count()` enforces the cap), so the cap
  is shared by tabs and panes, not split. A pane always keeps at least one tab.
- **Pane lifecycle.** `bind_surface(surface, bounds)` attaches an Atrium-admitted surface: a known
  surface only updates bounds; the first surface reuses the retained pane (session 0, and any
  tabs kept from before); each later surface creates a pane with one fresh session and fails if
  none is free. `unbind_surface(surface)` detaches a revoked surface: while other surfaces remain,
  its pane is freed and every session it owned is closed (generation bumped, a tagged
  `SessionClose` returned per session for the process to forward); the last surface's pane is only
  unbound, keeping its sessions as before (closing and reopening Terminal restores its tabs).
- **Routing.** Input arrives with the target surface (`AtriumSurfaceInput.surface`); the process
  calls `select(surface)` and then every existing method (`input`, tab switch/open/close,
  `resize_to_surface`) acts on that pane only. Tab handles are generation-safe and additionally
  checked against the selected pane, so a pane cannot switch to or close another pane's tab.
  Output still routes by the session tag from ADR-0090, independent of panes.
  `next_grid_row()` walks the panes round-robin and addresses each row to its pane's surface
  (Atrium fills in that surface's text-grid node id).
- **Wire (ABI_VERSION 13 -> 14).** `AtriumSurfaceRequest` gains `surface: SurfaceHandle`, naming
  the pane a tab-state report describes (`EMPTY` for the first request). Atrium looks the
  surface up by reference and owner instead of "the client's Terminal surface". A report naming a
  surface that no longer exists is answered `NotFound`. Terminal learns of a *new* pane from an
  ordinary `Ok` response carrying an unseen surface; no new message kind, endpoint or capability.
- **Atrium.** Only `AppId::Terminal` may own several surfaces per client (`request_surface`).
  `Launch(Terminal)` while a split pane is waiting (`Atrium::has_empty_leaf`) and a session is free
  creates a new Terminal surface into it; every other launch keeps refocusing the existing one.
  "A session is free" is the popcount of the union of the open-tab bitmaps Terminal last reported
  for its live surfaces (bits are session slots, so panes are disjoint). Per-surface Terminal state
  that was a single global becomes a table indexed by the surface slot: tab state, text-grid node
  id, last-sent bounds (with a pending `Update` per slot) and deferred revokes (logout revokes all
  panes). The admission response is never dropped for a busy response slot.
- **Open in a split.** No new shortcut: ADR-0083's `Ctrl+Shift+V`/`Ctrl+Shift+H` split the
  focused pane and `Ctrl+3` (or the launcher/Home tile) then launches Terminal into the waiting
  half. `Alt+F4` closes the focused pane and frees its session.
- **Text-grid budget.** Display already keeps one retained `TextGrid` store per surface
  (`MAX_GUI_TEXT_GRIDS` = 4). A pane needs a session, so panes <= sessions <= 4;
  `MAX_TERMINAL_PANES <= MAX_GUI_TEXT_GRIDS` is a compile-time assertion.
- **Appearance.** The service remembers the last appearance flags and applies them to sessions it
  creates afterwards, so a new tab or pane does not start in the default theme.

## Consequences

- Session and Flow are unchanged: a pane's sessions are ordinary session slots, tagged as in
  ADR-0090. Split panes therefore share Flow's arbitration queue like tabs do (a command in one
  pane delays another pane's command, never its typing).
- No drag-out of tabs into panes, no per-pane session limit; a pane holding all four sessions
  leaves none for a second pane.
- Known bounded race: Atrium decides "a session is free" from the last reported tab bitmaps. If
  the user opens a tab in another pane in the window between that decision and the new surface
  arriving at Terminal, `bind_surface` fails and the new pane stays blank until it is closed
  (`Alt+F4`); no session is ever shared or over-allocated.
- `ABI_VERSION` 14: any older `AtriumSurfaceRequest` producer (System, programs) uses
  `AtriumSurfaceRequest::new`, which leaves `surface` empty and behaves as before.
