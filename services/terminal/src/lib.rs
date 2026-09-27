#![no_std]

//! Bounded fixed-size terminal emulator.

#[cfg(test)]
extern crate std;

use logos_abi::{
    APPEARANCE_REDUCED_MOTION, CELL_ATTR_BOLD, CELL_ATTR_DIM, CELL_ATTR_UNDERLINE, Cell,
    DEFAULT_COLUMNS, DEFAULT_ROWS, GuiRect, GuiTextGridRow, InputMessage, IpcBytes, KeyCode,
    KeyState, MOD_ALT, MOD_CAPS_LOCK, MOD_CTRL, MOD_SHIFT, MessageKind, terminal_grid_metrics,
};

const MAX_PARAMS: usize = 16;
const REPLACEMENT_SCALAR: u32 = 0xfffd;
/// Service-local storage cap; the ABI maximum is a protocol-wide ceiling.
pub const TERMINAL_SCROLLBACK_LINES: usize = 64;
/// Half a blink period in timer ticks (100 Hz): 500 ms on, 500 ms off.
pub const CURSOR_BLINK_TICKS: u64 = 50;
/// Like GTK's cursor-blink-timeout: after 10 s without activity the cursor
/// stays solid, so an idle Terminal stops repainting.
pub const CURSOR_BLINK_IDLE_TICKS: u64 = 1_000;
const DEFAULT_FOREGROUND: u32 = 0x00d7_e3f4;
const DEFAULT_BACKGROUND: u32 = 0x000b_1020;
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
    cursor_hidden: bool,
    blink_restart: bool,
    blink_anchor: u64,
}

/// T3 (#76): the Terminal service hosts up to this many independent
/// sessions (grid + scrollback each), with a tab bar to switch between
/// them. The cap is fixed; no reordering or drag-out (out of scope).
pub const MAX_TERMINAL_SESSIONS: usize = 4;

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
}

impl Session {
    const fn new() -> Self {
        Self { terminal: TerminalState::new(), open: false, generation: 0 }
    }

    fn handle(&self, slot: usize) -> TabHandle {
        TabHandle { slot: slot as u8, generation: self.generation }
    }
}

/// Session's own prompt, appended to the end of a command's output once it
/// finishes (see `services/session/src/lib.rs`'s `PROMPT`). Recognizing it
/// here is how Terminal knows a tab's in-flight command is done and output
/// should stop being pinned to that tab (#76 follow-up: route output to the
/// submitting tab, not just the active one).
const SHELL_PROMPT: &[u8] = b"\x1b[36mlogos\x1b[0m \x1b[33m>\x1b[0m ";

pub struct TerminalService {
    sessions: [Session; MAX_TERMINAL_SESSIONS],
    active: usize,
    /// The tab whose command is currently running, set when that tab sends
    /// Enter and cleared once its reply's trailing prompt is seen (or the
    /// tab is closed first). `None` means output follows the active tab,
    /// same as before a command is submitted.
    running_command: Option<TabHandle>,
}

const _: () =
    assert!(core::mem::size_of::<TerminalService>() <= logos_abi::MAX_SERVICE_IMAGE_BYTES);

impl TerminalService {
    pub const fn new() -> Self {
        const SESSION: Session = Session::new();
        let mut sessions = [SESSION; MAX_TERMINAL_SESSIONS];
        sessions[0].open = true;
        sessions[0].generation = 1;
        Self { sessions, active: 0, running_command: None }
    }

    fn active_session(&mut self) -> &mut TerminalState<{ DEFAULT_COLUMNS * DEFAULT_ROWS }> {
        &mut self.sessions[self.active].terminal
    }

    /// The active tab's handle; always valid since the service always keeps
    /// at least one tab open.
    pub fn active_tab(&self) -> TabHandle {
        self.sessions[self.active].handle(self.active)
    }

    pub const fn tab_count(&self) -> usize {
        let mut count = 0;
        let mut slot = 0;
        while slot < MAX_TERMINAL_SESSIONS {
            if self.sessions[slot].open {
                count += 1;
            }
            slot += 1;
        }
        count
    }

    /// Handle of the open tab at `slot`, for tab-bar rendering; `None` if
    /// that slot is not currently open.
    pub fn tab_at(&self, slot: usize) -> Option<TabHandle> {
        self.sessions.get(slot).filter(|session| session.open).map(|session| session.handle(slot))
    }

    /// Which slots are open, one bit per slot (bit 0 = slot 0), for
    /// reporting tab-bar state to Atrium's scene builder.
    pub fn open_bitmap(&self) -> u8 {
        let mut bitmap = 0u8;
        for (slot, session) in self.sessions.iter().enumerate() {
            if session.open {
                bitmap |= 1 << slot;
            }
        }
        bitmap
    }

    pub const fn active_slot(&self) -> usize {
        self.active
    }

