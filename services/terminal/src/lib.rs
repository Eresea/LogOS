#![no_std]

//! Bounded fixed-size terminal emulator.

#[cfg(test)]
extern crate std;

use logos_abi::{
    APPEARANCE_LIGHT_THEME, APPEARANCE_REDUCED_MOTION, CELL_ATTR_BOLD, CELL_ATTR_DIM,
    CELL_ATTR_UNDERLINE, Cell, DEFAULT_COLUMNS, DEFAULT_ROWS, GuiRect, GuiTextGridRow,
    InputMessage, IpcBytes, KeyCode, KeyState, MOD_ALT, MOD_CAPS_LOCK, MOD_CTRL, MOD_SHIFT,
    MessageKind, SurfaceHandle, pack_terminal_tab_state, terminal_grid_metrics,
};

const MAX_PARAMS: usize = 16;
const REPLACEMENT_SCALAR: u32 = 0xfffd;
/// Service-local storage cap; the ABI maximum is a protocol-wide ceiling.
pub const TERMINAL_SCROLLBACK_LINES: usize = 64;
/// Scrollback lines per mouse-wheel notch.
const WHEEL_LINES: isize = 3;
/// Half a blink period in timer ticks (100 Hz): 500 ms on, 500 ms off.
pub const CURSOR_BLINK_TICKS: u64 = 50;
/// Like GTK's cursor-blink-timeout: after 10 s without activity the cursor
/// stays solid, so an idle Terminal stops repainting.
pub const CURSOR_BLINK_IDLE_TICKS: u64 = 1_000;
const DEFAULT_FOREGROUND_DARK: u32 = 0x00d7_e3f4;
const DEFAULT_BACKGROUND_DARK: u32 = 0x000b_1020;
/// S5 (#82): the reset (SGR 39/49) colours when the light theme is on,
/// matching `logos_ui_graphics::UiSceneTheme::LIGHT`'s `text`/`surface`. The
/// numbered ANSI colours (`ANSI_COLORS`/`ANSI_BRIGHT_COLORS`) stay fixed in
/// both themes, like a real terminal's palette does.
const DEFAULT_FOREGROUND_LIGHT: u32 = 0x0014_212c;
const DEFAULT_BACKGROUND_LIGHT: u32 = 0x00f5_f7fa;
const DEFAULT_FOREGROUND: u32 = DEFAULT_FOREGROUND_DARK;
const DEFAULT_BACKGROUND: u32 = DEFAULT_BACKGROUND_DARK;
const ANSI_COLORS: [u32; 8] = [
    0x000b_1020,
    0x00ff_6b6b,
    0x007e_d787,
    0x00ff_d866,
    0x0058_a6ff,
    0x00d2_a8ff,
    0x0056_d4dd,
    DEFAULT_FOREGROUND,
];
const ANSI_BRIGHT_COLORS: [u32; 8] = [
    0x0030_3845,
    0x00ff_7b72,
    0x00a5_d6a7,
    0x00f2_cc60,
    0x0079_c0ff,
    0x00d2_a8ff,
    0x00a5_d6ff,
    0x00ffffff,
];

