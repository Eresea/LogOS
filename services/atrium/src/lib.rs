#![no_std]

#[cfg(test)]
extern crate std;

use core::fmt::Write;

use logos_abi::{
    GuiRect, InputMessage, KeyCode, KeyState, MOD_ALT, MOD_CTRL, MOD_META, MOD_SHIFT, MessageKind,
    PointerState, ServiceHandle, SurfaceHandle,
};

pub const MAX_ATRIUM_SURFACES: usize = logos_abi::MAX_GUI_SURFACES;
const MAX_LAYOUT_NODES: usize = MAX_ATRIUM_SURFACES * 2 - 1;
const DEFAULT_SPLIT_RATIO: u16 = 512;
const MIN_PANE_WIDTH: u32 = 160;
const MIN_PANE_HEIGHT: u32 = 96;
const SPLITTER_HIT_RADIUS: i32 = 6;
pub const MAX_CALCULATOR_TEXT: usize = 32;
pub const SURFACE_MOVE_STEP: i32 = 32;
pub const FULLSCREEN_SURFACE_BOUNDS: GuiRect = GuiRect::new(
    0,
    0,
    logos_abi::DEFAULT_SCREEN_WIDTH as u32,
    logos_abi::DEFAULT_SCREEN_HEIGHT as u32,
);
pub const TERMINAL_SURFACE_BOUNDS: GuiRect = FULLSCREEN_SURFACE_BOUNDS;
pub const STATUS_BAR_BOUNDS: GuiRect =
    GuiRect::new(0, 0, logos_abi::DEFAULT_SCREEN_WIDTH as u32, 32);
pub const STATUS_BAR_CLOSE_BOUNDS: GuiRect = GuiRect::new(1200, 0, 80, 32);
pub const CALCULATOR_BUTTON_LEFT: i32 = 20;
pub const CALCULATOR_BUTTON_TOP: i32 = 100;
pub const CALCULATOR_BUTTON_WIDTH: i32 = 56;
pub const CALCULATOR_BUTTON_HEIGHT: i32 = 20;
pub const CALCULATOR_BUTTON_GAP: i32 = 8;
pub const CALCULATOR_BUTTON_LABELS: [u8; 16] = *b"789/456*123-0.=+";