    /// Opens a new tab (a fresh session, reset to the current surface
    /// bounds) and makes it active. `None` once `MAX_TERMINAL_SESSIONS` are
    /// already open.
    pub fn open_tab(&mut self) -> Option<TabHandle> {
        let (columns, rows) = {
            let active = &self.sessions[self.active].terminal;
            (active.columns, active.rows)
        };
        let slot = self.sessions.iter().position(|session| !session.open)?;
        let session = &mut self.sessions[slot];
        session.terminal = TerminalState::new();
        session.terminal.resize(columns, rows);
        session.open = true;
        session.generation = session.generation.wrapping_add(1).max(1);
        self.active = slot;
        self.sessions[slot].terminal.force_redraw();
        Some(self.sessions[slot].handle(slot))
    }

    /// Closes `handle`'s tab, freeing its slot for reuse (the slot's
    /// generation is bumped, so a stale handle into it is rejected). Never
    /// closes the last remaining tab. Picks a new active tab if the closed
    /// one was active.
    pub fn close_tab(&mut self, handle: TabHandle) -> bool {
        if !self.is_valid(handle) || self.tab_count() <= 1 {
            return false;
        }
        let slot = handle.slot();
        self.sessions[slot].open = false;
        self.sessions[slot].generation = self.sessions[slot].generation.wrapping_add(1).max(1);
        if self.active == slot {
            self.active = (0..MAX_TERMINAL_SESSIONS)
                .find(|&candidate| self.sessions[candidate].open)
                .unwrap_or(0);
            self.sessions[self.active].terminal.force_redraw();
        }
        true
    }

    /// Switches the active tab by click. Rejects a stale or closed handle.
    pub fn switch_tab(&mut self, handle: TabHandle) -> bool {
        if !self.is_valid(handle) {
            return false;
        }
        if self.active != handle.slot() {
            self.active = handle.slot();
            self.active_session().force_redraw();
        }
        true
    }

    /// Ctrl+Tab: switches to the next open tab, wrapping around.
    pub fn next_tab(&mut self) {
        let mut slot = (self.active + 1) % MAX_TERMINAL_SESSIONS;
        while !self.sessions[slot].open {
            slot = (slot + 1) % MAX_TERMINAL_SESSIONS;
        }
        self.active = slot;
        self.active_session().force_redraw();
    }

    fn is_valid(&self, handle: TabHandle) -> bool {
        handle.is_valid()
            && handle.slot() < MAX_TERMINAL_SESSIONS
            && self.sessions[handle.slot()].open
            && self.sessions[handle.slot()].generation == handle.generation
    }

    /// Input routes only to the active session; Ctrl+Tab switches tabs and
    /// Ctrl+Shift+T opens a new one (a conventional accelerator alongside
    /// the tab bar's own new-tab button and per-tab close control),
    /// instead of reaching the shell.
    pub fn input(&mut self, event: &InputMessage) -> Option<IpcBytes> {
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
        let outgoing = self.active_session().input(event);
        // Enter submits the active tab's line as a command (Session's line
        // editor starts running it on `\r`); pin output to this tab until
        // its reply's prompt comes back, so background output from another
        // tab's still-running command can't leak into whatever the user
        // switches to next.
        if matches!(outgoing.as_ref().and_then(IpcBytes::as_bytes), Some(b"\r")) {
            self.running_command = Some(self.active_tab());
        }
        outgoing
    }

    pub fn session_output(&mut self, message: &IpcBytes) {
        if let Some(bytes) = message.as_bytes() {
            self.session_output_bytes(bytes);
        }
    }

    /// Routes output to whichever tab submitted the command still in
    /// flight, if any, rather than always the active tab: a background
    /// command's own output keeps landing in (and scrolling) its own tab
    /// even while another tab is focused. Output for a tab that was closed
    /// while its command was still running is dropped (the closed slot's
    /// bumped generation is what makes `is_valid` catch this). Once the
    /// owning tab's reply reaches its trailing prompt, routing reverts to
    /// following the active tab, same as before any command was submitted.
    pub fn session_output_bytes(&mut self, bytes: &[u8]) {
        let target = match self.running_command {
            Some(handle) if !self.is_valid(handle) => {
                self.running_command = None;
                return;
            }
            Some(handle) => handle.slot(),
            None => self.active,
        };
        if bytes.windows(SHELL_PROMPT.len()).any(|window| window == SHELL_PROMPT) {
            self.running_command = None;
        }
        self.sessions[target].terminal.feed(bytes);
    }

    pub fn reset(&mut self) {
        self.active_session().reset();
    }

    pub fn resize_to_surface(&mut self, bounds: GuiRect) {
        // Shared with Atrium's `TextGrid` scene-node sizing (#75) so both
        // sides always agree on the grid shape; see `terminal_grid_metrics`.
        let (columns, rows, _) = terminal_grid_metrics(bounds);
        self.active_session().resize(columns, rows);
    }

    pub fn next_grid_row(&mut self) -> Option<GuiTextGridRow> {
        self.active_session().next_grid_row()
    }

