#![no_std]

#[cfg(test)]
extern crate std;

use core::fmt::Write;

use logos_abi::{GuiRect, InputMessage, KeyCode, KeyState, MOD_CTRL, SurfaceHandle};

pub const MAX_ATRIUM_WINDOWS: usize = 4;
pub const MAX_CALCULATOR_TEXT: usize = 32;
pub const WINDOW_MOVE_STEP: i32 = 32;

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
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WindowMode {
    Tiled,
    Floating,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Window {
    pub id: u16,
    pub app: AppId,
    pub surface: SurfaceHandle,
    pub bounds: GuiRect,
    pub mode: WindowMode,
    pub focused: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AtriumError {
    Locked,
    Capacity,
    NotFound,
    InvalidSurface,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AtriumAction {
    None,
    Launch(AppId),
    FocusNext,
    FocusPrevious,
    MoveFocused(i32, i32),
    CloseFocused,
    Logout,
}

pub struct Atrium {
    phase: AtriumPhase,
    windows: [Option<Window>; MAX_ATRIUM_WINDOWS],
    focused: Option<usize>,
    launcher_index: usize,
    next_window_id: u16,
    home_surface: SurfaceHandle,
    lock_surface: SurfaceHandle,
}

impl Atrium {
    pub const fn new() -> Self {
        Self {
            phase: AtriumPhase::Boot,
            windows: [None; MAX_ATRIUM_WINDOWS],
            focused: None,
            launcher_index: 0,
            next_window_id: 1,
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

    pub fn focused_window(&self) -> Option<Window> {
        match self.focused {
            Some(index) => self.windows[index],
            None => None,
        }
    }

    pub fn set_home_surface(&mut self, surface: SurfaceHandle) -> Result<(), AtriumError> {
        if !surface.is_valid() {
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
        if !home.is_valid() || !lock.is_valid() {
            return Err(AtriumError::InvalidSurface);
        }
        self.home_surface = home;
        self.lock_surface = lock;
        Ok(())
    }

    pub fn lock(&mut self) {
        self.phase = AtriumPhase::Locked;
        self.clear_windows();
    }

    pub fn authenticate(&mut self) {
        self.phase = AtriumPhase::Home;
        self.clear_windows();
    }

    pub fn logout(&mut self) {
        self.lock();
    }

    pub fn restart(&mut self) {
        self.phase = AtriumPhase::Boot;
        self.clear_windows();
        self.home_surface = SurfaceHandle::EMPTY;
        self.lock_surface = SurfaceHandle::EMPTY;
    }

    pub fn window(&self, id: u16) -> Option<Window> {
        self.windows.iter().flatten().copied().find(|window| window.id == id)
    }

    pub fn windows(&self) -> impl Iterator<Item = Window> + '_ {
        self.windows.iter().flatten().copied()
    }

    pub fn launch(
        &mut self,
        app: AppId,
        surface: SurfaceHandle,
        bounds: GuiRect,
    ) -> Result<u16, AtriumError> {
        if self.phase != AtriumPhase::Home {
            return Err(AtriumError::Locked);
        }
        if !surface.is_valid() {
            return Err(AtriumError::InvalidSurface);
        }
        let Some(index) = self.windows.iter().position(Option::is_none) else {
            return Err(AtriumError::Capacity);
        };
        let id = self.next_window_id;
        self.next_window_id = self.next_window_id.wrapping_add(1).max(1);
        let window = Window {
            id,
            app,
            surface,
            bounds,
            mode: if app == AppId::Terminal { WindowMode::Tiled } else { WindowMode::Floating },
            focused: true,
        };
        self.clear_focus();
        self.windows[index] = Some(window);
        self.focused = Some(index);
        Ok(id)
    }

    pub fn focus(&mut self, id: u16) -> Result<(), AtriumError> {
        let Some(index) = self.windows.iter().position(|window| window.is_some_and(|w| w.id == id))
        else {
            return Err(AtriumError::NotFound);
        };
        self.clear_focus();
        if let Some(window) = &mut self.windows[index] {
            window.focused = true;
        }
        self.focused = Some(index);
        Ok(())
    }

    pub fn move_focused(&mut self, dx: i32, dy: i32) -> Result<(), AtriumError> {
        let Some(index) = self.focused else { return Err(AtriumError::NotFound) };
        let Some(window) = &mut self.windows[index] else { return Err(AtriumError::NotFound) };
        if window.mode == WindowMode::Tiled {
            return Ok(());
        }
        window.bounds.x = window.bounds.x.saturating_add(dx);
        window.bounds.y = window.bounds.y.saturating_add(dy);
        Ok(())
    }

    pub fn close_focused(&mut self) -> Result<Window, AtriumError> {
        let Some(index) = self.focused else { return Err(AtriumError::NotFound) };
        let Some(window) = self.windows[index].take() else { return Err(AtriumError::NotFound) };
        self.focused = None;
        self.focus_next(1);
        Ok(window)
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
            (true, KeyCode::LEFT) => AtriumAction::MoveFocused(-WINDOW_MOVE_STEP, 0),
            (true, KeyCode::RIGHT) => AtriumAction::MoveFocused(WINDOW_MOVE_STEP, 0),
            (true, KeyCode::UP) => AtriumAction::MoveFocused(0, -WINDOW_MOVE_STEP),
            (true, KeyCode::DOWN) => AtriumAction::MoveFocused(0, WINDOW_MOVE_STEP),
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
                AtriumAction::None
            }
            (false, KeyCode::RIGHT) => {
                self.launcher_index = (self.launcher_index + 1).min(2);
                AtriumAction::None
            }
            (true, _) => match code.character_byte() {
                Some(b'l') => AtriumAction::Logout,
                // The default decoder is French AZERTY, where the number-row
                // semantic codes are &, é, and ". Keep the logical shortcuts
                // usable without making the shell depend on one layout.
                Some(b'1' | b'&') => AtriumAction::Launch(AppId::Calculator),
                Some(b'2' | b'e') => AtriumAction::Launch(AppId::Files),
                Some(b'3' | b'"') => AtriumAction::Launch(AppId::Terminal),
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
            AtriumAction::None | AtriumAction::Launch(_) => Ok(()),
        }
    }

    fn clear_windows(&mut self) {
        self.windows = [None; MAX_ATRIUM_WINDOWS];
        self.focused = None;
    }

    fn clear_focus(&mut self) {
        for window in self.windows.iter_mut().flatten() {
            window.focused = false;
        }
    }

    fn focus_next(&mut self, direction: isize) {
        let Some(current) = self.focused else {
            self.focused = self.windows.iter().position(Option::is_some);
            if let Some(index) = self.focused {
                self.clear_focus();
                if let Some(window) = &mut self.windows[index] {
                    window.focused = true;
                }
            }
            return;
        };
        let mut index = current as isize;
        for _ in 0..MAX_ATRIUM_WINDOWS {
            index = (index + direction).rem_euclid(MAX_ATRIUM_WINDOWS as isize);
            if self.windows[index as usize].is_some() {
                self.clear_focus();
                let index = index as usize;
                if let Some(window) = &mut self.windows[index] {
                    window.focused = true;
                }
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

    fn key(byte: u8) -> InputMessage {
        InputMessage::key(KeyCode::character(byte), KeyState::Pressed, 0)
    }

    fn ctrl(byte: u8) -> InputMessage {
        InputMessage::key(KeyCode::character(byte), KeyState::Pressed, MOD_CTRL)
    }

    #[test]
    fn phase_and_window_lifecycle_is_bounded() {
        let mut atrium = Atrium::new();
        assert_eq!(atrium.phase(), AtriumPhase::Boot);
        atrium.authenticate();
        for slot in 0..MAX_ATRIUM_WINDOWS {
            assert!(
                atrium
                    .launch(AppId::Calculator, surface(slot as u16), GuiRect::new(10, 10, 20, 20))
                    .is_ok()
            );
        }
        assert_eq!(
            atrium.launch(AppId::Files, surface(9), GuiRect::new(0, 0, 20, 20)),
            Err(AtriumError::Capacity)
        );
        atrium.logout();
        assert_eq!(atrium.phase(), AtriumPhase::Locked);
        assert_eq!(atrium.windows().count(), 0);
    }

    #[test]
    fn keyboard_actions_launch_move_and_logout() {
        let mut atrium = Atrium::new();
        atrium.authenticate();
        assert_eq!(atrium.input(&ctrl(b'1')), AtriumAction::Launch(AppId::Calculator));
        assert_eq!(atrium.input(&ctrl(b'&')), AtriumAction::Launch(AppId::Calculator));
        assert_eq!(atrium.input(&ctrl(b'e')), AtriumAction::Launch(AppId::Files));
        assert_eq!(atrium.input(&ctrl(b'"')), AtriumAction::Launch(AppId::Terminal));
        atrium.launch(AppId::Calculator, surface(1), GuiRect::new(10, 10, 20, 20)).unwrap();
        assert_eq!(atrium.input(&ctrl(b'j')), AtriumAction::None);
        assert_eq!(atrium.input(&ctrl(b'1')), AtriumAction::Launch(AppId::Calculator));
        assert_eq!(atrium.input(&ctrl(b'l')), AtriumAction::Logout);
        atrium.apply_action(AtriumAction::Logout).unwrap();
        assert_eq!(atrium.phase(), AtriumPhase::Locked);
    }

    #[test]
    fn floating_focus_move_close_and_restart_are_generation_safe() {
        let mut atrium = Atrium::new();
        atrium.authenticate();
        let id =
            atrium.launch(AppId::Calculator, surface(1), GuiRect::new(10, 20, 80, 60)).unwrap();
        assert_eq!(atrium.focused_window().unwrap().id, id);
        atrium.move_focused(WINDOW_MOVE_STEP, -WINDOW_MOVE_STEP).unwrap();
        assert_eq!(atrium.window(id).unwrap().bounds, GuiRect::new(42, -12, 80, 60));
        let closed = atrium.close_focused().unwrap();
        assert_eq!(closed.id, id);
        atrium.restart();
        assert_eq!(atrium.phase(), AtriumPhase::Boot);
        assert!(!atrium.home_surface().is_valid());
        assert_eq!(atrium.windows().count(), 0);
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
