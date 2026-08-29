#![no_std]

#[cfg(test)]
extern crate std;

use core::fmt::Write;

use logos_abi::{
    GuiRect, InputMessage, KeyCode, KeyState, MOD_CTRL, PointerState, ServiceHandle, SurfaceHandle,
};

pub const MAX_ATRIUM_SURFACES: usize = 4;
pub const MAX_CALCULATOR_TEXT: usize = 32;
pub const SURFACE_MOVE_STEP: i32 = 32;

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SurfaceMode {
    Tiled,
    Floating,
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
    launcher_index: usize,
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
            launcher_index: 0,
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
        self.launcher_index
    }

    pub const fn home_surface(&self) -> SurfaceHandle {
        self.home_surface
    }

    pub const fn lock_surface(&self) -> SurfaceHandle {
        self.lock_surface
    }

    pub fn focused_surface(&self) -> Option<Surface> {
        match self.focused {
            Some(index) => self.surfaces[index],
            None => None,
        }
    }

    pub const fn initial_surface_bounds(app: AppId) -> GuiRect {
        match app {
            AppId::Calculator => GuiRect::new(220, 72, 320, 220),
            AppId::Files => GuiRect::new(248, 88, 340, 190),
            AppId::Terminal => GuiRect::new(200, 48, 420, 300),
            AppId::System => GuiRect::new(176, 40, 448, 320),
        }
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
        Ok(SurfaceRequest {
            app,
            client,
            bounds: Self::initial_surface_bounds(app),
            mode: if app == AppId::Terminal { SurfaceMode::Tiled } else { SurfaceMode::Floating },
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
        self.clear_focus();
        self.surfaces[index] = Some(surface);
        self.focused = Some(index);
        Ok(surface)
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
        self.surfaces[index] = None;
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
        let Some(surface) = self.surfaces[index].take() else { return Err(AtriumError::NotFound) };
        if self.focused == Some(index) {
            self.focused = None;
            self.focus_next(1);
        }
        Ok(surface)
    }

    pub fn input(&mut self, input: &InputMessage) -> AtriumAction {
        if input.state != KeyState::Pressed && input.state != KeyState::Repeat {
            return AtriumAction::None;
        }
        let code = KeyCode::from_raw(input.code);
        if self.phase != AtriumPhase::Home {
            return AtriumAction::None;
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
            (false, KeyCode::ENTER) => match self.launcher_index {
                0 => AtriumAction::Launch(AppId::Calculator),
                1 => AtriumAction::Launch(AppId::Files),
                _ => AtriumAction::Launch(AppId::Terminal),
            },
            (false, KeyCode::LEFT) => {
                self.launcher_index = self.launcher_index.saturating_sub(1);
                AtriumAction::LauncherChanged
            }
            (false, KeyCode::RIGHT) => {
                self.launcher_index = (self.launcher_index + 1).min(2);
                AtriumAction::LauncherChanged
            }
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
        self.launcher_index = 0;
        self.next_focus_order = 1;
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

const _: () = assert!(core::mem::size_of::<Atrium>() <= 512);
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

    #[test]
    fn phase_and_surface_lifecycle_is_bounded() {
        let mut atrium = Atrium::new();
        assert_eq!(atrium.phase(), AtriumPhase::Boot);
        atrium.authenticate();
        for slot in 0..MAX_ATRIUM_SURFACES {
            let request =
                atrium.request_surface(AppId::Calculator, client(slot as u32 + 1)).unwrap();
            assert!(atrium.spawn_surface(request, surface(slot as u16)).is_ok());
        }
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
        let request = atrium.request_surface(AppId::Calculator, client(1)).unwrap();
        atrium.spawn_surface(request, surface(1)).unwrap();
        assert_eq!(atrium.input(&ctrl(b'j')), AtriumAction::None);
        assert_eq!(atrium.input(&ctrl(b'1')), AtriumAction::Launch(AppId::Calculator));
        assert_eq!(atrium.input(&ctrl(b'l')), AtriumAction::Logout);
        assert_eq!(
            atrium.input(&InputMessage::key(KeyCode::RIGHT, KeyState::Pressed, 0)),
            AtriumAction::LauncherChanged
        );
        assert!(!AtriumAction::LauncherChanged.routes_to_surface());
        atrium.apply_action(AtriumAction::Logout).unwrap();
        assert_eq!(atrium.phase(), AtriumPhase::Locked);
        assert!(AtriumAction::None.routes_to_surface());
        assert!(!AtriumAction::FocusNext.routes_to_surface());
        assert!(!AtriumAction::Launch(AppId::Terminal).routes_to_surface());
    }

    #[test]
    fn floating_focus_move_close_and_restart_are_generation_safe() {
        let mut atrium = Atrium::new();
        atrium.authenticate();
        let request = atrium.request_surface(AppId::Calculator, client(1)).unwrap();
        let admitted = atrium.spawn_surface(request, surface(1)).unwrap();
        assert_eq!(atrium.focused_surface().unwrap().id, admitted.id);
        atrium.move_focused(SURFACE_MOVE_STEP, -SURFACE_MOVE_STEP).unwrap();
        assert_eq!(atrium.surface(admitted.id).unwrap().bounds, GuiRect::new(252, 40, 320, 220));
        let closed = atrium.close_focused().unwrap();
        assert_eq!(closed.id, admitted.id);
        atrium.restart();
        assert_eq!(atrium.phase(), AtriumPhase::Boot);
        assert!(!atrium.home_surface().is_valid());
        assert_eq!(atrium.surfaces().count(), 0);
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
    fn surface_hit_testing_follows_focus_order() {
        let mut atrium = Atrium::new();
        atrium.authenticate();

        let calculator_request = atrium.request_surface(AppId::Calculator, client(1)).unwrap();
        let calculator = atrium.spawn_surface(calculator_request, surface(3)).unwrap();
        let files_request = atrium.request_surface(AppId::Files, client(1)).unwrap();
        let files = atrium.spawn_surface(files_request, surface(4)).unwrap();
        let overlap = (260, 100);

        assert_eq!(atrium.surface_at(overlap.0, overlap.1).unwrap().id, files.id);
        atrium.focus(calculator.id).unwrap();
        assert_eq!(atrium.surface_at(overlap.0, overlap.1).unwrap().id, calculator.id);
        assert_eq!(atrium.surface_at(0, 0), None);
        let stale = SurfaceHandle {
            generation: calculator.reference.generation + 1,
            ..calculator.reference
        };
        assert_eq!(atrium.focus_reference(stale), Err(AtriumError::NotFound));
        atrium.focus_reference(files.reference).unwrap();
        assert_eq!(atrium.focused_surface().unwrap().reference, files.reference);
        let focused = atrium.focus_at(overlap.0, overlap.1).unwrap();
        assert_eq!(focused.reference, files.reference);
        assert_eq!(atrium.focus_at(0, 0), Err(AtriumError::NotFound));
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
        assert_eq!(atrium.pointer_target(&move_event), None);
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
}
