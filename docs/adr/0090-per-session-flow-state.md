# ADR-0090: Per-session Flow state with one shared arbitration queue

Status: Accepted

## Decision

T3 (#76) gave Terminal up to 4 tabs, but one shared Session line editor and
one shared Flow interpreter sat behind all of them: variables and the line
buffer were global, and while one tab's command was running, keystrokes in
another tab could reach the same in-flight exchange. T3c (#98) gives each
Terminal session its own line-editor and Flow interpreter state, chosen
between the two designs #98 offered:

- per-session interpreter state **plus** each of Flow's seven single-flight
  clients (Storage, Network, Package, Device, User, Fetch, Completion)
  duplicated per session, or
- per-session interpreter state with **one shared arbitration queue**: only
  one session's command is ever in flight with Flow at a time, so the seven
  clients stay singletons.

The shared-queue design is chosen: it isolates exactly what differs per
session (variables, promises, and the line editor) while leaving Flow's
already-bounded client state untouched, for a much smaller diff. A tab's
own line editing is never blocked — only the moment a finished line is
handed to Flow is serialized against other tabs' commands, which matches
how a single-core interactive shell behaves today.

### Session tag

`IpcBytes.flags` had one used bit (`IPC_FLAG_MORE`); bits 1-2 now carry a
Terminal session tag (0..3, `IPC_SESSION_MASK`/`IPC_SESSION_SHIFT`,
`MAX_SHELL_SESSIONS = 4`). `FlowControl` gains a `session: u8` field (taken
from its `reserved` bytes, same wire size). Terminal tags every
`SessionInput` it sends with the active tab's slot; Session and Flow tag
every reply they send with the session that produced it. `ABI_VERSION`
moves 8 -> 9 for the `FlowControl` shape change and the now-significant
`IpcBytes.flags` bits. A new `MessageKind::SessionClose` (32) tells Session
a tab closed.

### Session (`services/images/src/session.rs`)

Becomes 4 `SessionSlot`s (line editor, output queue, and queued
flow/completion request), plus one `OWNER: Option<u8>` naming the slot
currently exchanging with Flow. Each tick: flush every slot's queued
output (tagged), advance `OWNER`'s exchange if one is set, otherwise start
the next slot with queued work (round-robin), and dispatch every incoming
tagged `SessionInput`/`SessionClose` to its own slot regardless of `OWNER`.
A tab's typing is never gated on `OWNER`; only starting a new exchange is.

### Flow (`services/images/src/flow.rs`)

`FLOWS: [FlowService; 4]` replaces the single `FlowService`; the incoming
message's session tag selects which one a `SessionInput`/`CompletionRequest`
is evaluated against. `ACTIVE_SESSION` records that tag for the duration of
the exchange (Session's `OWNER` invariant guarantees it cannot change
mid-command) and tags every reply Flow sends — `PendingOutput`, the
completion response, and fetch progress — so Session and Terminal can route
it back without re-deriving ownership.

### Closing a tab

`TerminalService::close_tab` now returns `Option<IpcBytes>` — a tagged
`SessionClose` — for the caller to forward. Session resets that slot (fresh
line editor, dropped queued work) and, if it was `OWNER`, sends a tagged
`FlowControl` cancel; the eventual (possibly-cancelled) reply, still tagged
to that slot, clears `OWNER` normally rather than being force-cleared,
so Flow's shared clients are never driven by two sessions at once.

### Accepted race

A reply already in flight from Flow when a tab closes can still arrive
tagged to that slot after Session has reset it for reuse by a new tab
opened in the same, since-reused slot. Terminal drops output for a slot it
no longer has open, and the new tab's own exchange only starts once
`OWNER` clears, so this cannot corrupt either tab's visible state or Flow's
shared clients — the observable effect is bounded to a stray, unrendered
`SessionOutput`/`CompletionResponse` chunk. A full fix needs a generation
counter carried alongside the session tag, which the 2 spare `flags` bits
used here do not have room for; if this ever matters in practice, it is a
smaller follow-up (e.g. moving the tag to a wider field) rather than a
reason to prefer the per-session-clients design.

## Rationale

Duplicating the seven Flow clients per session would quadruple their
already-bounded state for isolation none of them need: Storage, Network,
Package, Device, User, Fetch, and Completion already serialize one request
at a time by construction, and #98 scopes out job control (no bg/fg), so
nothing in this milestone needs two sessions' commands to make external
progress concurrently — only for their prompts, variables, and echoed
input to stay untangled while both tabs are otherwise responsive.

## Consequences

- Two sessions' commands cannot run concurrently against Storage/Network/
  etc.; a second session's command waits for the first's exchange with
  Flow to finish, though its own typing is never blocked while it waits.
- `MAX_SHELL_SESSIONS` (4) is the one source of truth for the tag's range;
  Terminal's own `MAX_TERMINAL_SESSIONS` must stay equal to it.
- `ABI_VERSION` 9: any future spare-bit user of `IpcBytes.flags` must avoid
  bits 1-2.