/// Collapse consecutive pointer motion while preserving keyboard and button
/// transition ordering. The receiver returns one queued event at a time.
pub fn coalesce_pointer_move<F>(
    first: InputMessage,
    receive: &mut F,
) -> (InputMessage, Option<InputMessage>)
where
    F: FnMut(&mut InputMessage) -> bool,
{
    let is_move = |event: InputMessage| {
        event.pointer_event().is_some_and(|pointer| pointer.state == PointerState::Move)
    };
    if !is_move(first) {
        return (first, None);
    }

    let mut latest = first;
    let mut next = first;
    while receive(&mut next) {
        if is_move(next) {
            latest = next;
        } else {
            return (latest, Some(next));
        }
    }
    (latest, None)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AtriumPhase {
    Boot,
    Locked,
    Home,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum AppId {
    Calculator = 1,
    Files = 2,
    Terminal = 3,
    System = 4,
}

pub const COMMAND_MENU_ITEMS: [AppId; 4] =
    [AppId::Calculator, AppId::Files, AppId::Terminal, AppId::System];
pub const COMMAND_MENU_LABELS: [&[u8]; 4] = [b"Calculator", b"Files", b"Terminal", b"System"];
pub const COMMAND_MENU_BOUNDS: GuiRect = GuiRect::new(320, 120, 640, 560);
pub const COMMAND_MENU_ITEM_LEFT: i32 = 384;
pub const COMMAND_MENU_ITEM_TOP: i32 = 304;
pub const COMMAND_MENU_ITEM_WIDTH: u32 = 512;
pub const COMMAND_MENU_ITEM_HEIGHT: u32 = 64;
pub const COMMAND_MENU_ITEM_GAP: i32 = 12;

pub const fn surface_close_bounds(surface: GuiRect) -> GuiRect {
    GuiRect::new(
        surface.width.saturating_sub(STATUS_BAR_CLOSE_BOUNDS.width) as i32,
        0,
        STATUS_BAR_CLOSE_BOUNDS.width,
        STATUS_BAR_CLOSE_BOUNDS.height,
    )
}

pub const fn command_menu_item_bounds(index: usize) -> GuiRect {
    GuiRect::new(
        COMMAND_MENU_ITEM_LEFT,
        COMMAND_MENU_ITEM_TOP
            + index as i32 * (COMMAND_MENU_ITEM_HEIGHT as i32 + COMMAND_MENU_ITEM_GAP),
        COMMAND_MENU_ITEM_WIDTH,
        COMMAND_MENU_ITEM_HEIGHT,
    )
}

fn query_matches(label: &[u8], query: &[u8]) -> bool {
    if query.is_empty() {
        return true;
    }
    label.windows(query.len()).any(|window| {
        window.iter().zip(query.iter()).all(|(label, query)| label.eq_ignore_ascii_case(query))
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SurfaceMode {
    Tiled,
    Floating,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SplitDirection {
    Vertical,
    Horizontal,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LayoutNode {
    Leaf {
        parent: Option<usize>,
        surface_id: Option<u16>,
    },
    Split {
        parent: Option<usize>,
        direction: SplitDirection,
        ratio: u16,
        first: usize,
        second: usize,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Surface {
    pub id: u16,
    pub app: AppId,
    pub client: ServiceHandle,
    pub reference: SurfaceHandle,
    pub bounds: GuiRect,
    pub mode: SurfaceMode,
    pub focused: bool,
    focus_order: u32,
}

#[derive(Debug, Eq, PartialEq)]
pub struct SurfaceRequest {
    app: AppId,
    client: ServiceHandle,
    bounds: GuiRect,
    mode: SurfaceMode,
    anchor: Option<u16>,
    target_leaf: Option<usize>,
    direction: SplitDirection,
}

impl SurfaceRequest {
    pub const fn app(&self) -> AppId {
        self.app
    }

    pub const fn client(&self) -> ServiceHandle {
        self.client
    }

    pub const fn bounds(&self) -> GuiRect {
        self.bounds
    }

    pub const fn mode(&self) -> SurfaceMode {
        self.mode
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AtriumError {
    Locked,
    Capacity,
    NotFound,
    InvalidSurface,
    AlreadyRegistered,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AtriumAction {
    None,
    LauncherChanged,
    Launch(AppId),
    FocusNext,
    FocusPrevious,
    MoveFocused(i32, i32),
    Split(SplitDirection),
    CloseFocused,
    Logout,
}

impl AtriumAction {
    pub const fn routes_to_surface(self) -> bool {
        matches!(self, Self::None)
    }
}

pub struct Atrium {
    phase: AtriumPhase,
    surfaces: [Option<Surface>; MAX_ATRIUM_SURFACES],
    focused: Option<usize>,
    pointer_capture: Option<SurfaceHandle>,
    splitter_capture: Option<usize>,
    splitter_last_position: (i32, i32),
    layout_nodes: [Option<LayoutNode>; MAX_LAYOUT_NODES],
    layout_root: Option<usize>,
    next_split: SplitDirection,
    command_menu: logos_ui::UiCommandMenu,
    command_menu_matches: [u8; COMMAND_MENU_ITEMS.len()],
    next_surface_id: u16,
    next_focus_order: u32,
    home_surface: SurfaceHandle,
    lock_surface: SurfaceHandle,
}

impl Atrium {
    pub const fn new() -> Self {
        Self {
            phase: AtriumPhase::Boot,
            surfaces: [None; MAX_ATRIUM_SURFACES],
            focused: None,
            pointer_capture: None,
            splitter_capture: None,
            splitter_last_position: (0, 0),
            layout_nodes: [None; MAX_LAYOUT_NODES],
            layout_root: None,
            next_split: SplitDirection::Vertical,
            command_menu: logos_ui::UiCommandMenu::new(COMMAND_MENU_ITEMS.len() as u8),
            command_menu_matches: [0, 1, 2, 3],
            next_surface_id: 1,
            next_focus_order: 1,
            home_surface: SurfaceHandle::EMPTY,
            lock_surface: SurfaceHandle::EMPTY,
        }
    }

    pub const fn phase(&self) -> AtriumPhase {
        self.phase
    }

    pub const fn launcher_index(&self) -> usize {
        let selected = self.command_menu.selected() as usize;
        if selected < self.command_menu.item_count() as usize {
            self.command_menu_matches[selected] as usize
        } else {
            0
        }
    }

    pub const fn launcher_result_count(&self) -> usize {
        self.command_menu.item_count() as usize
    }

    pub const fn launcher_result_app(&self, index: usize) -> Option<AppId> {
        if index >= self.launcher_result_count() || index >= self.command_menu_matches.len() {
            return None;
        }
        Some(COMMAND_MENU_ITEMS[self.command_menu_matches[index] as usize])
    }

    pub fn launcher_query(&self) -> logos_ui::UiText {
        self.command_menu.query()
    }

    pub const fn launcher_app(&self) -> AppId {
        COMMAND_MENU_ITEMS[self.launcher_index()]
    }

    pub fn command_menu_item_at(&mut self, x: i32, y: i32) -> Option<AppId> {
        if self.phase != AtriumPhase::Home {
            return None;
        }
        for index in 0..self.launcher_result_count() {
            let Some(app) = self.launcher_result_app(index) else { continue };
            if command_menu_item_bounds(index).contains(x, y) {
                self.command_menu.set_selected(index as u8);
                return Some(app);
            }
        }
        None
    }

    pub const fn home_surface(&self) -> SurfaceHandle {
        self.home_surface
    }

    pub const fn lock_surface(&self) -> SurfaceHandle {
        self.lock_surface
    }

    pub const fn next_split_direction(&self) -> SplitDirection {
        self.next_split
    }

    pub fn focused_surface(&self) -> Option<Surface> {
        match self.focused {
            Some(index) => self.surfaces[index],
            None => None,
        }
    }

    pub const fn initial_surface_bounds(app: AppId) -> GuiRect {
        let _ = app;
        FULLSCREEN_SURFACE_BOUNDS
    }

    pub fn set_home_surface(&mut self, surface: SurfaceHandle) -> Result<(), AtriumError> {
        if self.phase != AtriumPhase::Home {
            return Err(AtriumError::Locked);
        }
        if !surface.is_valid() || surface == self.lock_surface {
            return Err(AtriumError::InvalidSurface);
        }
        self.home_surface = surface;
        Ok(())
    }

    pub fn clear_surfaces(&mut self) {
        self.home_surface = SurfaceHandle::EMPTY;
        self.lock_surface = SurfaceHandle::EMPTY;
    }

    pub fn set_surfaces(
        &mut self,
        home: SurfaceHandle,
        lock: SurfaceHandle,
    ) -> Result<(), AtriumError> {
        if !home.is_valid() || !lock.is_valid() || home == lock {
            return Err(AtriumError::InvalidSurface);
        }
        self.home_surface = home;
        self.lock_surface = lock;
        Ok(())
    }

    pub fn lock(&mut self) {
        self.phase = AtriumPhase::Locked;
        self.clear_surface_records();
        self.home_surface = SurfaceHandle::EMPTY;
        self.lock_surface = SurfaceHandle::EMPTY;
    }

    pub fn authenticate(&mut self) {
        if self.phase == AtriumPhase::Home {
            return;
        }
        self.phase = AtriumPhase::Home;
        self.clear_surface_records();
    }

    pub fn logout(&mut self) {
        self.lock();
    }

    pub fn restart(&mut self) {
        self.phase = AtriumPhase::Boot;
        self.clear_surface_records();
        self.home_surface = SurfaceHandle::EMPTY;
        self.lock_surface = SurfaceHandle::EMPTY;
    }

    pub fn surface(&self, id: u16) -> Option<Surface> {
        self.surfaces.iter().flatten().copied().find(|surface| surface.id == id)
    }

    pub fn surfaces(&self) -> impl Iterator<Item = Surface> + '_ {
        self.surfaces.iter().flatten().copied()
    }

    pub fn surface_for_app(&self, app: AppId) -> Option<Surface> {
        self.surfaces.iter().flatten().copied().find(|surface| surface.app == app)
    }

    pub fn surface_for_client(&self, client: ServiceHandle, app: AppId) -> Option<Surface> {
        self.surfaces
            .iter()
            .flatten()
            .copied()
            .find(|surface| surface.client == client && surface.app == app)
    }

    pub fn surface_by_reference(&self, reference: SurfaceHandle) -> Option<Surface> {
        self.surfaces.iter().flatten().copied().find(|surface| surface.reference == reference)
    }

    pub fn surface_at(&self, x: i32, y: i32) -> Option<Surface> {
        self.surfaces
            .iter()
            .flatten()
            .filter(|surface| surface.bounds.contains(x, y))
            .max_by_key(|surface| surface.focus_order)
            .copied()
    }

    pub fn splitter_at(&self, x: i32, y: i32) -> Option<usize> {
        let root = self.layout_root?;
        self.find_splitter(root, FULLSCREEN_SURFACE_BOUNDS, x, y)
    }

    pub fn handle_splitter_pointer(&mut self, input: &InputMessage) -> bool {
        let Some(pointer) = input.pointer_event() else { return false };
        let position = (i32::from(pointer.x), i32::from(pointer.y));
        match pointer.state {
            PointerState::Down => {
                let Some(node) = self.splitter_at(position.0, position.1) else { return false };
                self.splitter_capture = Some(node);
                self.splitter_last_position = position;
                true
            }
            PointerState::Move => {
                let Some(node) = self.splitter_capture else { return false };
                let delta = match self.layout_nodes[node] {
                    Some(LayoutNode::Split { direction: SplitDirection::Vertical, .. }) => {
                        position.0.saturating_sub(self.splitter_last_position.0)
                    }
                    Some(LayoutNode::Split { direction: SplitDirection::Horizontal, .. }) => {
                        position.1.saturating_sub(self.splitter_last_position.1)
                    }
                    _ => 0,
                };
                if delta != 0 {
                    let _ = self.resize_split(node, delta);
                    self.splitter_last_position = position;
                }
                true
            }
            PointerState::Up => {
                if self.splitter_capture.take().is_some() {
                    self.splitter_last_position = position;
                    true
                } else {
                    false
                }
            }
        }
    }

    pub fn pointer_target(&mut self, input: &InputMessage) -> Option<Surface> {
        let pointer = input.pointer_event()?;
        if self.phase != AtriumPhase::Home {
            return None;
        }
        let hit = || self.surface_at(i32::from(pointer.x), i32::from(pointer.y));
        let target = match pointer.state {
            PointerState::Down => hit(),
            PointerState::Move | PointerState::Up => self
                .pointer_capture
                .and_then(|reference| self.surface_by_reference(reference))
                .or_else(hit),
        }?;
        if pointer.state == PointerState::Down {
            self.focus(target.id).ok()?;
            self.pointer_capture = Some(target.reference);
        } else if pointer.state == PointerState::Up {
            self.pointer_capture = None;
        }
        self.surface(target.id)
    }

    pub fn focus_at(&mut self, x: i32, y: i32) -> Result<Surface, AtriumError> {
        let surface = self.surface_at(x, y).ok_or(AtriumError::NotFound)?;
        self.focus(surface.id)?;
        self.surface(surface.id).ok_or(AtriumError::NotFound)
    }

    pub fn request_surface(
        &self,
        app: AppId,
        client: ServiceHandle,
    ) -> Result<SurfaceRequest, AtriumError> {
        if self.phase != AtriumPhase::Home {
            return Err(AtriumError::Locked);
        }
        if !client.is_valid() {
            return Err(AtriumError::InvalidSurface);
        }
        if self.surface_for_client(client, app).is_some() {
            return Err(AtriumError::AlreadyRegistered);
        }
        if !self.surfaces.iter().any(Option::is_none) {
            return Err(AtriumError::Capacity);
        }
        let target_leaf = self.find_empty_leaf(self.layout_root);
        if target_leaf.is_none()
            && self.layout_root.is_some()
            && !self.can_split_focused(self.next_split)
        {
            return Err(AtriumError::Capacity);
        }
        Ok(SurfaceRequest {
            app,
            client,
            bounds: self.preview_surface_bounds(),
            mode: SurfaceMode::Tiled,
            anchor: self.focused_surface().map(|surface| surface.id),
            target_leaf,
            direction: self.next_split,
        })
    }

    pub fn spawn_surface(
        &mut self,
        request: SurfaceRequest,
        reference: SurfaceHandle,
    ) -> Result<Surface, AtriumError> {
        if self.phase != AtriumPhase::Home {
            return Err(AtriumError::Locked);
        }
        if !reference.is_valid() || reference == self.home_surface || reference == self.lock_surface
        {
            return Err(AtriumError::InvalidSurface);
        }
        if self.surfaces.iter().flatten().any(|surface| surface.reference == reference) {
            return Err(AtriumError::AlreadyRegistered);
        }
        let Some(index) = self.surfaces.iter().position(Option::is_none) else {
            return Err(AtriumError::Capacity);
        };
        let id = self.next_surface_id;
        self.next_surface_id = self.next_surface_id.wrapping_add(1).max(1);
        if request.anchor != self.focused_surface().map(|surface| surface.id) {
            return Err(AtriumError::NotFound);
        }
        let surface = Surface {
            id,
            app: request.app,
            client: request.client,
            reference,
            bounds: request.bounds,
            mode: request.mode,
            focused: true,
            focus_order: self.next_focus_order,
        };
        self.advance_focus_order();
        self.surfaces[index] = Some(surface);
        if let Err(error) = self.insert_layout_leaf(
            surface.id,
            request.target_leaf,
            request.anchor,
            request.direction,
        ) {
            self.surfaces[index] = None;
            return Err(error);
        }
        self.clear_focus();
        self.recompute_layout();
        self.focused = Some(index);
        Ok(self.surface(surface.id).unwrap_or(surface))
    }

    pub fn focus(&mut self, id: u16) -> Result<(), AtriumError> {
        let Some(index) =
            self.surfaces.iter().position(|surface| surface.is_some_and(|s| s.id == id))
        else {
            return Err(AtriumError::NotFound);
        };
        self.clear_focus();
        if let Some(surface) = &mut self.surfaces[index] {
            surface.focused = true;
            surface.focus_order = self.next_focus_order;
        }
        self.advance_focus_order();
        self.focused = Some(index);
        Ok(())
    }

    pub fn focus_reference(&mut self, reference: SurfaceHandle) -> Result<(), AtriumError> {
        let Some(surface) = self.surface_by_reference(reference) else {
            return Err(AtriumError::NotFound);
        };
        self.focus(surface.id)
    }

    pub fn move_focused(&mut self, dx: i32, dy: i32) -> Result<(), AtriumError> {
        let Some(index) = self.focused else { return Err(AtriumError::NotFound) };
        let Some(surface) = &mut self.surfaces[index] else { return Err(AtriumError::NotFound) };
        if surface.mode == SurfaceMode::Tiled {
            return Ok(());
        }
        surface.bounds.x = surface.bounds.x.saturating_add(dx);
        surface.bounds.y = surface.bounds.y.saturating_add(dy);
        Ok(())
    }

    pub fn close_focused(&mut self) -> Result<Surface, AtriumError> {
        let Some(index) = self.focused else { return Err(AtriumError::NotFound) };
        let Some(surface) = self.surfaces[index] else { return Err(AtriumError::NotFound) };
        self.remove_layout_leaf(surface.id)?;
        self.surfaces[index] = None;
        self.recompute_layout();
        self.focused = None;
        self.focus_next(1);
        Ok(surface)
    }

    pub fn close_reference(&mut self, reference: SurfaceHandle) -> Result<Surface, AtriumError> {
        let Some(index) = self
            .surfaces
            .iter()
            .position(|surface| surface.is_some_and(|surface| surface.reference == reference))
        else {
            return Err(AtriumError::NotFound);
        };
        let Some(surface) = self.surfaces[index] else { return Err(AtriumError::NotFound) };
        self.remove_layout_leaf(surface.id)?;
        self.surfaces[index] = None;
        self.recompute_layout();
        if self.focused == Some(index) {
            self.focused = None;
            self.focus_next(1);
        }
        Ok(surface)
    }

    pub fn input(&mut self, input: &InputMessage) -> AtriumAction {
        if self.phase != AtriumPhase::Home {
            return AtriumAction::None;
        }
        if self.focused.is_none() && matches!(input.kind, MessageKind::Text | MessageKind::Paste) {
            if let Some(text) = input.text_bytes() {
                if self.command_menu.append_text(text) {
                    self.refresh_command_menu_results();
                    return AtriumAction::LauncherChanged;
                }
            }
            return AtriumAction::None;
        }
        if input.state != KeyState::Pressed && input.state != KeyState::Repeat {
            return AtriumAction::None;
        }
        let code = KeyCode::from_raw(input.code);
        if input.modifiers & (MOD_CTRL | MOD_SHIFT) == (MOD_CTRL | MOD_SHIFT) {
            match code.character_byte() {
                Some(b'v') => return AtriumAction::Split(SplitDirection::Vertical),
                Some(b'h') => return AtriumAction::Split(SplitDirection::Horizontal),
                _ => {}
            }
        }
        if input.modifiers & MOD_ALT != 0 && code == KeyCode::function(4) {
            return AtriumAction::CloseFocused;
        }
        if code == KeyCode::TAB
            && input.modifiers & (MOD_SHIFT | MOD_CTRL | MOD_ALT | MOD_META) == 0
            && self.focused_surface().is_some_and(|surface| surface.app == AppId::Terminal)
        {
            return AtriumAction::None;
        }
        if self.focused.is_none() {
            if let Some(action) = self.command_menu_action(input.code) {
                return action;
            }
        }
        match (input.modifiers & MOD_CTRL != 0, code) {
            (true, KeyCode::TAB) => AtriumAction::FocusNext,
            (true, KeyCode::BackTab) => AtriumAction::FocusPrevious,
            (true, KeyCode::ESCAPE) => AtriumAction::CloseFocused,
            (true, KeyCode::LEFT) => AtriumAction::MoveFocused(-SURFACE_MOVE_STEP, 0),
            (true, KeyCode::RIGHT) => AtriumAction::MoveFocused(SURFACE_MOVE_STEP, 0),
            (true, KeyCode::UP) => AtriumAction::MoveFocused(0, -SURFACE_MOVE_STEP),
            (true, KeyCode::DOWN) => AtriumAction::MoveFocused(0, SURFACE_MOVE_STEP),
            (false, KeyCode::TAB) => AtriumAction::FocusNext,
            (false, KeyCode::BackTab) => AtriumAction::FocusPrevious,
            (false, KeyCode::ESCAPE) => AtriumAction::CloseFocused,
            (true, _) => match code.character_byte() {
                Some(b'l') => AtriumAction::Logout,
                // The default decoder is French AZERTY, where the number-row
                // semantic codes are &, é, and ". Keep the logical shortcuts
                // usable without making the shell depend on one layout.
                Some(b'1' | b'&') => AtriumAction::Launch(AppId::Calculator),
                Some(b'2' | b'e') => AtriumAction::Launch(AppId::Files),
                Some(b'3' | b'"') => AtriumAction::Launch(AppId::Terminal),
                Some(b'4' | b'\'') => AtriumAction::Launch(AppId::System),
                _ => AtriumAction::None,
            },
            _ => AtriumAction::None,
        }
    }

    fn command_menu_action(&mut self, code: u16) -> Option<AtriumAction> {
        let mut output = logos_ui::UiOutput::new();
        self.command_menu
            .handle_event(logos_ui::UiInputEvent::KeyDown { code, modifiers: 0 }, &mut output)
            .ok()?;
        match output.pop()? {
            logos_ui::UiCommandMenuEvent::SelectionChanged { .. } => {
                Some(AtriumAction::LauncherChanged)
            }
            logos_ui::UiCommandMenuEvent::QueryChanged { .. } => {
                self.refresh_command_menu_results();
                Some(AtriumAction::LauncherChanged)
            }
            logos_ui::UiCommandMenuEvent::Submitted { index } => {
                self.launcher_result_app(usize::from(index)).map(AtriumAction::Launch)
            }
        }
    }

    fn refresh_command_menu_results(&mut self) {
        let query_value = self.command_menu.query();
        let query = query_value.as_bytes();
        let mut count = 0;
        for (index, label) in COMMAND_MENU_LABELS.into_iter().enumerate() {
            if query_matches(label, query) {
                self.command_menu_matches[count] = index as u8;
                count += 1;
            }
        }
        self.command_menu.set_item_count(count as u8);
    }

    pub fn apply_action(&mut self, action: AtriumAction) -> Result<(), AtriumError> {
        match action {
            AtriumAction::FocusNext => {
                self.focus_next(1);
                Ok(())
            }
            AtriumAction::FocusPrevious => {
                self.focus_next(-1);
                Ok(())
            }
            AtriumAction::MoveFocused(dx, dy) => self.move_focused(dx, dy),
            AtriumAction::Split(direction) => {
                self.next_split = direction;
                self.split_focused(direction)
            }
            AtriumAction::CloseFocused => self.close_focused().map(|_| ()),
            AtriumAction::Logout => {
                self.logout();
                Ok(())
            }
            AtriumAction::None | AtriumAction::LauncherChanged | AtriumAction::Launch(_) => Ok(()),
        }
    }

    fn clear_surface_records(&mut self) {
        self.surfaces = [None; MAX_ATRIUM_SURFACES];
        self.focused = None;
        self.pointer_capture = None;
        self.splitter_capture = None;
        self.layout_nodes = [None; MAX_LAYOUT_NODES];
        self.layout_root = None;
        self.command_menu.clear_query();
        self.command_menu.set_item_count(COMMAND_MENU_ITEMS.len() as u8);
        self.command_menu_matches = [0, 1, 2, 3];
        self.command_menu.set_selected(0);
        self.next_focus_order = 1;
    }

    fn preview_surface_bounds(&self) -> GuiRect {
        if let Some(target_leaf) = self.find_empty_leaf(self.layout_root) {
            return self
                .node_bounds(self.layout_root, FULLSCREEN_SURFACE_BOUNDS, target_leaf)
                .unwrap_or(FULLSCREEN_SURFACE_BOUNDS);
        }
        let Some(anchor) = self.focused_surface().map(|surface| surface.id) else {
            return FULLSCREEN_SURFACE_BOUNDS;
        };
        let Some(node) = self.find_leaf(self.layout_root, anchor) else {
            return FULLSCREEN_SURFACE_BOUNDS;
        };
        let Some(bounds) = self.node_bounds(self.layout_root, FULLSCREEN_SURFACE_BOUNDS, node)
        else {
            return FULLSCREEN_SURFACE_BOUNDS;
        };
        self.split_rect(bounds, self.next_split).1
    }

    fn insert_layout_leaf(
        &mut self,
        surface_id: u16,
        target_leaf: Option<usize>,
        anchor: Option<u16>,
        direction: SplitDirection,
    ) -> Result<(), AtriumError> {
        let Some(root) = self.layout_root else {
            let index = self.allocate_layout_node(LayoutNode::Leaf {
                parent: None,
                surface_id: Some(surface_id),
            })?;
            self.layout_root = Some(index);
            return Ok(());
        };
        if let Some(target_leaf) = target_leaf {
            let parent = match self.layout_nodes[target_leaf] {
                Some(LayoutNode::Leaf { parent, surface_id: None }) => parent,
                _ => return Err(AtriumError::NotFound),
            };
            self.layout_nodes[target_leaf] =
                Some(LayoutNode::Leaf { parent, surface_id: Some(surface_id) });
            return Ok(());
        }
        let Some(anchor) = anchor else { return Err(AtriumError::NotFound) };
        let leaf = self.find_leaf(Some(root), anchor).ok_or(AtriumError::NotFound)?;
        let parent = match self.layout_nodes[leaf] {
            Some(LayoutNode::Leaf { parent, .. }) => parent,
            _ => return Err(AtriumError::NotFound),
        };
        let first = self.allocate_layout_node(LayoutNode::Leaf {
            parent: Some(leaf),
            surface_id: Some(anchor),
        })?;
        let second = match self.allocate_layout_node(LayoutNode::Leaf {
            parent: Some(leaf),
            surface_id: Some(surface_id),
        }) {
            Ok(index) => index,
            Err(error) => {
                self.layout_nodes[first] = None;
                return Err(error);
            }
        };
        self.layout_nodes[leaf] = Some(LayoutNode::Split {
            parent,
            direction,
            ratio: DEFAULT_SPLIT_RATIO,
            first,
            second,
        });
        Ok(())
    }

    fn remove_layout_leaf(&mut self, surface_id: u16) -> Result<(), AtriumError> {
        let leaf = self.find_leaf(self.layout_root, surface_id).ok_or(AtriumError::NotFound)?;
        let parent = match self.layout_nodes[leaf] {
            Some(LayoutNode::Leaf { parent, .. }) => parent,
            _ => return Err(AtriumError::NotFound),
        };
        let Some(parent) = parent else {
            self.layout_nodes[leaf] = None;
            self.layout_root = None;
            return Ok(());
        };
        let (grandparent, sibling) = match self.layout_nodes[parent] {
            Some(LayoutNode::Split { parent, first, second, .. }) => {
                (parent, if first == leaf { second } else { first })
            }
            _ => return Err(AtriumError::NotFound),
        };
        let sibling_node = self.layout_nodes[sibling].ok_or(AtriumError::NotFound)?;
        self.layout_nodes[parent] = Some(match sibling_node {
            LayoutNode::Leaf { surface_id, .. } => {
                LayoutNode::Leaf { parent: grandparent, surface_id }
            }
            LayoutNode::Split { direction, ratio, first, second, .. } => {
                self.layout_nodes[first] = Some(set_parent(
                    self.layout_nodes[first].ok_or(AtriumError::NotFound)?,
                    parent,
                ));
                self.layout_nodes[second] = Some(set_parent(
                    self.layout_nodes[second].ok_or(AtriumError::NotFound)?,
                    parent,
                ));
                LayoutNode::Split { parent: grandparent, direction, ratio, first, second }
            }
        });
        self.layout_nodes[leaf] = None;
        self.layout_nodes[sibling] = None;
        if grandparent.is_none() {
            self.layout_root = Some(parent);
        }
        Ok(())
    }

    fn allocate_layout_node(&mut self, node: LayoutNode) -> Result<usize, AtriumError> {
        let Some(index) = self.layout_nodes.iter().position(Option::is_none) else {
            return Err(AtriumError::Capacity);
        };
        self.layout_nodes[index] = Some(node);
        Ok(index)
    }

    fn recompute_layout(&mut self) {
        let Some(root) = self.layout_root else { return };
        recompute_node(&self.layout_nodes, root, FULLSCREEN_SURFACE_BOUNDS, &mut self.surfaces);
    }

    fn find_leaf(&self, node: Option<usize>, surface_id: u16) -> Option<usize> {
        let node = node?;
        match self.layout_nodes[node]? {
            LayoutNode::Leaf { surface_id: Some(id), .. } => (id == surface_id).then_some(node),
            LayoutNode::Leaf { surface_id: None, .. } => None,
            LayoutNode::Split { first, second, .. } => self
                .find_leaf(Some(first), surface_id)
                .or_else(|| self.find_leaf(Some(second), surface_id)),
        }
    }

    fn find_empty_leaf(&self, node: Option<usize>) -> Option<usize> {
        let node = node?;
        match self.layout_nodes[node]? {
            LayoutNode::Leaf { surface_id: None, .. } => Some(node),
            LayoutNode::Leaf { surface_id: Some(_), .. } => None,
            LayoutNode::Split { first, second, .. } => {
                self.find_empty_leaf(Some(first)).or_else(|| self.find_empty_leaf(Some(second)))
            }
        }
    }

    fn split_focused(&mut self, direction: SplitDirection) -> Result<(), AtriumError> {
        if self.find_empty_leaf(self.layout_root).is_some() {
            return Err(AtriumError::Capacity);
        }
        let Some(surface) = self.focused_surface() else { return Err(AtriumError::NotFound) };
        let Some(root) = self.layout_root else { return Err(AtriumError::NotFound) };
        let leaf = self.find_leaf(Some(root), surface.id).ok_or(AtriumError::NotFound)?;
        let parent = match self.layout_nodes[leaf] {
            Some(LayoutNode::Leaf { parent, .. }) => parent,
            _ => return Err(AtriumError::NotFound),
        };
        let first = self.allocate_layout_node(LayoutNode::Leaf {
            parent: Some(leaf),
            surface_id: Some(surface.id),
        })?;
        let second = match self
            .allocate_layout_node(LayoutNode::Leaf { parent: Some(leaf), surface_id: None })
        {
            Ok(index) => index,
            Err(error) => {
                self.layout_nodes[first] = None;
                return Err(error);
            }
        };
        self.layout_nodes[leaf] = Some(LayoutNode::Split {
            parent,
            direction,
            ratio: DEFAULT_SPLIT_RATIO,
            first,
            second,
        });
        self.recompute_layout();
        Ok(())
    }

    fn node_bounds(&self, node: Option<usize>, bounds: GuiRect, target: usize) -> Option<GuiRect> {
        let node = node?;
        if node == target {
            return Some(bounds);
        }
        match self.layout_nodes[node]? {
            LayoutNode::Leaf { .. } => None,
            LayoutNode::Split { direction, ratio, first, second, .. } => {
                let (first_bounds, second_bounds) =
                    self.split_rect_with_ratio(bounds, direction, ratio);
                self.node_bounds(Some(first), first_bounds, target)
                    .or_else(|| self.node_bounds(Some(second), second_bounds, target))
            }
        }
    }

    fn find_splitter(&self, node: usize, bounds: GuiRect, x: i32, y: i32) -> Option<usize> {
        match self.layout_nodes[node]? {
            LayoutNode::Leaf { .. } => None,
            LayoutNode::Split { direction, ratio, first, second, .. } => {
                let (first_bounds, second_bounds) =
                    self.split_rect_with_ratio(bounds, direction, ratio);
                let hit = match direction {
                    SplitDirection::Vertical => {
                        let edge = second_bounds.x;
                        (x - edge).abs() <= SPLITTER_HIT_RADIUS
                            && y >= bounds.y
                            && y < bounds.y.saturating_add(bounds.height as i32)
                    }
                    SplitDirection::Horizontal => {
                        let edge = second_bounds.y;
                        (y - edge).abs() <= SPLITTER_HIT_RADIUS
                            && x >= bounds.x
                            && x < bounds.x.saturating_add(bounds.width as i32)
                    }
                };
                if hit {
                    Some(node)
                } else {
                    self.find_splitter(first, first_bounds, x, y)
                        .or_else(|| self.find_splitter(second, second_bounds, x, y))
                }
            }
        }
    }

    fn resize_split(&mut self, node: usize, delta: i32) -> Result<(), AtriumError> {
        let Some(bounds) = self.node_bounds(self.layout_root, FULLSCREEN_SURFACE_BOUNDS, node)
        else {
            return Err(AtriumError::NotFound);
        };
        let Some(LayoutNode::Split { direction, .. }) = self.layout_nodes[node] else {
            return Err(AtriumError::NotFound);
        };
        let current_ratio = match self.layout_nodes[node] {
            Some(LayoutNode::Split { ratio, .. }) => ratio,
            _ => return Err(AtriumError::NotFound),
        };
        let (first, second) = self.split_rect_with_ratio(bounds, direction, current_ratio);
        let first_size = match direction {
            SplitDirection::Vertical => first.width as i32,
            SplitDirection::Horizontal => first.height as i32,
        };
        let total = match direction {
            SplitDirection::Vertical => bounds.width,
            SplitDirection::Horizontal => bounds.height,
        } as i32;
        let minimum = match direction {
            SplitDirection::Vertical => MIN_PANE_WIDTH as i32,
            SplitDirection::Horizontal => MIN_PANE_HEIGHT as i32,
        };
        let next = first_size.saturating_add(delta).clamp(minimum, total.saturating_sub(minimum));
        let ratio = ((next * 1024) / total).clamp(1, 1023) as u16;
        if let Some(LayoutNode::Split { ratio: current, .. }) = &mut self.layout_nodes[node] {
            *current = ratio;
        }
        self.recompute_layout();
        let _ = second;
        Ok(())
    }

    fn can_split_focused(&self, direction: SplitDirection) -> bool {
        let Some(surface) = self.focused_surface() else { return false };
        let Some(node) = self.find_leaf(self.layout_root, surface.id) else { return false };
        let Some(bounds) = self.node_bounds(self.layout_root, FULLSCREEN_SURFACE_BOUNDS, node)
        else {
            return false;
        };
        match direction {
            SplitDirection::Vertical => bounds.width >= MIN_PANE_WIDTH * 2,
            SplitDirection::Horizontal => bounds.height >= MIN_PANE_HEIGHT * 2,
        }
    }

    fn split_rect(&self, bounds: GuiRect, direction: SplitDirection) -> (GuiRect, GuiRect) {
        self.split_rect_with_ratio(bounds, direction, DEFAULT_SPLIT_RATIO)
    }

    fn split_rect_with_ratio(
        &self,
        bounds: GuiRect,
        direction: SplitDirection,
        ratio: u16,
    ) -> (GuiRect, GuiRect) {
        match direction {
            SplitDirection::Vertical => {
                if bounds.width < MIN_PANE_WIDTH * 2 {
                    let first_width = bounds.width / 2;
                    return (
                        GuiRect::new(bounds.x, bounds.y, first_width, bounds.height),
                        GuiRect::new(
                            bounds.x.saturating_add(first_width as i32),
                            bounds.y,
                            bounds.width.saturating_sub(first_width),
                            bounds.height,
                        ),
                    );
                }
                let first_width = ((bounds.width as u64 * ratio as u64) / 1024) as u32;
                let first_width =
                    first_width.clamp(MIN_PANE_WIDTH, bounds.width.saturating_sub(MIN_PANE_WIDTH));
                (
                    GuiRect::new(bounds.x, bounds.y, first_width, bounds.height),
                    GuiRect::new(
                        bounds.x.saturating_add(first_width as i32),
                        bounds.y,
                        bounds.width.saturating_sub(first_width),
                        bounds.height,
                    ),
                )
            }
            SplitDirection::Horizontal => {
                if bounds.height < MIN_PANE_HEIGHT * 2 {
                    let first_height = bounds.height / 2;
                    return (
                        GuiRect::new(bounds.x, bounds.y, bounds.width, first_height),
                        GuiRect::new(
                            bounds.x,
                            bounds.y.saturating_add(first_height as i32),
                            bounds.width,
                            bounds.height.saturating_sub(first_height),
                        ),
                    );
                }
                let first_height = ((bounds.height as u64 * ratio as u64) / 1024) as u32;
                let first_height = first_height
                    .clamp(MIN_PANE_HEIGHT, bounds.height.saturating_sub(MIN_PANE_HEIGHT));
                (
                    GuiRect::new(bounds.x, bounds.y, bounds.width, first_height),
                    GuiRect::new(
                        bounds.x,
                        bounds.y.saturating_add(first_height as i32),
                        bounds.width,
                        bounds.height.saturating_sub(first_height),
                    ),
                )
            }
        }
    }

    fn clear_focus(&mut self) {
        for surface in self.surfaces.iter_mut().flatten() {
            surface.focused = false;
        }
    }

    fn advance_focus_order(&mut self) {
        self.next_focus_order = self.next_focus_order.wrapping_add(1).max(1);
    }

    fn focus_next(&mut self, direction: isize) {
        let Some(current) = self.focused else {
            self.focused = self.surfaces.iter().position(Option::is_some);
            if let Some(index) = self.focused {
                self.clear_focus();
                if let Some(surface) = &mut self.surfaces[index] {
                    surface.focused = true;
                    surface.focus_order = self.next_focus_order;
                }
                self.advance_focus_order();
            }
            return;
        };
        let mut index = current as isize;
        for _ in 0..MAX_ATRIUM_SURFACES {
            index = (index + direction).rem_euclid(MAX_ATRIUM_SURFACES as isize);
            if self.surfaces[index as usize].is_some() {
                self.clear_focus();
                let index = index as usize;
                if let Some(surface) = &mut self.surfaces[index] {
                    surface.focused = true;
                    surface.focus_order = self.next_focus_order;
                }
                self.advance_focus_order();
                self.focused = Some(index);
                return;
            }
        }
    }
}

impl Default for Atrium {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CalculatorOperation {
    Add,
    Subtract,
    Multiply,
    Divide,
}

pub struct Calculator {
    display: [u8; MAX_CALCULATOR_TEXT],
    length: usize,
    accumulator: f64,
    operation: Option<CalculatorOperation>,
    entering: bool,
    error: bool,
}

impl Calculator {
    pub const fn new() -> Self {
        let mut calculator = Self {
            display: [0; MAX_CALCULATOR_TEXT],
            length: 1,
            accumulator: 0.0,
            operation: None,
            entering: false,
            error: false,
        };
        calculator.display[0] = b'0';
        calculator
    }

    pub fn input(&mut self, input: &InputMessage) -> bool {
        if input.state != KeyState::Pressed && input.state != KeyState::Repeat {
            return false;
        }
        if let Some(pointer) = input.pointer_event() {
            if pointer.state != PointerState::Down || pointer.buttons & 1 == 0 {
                return false;
            }
            let x = i32::from(pointer.x).saturating_sub(CALCULATOR_BUTTON_LEFT);
            let y = i32::from(pointer.y).saturating_sub(CALCULATOR_BUTTON_TOP);
            let stride = CALCULATOR_BUTTON_WIDTH + CALCULATOR_BUTTON_GAP;
            let row_stride = CALCULATOR_BUTTON_HEIGHT + CALCULATOR_BUTTON_GAP;
            if x < 0
                || y < 0
                || x >= stride * 4 - CALCULATOR_BUTTON_GAP
                || y >= row_stride * 4
                || x % stride >= CALCULATOR_BUTTON_WIDTH
                || y % row_stride >= CALCULATOR_BUTTON_HEIGHT
            {
                return false;
            }
            let index = ((y / row_stride) * 4 + x / stride) as usize;
            let character = CALCULATOR_BUTTON_LABELS[index];
            let code =
                if character == b'=' { KeyCode::ENTER } else { KeyCode::character(character) };
            return self.input(&InputMessage::key(code, KeyState::Pressed, 0));
        }
        if let Some(text) = input.text_bytes() {
            let mut changed = false;
            for byte in text.iter().copied() {
                let key = InputMessage::key(KeyCode::character(byte), input.state, 0);
                changed |= self.input(&key);
            }
            return changed;
        }
        let code = KeyCode::from_raw(input.code);
        if code == KeyCode::ESCAPE {
            self.clear();
            return true;
        }
        if code == KeyCode::BACKSPACE {
            if self.length > 1 {
                self.length -= 1;
                self.display[self.length] = 0;
            }
            return true;
        }
        if code == KeyCode::ENTER {
            self.equals();
            return true;
        }
        let Some(character) = code.character_byte() else { return false };
        match character {
            b'0'..=b'9' | b'.' => self.push_digit(character),
            b'+' => self.set_operation(CalculatorOperation::Add),
            b'-' => self.set_operation(CalculatorOperation::Subtract),
            b'*' | b'x' => self.set_operation(CalculatorOperation::Multiply),
            b'/' => self.set_operation(CalculatorOperation::Divide),
            _ => false,
        }
    }

    pub fn display(&self) -> &[u8] {
        &self.display[..self.length]
    }

    fn push_digit(&mut self, character: u8) -> bool {
        if self.error {
            self.clear();
        }
        if !self.entering {
            self.length = 0;
            self.entering = true;
        }
        if character == b'.' && self.display[..self.length].contains(&b'.') {
            return false;
        }
        if self.length == MAX_CALCULATOR_TEXT {
            return false;
        }
        if self.length == 0 && character == b'.' {
            self.display[0] = b'0';
            self.length = 1;
        }
        self.display[self.length] = character;
        self.length += 1;
        true
    }

    fn set_operation(&mut self, operation: CalculatorOperation) -> bool {
        if self.error {
            return false;
        }
        let value = self.value();
        if self.operation.is_some() && self.entering {
            if !self.apply(value) {
                return false;
            }
        } else {
            self.accumulator = value;
        }
        self.operation = Some(operation);
        self.entering = false;
        true
    }

    fn equals(&mut self) {
        if self.error || self.operation.is_none() {
            return;
        }
        if !self.apply(self.value()) {
            return;
        }
        self.operation = None;
        self.entering = false;
        self.write_value(self.accumulator);
    }

    fn apply(&mut self, value: f64) -> bool {
        let Some(operation) = self.operation.take() else { return true };
        self.accumulator = match operation {
            CalculatorOperation::Add => self.accumulator + value,
            CalculatorOperation::Subtract => self.accumulator - value,
            CalculatorOperation::Multiply => self.accumulator * value,
            CalculatorOperation::Divide if value != 0.0 => self.accumulator / value,
            CalculatorOperation::Divide => {
                self.error = true;
                self.write_bytes(b"ERR");
                return false;
            }
        };
        true
    }

    fn value(&self) -> f64 {
        let mut value = 0.0;
        let mut fraction = 0.1;
        let mut decimal = false;
        let mut negative = false;
        for &byte in self.display() {
            match byte {
                b'-' if value == 0.0 => negative = true,
                b'.' => decimal = true,
                b'0'..=b'9' if decimal => {
                    value += f64::from(byte - b'0') * fraction;
                    fraction *= 0.1;
                }
                b'0'..=b'9' => value = value * 10.0 + f64::from(byte - b'0'),
                _ => {}
            }
        }
        if negative { -value } else { value }
    }

    fn write_value(&mut self, value: f64) {
        self.length = 0;
        if value.is_nan() || value.is_infinite() {
            self.write_bytes(b"ERR");
            self.error = true;
            return;
        }
        let _ = write!(
            FixedBuffer { bytes: &mut self.display, length: &mut self.length },
            "{value:.3}"
        );
        while self.length > 1 && self.display[self.length - 1] == b'0' {
            self.length -= 1;
        }
        if self.length > 1 && self.display[self.length - 1] == b'.' {
            self.length -= 1;
        }
    }

    fn write_bytes(&mut self, bytes: &[u8]) {
        self.length = bytes.len().min(MAX_CALCULATOR_TEXT);
        self.display[..self.length].copy_from_slice(&bytes[..self.length]);
    }

    fn clear(&mut self) {
        self.display = [0; MAX_CALCULATOR_TEXT];
        self.display[0] = b'0';
        self.length = 1;
        self.accumulator = 0.0;
        self.operation = None;
        self.entering = false;
        self.error = false;
    }
}

struct FixedBuffer<'a> {
    bytes: &'a mut [u8; MAX_CALCULATOR_TEXT],
    length: &'a mut usize,
}

impl Write for FixedBuffer<'_> {
    fn write_str(&mut self, value: &str) -> core::fmt::Result {
        for byte in value.bytes() {
            if *self.length == self.bytes.len() {
                break;
            }
            self.bytes[*self.length] = byte;
            *self.length += 1;
        }
        Ok(())
    }
}

impl Default for Calculator {
    fn default() -> Self {
        Self::new()
    }
}

const _: () = assert!(core::mem::size_of::<Atrium>() <= 2048);

fn set_parent(node: LayoutNode, parent: usize) -> LayoutNode {
    match node {
        LayoutNode::Leaf { surface_id, .. } => {
            LayoutNode::Leaf { parent: Some(parent), surface_id }
        }
        LayoutNode::Split { direction, ratio, first, second, .. } => {
            LayoutNode::Split { parent: Some(parent), direction, ratio, first, second }
        }
    }
}

fn recompute_node(
    nodes: &[Option<LayoutNode>; MAX_LAYOUT_NODES],
    node: usize,
    bounds: GuiRect,
    surfaces: &mut [Option<Surface>; MAX_ATRIUM_SURFACES],
) {
    let Some(layout) = nodes[node] else { return };
    match layout {
        LayoutNode::Leaf { surface_id: Some(surface_id), .. } => {
            if let Some(surface) =
                surfaces.iter_mut().flatten().find(|surface| surface.id == surface_id)
            {
                surface.bounds = bounds;
            }
        }
        LayoutNode::Leaf { surface_id: None, .. } => {}
        LayoutNode::Split { direction, ratio, first, second, .. } => {
            let (first_bounds, second_bounds) = split_rect_static(bounds, direction, ratio);
            recompute_node(nodes, first, first_bounds, surfaces);
            recompute_node(nodes, second, second_bounds, surfaces);
        }
    }
}

fn split_rect_static(bounds: GuiRect, direction: SplitDirection, ratio: u16) -> (GuiRect, GuiRect) {
    match direction {
        SplitDirection::Vertical => {
            let first_width = if bounds.width < MIN_PANE_WIDTH * 2 {
                bounds.width / 2
            } else {
                (((bounds.width as u64 * ratio as u64) / 1024) as u32)
                    .clamp(MIN_PANE_WIDTH, bounds.width.saturating_sub(MIN_PANE_WIDTH))
            };
            (
                GuiRect::new(bounds.x, bounds.y, first_width, bounds.height),
                GuiRect::new(
                    bounds.x.saturating_add(first_width as i32),
                    bounds.y,
                    bounds.width.saturating_sub(first_width),
                    bounds.height,
                ),
            )
        }
        SplitDirection::Horizontal => {
            let first_height = if bounds.height < MIN_PANE_HEIGHT * 2 {
                bounds.height / 2
            } else {
                (((bounds.height as u64 * ratio as u64) / 1024) as u32)
                    .clamp(MIN_PANE_HEIGHT, bounds.height.saturating_sub(MIN_PANE_HEIGHT))
            };
            (
                GuiRect::new(bounds.x, bounds.y, bounds.width, first_height),
                GuiRect::new(
                    bounds.x,
                    bounds.y.saturating_add(first_height as i32),
                    bounds.width,
                    bounds.height.saturating_sub(first_height),
                ),
            )
        }
    }
}
const _: () = assert!(core::mem::size_of::<Calculator>() <= 128);

#[cfg(test)]
mod tests {
    use super::*;

    fn surface(slot: u16) -> SurfaceHandle {
        SurfaceHandle::new(slot, 1, 13).unwrap()
    }

    fn client(slot: u32) -> ServiceHandle {
        ServiceHandle::new(slot, 1).unwrap()
    }

    fn key(byte: u8) -> InputMessage {
        InputMessage::key(KeyCode::character(byte), KeyState::Pressed, 0)
    }

    fn ctrl(byte: u8) -> InputMessage {
        InputMessage::key(KeyCode::character(byte), KeyState::Pressed, MOD_CTRL)
    }

    fn ctrl_shift(byte: u8) -> InputMessage {
        InputMessage::key(KeyCode::character(byte), KeyState::Pressed, MOD_CTRL | MOD_SHIFT)
    }

    #[test]
    fn phase_and_surface_lifecycle_is_bounded() {
        let mut atrium = Atrium::new();
        assert_eq!(atrium.phase(), AtriumPhase::Boot);
        atrium.lock();
        assert_eq!(atrium.phase(), AtriumPhase::Locked);
        assert_eq!(atrium.request_surface(AppId::Calculator, client(1)), Err(AtriumError::Locked));
        atrium.authenticate();
        let first = atrium
            .spawn_surface(
                atrium.request_surface(AppId::Calculator, client(1)).unwrap(),
                surface(0),
            )
            .unwrap();
        let second = atrium
            .spawn_surface(
                atrium.request_surface(AppId::Calculator, client(2)).unwrap(),
                surface(1),
            )
            .unwrap();
        atrium.focus(first.id).unwrap();
        atrium.apply_action(AtriumAction::Split(SplitDirection::Horizontal)).unwrap();
        let third = atrium
            .spawn_surface(
                atrium.request_surface(AppId::Calculator, client(3)).unwrap(),
                surface(2),
            )
            .unwrap();
        atrium.focus(second.id).unwrap();
        let fourth = atrium
            .spawn_surface(
                atrium.request_surface(AppId::Calculator, client(4)).unwrap(),
                surface(3),
            )
            .unwrap();
        atrium.focus(first.id).unwrap();
        atrium.apply_action(AtriumAction::Split(SplitDirection::Vertical)).unwrap();
        let fifth = atrium
            .spawn_surface(
                atrium.request_surface(AppId::Calculator, client(5)).unwrap(),
                surface(4),
            )
            .unwrap();
        atrium.focus(third.id).unwrap();
        let sixth = atrium
            .spawn_surface(
                atrium.request_surface(AppId::Calculator, client(6)).unwrap(),
                surface(5),
            )
            .unwrap();
        atrium.focus(second.id).unwrap();
        let seventh = atrium
            .spawn_surface(
                atrium.request_surface(AppId::Calculator, client(7)).unwrap(),
                surface(6),
            )
            .unwrap();
        atrium.focus(fourth.id).unwrap();
        let eighth = atrium
            .spawn_surface(
                atrium.request_surface(AppId::Calculator, client(8)).unwrap(),
                surface(7),
            )
            .unwrap();
        let _ = (fifth, sixth, seventh, eighth);
        assert_eq!(atrium.surfaces().count(), MAX_ATRIUM_SURFACES);
        let request = atrium.request_surface(AppId::Files, client(1)).unwrap_err();
        assert_eq!(request, AtriumError::Capacity);
        atrium.logout();
        assert_eq!(atrium.phase(), AtriumPhase::Locked);
        assert_eq!(atrium.surfaces().count(), 0);
    }

    #[test]
    fn keyboard_actions_launch_move_and_logout() {
        let mut atrium = Atrium::new();
        atrium.authenticate();
        assert_eq!(atrium.input(&ctrl(b'1')), AtriumAction::Launch(AppId::Calculator));
        assert_eq!(atrium.input(&ctrl(b'&')), AtriumAction::Launch(AppId::Calculator));
        assert_eq!(atrium.input(&ctrl(b'e')), AtriumAction::Launch(AppId::Files));
        assert_eq!(atrium.input(&ctrl(b'"')), AtriumAction::Launch(AppId::Terminal));
        assert_eq!(atrium.input(&ctrl(b'4')), AtriumAction::Launch(AppId::System));
        assert_eq!(
            atrium.input(&ctrl_shift(b'h')),
            AtriumAction::Split(SplitDirection::Horizontal)
        );
        assert_eq!(atrium.input(&ctrl_shift(b'v')), AtriumAction::Split(SplitDirection::Vertical));
        let request = atrium.request_surface(AppId::Calculator, client(1)).unwrap();
        atrium.spawn_surface(request, surface(1)).unwrap();
        assert_eq!(
            atrium.input(&InputMessage::key(KeyCode::ENTER, KeyState::Pressed, 0)),
            AtriumAction::None
        );
        assert_eq!(
            atrium.input(&InputMessage::key(KeyCode::function(4), KeyState::Pressed, MOD_ALT)),
            AtriumAction::CloseFocused
        );
        assert_eq!(atrium.input(&ctrl(b'j')), AtriumAction::None);
        assert_eq!(atrium.input(&ctrl(b'1')), AtriumAction::Launch(AppId::Calculator));
        assert_eq!(atrium.input(&ctrl(b'l')), AtriumAction::Logout);
        assert_eq!(
            atrium.input(&InputMessage::key(KeyCode::RIGHT, KeyState::Pressed, 0)),
            AtriumAction::None
        );
        assert!(AtriumAction::None.routes_to_surface());
        atrium.apply_action(AtriumAction::Logout).unwrap();
        assert_eq!(atrium.phase(), AtriumPhase::Locked);
        assert!(AtriumAction::None.routes_to_surface());
        assert!(!AtriumAction::FocusNext.routes_to_surface());
        assert!(!AtriumAction::Launch(AppId::Terminal).routes_to_surface());
    }

    #[test]
    fn split_shortcut_immediately_exposes_the_next_pane() {
        let mut atrium = Atrium::new();
        atrium.authenticate();
        let calculator = atrium
            .spawn_surface(
                atrium.request_surface(AppId::Calculator, client(1)).unwrap(),
                surface(1),
            )
            .unwrap();

        let split = atrium.input(&ctrl_shift(b'v'));
        atrium.apply_action(split).unwrap();
        let files = atrium
            .spawn_surface(atrium.request_surface(AppId::Files, client(2)).unwrap(), surface(2))
            .unwrap();

        let calculator = atrium.surface(calculator.id).unwrap();
        assert!(calculator.bounds.width < FULLSCREEN_SURFACE_BOUNDS.width);
        assert!(files.bounds.x > calculator.bounds.x);
        assert_eq!(files.bounds.width, calculator.bounds.width);
    }

    #[test]
    fn fullscreen_surfaces_ignore_move_and_close_safely() {
        let mut atrium = Atrium::new();
        atrium.authenticate();
        let request = atrium.request_surface(AppId::Calculator, client(1)).unwrap();
        let admitted = atrium.spawn_surface(request, surface(1)).unwrap();
        assert_eq!(atrium.focused_surface().unwrap().id, admitted.id);
        atrium.move_focused(SURFACE_MOVE_STEP, -SURFACE_MOVE_STEP).unwrap();
        assert_eq!(atrium.surface(admitted.id).unwrap().bounds, FULLSCREEN_SURFACE_BOUNDS);
        let closed = atrium.close_focused().unwrap();
        assert_eq!(closed.id, admitted.id);
        atrium.restart();
        assert_eq!(atrium.phase(), AtriumPhase::Boot);
        assert!(!atrium.home_surface().is_valid());
        assert_eq!(atrium.surfaces().count(), 0);
    }

    #[test]
    fn plain_tab_routes_to_a_focused_terminal_surface() {
        let mut atrium = Atrium::new();
        atrium.authenticate();
        let request = atrium.request_surface(AppId::Terminal, client(1)).unwrap();
        atrium.spawn_surface(request, surface(1)).unwrap();
        assert_eq!(
            atrium.input(&InputMessage::key(
                KeyCode::TAB,
                KeyState::Pressed,
                logos_abi::MOD_NUM_LOCK,
            )),
            AtriumAction::None
        );
        assert!(AtriumAction::None.routes_to_surface());
    }

    #[test]
    fn focused_terminal_navigation_routes_to_the_surface() {
        let mut atrium = Atrium::new();
        atrium.authenticate();
        let request = atrium.request_surface(AppId::Terminal, client(1)).unwrap();
        atrium.spawn_surface(request, surface(1)).unwrap();

        for code in
            [KeyCode::UP, KeyCode::DOWN, KeyCode::LEFT, KeyCode::RIGHT, KeyCode::HOME, KeyCode::END]
        {
            assert_eq!(
                atrium.input(&InputMessage::key(code, KeyState::Pressed, 0)),
                AtriumAction::None
            );
        }
    }

    #[test]
    fn command_menu_selection_is_bounded_and_launchable() {
        let mut atrium = Atrium::new();
        atrium.authenticate();
        assert_eq!(atrium.command_menu_item_at(151, COMMAND_MENU_ITEM_TOP), None);
        assert_eq!(
            atrium.command_menu_item_at(
                COMMAND_MENU_ITEM_LEFT + 1,
                COMMAND_MENU_ITEM_TOP
                    + 2 * (COMMAND_MENU_ITEM_HEIGHT as i32 + COMMAND_MENU_ITEM_GAP)
                    + 1,
            ),
            Some(AppId::Terminal)
        );
        assert_eq!(atrium.launcher_index(), 2);
        assert_eq!(
            atrium.input(&InputMessage::key(KeyCode::ENTER, KeyState::Pressed, 0)),
            AtriumAction::Launch(AppId::Terminal)
        );
    }

    #[test]
    fn command_menu_filters_text_and_launches_the_selected_result() {
        let mut atrium = Atrium::new();
        atrium.authenticate();
        assert_eq!(
            atrium.input(&InputMessage::text(b"calcu").unwrap()),
            AtriumAction::LauncherChanged
        );
        assert_eq!(atrium.launcher_result_count(), 1);
        assert_eq!(atrium.launcher_app(), AppId::Calculator);
        assert_eq!(
            atrium.input(&InputMessage::key(KeyCode::ENTER, KeyState::Pressed, 0)),
            AtriumAction::Launch(AppId::Calculator)
        );
    }

    #[test]
    fn app_surfaces_use_fullscreen_composition_and_close_button_is_bounded() {
        let mut atrium = Atrium::new();
        atrium.authenticate();
        for (index, app) in [AppId::Calculator, AppId::Files, AppId::Terminal, AppId::System]
            .into_iter()
            .enumerate()
        {
            let request = atrium.request_surface(app, client(index as u32 + 1)).unwrap();
            assert_eq!(request.bounds(), FULLSCREEN_SURFACE_BOUNDS);
            assert_eq!(request.mode(), SurfaceMode::Tiled);
        }
        assert!(STATUS_BAR_BOUNDS.contains(320, 16));
        assert!(STATUS_BAR_CLOSE_BOUNDS.contains(1240, 16));
        assert!(!STATUS_BAR_CLOSE_BOUNDS.contains(599, 16));
    }

    #[test]
    fn surface_requests_reject_duplicate_and_reserved_references() {
        let mut atrium = Atrium::new();
        atrium.authenticate();
        let home = surface(10);
        let lock = surface(11);
        atrium.set_surfaces(home, lock).unwrap();
        let request = atrium.request_surface(AppId::Files, client(1)).unwrap();
        assert_eq!(atrium.spawn_surface(request, home), Err(AtriumError::InvalidSurface));

        let reference = surface(12);
        let request = atrium.request_surface(AppId::Files, client(1)).unwrap();
        atrium.spawn_surface(request, reference).unwrap();
        let request = atrium.request_surface(AppId::Terminal, client(1)).unwrap();
        assert_eq!(atrium.spawn_surface(request, reference), Err(AtriumError::AlreadyRegistered));
    }

    #[test]
    fn surface_requests_allow_one_surface_per_client_and_app() {
        let mut atrium = Atrium::new();
        atrium.authenticate();
        let request = atrium.request_surface(AppId::Files, client(1)).unwrap();
        atrium.spawn_surface(request, surface(12)).unwrap();
        assert_eq!(
            atrium.request_surface(AppId::Files, client(1)),
            Err(AtriumError::AlreadyRegistered)
        );
        assert!(atrium.request_surface(AppId::Terminal, client(1)).is_ok());
        assert!(atrium.request_surface(AppId::Files, client(2)).is_ok());
    }

    #[test]
    fn surface_reference_lookup_is_exact_and_generation_safe() {
        let mut atrium = Atrium::new();
        atrium.authenticate();
        let request = atrium.request_surface(AppId::Calculator, client(1)).unwrap();
        let reference = surface(2);
        let created = atrium.spawn_surface(request, reference).unwrap();
        assert_eq!(created.client, client(1));
        assert_eq!(atrium.surface_by_reference(reference), Some(created));
        assert_eq!(atrium.surface_for_client(client(1), AppId::Calculator), Some(created));
        assert_eq!(atrium.surface_for_client(client(2), AppId::Calculator), None);
        let stale = SurfaceHandle { generation: reference.generation + 1, ..reference };
        assert_eq!(atrium.surface_by_reference(stale), None);
    }

    #[test]
    fn surface_requests_require_a_live_client_identity() {
        let mut atrium = Atrium::new();
        atrium.authenticate();
        assert_eq!(
            atrium.request_surface(AppId::Terminal, ServiceHandle::EMPTY),
            Err(AtriumError::InvalidSurface)
        );
    }

    #[test]
    fn stale_client_surface_can_be_retired_by_exact_reference() {
        let mut atrium = Atrium::new();
        atrium.authenticate();
        let request = atrium.request_surface(AppId::Terminal, client(1)).unwrap();
        let created = atrium.spawn_surface(request, surface(9)).unwrap();
        assert_eq!(atrium.close_reference(created.reference), Ok(created));
        assert_eq!(atrium.surface_by_reference(created.reference), None);
        assert_eq!(atrium.close_reference(created.reference), Err(AtriumError::NotFound));
    }

    #[test]
    fn repeated_authentication_preserves_live_surfaces() {
        let mut atrium = Atrium::new();
        atrium.authenticate();
        let request = atrium.request_surface(AppId::Files, client(1)).unwrap();
        let created = atrium.spawn_surface(request, surface(5)).unwrap();

        atrium.authenticate();

        assert_eq!(atrium.surface(created.id), Some(created));
        assert_eq!(atrium.focused_surface(), Some(created));
    }

    #[test]
    fn home_surface_admission_is_session_bound() {
        let mut atrium = Atrium::new();
        let home = surface(6);
        assert_eq!(atrium.set_home_surface(home), Err(AtriumError::Locked));

        atrium.authenticate();
        atrium.set_surfaces(home, surface(7)).unwrap();
        atrium.set_home_surface(home).unwrap();
        assert_eq!(
            atrium.input(&InputMessage::key(KeyCode::RIGHT, KeyState::Pressed, 0)),
            AtriumAction::LauncherChanged
        );
        atrium.logout();
        assert_eq!(atrium.set_home_surface(surface(7)), Err(AtriumError::Locked));
        assert!(!atrium.home_surface().is_valid());
        assert!(!atrium.lock_surface().is_valid());
        assert_eq!(atrium.launcher_index(), 0);
    }

    #[test]
    fn dynamic_layout_updates_bounds_and_splitter_dragging() {
        let mut atrium = Atrium::new();
        atrium.authenticate();

        let calculator_request = atrium.request_surface(AppId::Calculator, client(1)).unwrap();
        let calculator = atrium.spawn_surface(calculator_request, surface(3)).unwrap();
        let files_request = atrium.request_surface(AppId::Files, client(1)).unwrap();
        let files = atrium.spawn_surface(files_request, surface(4)).unwrap();
        assert_eq!(atrium.surface(calculator.id).unwrap().bounds, GuiRect::new(0, 0, 640, 800));
        assert_eq!(files.bounds, GuiRect::new(640, 0, 640, 800));
        assert_eq!(atrium.surface_at(100, 100).unwrap().id, calculator.id);
        assert_eq!(atrium.surface_at(700, 100).unwrap().id, files.id);
        assert_eq!(atrium.splitter_at(640, 100), Some(0));
        atrium.focus(calculator.id).unwrap();
        assert_eq!(atrium.surface_at(700, 100).unwrap().id, files.id);
        assert_eq!(atrium.surface_at(0, 0).unwrap().id, calculator.id);
        assert!(atrium.handle_splitter_pointer(
            &InputMessage::pointer(640, 100, 1, PointerState::Down).unwrap()
        ));
        assert!(atrium.handle_splitter_pointer(
            &InputMessage::pointer(700, 100, 1, PointerState::Move).unwrap()
        ));
        assert!(atrium.handle_splitter_pointer(
            &InputMessage::pointer(700, 100, 0, PointerState::Up).unwrap()
        ));
        assert_eq!(atrium.surface(calculator.id).unwrap().bounds.width, 700);
        assert_eq!(atrium.surface(files.id).unwrap().bounds.x, 700);
        let stale = SurfaceHandle {
            generation: calculator.reference.generation + 1,
            ..calculator.reference
        };
        assert_eq!(atrium.focus_reference(stale), Err(AtriumError::NotFound));
        atrium.focus_reference(files.reference).unwrap();
        assert_eq!(atrium.focused_surface().unwrap().reference, files.reference);
        let focused = atrium.focus_at(800, 100).unwrap();
        assert_eq!(focused.reference, files.reference);
        assert_eq!(atrium.focus_at(0, 0).unwrap().id, calculator.id);
    }

    #[test]
    fn closing_a_surface_collapses_its_split() {
        let mut atrium = Atrium::new();
        atrium.authenticate();
        let first = atrium
            .spawn_surface(
                atrium.request_surface(AppId::Calculator, client(1)).unwrap(),
                surface(1),
            )
            .unwrap();
        let second = atrium
            .spawn_surface(atrium.request_surface(AppId::Files, client(1)).unwrap(), surface(2))
            .unwrap();

        assert_eq!(atrium.close_reference(second.reference), Ok(second));
        assert_eq!(atrium.surfaces().count(), 1);
        assert_eq!(atrium.surface(first.id).unwrap().bounds, FULLSCREEN_SURFACE_BOUNDS);
    }

    #[test]
    fn pointer_focuses_and_captures_surface_until_release() {
        let mut atrium = Atrium::new();
        atrium.authenticate();
        let request = atrium.request_surface(AppId::Files, client(1)).unwrap();
        let files = atrium.spawn_surface(request, surface(8)).unwrap();

        let down = InputMessage::pointer(260, 100, 1, PointerState::Down).unwrap();
        assert_eq!(
            atrium.pointer_target(&down).map(|surface| surface.reference),
            Some(files.reference)
        );
        assert_eq!(
            atrium.focused_surface().map(|surface| surface.reference),
            Some(files.reference)
        );

        let move_event = InputMessage::pointer(0, 0, 1, PointerState::Move).unwrap();
        assert_eq!(
            atrium.pointer_target(&move_event).map(|surface| surface.reference),
            Some(files.reference)
        );
        let up = InputMessage::pointer(0, 0, 0, PointerState::Up).unwrap();
        assert_eq!(
            atrium.pointer_target(&up).map(|surface| surface.reference),
            Some(files.reference)
        );
        assert_eq!(
            atrium.pointer_target(&move_event).map(|surface| surface.reference),
            Some(files.reference)
        );
    }

    #[test]
    fn coalesces_motion_without_dropping_button_edges() {
        let first = InputMessage::pointer(1, 1, 0, PointerState::Move).unwrap();
        let queued = [
            InputMessage::pointer(2, 2, 0, PointerState::Move).unwrap(),
            InputMessage::pointer(3, 3, 0, PointerState::Move).unwrap(),
            InputMessage::pointer(3, 3, 1, PointerState::Down).unwrap(),
        ];
        let mut index = 0;
        let (latest, deferred) = coalesce_pointer_move(first, &mut |event| {
            let Some(next) = queued.get(index).copied() else { return false };
            *event = next;
            index += 1;
            true
        });

        assert_eq!(latest.pointer_event().unwrap().x, 3);
        assert_eq!(latest.pointer_event().unwrap().y, 3);
        assert_eq!(deferred.unwrap().pointer_event().unwrap().state, PointerState::Down);
    }

    #[test]
    fn calculator_handles_four_operations_and_division_by_zero() {
        let mut calculator = Calculator::new();
        for byte in b"12" {
            calculator.input(&key(*byte));
        }
        calculator.input(&key(b'+'));
        for byte in b"3" {
            calculator.input(&key(*byte));
        }
        calculator.input(&InputMessage::key(KeyCode::ENTER, KeyState::Pressed, 0));
        assert_eq!(calculator.display(), b"15");

        calculator.input(&key(b'/'));
        calculator.input(&key(b'0'));
        calculator.input(&InputMessage::key(KeyCode::ENTER, KeyState::Pressed, 0));
        assert_eq!(calculator.display(), b"ERR");
    }

    #[test]
    fn calculator_accepts_committed_keyboard_text() {
        let mut calculator = Calculator::new();
        let text = InputMessage::text(b"12+3").unwrap();
        assert!(calculator.input(&text));
        calculator.input(&InputMessage::key(KeyCode::ENTER, KeyState::Pressed, 0));
        assert_eq!(calculator.display(), b"15");
    }

    #[test]
    fn calculator_accepts_keypad_pointer_buttons() {
        let mut calculator = Calculator::new();
        for index in [0, 15, 2, 14] {
            let row = index / 4;
            let column = index % 4;
            let x = CALCULATOR_BUTTON_LEFT
                + column * (CALCULATOR_BUTTON_WIDTH + CALCULATOR_BUTTON_GAP)
                + 1;
            let y = CALCULATOR_BUTTON_TOP
                + row * (CALCULATOR_BUTTON_HEIGHT + CALCULATOR_BUTTON_GAP)
                + 1;
            let event = InputMessage::pointer(x as i16, y as i16, 1, PointerState::Down).unwrap();
            assert!(calculator.input(&event));
        }
        assert_eq!(calculator.display(), b"16");
    }
}