    pub fn blink(&mut self, now_ticks: u64) {
        self.active_session().blink(now_ticks);
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
            return None;
        }
        if event.kind != MessageKind::Pointer {
            self.restart_blink();
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
        let mut terminal = TerminalService::new();
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
        let mut service = TerminalService::new();
        drain_service(&mut service);
        service.session_output_bytes(b"hi");
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
        let mut service = TerminalService::new();
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
        let mut service = TerminalService::new();
        let second = service.open_tab().unwrap();
        assert_eq!(service.tab_count(), 2);
        assert!(service.close_tab(second));
        assert_eq!(service.tab_count(), 1);
        // A stale handle into the freed slot is rejected.
        assert!(!service.switch_tab(second));
        assert!(!service.close_tab(second));
        // Reopening reuses the freed slot with a fresh generation.
        let third = service.open_tab().unwrap();
        assert_eq!(third.slot(), second.slot());
        assert_ne!(third, second);
        assert!(service.switch_tab(third));
    }

    #[test]
    fn the_last_remaining_tab_cannot_be_closed() {
        let mut service = TerminalService::new();
        let only = service.active_tab();
        assert!(!service.close_tab(only));
        assert_eq!(service.tab_count(), 1);
    }

    #[test]
    fn input_routes_only_to_the_active_session() {
        let mut service = TerminalService::new();
        let first = service.active_tab();
        service.session_output_bytes(b"one");
        let second = service.open_tab().unwrap();
        assert_eq!(service.active_tab(), second);
        service.session_output_bytes(b"two");
        // Switching back to the first tab redraws its own, untouched grid:
        // "one" is still there and "two" never reached it.
        assert!(service.switch_tab(first));
        drain_service(&mut service);
        service.session_output_bytes(b"?");
        let mut saw_one = false;
        while let Some(row) = service.next_grid_row() {
            if row.cells[0].codepoint == b'o' as u32 {
                saw_one = true;
            }
            assert_ne!(row.cells[0].codepoint, b't' as u32, "tab two's output leaked into tab one");
        }
        assert!(saw_one);
    }

    #[test]
    fn output_from_a_submitted_command_follows_its_own_tab_while_another_is_active() {
        // Submit in tab one (Enter pins output to it), switch to a new tab
        // two, then deliver "background" output: it must land in tab one,
        // not tab two, even though tab two is the one now focused.
        let mut service = TerminalService::new();
        let tab_one = service.active_tab();
        let enter = InputMessage::key(KeyCode::Enter, KeyState::Pressed, 0);
        assert_eq!(service.input(&enter).unwrap().as_bytes(), Some(&b"\r"[..]));
        let tab_two = service.open_tab().unwrap();
        assert_eq!(service.active_tab(), tab_two);
        drain_service(&mut service);
        service.session_output_bytes(b"background");
        // Tab two (still active) never saw it.
        let mut saw_leak = false;
        while let Some(row) = service.next_grid_row() {
            if row.cells[0].codepoint == b'b' as u32 {
                saw_leak = true;
            }
        }
        assert!(!saw_leak, "background output leaked into the active tab");
        // Tab one (inactive) has it.
        assert!(service.switch_tab(tab_one));
        drain_service(&mut service);
        service.session_output_bytes(b"?");
        let mut saw_background = false;
        while let Some(row) = service.next_grid_row() {
            if row.cells[0].codepoint == b'b' as u32 {
                saw_background = true;
            }
        }
        assert!(saw_background, "background output never reached the tab that submitted it");
    }

    #[test]
    fn output_for_a_running_command_is_dropped_once_its_tab_is_closed() {
        // Submit in tab two, close it while its command is still "running",
        // then deliver output: the generation check must reject the stale
        // handle and the bytes are dropped rather than landing anywhere
        // (in particular not the now-active tab one).
        let mut service = TerminalService::new();
        let tab_one = service.active_tab();
        let tab_two = service.open_tab().unwrap();
        let enter = InputMessage::key(KeyCode::Enter, KeyState::Pressed, 0);
        assert_eq!(service.input(&enter).unwrap().as_bytes(), Some(&b"\r"[..]));
        assert!(service.switch_tab(tab_one));
        assert!(service.close_tab(tab_two));
        drain_service(&mut service);
        service.session_output_bytes(b"orphaned");
        let mut saw_orphaned = false;
        while let Some(row) = service.next_grid_row() {
            if row.cells[0].codepoint == b'o' as u32 {
                saw_orphaned = true;
            }
        }
        assert!(!saw_orphaned, "output for a closed tab's command must be dropped, not rerouted");
        // Routing is back to normal (the active tab) for anything after.
        service.session_output_bytes(b"z");
        let mut saw_z = false;
        while let Some(row) = service.next_grid_row() {
            if row.cells[0].codepoint == b'z' as u32 {
                saw_z = true;
            }
        }
        assert!(saw_z, "routing should fall back to the active tab after the drop");
    }

    #[test]
    fn ctrl_tab_switches_sessions_without_reaching_the_shell() {
        let mut service = TerminalService::new();
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
        let mut service = TerminalService::new();
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
}