const fn blank_cell() -> Cell {
    Cell {
        codepoint: b' ' as u32,
        foreground: DEFAULT_FOREGROUND,
        background: DEFAULT_BACKGROUND,
        attributes: 0,
        width: 1,
        reserved: 0,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ParserState {
    Ground,
    Escape,
    Csi,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Parser {
    state: ParserState,
    params: [u16; MAX_PARAMS],
    param_count: usize,
    current: u16,
    has_current: bool,
}

impl Parser {
    const fn new() -> Self {
        Self {
            state: ParserState::Ground,
            params: [0; MAX_PARAMS],
            param_count: 0,
            current: 0,
            has_current: false,
        }
    }

    fn reset_csi(&mut self) {
        self.param_count = 0;
        self.current = 0;
        self.has_current = false;
    }

    fn push_param(&mut self) {
        if self.param_count < MAX_PARAMS {
            self.params[self.param_count] = if self.has_current { self.current } else { 0 };
            self.param_count += 1;
        }
        self.current = 0;
        self.has_current = false;
    }

    fn param(&self, index: usize, default: u16) -> u16 {
        if index >= self.param_count {
            return default;
        }
        match self.params[index] {
            0 => default,
            value => value,
        }
    }
}

pub struct TerminalState<const CELL_COUNT: usize> {
    columns: usize,
    rows: usize,
    cursor_column: usize,
    cursor_row: usize,
    saved_cursor_column: usize,
    saved_cursor_row: usize,
    wrap_pending: bool,
    cursor_dirty: bool,
    screen: [Cell; CELL_COUNT],
    dirty: [bool; CELL_COUNT],
    full_redraw_pending: bool,
    parser: Parser,
    foreground: u32,
    background: u32,
    attributes: u16,
    utf8_codepoint: u32,
    utf8_remaining: u8,
    utf8_min: u32,
    scrollback: [Cell; DEFAULT_COLUMNS * TERMINAL_SCROLLBACK_LINES],
    scrollback_start: usize,
    scrollback_len: usize,
    view_offset: usize,
    reduced_motion: bool,
    light_theme: bool,
    cursor_hidden: bool,
    blink_restart: bool,
    blink_anchor: u64,
}

/// T3 (#76): the Terminal service hosts up to this many independent
/// sessions (grid + scrollback each), with a tab bar to switch between
/// them. The cap is fixed; no reordering or drag-out (out of scope).
/// T3b (#97): the cap is shared by every pane (Atrium surface) -- each
/// session belongs to exactly one pane, so there can never be more panes
/// than sessions, and never more than `MAX_GUI_TEXT_GRIDS` text grids.
pub const MAX_TERMINAL_SESSIONS: usize = 4;
/// One pane owns at least one session, so the pane bound equals the session
/// bound; Display keeps one retained text grid per pane surface.
pub const MAX_TERMINAL_PANES: usize = MAX_TERMINAL_SESSIONS;
const _: () = assert!(MAX_TERMINAL_PANES <= logos_abi::MAX_GUI_TEXT_GRIDS);

/// Generation-safe handle to a tab, mirroring the `slot`/`generation`
/// pattern `SurfaceHandle` already uses elsewhere in the ABI: closing a tab
/// frees its slot and bumps the generation, so a handle captured before the
/// close is rejected by `switch_tab`/`close_tab` rather than silently
/// hitting whatever session was reused into that slot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TabHandle {
    slot: u8,
    generation: u8,
}

impl TabHandle {
    pub const EMPTY: Self = Self { slot: u8::MAX, generation: 0 };

    pub const fn is_valid(self) -> bool {
        self.slot != u8::MAX && self.generation != 0
    }

    pub const fn slot(self) -> usize {
        self.slot as usize
    }
}

struct Session {
    terminal: TerminalState<{ DEFAULT_COLUMNS * DEFAULT_ROWS }>,
    open: bool,
    generation: u8,
    /// Index of the pane whose tab strip lists this session (T3b, #97).
    owner: u8,
}

impl Session {
    const fn new() -> Self {
        Self { terminal: TerminalState::new(), open: false, generation: 0, owner: 0 }
    }

    fn handle(&self, slot: usize) -> TabHandle {
        TabHandle { slot: slot as u8, generation: self.generation }
    }
}

/// One Atrium Terminal surface (T3b, #97): the tab strip it shows is the
/// set of sessions it owns, and `active` is the one whose grid it displays.
#[derive(Clone, Copy)]
struct Pane {
    in_use: bool,
    /// `EMPTY` for a retained pane whose surface was closed while it was the
    /// last one: its sessions survive so reopening Terminal restores them.
    surface: SurfaceHandle,
    bounds: GuiRect,
    active: u8,
}

impl Pane {
    const EMPTY: Self =
        Self { in_use: false, surface: SurfaceHandle::EMPTY, bounds: GuiRect::EMPTY, active: 0 };
}

/// `SessionClose` messages for the sessions a closed pane released.
pub type PaneCloses = [Option<IpcBytes>; MAX_TERMINAL_SESSIONS];

pub struct TerminalService {
    sessions: [Session; MAX_TERMINAL_SESSIONS],
    panes: [Pane; MAX_TERMINAL_PANES],
    /// The pane every tab and input method below acts on; the caller picks
    /// it with `select` from the surface an event arrived for.
    current: usize,
    render_cursor: usize,
    /// Last desktop appearance flags received, applied to every session
    /// created afterwards (a new tab or pane must not start dark/animated).
    appearance: u16,
}

const _: () =
    assert!(core::mem::size_of::<TerminalService>() <= logos_abi::MAX_SERVICE_IMAGE_BYTES);

impl TerminalService {
    pub const fn new() -> Self {
        const SESSION: Session = Session::new();
        let mut sessions = [SESSION; MAX_TERMINAL_SESSIONS];
        sessions[0].open = true;
        sessions[0].generation = 1;
        let mut panes = [Pane::EMPTY; MAX_TERMINAL_PANES];
        panes[0].in_use = true;
        Self { sessions, panes, current: 0, render_cursor: 0, appearance: 0 }
    }

    fn active_slot_of(&self, pane: usize) -> usize {
        self.panes[pane].active as usize
    }

    fn active_session(&mut self) -> &mut TerminalState<{ DEFAULT_COLUMNS * DEFAULT_ROWS }> {
        let slot = self.active_slot_of(self.current);
        &mut self.sessions[slot].terminal
    }

    fn owns(&self, pane: usize, slot: usize) -> bool {
        self.sessions[slot].open && self.sessions[slot].owner as usize == pane
    }

    /// The current pane's active tab handle; always valid since a pane
    /// always keeps at least one tab open.
    pub fn active_tab(&self) -> TabHandle {
        let slot = self.active_slot_of(self.current);
        self.sessions[slot].handle(slot)
    }

    /// Tabs in the current pane.
    pub fn tab_count(&self) -> usize {
        (0..MAX_TERMINAL_SESSIONS).filter(|&slot| self.owns(self.current, slot)).count()
    }

    /// Open sessions across every pane: the shared cap counts these.
    pub fn open_session_count(&self) -> usize {
        self.sessions.iter().filter(|session| session.open).count()
    }

    /// Handle of the current pane's open tab at `slot`, for tab-bar
    /// rendering; `None` if that slot is not one of its tabs.
    pub fn tab_at(&self, slot: usize) -> Option<TabHandle> {
        (slot < MAX_TERMINAL_SESSIONS && self.owns(self.current, slot))
            .then(|| self.sessions[slot].handle(slot))
    }

    fn bitmap_of(&self, pane: usize) -> u8 {
        (0..MAX_TERMINAL_SESSIONS)
            .filter(|&slot| self.owns(pane, slot))
            .fold(0u8, |bitmap, slot| bitmap | (1 << slot))
    }

    /// Which slots are the current pane's tabs, one bit per slot (bit 0 =
    /// slot 0), for reporting tab-bar state to Atrium's scene builder.
    pub fn open_bitmap(&self) -> u8 {
        self.bitmap_of(self.current)
    }

    pub fn active_slot(&self) -> usize {
        self.active_slot_of(self.current)
    }

    /// Makes the pane showing `surface` the one the other methods act on.
    /// `false` (and no change) for a surface no pane is bound to.
    pub fn select(&mut self, surface: SurfaceHandle) -> bool {
        match self.pane_index(surface) {
            Some(pane) => {
                self.current = pane;
                true
            }
            None => false,
        }
    }

    fn pane_index(&self, surface: SurfaceHandle) -> Option<usize> {
        if !surface.is_valid() {
            return None;
        }
        self.panes.iter().position(|pane| pane.in_use && pane.surface == surface)
    }

    fn bound_pane_count(&self) -> usize {
        self.panes.iter().filter(|pane| pane.in_use && pane.surface.is_valid()).count()
    }

    /// Surface and packed tab-bar state of pane `index`, for the caller to
    /// report to Atrium; `None` for an unbound pane.
    pub fn pane_report(&self, index: usize) -> Option<(SurfaceHandle, u16)> {
        let pane = self.panes.get(index).filter(|p| p.in_use && p.surface.is_valid())?;
        Some((pane.surface, pack_terminal_tab_state(self.bitmap_of(index), pane.active)))
    }

    /// The current pane's last known surface bounds.
    pub fn pane_bounds(&self) -> GuiRect {
        self.panes[self.current].bounds
    }

    /// Attaches a surface Atrium admitted (T3b, #97) and makes its pane
    /// current. A surface already bound just takes the new `bounds`. The
    /// first surface reuses the retained pane (and its sessions); each later
    /// one gets a fresh pane with one new session. `false` if no session is
    /// free for it, which the shared cap of `MAX_TERMINAL_SESSIONS` allows
    /// only when Atrium raced a Terminal tab opening; that surface stays
    /// blank until it is closed.
    pub fn bind_surface(&mut self, surface: SurfaceHandle, bounds: GuiRect) -> bool {
        if !surface.is_valid() {
            return false;
        }
        if let Some(pane) = self.pane_index(surface) {
            self.current = pane;
            if self.panes[pane].bounds != bounds {
                self.resize_to_surface(bounds);
            }
            return true;
        }
        if let Some(pane) = self.panes.iter().position(|p| p.in_use && !p.surface.is_valid()) {
            self.panes[pane].surface = surface;
            self.current = pane;
            self.active_session().reset();
            self.resize_to_surface(bounds);
            self.active_session().force_redraw();
            return true;
        }
        let Some(pane) = self.panes.iter().position(|p| !p.in_use) else { return false };
        let Some(slot) = self.sessions.iter().position(|session| !session.open) else {
            return false;
        };
        let session = &mut self.sessions[slot];
        session.terminal = TerminalState::new();
        session.terminal.set_reduced_motion(self.appearance & APPEARANCE_REDUCED_MOTION != 0);
        session.terminal.set_light_theme(self.appearance & APPEARANCE_LIGHT_THEME != 0);
        session.open = true;
        session.owner = pane as u8;
        session.generation = session.generation.wrapping_add(1).max(1);
        self.panes[pane] =
            Pane { in_use: true, surface, bounds: GuiRect::EMPTY, active: slot as u8 };
        self.current = pane;
        self.resize_to_surface(bounds);
        self.active_session().force_redraw();
        true
    }

    /// Detaches a closed (revoked) surface. While other surfaces remain,
    /// its pane and every session it owned are freed (generation-safe) and
    /// a `SessionClose` per session is returned for the caller to forward;
    /// the last surface's pane is only unbound, keeping its sessions.
    pub fn unbind_surface(&mut self, surface: SurfaceHandle) -> PaneCloses {
        let mut closes = [None; MAX_TERMINAL_SESSIONS];
        let Some(pane) = self.pane_index(surface) else { return closes };
        if self.bound_pane_count() <= 1 {
            self.panes[pane].surface = SurfaceHandle::EMPTY;
            return closes;
        }
        for (slot, close) in closes.iter_mut().enumerate() {
            if self.owns(pane, slot) {
                let session = &mut self.sessions[slot];
                session.open = false;
                session.generation = session.generation.wrapping_add(1).max(1);
                *close = Some(IpcBytes::empty(MessageKind::SessionClose).with_session(slot as u8));
            }
        }
        self.panes[pane] = Pane::EMPTY;
        if self.current == pane {
            self.current =
                self.panes.iter().position(|p| p.in_use && p.surface.is_valid()).unwrap_or(0);
        }
        closes
    }

    /// Opens a new tab in the current pane (a fresh session, reset to the
    /// pane's size) and makes it active. `None` once `MAX_TERMINAL_SESSIONS`
    /// are already open across all panes.
    pub fn open_tab(&mut self) -> Option<TabHandle> {
        let (columns, rows) = {
            let active = &self.sessions[self.active_slot_of(self.current)].terminal;
            (active.columns, active.rows)
        };
        let slot = self.sessions.iter().position(|session| !session.open)?;
        let session = &mut self.sessions[slot];
        session.terminal = TerminalState::new();
        session.terminal.set_reduced_motion(self.appearance & APPEARANCE_REDUCED_MOTION != 0);
        session.terminal.set_light_theme(self.appearance & APPEARANCE_LIGHT_THEME != 0);
        session.terminal.resize(columns, rows);
        session.open = true;
        session.owner = self.current as u8;
        session.generation = session.generation.wrapping_add(1).max(1);
        self.panes[self.current].active = slot as u8;
        self.sessions[slot].terminal.force_redraw();
        Some(self.sessions[slot].handle(slot))
    }

    /// Closes `handle`'s tab, freeing its slot for reuse (the slot's
    /// generation is bumped, so a stale handle into it is rejected). Never
    /// closes a pane's last remaining tab. Picks a new active tab if the
    /// closed one was active. Returns a tagged `SessionClose` message (T3c,
    /// #98) for the caller to forward to Session, so it cancels that
    /// session's in-flight or queued command and frees its per-session state.
    pub fn close_tab(&mut self, handle: TabHandle) -> Option<IpcBytes> {
        if !self.is_valid(handle) || self.tab_count() <= 1 {
            return None;
        }
        let slot = handle.slot();
        self.sessions[slot].open = false;
        self.sessions[slot].generation = self.sessions[slot].generation.wrapping_add(1).max(1);
        if self.active_slot() == slot {
            let next = (0..MAX_TERMINAL_SESSIONS)
                .find(|&candidate| self.owns(self.current, candidate))
                .unwrap_or(0);
            self.panes[self.current].active = next as u8;
            self.active_session().force_redraw();
        }
        Some(IpcBytes::empty(MessageKind::SessionClose).with_session(slot as u8))
    }

    /// Switches the active tab by click. Rejects a stale, closed, or
    /// other-pane handle.
    pub fn switch_tab(&mut self, handle: TabHandle) -> bool {
        if !self.is_valid(handle) {
            return false;
        }
        if self.active_slot() != handle.slot() {
            self.panes[self.current].active = handle.slot() as u8;
            self.active_session().force_redraw();
        }
        true
    }

    /// Ctrl+Tab: switches to the pane's next tab, wrapping around.
    pub fn next_tab(&mut self) {
        let mut slot = (self.active_slot() + 1) % MAX_TERMINAL_SESSIONS;
        while !self.owns(self.current, slot) {
            slot = (slot + 1) % MAX_TERMINAL_SESSIONS;
        }
        self.panes[self.current].active = slot as u8;
        self.active_session().force_redraw();
    }

    fn is_valid(&self, handle: TabHandle) -> bool {
        handle.is_valid()
            && handle.slot() < MAX_TERMINAL_SESSIONS
            && self.owns(self.current, handle.slot())
            && self.sessions[handle.slot()].generation == handle.generation
    }

    /// Input routes only to the current pane's active session; Ctrl+Tab
    /// switches tabs and Ctrl+Shift+T opens a new one (a conventional
    /// accelerator alongside the tab bar's own new-tab button and per-tab
    /// close control), instead of reaching the shell.
    pub fn input(&mut self, event: &InputMessage) -> Option<IpcBytes> {
        // Applies to every tab, not just the active one: reduced motion and
        // the light theme (S5, #82) are desktop-wide, so a session the user
        // switches to later must already match instead of showing its old
        // appearance until it next redraws.
        if let Some(flags) = event.appearance_flags() {
            self.appearance = flags;
            for session in &mut self.sessions {
                session.terminal.set_reduced_motion(flags & APPEARANCE_REDUCED_MOTION != 0);
                session.terminal.set_light_theme(flags & APPEARANCE_LIGHT_THEME != 0);
            }
            return None;
        }
        if event.kind == MessageKind::Key
            && matches!(event.state, KeyState::Pressed | KeyState::Repeat)
            && event.modifiers & MOD_CTRL != 0
        {
            let code = KeyCode::from_raw(event.code);
            if matches!(code, KeyCode::Tab) {
                self.next_tab();
                return None;
            }
            if event.state == KeyState::Pressed
                && event.modifiers & MOD_SHIFT != 0
                && code == KeyCode::character(b't')
            {
                self.open_tab();
                return None;
            }
        }
        // Tag with the active tab's slot (T3c, #98) so Session and Flow can
        // keep this tab's line-editor and variables separate from every
        // other open tab's, even while another tab's command is still
        // running on the shared channel.
        let slot = self.active_slot();
        self.active_session().input(event).map(|message| message.with_session(slot as u8))
    }

    pub fn session_output(&mut self, message: &IpcBytes) {
        if let Some(bytes) = message.as_bytes() {
            self.session_output_bytes(message.session(), bytes);
        }
    }

    /// Routes output to the tab named by `session` (T3c, #98): Session
    /// tags every reply with the session whose command produced it, so
    /// Terminal no longer has to guess from a shell-prompt heuristic.
    /// Output for a tab that was since closed (a stale/out-of-range slot)
    /// is dropped.
    pub fn session_output_bytes(&mut self, session: u8, bytes: &[u8]) {
        let Some(target) = self.sessions.get_mut(session as usize).filter(|s| s.open) else {
            return;
        };
        target.terminal.feed(bytes);
    }

    pub fn reset(&mut self) {
        self.active_session().reset();
    }

    /// Resizes every session of the current pane to its surface `bounds`.
    pub fn resize_to_surface(&mut self, bounds: GuiRect) {
        // Shared with Atrium's `TextGrid` scene-node sizing (#75) so both
        // sides always agree on the grid shape; see `terminal_grid_metrics`.
        let (columns, rows, _) = terminal_grid_metrics(bounds);
        self.panes[self.current].bounds = bounds;
        for slot in 0..MAX_TERMINAL_SESSIONS {
            if self.owns(self.current, slot) {
                self.sessions[slot].terminal.resize(columns, rows);
            }
        }
    }

    /// Next dirty grid row of any bound pane's active session, addressed to
    /// that pane's surface; panes take turns so one busy pane cannot starve
    /// another.
    pub fn next_grid_row(&mut self) -> Option<GuiTextGridRow> {
        for step in 0..MAX_TERMINAL_PANES {
            let index = (self.render_cursor + step) % MAX_TERMINAL_PANES;
            let pane = self.panes[index];
            if !pane.in_use || !pane.surface.is_valid() {
                continue;
            }
            if let Some(mut row) = self.sessions[pane.active as usize].terminal.next_grid_row() {
                row.surface = pane.surface;
                self.render_cursor = (index + 1) % MAX_TERMINAL_PANES;
                return Some(row);
            }
        }
        None
    }

    pub fn blink(&mut self, now_ticks: u64) {
        for pane in self.panes {
            if pane.in_use && pane.surface.is_valid() {
                self.sessions[pane.active as usize].terminal.blink(now_ticks);
            }
        }
    }
}

impl Default for TerminalService {
    fn default() -> Self {
        Self::new()
    }
}

impl<const CELL_COUNT: usize> TerminalState<CELL_COUNT> {
    pub const fn new() -> Self {
        assert!(DEFAULT_COLUMNS * DEFAULT_ROWS <= CELL_COUNT);
        Self {
            columns: DEFAULT_COLUMNS,
            rows: DEFAULT_ROWS,
            cursor_column: 0,
            cursor_row: 0,
            saved_cursor_column: 0,
            saved_cursor_row: 0,
            wrap_pending: false,
            cursor_dirty: false,
            screen: [blank_cell(); CELL_COUNT],
            dirty: [true; CELL_COUNT],
            full_redraw_pending: true,
            parser: Parser::new(),
            foreground: DEFAULT_FOREGROUND,
            background: DEFAULT_BACKGROUND,
            attributes: 0,
            utf8_codepoint: 0,
            utf8_remaining: 0,
            utf8_min: 0,
            scrollback: [blank_cell(); DEFAULT_COLUMNS * TERMINAL_SCROLLBACK_LINES],
            scrollback_start: 0,
            scrollback_len: 0,
            view_offset: 0,
            reduced_motion: false,
            light_theme: false,
            cursor_hidden: false,
            blink_restart: true,
            blink_anchor: 0,
        }
    }

    /// Reduced motion keeps the cursor solid (ADR-0089).
    pub fn set_reduced_motion(&mut self, reduced: bool) {
        self.reduced_motion = reduced;
        self.restart_blink();
    }

    /// S5 (#82): recolours every cell still at the reset (SGR 39/49) colour
    /// -- the default text a program never styled -- to the new theme's
    /// default, and forces a full redraw. Text a program explicitly coloured
    /// (an ANSI colour, or a previous theme's default it happens to match
    /// after this call) is left alone, like a real terminal's theme switch.
    pub fn set_light_theme(&mut self, light: bool) {
        if self.light_theme == light {
            return;
        }
        let (old_fg, old_bg) = self.default_colors();
        self.light_theme = light;
        let (new_fg, new_bg) = self.default_colors();
        for cell in self.screen.iter_mut().chain(self.scrollback.iter_mut()) {
            if cell.foreground == old_fg {
                cell.foreground = new_fg;
            }
            if cell.background == old_bg {
                cell.background = new_bg;
            }
        }
        if self.foreground == old_fg {
            self.foreground = new_fg;
        }
        if self.background == old_bg {
            self.background = new_bg;
        }
        self.mark_all_dirty();
    }

    const fn default_colors(&self) -> (u32, u32) {
        if self.light_theme {
            (DEFAULT_FOREGROUND_LIGHT, DEFAULT_BACKGROUND_LIGHT)
        } else {
            (DEFAULT_FOREGROUND_DARK, DEFAULT_BACKGROUND_DARK)
        }
    }

    /// Shows the cursor and restarts its blink phase on the next `blink`.
    fn restart_blink(&mut self) {
        self.blink_restart = true;
        if self.cursor_hidden {
            self.cursor_hidden = false;
            self.mark_cell_dirty(self.cursor_column, self.cursor_row);
        }
    }

    /// Advances the cursor blink: visible for `CURSOR_BLINK_TICKS`, hidden
    /// for as long, restarting visible after any activity and settling
    /// visible once idle. Reduced motion and a scrolled-back view keep it
    /// steady.
    pub fn blink(&mut self, now_ticks: u64) {
        if self.blink_restart {
            self.blink_restart = false;
            self.blink_anchor = now_ticks;
        }
        let elapsed = now_ticks.saturating_sub(self.blink_anchor);
        let hidden = !self.reduced_motion
            && self.view_offset == 0
            && elapsed < CURSOR_BLINK_IDLE_TICKS
            && (elapsed / CURSOR_BLINK_TICKS) % 2 == 1;
        if hidden != self.cursor_hidden {
            self.cursor_hidden = hidden;
            self.mark_cell_dirty(self.cursor_column, self.cursor_row);
        }
    }

    pub const fn cursor(&self) -> (usize, usize) {
        (self.cursor_column, self.cursor_row)
    }

    pub fn reset(&mut self) {
        self.cursor_column = 0;
        self.cursor_row = 0;
        self.saved_cursor_column = 0;
        self.saved_cursor_row = 0;
        self.wrap_pending = false;
        self.cursor_dirty = true;
        self.full_redraw_pending = true;
        self.parser = Parser::new();
        self.foreground = DEFAULT_FOREGROUND;
        self.background = DEFAULT_BACKGROUND;
        self.attributes = 0;
        self.utf8_codepoint = 0;
        self.utf8_remaining = 0;
        self.utf8_min = 0;
        self.scrollback_start = 0;
        self.scrollback_len = 0;
        self.view_offset = 0;
        self.cursor_hidden = false;
        self.blink_restart = true;
        self.screen.fill(blank_cell());
        self.mark_all_dirty();
    }

    /// Forces every cell to redraw on the next `next_grid_row` drain
    /// without otherwise touching cursor/scrollback state. Used when a
    /// hidden session becomes visible again (tab switch, #76): its grid
    /// store on the Display side holds whatever the previously active
    /// session last drew, so it needs a full repaint.
    pub fn force_redraw(&mut self) {
        self.full_redraw_pending = true;
        self.mark_all_dirty();
    }

    pub fn resize(&mut self, columns: usize, rows: usize) {
        self.columns = columns.clamp(1, DEFAULT_COLUMNS);
        self.rows = rows.clamp(1, DEFAULT_ROWS);
        self.cursor_column = self.cursor_column.min(self.columns - 1);
        self.cursor_row = self.cursor_row.min(self.rows - 1);
        self.saved_cursor_column = self.saved_cursor_column.min(self.columns - 1);
        self.saved_cursor_row = self.saved_cursor_row.min(self.rows - 1);
        self.wrap_pending = false;
        self.full_redraw_pending = true;
        self.cursor_dirty = true;
        self.mark_all_dirty();
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        let cursor = (self.cursor_column, self.cursor_row);
        self.show_live_view();
        for &byte in bytes {
            self.feed_byte(byte);
        }
        let moved = cursor != (self.cursor_column, self.cursor_row);
        self.cursor_dirty |= moved;
        if moved {
            self.restart_blink();
        }
        if moved {
            // The cursor is drawn by inverting a cell's colors in
            // `next_grid_row`, not stored in `screen`, so a cursor-only move
            // (no character written) still needs both the vacated and the
            // newly-occupied cell marked dirty to repaint the visible block.
            self.mark_cell_dirty(cursor.0, cursor.1);
            self.mark_cell_dirty(self.cursor_column, self.cursor_row);
        }
    }

    fn mark_cell_dirty(&mut self, column: usize, row: usize) {
        if column < self.columns && row < self.rows {
            self.dirty[Self::index(column, row)] = true;
        }
    }

    fn feed_byte(&mut self, byte: u8) {
        match self.parser.state {
            ParserState::Ground => self.feed_ground_byte(byte),
            ParserState::Escape => match byte {
                b'[' => {
                    self.parser.reset_csi();
                    self.parser.state = ParserState::Csi;
                }
                b'c' => self.reset(),
                _ => self.parser.state = ParserState::Ground,
            },
            ParserState::Csi => self.feed_csi(byte),
        }
    }

    fn feed_ground_byte(&mut self, byte: u8) {
        if self.utf8_remaining != 0 {
            if byte & 0xc0 == 0x80 {
                self.utf8_codepoint = (self.utf8_codepoint << 6) | u32::from(byte & 0x3f);
                self.utf8_remaining -= 1;
                if self.utf8_remaining == 0 {
                    let scalar = self.utf8_codepoint;
                    let valid = scalar >= self.utf8_min
                        && scalar <= 0x10ffff
                        && !(0xd800..=0xdfff).contains(&scalar);
                    self.utf8_codepoint = 0;
                    self.utf8_min = 0;
                    self.put(if valid { scalar } else { REPLACEMENT_SCALAR });
                }
                return;
            }
            self.utf8_codepoint = 0;
            self.utf8_remaining = 0;
            self.utf8_min = 0;
            self.put(REPLACEMENT_SCALAR);
            self.feed_ground_byte(byte);
            return;
        }
        match byte {
            0x1b => {
                self.wrap_pending = false;
                self.parser.state = ParserState::Escape;
            }
            0x08 | 0x7f => {
                self.wrap_pending = false;
                self.cursor_column = self.cursor_column.saturating_sub(1);
            }
            0x09 => {
                self.wrap_pending = false;
                self.cursor_column =
                    ((self.cursor_column / 8) + 1).saturating_mul(8).min(self.columns - 1)
            }
            0x0a..=0x0c => {
                self.wrap_pending = false;
                self.line_feed();
            }
            0x0d => {
                self.wrap_pending = false;
                self.cursor_column = 0;
            }
            0x20..=0x7e => self.put(byte as u32),
            0xc2..=0xdf => {
                self.utf8_codepoint = u32::from(byte & 0x1f);
                self.utf8_remaining = 1;
                self.utf8_min = 0x80;
            }
            0xe0..=0xef => {
                self.utf8_codepoint = u32::from(byte & 0x0f);
                self.utf8_remaining = 2;
                self.utf8_min = 0x800;
            }
            0xf0..=0xf4 => {
                self.utf8_codepoint = u32::from(byte & 0x07);
                self.utf8_remaining = 3;
                self.utf8_min = 0x10000;
            }
            0x80..=0xbf | 0xc0..=0xc1 | 0xf5..=0xff => self.put(REPLACEMENT_SCALAR),
            _ => {}
        }
    }

    fn feed_csi(&mut self, byte: u8) {
        match byte {
            b'0'..=b'9' => {
                self.parser.current =
                    self.parser.current.saturating_mul(10).saturating_add(u16::from(byte - b'0'));
                self.parser.has_current = true;
            }
            b';' => self.parser.push_param(),
            0x40..=0x7e => {
                if self.parser.has_current || self.parser.param_count > 0 {
                    self.parser.push_param();
                }
                self.dispatch_csi(byte);
                self.parser.state = ParserState::Ground;
            }
            _ => self.parser.state = ParserState::Ground,
        }
    }

    fn dispatch_csi(&mut self, final_byte: u8) {
        self.wrap_pending = false;
        match final_byte {
            b'C' => {
                self.cursor_column = self
                    .cursor_column
                    .saturating_add(self.parser.param(0, 1) as usize)
                    .min(self.columns - 1);
            }
            b'D' => {
                self.cursor_column =
                    self.cursor_column.saturating_sub(self.parser.param(0, 1) as usize);
            }
            b'A' => {
                self.cursor_row = self.cursor_row.saturating_sub(self.parser.param(0, 1) as usize);
            }
            b'B' => {
                self.cursor_row = self
                    .cursor_row
                    .saturating_add(self.parser.param(0, 1) as usize)
                    .min(self.rows - 1);
            }
            b'H' | b'f' => {
                self.cursor_row = self.parser.param(0, 1).saturating_sub(1) as usize;
                self.cursor_column = self.parser.param(1, 1).saturating_sub(1) as usize;
                self.cursor_row = self.cursor_row.min(self.rows - 1);
                self.cursor_column = self.cursor_column.min(self.columns - 1);
            }
            b'G' | b'`' => {
                self.cursor_column = self.parser.param(0, 1).saturating_sub(1) as usize;
                self.cursor_column = self.cursor_column.min(self.columns - 1);
            }
            b'd' => {
                self.cursor_row = self.parser.param(0, 1).saturating_sub(1) as usize;
                self.cursor_row = self.cursor_row.min(self.rows - 1);
            }
            b'E' => {
                self.cursor_row = self
                    .cursor_row
                    .saturating_add(self.parser.param(0, 1) as usize)
                    .min(self.rows - 1);
                self.cursor_column = 0;
            }
            b'F' => {
                self.cursor_row = self.cursor_row.saturating_sub(self.parser.param(0, 1) as usize);
                self.cursor_column = 0;
            }
            b's' => {
                self.saved_cursor_column = self.cursor_column;
                self.saved_cursor_row = self.cursor_row;
            }
            b'u' => {
                self.cursor_column = self.saved_cursor_column.min(self.columns - 1);
                self.cursor_row = self.saved_cursor_row.min(self.rows - 1);
            }
            b'J' => self.erase_display(self.parser.param(0, 0)),
            b'K' => self.erase_line(self.parser.param(0, 0)),
            b'm' => self.apply_sgr(),
            _ => {}
        }
    }

    fn apply_sgr(&mut self) {
        if self.parser.param_count == 0 {
            self.reset_style();
            return;
        }
        for index in 0..self.parser.param_count {
            match self.parser.params[index] {
                0 => self.reset_style(),
                1 => self.attributes |= CELL_ATTR_BOLD,
                2 => self.attributes |= CELL_ATTR_DIM,
                4 => self.attributes |= CELL_ATTR_UNDERLINE,
                22 => self.attributes &= !(CELL_ATTR_BOLD | CELL_ATTR_DIM),
                24 => self.attributes &= !CELL_ATTR_UNDERLINE,
                30..=37 => self.foreground = ANSI_COLORS[(self.parser.params[index] - 30) as usize],
                39 => self.foreground = DEFAULT_FOREGROUND,
                40..=47 => self.background = ANSI_COLORS[(self.parser.params[index] - 40) as usize],
                49 => self.background = DEFAULT_BACKGROUND,
                90..=97 => {
                    self.foreground = ANSI_BRIGHT_COLORS[(self.parser.params[index] - 90) as usize]
                }
                100..=107 => {
                    self.background = ANSI_BRIGHT_COLORS[(self.parser.params[index] - 100) as usize]
                }
                _ => {}
            }
        }
    }

    fn reset_style(&mut self) {
        self.foreground = DEFAULT_FOREGROUND;
        self.background = DEFAULT_BACKGROUND;
        self.attributes = 0;
    }

    fn index(column: usize, row: usize) -> usize {
        row * DEFAULT_COLUMNS + column
    }

    fn put(&mut self, codepoint: u32) {
        if self.wrap_pending {
            self.line_feed();
            self.cursor_column = 0;
            self.wrap_pending = false;
        }
        let index = Self::index(self.cursor_column, self.cursor_row);
        self.screen[index] = Cell {
            codepoint,
            foreground: self.foreground,
            background: self.background,
            attributes: self.attributes,
            ..blank_cell()
        };
        self.dirty[index] = true;
        if self.cursor_column + 1 >= self.columns {
            self.cursor_column = self.columns - 1;
            self.wrap_pending = true;
        } else {
            self.cursor_column += 1;
        }
    }

    fn line_feed(&mut self) {
        self.wrap_pending = false;
        if self.cursor_row + 1 >= self.rows {
            self.scroll_up();
        } else {
            self.cursor_row += 1;
        }
    }

    fn scroll_up(&mut self) {
        self.full_redraw_pending = true;
        self.store_scrollback_line(0);
        for row in 0..self.rows - 1 {
            for column in 0..self.columns {
                let source = Self::index(column, row + 1);
                let target = Self::index(column, row);
                self.screen[target] = self.screen[source];
                self.dirty[target] = true;
            }
        }
        for column in 0..self.columns {
            let index = Self::index(column, self.rows - 1);
            self.screen[index] = blank_cell();
            self.dirty[index] = true;
        }
    }

    fn store_scrollback_line(&mut self, row: usize) {
        let slot = if self.scrollback_len < TERMINAL_SCROLLBACK_LINES {
            (self.scrollback_start + self.scrollback_len) % TERMINAL_SCROLLBACK_LINES
        } else {
            let slot = self.scrollback_start;
            self.scrollback_start = (self.scrollback_start + 1) % TERMINAL_SCROLLBACK_LINES;
            slot
        };
        let source = row * DEFAULT_COLUMNS;
        let target = slot * DEFAULT_COLUMNS;
        self.scrollback[target..target + DEFAULT_COLUMNS]
            .copy_from_slice(&self.screen[source..source + DEFAULT_COLUMNS]);
        self.scrollback_len = self.scrollback_len.saturating_add(1).min(TERMINAL_SCROLLBACK_LINES);
    }

    fn show_live_view(&mut self) {
        if self.view_offset != 0 {
            self.view_offset = 0;
            self.mark_all_dirty();
        }
    }

    fn scroll_view(&mut self, lines: isize) {
        let old_offset = self.view_offset;
        if lines.is_positive() {
            self.view_offset =
                self.view_offset.saturating_add(lines as usize).min(self.scrollback_len);
        } else {
            self.view_offset = self.view_offset.saturating_sub(lines.unsigned_abs());
        }
        if old_offset != self.view_offset {
            self.mark_all_dirty();
        }
    }

    fn visible_cell(&self, row: usize, column: usize) -> Cell {
        if self.view_offset == 0 {
            return self.screen[Self::index(column, row)];
        }
        let top_line = self.scrollback_len.saturating_sub(self.view_offset);
        let line = top_line + row;
        if line < self.scrollback_len {
            let slot = (self.scrollback_start + line) % TERMINAL_SCROLLBACK_LINES;
            self.scrollback[slot * DEFAULT_COLUMNS + column]
        } else {
            self.screen[(line - self.scrollback_len) * DEFAULT_COLUMNS + column]
        }
    }

    fn erase_display(&mut self, mode: u16) {
        match mode {
            0 => {
                self.erase_line(0);
                for row in self.cursor_row + 1..self.rows {
                    self.erase_row(row);
                }
            }
            1 => {
                for row in 0..self.cursor_row {
                    self.erase_row(row);
                }
                self.erase_line(1);
            }
            2 | 3 => {
                self.full_redraw_pending = true;
                for row in 0..self.rows {
                    self.erase_row(row);
                }
            }
            _ => {}
        }
    }

    fn erase_row(&mut self, row: usize) {
        for column in 0..self.columns {
            let index = Self::index(column, row);
            self.screen[index] = blank_cell();
            self.dirty[index] = true;
        }
    }

    fn erase_line(&mut self, mode: u16) {
        let (start, end) = match mode {
            0 => (self.cursor_column, self.columns),
            1 => (0, self.cursor_column + 1),
            2 => (0, self.columns),
            _ => return,
        };
        for column in start..end.min(self.columns) {
            let index = Self::index(column, self.cursor_row);
            self.screen[index] = blank_cell();
            self.dirty[index] = true;
        }
    }

    fn mark_all_dirty(&mut self) {
        for row in 0..self.rows {
            for column in 0..self.columns {
                self.dirty[Self::index(column, row)] = true;
            }
        }
    }

    /// Drains one dirty row at a time as a whole-row `GuiTextGridRow`
    /// update (repeated calls drain the dirty set, `None` once every
    /// dirty row has been sent). `surface`/`node_id` are left at
    /// `SurfaceHandle::EMPTY`/0: only Atrium knows this surface's own
    /// text-grid node id, so it fills those in before relaying the row to
    /// Display (#74). A full redraw (reset/resize/`\x1bc`/`\x1b[2J`) marks
    /// every row dirty, so it drains as an ordinary sequence of whole-row
    /// updates — there is no separate "clear" signal to send; a shrunk
    /// grid's stale trailing cells are cleared by the bound store itself
    /// (ADR-0087's `sync_text_grid`) when Atrium republishes the node.
    pub fn next_grid_row(&mut self) -> Option<GuiTextGridRow> {
        for row in 0..self.rows {
            let row_dirty = (0..self.columns).any(|column| self.dirty[Self::index(column, row)]);
            if !row_dirty {
                continue;
            }
            let mut message = GuiTextGridRow::EMPTY;
            message.row = row as u16;
            message.cell_count = self.columns as u16;
            let cursor_column =
                (self.view_offset == 0 && !self.cursor_hidden && row == self.cursor_row)
                    .then_some(self.cursor_column);
            for column in 0..self.columns {
                let index = Self::index(column, row);
                let mut cell = self.visible_cell(row, column);
                if Some(column) == cursor_column {
                    // Solid block cursor: invert the cell's own colors so it
                    // reads correctly against any foreground/background the
                    // program set there; `blink` hides it every other phase.
                    core::mem::swap(&mut cell.foreground, &mut cell.background);
                }
                message.cells[column] = cell;
                self.dirty[index] = false;
            }
            self.full_redraw_pending = false;
            return Some(message);
        }
        self.cursor_dirty = false;
        None
    }

    pub fn input(&mut self, event: &InputMessage) -> Option<IpcBytes> {
        if let Some(flags) = event.appearance_flags() {
            self.set_reduced_motion(flags & APPEARANCE_REDUCED_MOTION != 0);
            self.set_light_theme(flags & APPEARANCE_LIGHT_THEME != 0);
            return None;
        }
        if event.kind != MessageKind::Pointer {
            self.restart_blink();
        }
        if let Some(pointer) = event.pointer_event() {
            // Wheel up (positive) scrolls back; one notch is WHEEL_LINES (ADR-0092).
            self.scroll_view(isize::from(pointer.wheel) * WHEEL_LINES);
            return None;
        }
        if event.kind == MessageKind::Key
            && matches!(event.state, KeyState::Pressed | KeyState::Repeat)
            && event.modifiers & MOD_SHIFT != 0
        {
            match KeyCode::from_raw(event.code) {
                KeyCode::PageUp => {
                    self.scroll_view(self.rows.saturating_sub(1) as isize);
                    return None;
                }
                KeyCode::PageDown => {
                    self.scroll_view(-(self.rows.saturating_sub(1) as isize));
                    return None;
                }
                _ => {}
            }
        }
        if matches!(event.kind, MessageKind::Text | MessageKind::Paste) {
            return IpcBytes::from_bytes(MessageKind::SessionInput, event.text_bytes()?);
        }
        if event.kind != MessageKind::Key || event.state == KeyState::Released {
            return None;
        }
        let code = KeyCode::from_raw(event.code);
        if let Some(byte) = code.character_byte() {
            let byte = modified_character(byte, event.modifiers);
            if event.modifiers & MOD_CTRL != 0 {
                return IpcBytes::from_bytes(MessageKind::SessionInput, &[control_byte(byte)?]);
            }
            if event.modifiers & MOD_ALT != 0 {
                return IpcBytes::from_bytes(MessageKind::SessionInput, &[b'\x1b', byte]);
            }
        }
        let bytes: &[u8] = match code {
            KeyCode::Escape => b"\x1b",
            KeyCode::Enter if event.modifiers & MOD_CTRL != 0 => b"\x17",
            KeyCode::Enter => b"\r",
            KeyCode::Backspace if event.modifiers & MOD_CTRL != 0 => b"\x17",
            KeyCode::Backspace => b"\x7f",
            KeyCode::Tab => b"\t",
            KeyCode::Up => b"\x1b[A",
            KeyCode::Down => b"\x1b[B",
            KeyCode::Left if event.modifiers & MOD_CTRL != 0 => b"\x1b[1;5D",
            KeyCode::Right if event.modifiers & MOD_CTRL != 0 => b"\x1b[1;5C",
            KeyCode::Left => b"\x1b[D",
            KeyCode::Right => b"\x1b[C",
            KeyCode::Home => b"\x1b[H",
            KeyCode::End => b"\x1b[F",
            KeyCode::Delete if event.modifiers & MOD_CTRL != 0 => b"\x1b[3;5~",
            KeyCode::Delete => b"\x1b[3~",
            _ => return None,
        };
        IpcBytes::from_bytes(MessageKind::SessionInput, bytes)
    }
}

fn modified_character(byte: u8, modifiers: u16) -> u8 {
    if byte.is_ascii_alphabetic() {
        let upper = (modifiers & MOD_SHIFT != 0) ^ (modifiers & MOD_CAPS_LOCK != 0);
        if upper { byte.to_ascii_uppercase() } else { byte.to_ascii_lowercase() }
    } else if modifiers & MOD_SHIFT != 0 {
        shifted_ascii(byte)
    } else {
        byte
    }
}

fn control_byte(byte: u8) -> Option<u8> {
    match byte {
        b'?' => Some(0x7f),
        b'a'..=b'z' | b'A'..=b'Z' => Some(byte.to_ascii_uppercase() & 0x1f),
        b' '..=b'_' => Some(byte & 0x1f),
        _ => None,
    }
}

const fn shifted_ascii(byte: u8) -> u8 {
    match byte {
        b'1' => b'!',
        b'2' => b'@',
        b'3' => b'#',
        b'4' => b'$',
        b'5' => b'%',
        b'6' => b'^',
        b'7' => b'&',
        b'8' => b'*',
        b'9' => b'(',
        b'0' => b')',
        b'-' => b'_',
        b'=' => b'+',
        b'[' => b'{',
        b']' => b'}',
        b';' => b':',
        b'\'' => b'"',
        b',' => b'<',
        b'.' => b'>',
        b'/' => b'?',
        b'`' => b'~',
        b'\\' => b'|',
        _ => byte,
    }
}

impl<const CELL_COUNT: usize> Default for TerminalState<CELL_COUNT> {
    fn default() -> Self {
        Self::new()
    }
}

pub type Terminal = TerminalState<{ DEFAULT_COLUMNS * DEFAULT_ROWS }>;

#[cfg(test)]
mod tests {
    use super::*;
    use logos_abi::PointerState;

    fn pane_surface(id: u16) -> SurfaceHandle {
        SurfaceHandle::new(id, 1, 7).unwrap()
    }

    const PANE_BOUNDS: GuiRect = GuiRect::new(0, 0, 1280, 720);

    /// A service whose primary pane is bound to one surface, as it is once
    /// Atrium has admitted Terminal's first surface.
    fn bound_service() -> TerminalService {
        let mut service = TerminalService::new();
        assert!(service.bind_surface(pane_surface(0), PANE_BOUNDS));
        service
    }

    fn drain(terminal: &mut Terminal) -> usize {
        let mut count = 0;
        while terminal.next_grid_row().is_some() {
            count += 1;
        }
        count
    }

    #[test]
    fn text_and_scroll_are_bounded() {
        let mut terminal = Terminal::new();
        drain(&mut terminal);
        terminal.feed(b"hello\nworld");
        assert!(drain(&mut terminal) > 0);
        terminal.feed(b"\x1b[2J");
        assert!(drain(&mut terminal) > 0);
    }

    #[test]
    fn resize_updates_render_dimensions() {
        // Bounds minus `TERMINAL_CHROME_HEIGHT`, `TERMINAL_TAB_BAR_HEIGHT`
        // and `TERMINAL_CONTENT_PADDING` on every edge (#75, #76), matching
        // `terminal_grid_metrics` exactly so Atrium's grid node and this
        // resize can never disagree on shape.
        let mut terminal = bound_service();
        terminal.resize_to_surface(GuiRect::new(0, 0, 640, 352));
        let mut rows_seen = 0;
        while let Some(message) = terminal.next_grid_row() {
            assert_eq!(message.cell_count, 78);
            rows_seen += 1;
        }
        assert_eq!(rows_seen, 17);
    }

    #[test]
    fn session_output_bytes_become_grid_rows() {
        // #74: Terminal content reaches Display as `GuiTextGridRow`s, not
        // raw `RenderCells`. Feeding session bytes should surface as a
        // dirty row whose cells carry the fed codepoints, addressed by
        // row index rather than a flat cell position.
        let mut service = bound_service();
        drain_service(&mut service);
        service.session_output_bytes(0, b"hi");
        let row = service.next_grid_row().unwrap();
        assert_eq!(row.row, 0);
        assert_eq!(row.cells[0].codepoint, b'h' as u32);
        assert_eq!(row.cells[1].codepoint, b'i' as u32);
    }

    fn drain_service(service: &mut TerminalService) {
        while service.next_grid_row().is_some() {}
    }

    #[test]
    fn utf8_text_decodes_to_one_terminal_cell() {
        let mut terminal = Terminal::new();
        terminal.feed(b"\xc3\xa9");
        assert_eq!(terminal.screen[0].codepoint, 0xe9);
        assert_eq!(terminal.cursor(), (1, 0));
    }

    #[test]
    fn full_width_text_keeps_cursor_in_the_last_cell_until_wrap() {
        let mut terminal = Terminal::new();
        terminal.feed(&[b'x'; DEFAULT_COLUMNS]);
        assert_eq!(terminal.cursor(), (DEFAULT_COLUMNS - 1, 0));
        terminal.feed(b"y");
        assert_eq!(terminal.screen[DEFAULT_COLUMNS].codepoint, b'y' as u32);
        assert_eq!(terminal.cursor(), (1, 1));
    }

    #[test]
    fn cursor_renders_on_the_wrapped_row_not_the_last_column_of_the_old_row() {
        // #75 host test: after a wrap, the rendered block cursor follows the
        // cursor to its new (column, row), not the column it wrapped from.
        let mut terminal = Terminal::new();
        drain(&mut terminal);
        terminal.feed(&[b'x'; DEFAULT_COLUMNS]);
        terminal.feed(b"y");
        assert_eq!(terminal.cursor(), (1, 1));
        let mut saw_cursor_on_row1_col1 = false;
        let mut saw_cursor_elsewhere = false;
        while let Some(row) = terminal.next_grid_row() {
            for (column, cell) in row.cells[..row.cell_count as usize].iter().enumerate() {
                let is_cursor = cell.foreground == DEFAULT_BACKGROUND
                    && cell.background == DEFAULT_FOREGROUND
                    && cell.codepoint == b' ' as u32;
                if is_cursor {
                    if row.row == 1 && column == 1 {
                        saw_cursor_on_row1_col1 = true;
                    } else {
                        saw_cursor_elsewhere = true;
                    }
                }
            }
        }
        assert!(saw_cursor_on_row1_col1);
        assert!(!saw_cursor_elsewhere);
    }

    #[test]
    fn cursor_motion_cancels_pending_wrap() {
        let mut terminal = Terminal::new();
        terminal.feed(&[b'x'; DEFAULT_COLUMNS]);
        terminal.feed(b"\x1b[Dz");
        assert_eq!(terminal.screen[DEFAULT_COLUMNS - 2].codepoint, b'z' as u32);
        assert_eq!(terminal.cursor(), (DEFAULT_COLUMNS - 1, 0));
    }

    #[test]
    fn cursor_and_erase_are_bounded() {
        let mut terminal = Terminal::new();
        terminal.feed(b"\x1b[2;4Hx\x1b[2K");
        assert_eq!(terminal.cursor(), (4, 1));
        assert!(drain(&mut terminal) > 0);
    }

    #[test]
    fn ansi_sgr_applies_theme_colors_to_cells() {
        let mut terminal = Terminal::new();
        drain(&mut terminal);
        terminal.feed(b"\x1b[31mred\x1b[0m");
        assert_eq!(terminal.screen[0].foreground, ANSI_COLORS[1]);
        assert_eq!(terminal.screen[3].foreground, DEFAULT_FOREGROUND);
        assert_eq!(terminal.screen[0].background, DEFAULT_BACKGROUND);
    }

    #[test]
    fn page_navigation_uses_bounded_scrollback() {
        let mut terminal = Terminal::new();
        for _ in 0..(DEFAULT_ROWS + 2) {
            terminal.feed(b"x\n");
        }
        drain(&mut terminal);
        // Plain PageUp (no Shift) does not scroll: it's reserved for
        // whatever the running program wants it to mean (#75).
        let unshifted = InputMessage::key(KeyCode::PageUp, KeyState::Pressed, 0);
        assert!(terminal.input(&unshifted).is_none());
        assert_eq!(terminal.view_offset, 0);
        let event = InputMessage::key(KeyCode::PageUp, KeyState::Pressed, MOD_SHIFT);
        assert!(terminal.input(&event).is_none());
        assert!(terminal.view_offset > 0);
        assert!(drain(&mut terminal) > 0);
        let event = InputMessage::key(KeyCode::PageUp, KeyState::Repeat, MOD_SHIFT);
        assert!(terminal.input(&event).is_none());
        let event = InputMessage::key(KeyCode::PageDown, KeyState::Pressed, MOD_SHIFT);
        assert!(terminal.input(&event).is_none());
        assert_eq!(terminal.view_offset, 0);
    }

    #[test]
    fn typing_snaps_the_scrolled_view_back_to_the_bottom() {
        // Ticket acceptance: scrolling up, then any input (a keystroke that
        // reaches the session), snaps the view back to live (offset 0).
        let mut terminal = Terminal::new();
        for _ in 0..(DEFAULT_ROWS + 2) {
            terminal.feed(b"x\n");
        }
        drain(&mut terminal);
        let scroll_up = InputMessage::key(KeyCode::PageUp, KeyState::Pressed, MOD_SHIFT);
        assert!(terminal.input(&scroll_up).is_none());
        assert!(terminal.view_offset > 0);
        terminal.feed(b"y");
        assert_eq!(terminal.view_offset, 0);
    }

    #[test]
    fn scrollback_ring_is_bounded_to_its_configured_line_count() {
        // Ring bounds: feeding far more lines than the configured cap never
        // grows `scrollback_len` past it, and the oldest lines fall off.
        let mut terminal = Terminal::new();
        for line in 0..(TERMINAL_SCROLLBACK_LINES * 3) {
            let byte = b'a' + (line % 26) as u8;
            terminal.feed(&[byte, b'\n']);
        }
        drain(&mut terminal);
        assert_eq!(terminal.scrollback_len, TERMINAL_SCROLLBACK_LINES);
    }

    #[test]
    fn scroll_offset_clamps_to_available_scrollback() {
        // Offset clamping: scrolling up further than the stored history
        // clamps at `scrollback_len`, it never goes negative or past it.
        let mut terminal = Terminal::new();
        for _ in 0..(DEFAULT_ROWS + 4) {
            terminal.feed(b"x\n");
        }
        drain(&mut terminal);
        terminal.scroll_view(isize::MAX);
        assert_eq!(terminal.view_offset, terminal.scrollback_len);
        terminal.scroll_view(isize::MIN);
        assert_eq!(terminal.view_offset, 0);
    }

    #[test]
    fn wheel_scrolls_and_clamps_at_both_ends() {
        let mut terminal = Terminal::new();
        for _ in 0..(DEFAULT_ROWS + 4) {
            terminal.feed(
                b"x
",
            );
        }
        drain(&mut terminal);
        let wheel =
            |delta| InputMessage::pointer_wheel(0, 0, 0, PointerState::Move, delta).unwrap();
        assert!(terminal.input(&wheel(1)).is_none());
        assert_eq!(terminal.view_offset, WHEEL_LINES as usize);
        terminal.input(&wheel(-1));
        assert_eq!(terminal.view_offset, 0);
        terminal.input(&wheel(-8));
        assert_eq!(terminal.view_offset, 0);
        terminal.input(&wheel(i8::MAX));
        terminal.input(&wheel(i8::MAX));
        assert_eq!(terminal.view_offset, terminal.scrollback_len);
        // A plain move is not a scroll.
        terminal.input(&InputMessage::pointer(0, 0, 0, PointerState::Move).unwrap());
        assert_eq!(terminal.view_offset, terminal.scrollback_len);
    }

    #[test]
    fn semantic_input_is_small_and_stable() {
        let mut terminal = Terminal::new();
        let event = InputMessage::key(KeyCode::Up, KeyState::Pressed, 0);
        assert_eq!(terminal.input(&event).unwrap().as_bytes(), Some(&b"\x1b[A"[..]));
    }

    #[test]
    fn ctrl_arrows_use_word_navigation_sequences() {
        let mut terminal = Terminal::new();
        let left = InputMessage::key(KeyCode::Left, KeyState::Pressed, MOD_CTRL);
        assert_eq!(terminal.input(&left).unwrap().as_bytes(), Some(&b"\x1b[1;5D"[..]));
        let right = InputMessage::key(KeyCode::Right, KeyState::Pressed, MOD_CTRL);
        assert_eq!(terminal.input(&right).unwrap().as_bytes(), Some(&b"\x1b[1;5C"[..]));
        let home = InputMessage::key(KeyCode::Home, KeyState::Pressed, 0);
        assert_eq!(terminal.input(&home).unwrap().as_bytes(), Some(&b"\x1b[H"[..]));
        let end = InputMessage::key(KeyCode::End, KeyState::Pressed, 0);
        assert_eq!(terminal.input(&end).unwrap().as_bytes(), Some(&b"\x1b[F"[..]));
    }

    #[test]
    fn ctrl_word_keys_use_delete_sequences() {
        let mut terminal = Terminal::new();
        let backspace = InputMessage::key(KeyCode::Backspace, KeyState::Pressed, MOD_CTRL);
        assert_eq!(terminal.input(&backspace).unwrap().as_bytes(), Some(&b"\x17"[..]));
        let enter = InputMessage::key(KeyCode::Enter, KeyState::Pressed, MOD_CTRL);
        assert_eq!(terminal.input(&enter).unwrap().as_bytes(), Some(&b"\x17"[..]));
        let delete = InputMessage::key(KeyCode::Delete, KeyState::Pressed, MOD_CTRL);
        assert_eq!(terminal.input(&delete).unwrap().as_bytes(), Some(&b"\x1b[3;5~"[..]));
    }

    #[test]
    fn ctrl_c_emits_the_interrupt_byte() {
        let mut terminal = Terminal::new();
        let control_c = InputMessage::key(KeyCode::character(b'c'), KeyState::Pressed, MOD_CTRL);
        assert_eq!(terminal.input(&control_c).unwrap().as_bytes(), Some(&[0x03][..]));
    }

    #[test]
    fn cursor_only_motion_repaints_old_and_new_cursor_rows() {
        // #75: the cursor is a solid block drawn by inverting a cell's own
        // colors, so a cursor-only move (no character written) must still
        // redraw the vacated row and, if different, the newly-occupied row.
        let mut terminal = Terminal::new();
        terminal.feed(b"hello");
        drain(&mut terminal);
        terminal.feed(b"\x1b[2D");
        assert_eq!(terminal.cursor(), (3, 0));
        let row = terminal.next_grid_row().unwrap();
        assert_eq!(row.row, 0);
        // Cursor cell's colors are inverted relative to an ordinary blank cell.
        assert_eq!(row.cells[3].foreground, DEFAULT_BACKGROUND);
        assert_eq!(row.cells[3].background, DEFAULT_FOREGROUND);
        // The character cell one column to the left is untouched.
        assert_eq!(row.cells[2].codepoint, b'l' as u32);
        assert_eq!(row.cells[2].foreground, DEFAULT_FOREGROUND);
        assert!(terminal.next_grid_row().is_none());
    }

    fn cursor_cell_inverted(terminal: &mut Terminal) -> Option<bool> {
        let (column, cursor_row) = terminal.cursor();
        let mut inverted = None;
        while let Some(row) = terminal.next_grid_row() {
            if usize::from(row.row) == cursor_row {
                inverted = Some(row.cells[column].background == DEFAULT_FOREGROUND);
            }
        }
        inverted
    }

    #[test]
    fn cursor_blinks_and_restarts_visible_on_activity() {
        let mut terminal = Terminal::new();
        terminal.feed(b"ab");
        terminal.blink(1_000);
        drain(&mut terminal);
        terminal.blink(1_000 + CURSOR_BLINK_TICKS - 1);
        assert_eq!(cursor_cell_inverted(&mut terminal), None, "still in the visible phase");
        terminal.blink(1_000 + CURSOR_BLINK_TICKS);
        assert_eq!(cursor_cell_inverted(&mut terminal), Some(false), "hidden phase repaints");
        terminal.blink(1_000 + 2 * CURSOR_BLINK_TICKS);
        assert_eq!(cursor_cell_inverted(&mut terminal), Some(true), "visible again");
        terminal.blink(1_000 + 3 * CURSOR_BLINK_TICKS);
        assert_eq!(cursor_cell_inverted(&mut terminal), Some(false));
        // Typing shows the cursor at once and restarts the phase.
        let key = InputMessage::key(KeyCode::Left, KeyState::Pressed, 0);
        let _ = terminal.input(&key);
        assert_eq!(cursor_cell_inverted(&mut terminal), Some(true));
        terminal.blink(5_000);
        terminal.blink(5_000 + CURSOR_BLINK_TICKS - 1);
        assert_eq!(cursor_cell_inverted(&mut terminal), None);
        // Idle long enough and the cursor settles visible for good.
        terminal.blink(5_000 + CURSOR_BLINK_IDLE_TICKS - CURSOR_BLINK_TICKS);
        assert_eq!(cursor_cell_inverted(&mut terminal), Some(false));
        terminal.blink(5_000 + CURSOR_BLINK_IDLE_TICKS);
        assert_eq!(cursor_cell_inverted(&mut terminal), Some(true));
        for later in 1..4 {
            terminal.blink(5_000 + CURSOR_BLINK_IDLE_TICKS + later * CURSOR_BLINK_TICKS);
            assert_eq!(cursor_cell_inverted(&mut terminal), None, "idle, no repaints");
        }
    }

    #[test]
    fn reduced_motion_appearance_keeps_the_cursor_solid() {
        let mut terminal = Terminal::new();
        terminal.feed(b"ab");
        terminal.blink(0);
        drain(&mut terminal);
        terminal.blink(CURSOR_BLINK_TICKS);
        assert_eq!(cursor_cell_inverted(&mut terminal), Some(false));
        assert!(
            terminal.input(&InputMessage::appearance(APPEARANCE_REDUCED_MOTION)).is_none(),
            "appearance never reaches the session"
        );
        assert_eq!(cursor_cell_inverted(&mut terminal), Some(true), "shown immediately");
        for phase in 1..6 {
            terminal.blink(phase * CURSOR_BLINK_TICKS);
            assert_eq!(cursor_cell_inverted(&mut terminal), None, "phase {phase}");
        }
        let _ = terminal.input(&InputMessage::appearance(0));
        terminal.blink(10 * CURSOR_BLINK_TICKS);
        terminal.blink(11 * CURSOR_BLINK_TICKS);
        assert_eq!(cursor_cell_inverted(&mut terminal), Some(false), "blinks again");
    }

    #[test]
    fn light_theme_appearance_recolors_default_cells_but_not_explicit_ansi_colors() {
        let mut terminal = Terminal::new();
        drain(&mut terminal);
        terminal.feed(b"\x1b[31mred\x1b[0m plain");
        assert_eq!(terminal.screen[0].foreground, ANSI_COLORS[1], "explicit red");
        assert_eq!(terminal.screen[4].foreground, DEFAULT_FOREGROUND_DARK, "plain text");
        assert_eq!(terminal.screen[4].background, DEFAULT_BACKGROUND_DARK);

        assert!(terminal.input(&InputMessage::appearance(APPEARANCE_LIGHT_THEME)).is_none());
        assert_eq!(terminal.screen[0].foreground, ANSI_COLORS[1], "still explicit red");
        assert_eq!(terminal.screen[4].foreground, DEFAULT_FOREGROUND_LIGHT, "recoloured");
        assert_eq!(terminal.screen[4].background, DEFAULT_BACKGROUND_LIGHT);
        // New output keeps using the theme that's now current.
        terminal.feed(b" more");
        let more = usize::from(b' ') + 4; // just past "red plain"
        assert_eq!(terminal.screen[more + 1].foreground, DEFAULT_FOREGROUND_LIGHT);

        assert!(terminal.input(&InputMessage::appearance(0)).is_none());
        assert_eq!(terminal.screen[4].foreground, DEFAULT_FOREGROUND_DARK, "back to dark");
        assert_eq!(terminal.screen[0].foreground, ANSI_COLORS[1], "red is untouched throughout");
    }

    #[test]
    fn light_theme_appearance_reaches_every_tab_not_just_the_active_one() {
        let mut service = bound_service();
        service.open_tab();
        assert!(
            service.input(&InputMessage::appearance(APPEARANCE_LIGHT_THEME)).is_none(),
            "appearance never reaches a session's own output"
        );
        assert!(service.sessions[0].terminal.light_theme);
        assert!(service.sessions[1].terminal.light_theme, "background tab is themed too");
    }

    /// WCAG relative luminance of a `0x00RRGGBB` colour (sRGB gamma-corrected).
    fn relative_luminance(rgb: u32) -> f64 {
        let channel = |shift: u32| {
            let c = ((rgb >> shift) & 0xff) as f64 / 255.0;
            if c <= 0.039_28 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * channel(16) + 0.7152 * channel(8) + 0.0722 * channel(0)
    }

    fn contrast_ratio(a: u32, b: u32) -> f64 {
        let (la, lb) = (relative_luminance(a), relative_luminance(b));
        (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
    }

    /// S5 (#82): the reset foreground/background pair meets WCAG AA body
    /// text (>= 4.5:1) in both themes.
    #[test]
    fn default_terminal_colors_meet_wcag_aa_in_both_themes() {
        let dark = contrast_ratio(DEFAULT_FOREGROUND_DARK, DEFAULT_BACKGROUND_DARK);
        assert!(dark >= 4.5, "dark: {dark:.2}");
        let light = contrast_ratio(DEFAULT_FOREGROUND_LIGHT, DEFAULT_BACKGROUND_LIGHT);
        assert!(light >= 4.5, "light: {light:.2}");
    }

    #[test]
    fn cursor_save_and_restore_preserves_the_editing_position() {
        let mut terminal = Terminal::new();
        drain(&mut terminal);
        terminal.feed(b"ab\x1b[sXY\x1b[uZ");
        assert_eq!(terminal.screen[2].codepoint, b'Z' as u32);
        assert_eq!(terminal.cursor(), (3, 0));
    }

    #[test]
    fn completion_row_erase_clears_the_saved_menu_row() {
        let mut terminal = Terminal::new();
        drain(&mut terminal);
        terminal.feed(b"            \x1b[sstatus\x1b[1B\x1b[6Dping()\x1b[u");
        assert_eq!(terminal.cursor(), (12, 0));
        terminal.feed(b"\x1b[s\x1b[1B");
        assert_eq!(terminal.cursor(), (12, 1));
        terminal.feed(b"\x1b[K");
        assert_eq!(terminal.screen[DEFAULT_COLUMNS + 13].codepoint, b' ' as u32);
        terminal.feed(b"\x1b[u");
        assert_eq!(terminal.cursor(), (12, 0));
    }

    #[test]
    fn vertical_cursor_motion_is_bounded() {
        let mut terminal = Terminal::new();
        terminal.feed(b"\x1b[10B\x1b[3A");
        assert_eq!(terminal.cursor(), (0, 7));
        terminal.feed(b"\x1b[99B");
        assert_eq!(terminal.cursor(), (0, DEFAULT_ROWS - 1));
    }

    #[test]
    fn common_absolute_and_relative_cursor_sequences_are_bounded() {
        let mut terminal = Terminal::new();
        terminal.feed(b"\x1b[12G\x1b[8d\x1b[2E");
        assert_eq!(terminal.cursor(), (0, 9));
        terminal.feed(b"\x1b[3F");
        assert_eq!(terminal.cursor(), (0, 6));
    }

    #[test]
    fn render_is_chunked_one_row_at_a_time() {
        let mut terminal = Terminal::new();
        let mut messages = 0;
        while let Some(message) = terminal.next_grid_row() {
            assert_eq!(message.row as usize, messages);
            assert_eq!(message.cell_count as usize, DEFAULT_COLUMNS);
            messages += 1;
        }
        assert_eq!(messages, DEFAULT_ROWS);
        assert!(terminal.next_grid_row().is_none());
    }

    #[test]
    fn first_render_after_reset_redraws_every_row() {
        let mut terminal = Terminal::new();
        assert_eq!(drain(&mut terminal), DEFAULT_ROWS);
        terminal.feed(b"\x1bc");
        assert_eq!(drain(&mut terminal), DEFAULT_ROWS);
    }

    #[test]
    fn full_screen_erase_redraws_every_row() {
        let mut terminal = Terminal::new();
        drain(&mut terminal);
        terminal.feed(b"x\x1b[2J");
        assert_eq!(drain(&mut terminal), DEFAULT_ROWS);
    }

    #[test]
    fn session_cap_is_enforced_at_four_tabs() {
        // #76: the service starts with one tab open; three more can be
        // opened up to MAX_TERMINAL_SESSIONS, and a fifth is refused.
        let mut service = bound_service();
        assert_eq!(service.tab_count(), 1);
        for _ in 0..(MAX_TERMINAL_SESSIONS - 1) {
            assert!(service.open_tab().is_some());
        }
        assert_eq!(service.tab_count(), MAX_TERMINAL_SESSIONS);
        assert!(service.open_tab().is_none());
        assert_eq!(service.tab_count(), MAX_TERMINAL_SESSIONS);
    }

    #[test]
    fn closing_a_tab_frees_and_reuses_its_slot_generation_safely() {
        let mut service = bound_service();
        let second = service.open_tab().unwrap();
        assert_eq!(service.tab_count(), 2);
        assert!(service.close_tab(second).is_some());
        assert_eq!(service.tab_count(), 1);
        // A stale handle into the freed slot is rejected.
        assert!(!service.switch_tab(second));
        assert!(service.close_tab(second).is_none());
        // Reopening reuses the freed slot with a fresh generation.
        let third = service.open_tab().unwrap();
        assert_eq!(third.slot(), second.slot());
        assert_ne!(third, second);
        assert!(service.switch_tab(third));
    }

    #[test]
    fn closing_a_tab_returns_a_session_close_tagged_with_its_slot() {
        let mut service = bound_service();
        let second = service.open_tab().unwrap();
        let message = service.close_tab(second).expect("closing an open tab");
        assert_eq!(message.kind, MessageKind::SessionClose);
        assert_eq!(message.session(), second.slot() as u8);
    }

    #[test]
    fn the_last_remaining_tab_cannot_be_closed() {
        let mut service = bound_service();
        let only = service.active_tab();
        assert!(service.close_tab(only).is_none());
        assert_eq!(service.tab_count(), 1);
    }

    #[test]
    fn input_is_tagged_with_the_active_tabs_slot() {
        // T3c (#98): Session and Flow tell tabs apart by this tag alone,
        // so it must always match whichever tab is focused, not just the
        // first one.
        let mut service = bound_service();
        let second = service.open_tab().unwrap();
        assert_eq!(service.active_tab(), second);
        let key = InputMessage::text(b"a").unwrap();
        let message = service.input(&key).unwrap();
        assert_eq!(message.session(), second.slot() as u8);
    }

    #[test]
    fn output_routes_by_its_tagged_session_not_the_active_tab() {
        // Session/Flow (T3c, #98) tag every reply with the session that
        // produced it; Terminal just trusts the tag, regardless of which
        // tab is currently focused.
        let mut service = bound_service();
        let tab_one = service.active_tab();
        let tab_two = service.open_tab().unwrap();
        assert_eq!(service.active_tab(), tab_two);
        drain_service(&mut service);
        service.session_output_bytes(tab_one.slot() as u8, b"background");
        // Tab two (active) never saw it.
        let mut saw_leak = false;
        while let Some(row) = service.next_grid_row() {
            if row.cells[0].codepoint == b'b' as u32 {
                saw_leak = true;
            }
        }
        assert!(!saw_leak, "output tagged for tab one leaked into the active tab");
        // Tab one (inactive) has it.
        assert!(service.switch_tab(tab_one));
        let mut saw_background = false;
        while let Some(row) = service.next_grid_row() {
            if row.cells[0].codepoint == b'b' as u32 {
                saw_background = true;
            }
        }
        assert!(saw_background, "output never reached the tab named by its tag");
    }

    #[test]
    fn output_for_a_closed_tabs_slot_is_dropped() {
        let mut service = bound_service();
        let tab_two = service.open_tab().unwrap();
        let closed_slot = tab_two.slot() as u8;
        assert!(service.close_tab(tab_two).is_some());
        drain_service(&mut service);
        service.session_output_bytes(closed_slot, b"orphaned");
        let mut saw_orphaned = false;
        while let Some(row) = service.next_grid_row() {
            if row.cells[0].codepoint == b'o' as u32 {
                saw_orphaned = true;
            }
        }
        assert!(!saw_orphaned, "output tagged for a closed slot must be dropped");
    }

    #[test]
    fn ctrl_tab_switches_sessions_without_reaching_the_shell() {
        let mut service = bound_service();
        let first = service.active_tab();
        let second = service.open_tab().unwrap();
        assert_eq!(service.active_tab(), second);
        let ctrl_tab = InputMessage::key(KeyCode::Tab, KeyState::Pressed, MOD_CTRL);
        assert!(service.input(&ctrl_tab).is_none());
        assert_eq!(service.active_tab(), first);
        assert!(service.input(&ctrl_tab).is_none());
        assert_eq!(service.active_tab(), second);
    }

    #[test]
    fn ctrl_shift_t_opens_a_new_tab_without_reaching_the_shell() {
        let mut service = bound_service();
        assert_eq!(service.tab_count(), 1);
        let new_tab =
            InputMessage::key(KeyCode::character(b't'), KeyState::Pressed, MOD_CTRL | MOD_SHIFT);
        assert!(service.input(&new_tab).is_none());
        assert_eq!(service.tab_count(), 2);
        assert_eq!(service.active_tab().slot(), 1);
    }

    #[test]
    fn grid_rows_are_addressed_directly_not_by_flat_position() {
        let mut terminal = Terminal::new();
        let mut saw_second_row = false;
        while let Some(message) = terminal.next_grid_row() {
            if message.row == 1 {
                saw_second_row = true;
            }
        }
        assert!(saw_second_row);
    }

    // T3b (#97): panes.

    #[test]
    fn two_surfaces_map_to_distinct_sessions() {
        let mut service = bound_service();
        let first = service.active_tab();
        assert!(service.bind_surface(pane_surface(1), PANE_BOUNDS));
        let second = service.active_tab();
        assert_ne!(first.slot(), second.slot());
        // Each pane lists only its own session.
        assert_eq!(service.open_bitmap(), 1 << second.slot());
        assert!(service.select(pane_surface(0)));
        assert_eq!(service.open_bitmap(), 1 << first.slot());
        assert_eq!(service.open_session_count(), 2);
        // Tab state reported per surface.
        let (surface, state) = service.pane_report(1).unwrap();
        assert_eq!(surface, pane_surface(1));
        assert_eq!(logos_abi::terminal_tab_open_bitmap(state), 1 << second.slot());
    }

    #[test]
    fn output_and_rows_stay_with_their_own_pane() {
        let mut service = bound_service();
        assert!(service.bind_surface(pane_surface(1), PANE_BOUNDS));
        drain_service(&mut service);
        service.session_output_bytes(0, b"left");
        service.session_output_bytes(1, b"right");
        let mut rows = std::vec::Vec::new();
        while let Some(row) = service.next_grid_row() {
            if row.cells[0].codepoint != b' ' as u32 {
                rows.push((row.surface, row.cells[0].codepoint));
            }
        }
        assert!(rows.contains(&(pane_surface(0), b'l' as u32)));
        assert!(rows.contains(&(pane_surface(1), b'r' as u32)));
        assert!(!rows.contains(&(pane_surface(0), b'r' as u32)));
        assert!(!rows.contains(&(pane_surface(1), b'l' as u32)));
    }

    #[test]
    fn input_routes_only_to_the_selected_panes_session() {
        let mut service = bound_service();
        assert!(service.bind_surface(pane_surface(1), PANE_BOUNDS));
        let key = InputMessage::text(b"a").unwrap();
        assert!(service.select(pane_surface(0)));
        assert_eq!(service.input(&key).unwrap().session(), 0);
        assert!(service.select(pane_surface(1)));
        assert_eq!(service.input(&key).unwrap().session(), 1);
        // An unknown surface selects nothing and leaves the focus alone.
        assert!(!service.select(pane_surface(5)));
        assert_eq!(service.input(&key).unwrap().session(), 1);
    }

    #[test]
    fn tab_operations_cannot_reach_another_panes_sessions() {
        let mut service = bound_service();
        let left = service.active_tab();
        assert!(service.bind_surface(pane_surface(1), PANE_BOUNDS));
        // The right pane cannot switch to or close the left pane's tab.
        assert!(!service.switch_tab(left));
        assert!(service.close_tab(left).is_none());
        assert!(service.tab_at(left.slot()).is_none());
        // Ctrl+Tab cycles only inside its own pane.
        let only = service.active_tab();
        service.next_tab();
        assert_eq!(service.active_tab(), only);
    }

    #[test]
    fn the_session_cap_is_shared_across_tabs_and_panes() {
        let mut service = bound_service();
        assert!(service.open_tab().is_some());
        assert!(service.open_tab().is_some());
        assert!(service.bind_surface(pane_surface(1), PANE_BOUNDS));
        assert_eq!(service.open_session_count(), MAX_TERMINAL_SESSIONS);
        // No session left for a tab or for a third pane.
        assert!(service.open_tab().is_none());
        assert!(!service.bind_surface(pane_surface(2), PANE_BOUNDS));
        assert!(service.select(pane_surface(0)));
        assert!(service.open_tab().is_none());
        assert_eq!(service.open_session_count(), MAX_TERMINAL_SESSIONS);
    }

    #[test]
    fn closing_a_pane_frees_its_sessions_generation_safely() {
        let mut service = bound_service();
        assert!(service.bind_surface(pane_surface(1), PANE_BOUNDS));
        let right = service.active_tab();
        let closes = service.unbind_surface(pane_surface(1));
        let closed: std::vec::Vec<_> = closes.iter().flatten().collect();
        assert_eq!(closed.len(), 1);
        assert_eq!(closed[0].kind, MessageKind::SessionClose);
        assert_eq!(closed[0].session(), right.slot() as u8);
        assert_eq!(service.open_session_count(), 1);
        // The surviving pane is current again and the stale handle is dead.
        assert_eq!(service.active_tab().slot(), 0);
        assert!(!service.switch_tab(right));
        // Output for the freed slot is dropped.
        drain_service(&mut service);
        service.session_output_bytes(right.slot() as u8, b"ghost");
        assert!(service.next_grid_row().is_none());
        // A new pane reuses the slot under a fresh generation.
        assert!(service.bind_surface(pane_surface(2), PANE_BOUNDS));
        let reused = service.active_tab();
        assert_eq!(reused.slot(), right.slot());
        assert_ne!(reused, right);
    }

    #[test]
    fn closing_the_last_surface_keeps_its_sessions_for_the_next_one() {
        let mut service = bound_service();
        assert!(service.open_tab().is_some());
        let closes = service.unbind_surface(pane_surface(0));
        assert!(closes.iter().all(Option::is_none));
        assert_eq!(service.open_session_count(), 2);
        assert!(service.next_grid_row().is_none());
        // Reopening Terminal reattaches the same two tabs.
        assert!(service.bind_surface(pane_surface(3), PANE_BOUNDS));
        assert_eq!(service.tab_count(), 2);
        assert_eq!(service.open_session_count(), 2);
    }

    #[test]
    fn closing_a_pane_with_several_tabs_closes_each_of_them() {
        let mut service = bound_service();
        assert!(service.bind_surface(pane_surface(1), PANE_BOUNDS));
        assert!(service.open_tab().is_some());
        assert!(service.open_tab().is_some());
        let closes = service.unbind_surface(pane_surface(1));
        assert_eq!(closes.iter().flatten().count(), 3);
        assert_eq!(service.open_session_count(), 1);
    }

    #[test]
    fn resizing_a_pane_resizes_only_its_own_sessions() {
        let mut service = bound_service();
        assert!(service.bind_surface(pane_surface(1), PANE_BOUNDS));
        let (columns, rows, _) = terminal_grid_metrics(PANE_BOUNDS);
        let small = GuiRect::new(0, 0, 400, 300);
        let (small_columns, small_rows, _) = terminal_grid_metrics(small);
        assert!(small_columns < columns && small_rows < rows);
        assert!(service.select(pane_surface(0)));
        service.resize_to_surface(small);
        assert_eq!(service.sessions[0].terminal.columns, small_columns);
        assert_eq!(service.sessions[1].terminal.columns, columns);
    }

    #[test]
    fn rows_alternate_between_busy_panes() {
        let mut service = bound_service();
        assert!(service.bind_surface(pane_surface(1), PANE_BOUNDS));
        service.session_output_bytes(0, b"a");
        service.session_output_bytes(1, b"b");
        let first = service.next_grid_row().unwrap().surface;
        let second = service.next_grid_row().unwrap().surface;
        assert_ne!(first, second);
    }
}
