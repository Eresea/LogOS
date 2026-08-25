#![no_std]

#[cfg(test)]
extern crate std;

use logos_abi::{
    InputMessage, KeyCode, KeyState, MAX_TEXT_BYTES, MessageKind, UserOperation, UserStatus,
};

pub const MAX_FIELD_BYTES: usize = 32;
pub const MAX_RETRIES: u8 = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LockScreenMode {
    Claim,
    Login,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LockScreenField {
    Username,
    Password,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LockScreenAction {
    Changed,
    Submit(UserOperation),
    Ignored,
}

pub struct LockScreen {
    mode: LockScreenMode,
    field: LockScreenField,
    username: [u8; MAX_FIELD_BYTES],
    username_len: u8,
    password: [u8; MAX_FIELD_BYTES],
    password_len: u8,
    retries: u8,
    failure: bool,
}

impl LockScreen {
    pub const fn new() -> Self {
        Self {
            mode: LockScreenMode::Login,
            field: LockScreenField::Username,
            username: [0; MAX_FIELD_BYTES],
            username_len: 0,
            password: [0; MAX_FIELD_BYTES],
            password_len: 0,
            retries: 0,
            failure: false,
        }
    }

    pub const fn mode(&self) -> LockScreenMode {
        self.mode
    }
    pub const fn field(&self) -> LockScreenField {
        self.field
    }
    pub const fn retries(&self) -> u8 {
        self.retries
    }
    pub const fn failure(&self) -> bool {
        self.failure
    }

    pub fn set_unclaimed(&mut self) {
        self.mode = LockScreenMode::Claim;
        self.reset_fields();
    }

    pub fn set_locked(&mut self) {
        self.mode = LockScreenMode::Login;
        self.reset_fields();
    }

    pub fn input(&mut self, input: InputMessage) -> LockScreenAction {
        match input.kind {
            MessageKind::Text | MessageKind::Paste => {
                let Some(text) = input.text_bytes() else { return LockScreenAction::Ignored };
                if text.len() > MAX_TEXT_BYTES {
                    return LockScreenAction::Ignored;
                }
                for byte in text.iter().copied() {
                    self.push(byte);
                }
                LockScreenAction::Changed
            }
            MessageKind::Key if input.state == KeyState::Pressed => match KeyCode(input.code) {
                code if code == KeyCode::Tab || code == KeyCode::Down => {
                    self.field = LockScreenField::Password;
                    LockScreenAction::Changed
                }
                code if code == KeyCode::Up => {
                    self.field = LockScreenField::Username;
                    LockScreenAction::Changed
                }
                code if code == KeyCode::Backspace => {
                    self.pop();
                    LockScreenAction::Changed
                }
                code if code == KeyCode::Enter => {
                    if self.username_len == 0 || self.password_len == 0 {
                        LockScreenAction::Ignored
                    } else {
                        LockScreenAction::Submit(self.mode.operation())
                    }
                }
                _ => LockScreenAction::Ignored,
            },
            _ => LockScreenAction::Ignored,
        }
    }

    pub fn credentials(&self) -> (&[u8], &[u8]) {
        (&self.username[..self.username_len as usize], &self.password[..self.password_len as usize])
    }

    pub fn apply_status(&mut self, status: UserStatus) {
        self.password.fill(0);
        self.password_len = 0;
        match status {
            UserStatus::Unclaimed => self.set_unclaimed(),
            UserStatus::BadCredentials => {
                self.failure = true;
                self.retries = self.retries.saturating_add(1).min(MAX_RETRIES);
                self.field = LockScreenField::Password;
            }
            UserStatus::Ok => self.set_locked(),
            _ => self.failure = true,
        }
    }

    pub fn clear_password(&mut self) {
        self.password.fill(0);
        self.password_len = 0;
    }

    fn push(&mut self, byte: u8) {
        let (buffer, length) = match self.field {
            LockScreenField::Username => (&mut self.username, &mut self.username_len),
            LockScreenField::Password => (&mut self.password, &mut self.password_len),
        };
        if usize::from(*length) < buffer.len() && byte.is_ascii_graphic() {
            buffer[*length as usize] = byte;
            *length += 1;
        }
    }

    fn pop(&mut self) {
        let length = match self.field {
            LockScreenField::Username => &mut self.username_len,
            LockScreenField::Password => &mut self.password_len,
        };
        if *length > 0 {
            *length -= 1;
        }
    }

    fn reset_fields(&mut self) {
        self.username.fill(0);
        self.password.fill(0);
        self.username_len = 0;
        self.password_len = 0;
        self.retries = 0;
        self.failure = false;
        self.field = LockScreenField::Username;
    }
}

impl LockScreenMode {
    const fn operation(self) -> UserOperation {
        match self {
            Self::Claim => UserOperation::Claim,
            Self::Login => UserOperation::Login,
        }
    }
}

impl Default for LockScreen {
    fn default() -> Self {
        Self::new()
    }
}

const _: () = assert!(core::mem::size_of::<LockScreen>() <= 256);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claim_and_login_input_is_bounded() {
        let mut lock = LockScreen::new();
        lock.set_unclaimed();
        assert_eq!(lock.input(InputMessage::text(b"alice").unwrap()), LockScreenAction::Changed);
        assert_eq!(
            lock.input(InputMessage::key(KeyCode::Tab, KeyState::Pressed, 0)),
            LockScreenAction::Changed
        );
        assert_eq!(lock.input(InputMessage::text(b"secret").unwrap()), LockScreenAction::Changed);
        assert_eq!(
            lock.input(InputMessage::key(KeyCode::Enter, KeyState::Pressed, 0)),
            LockScreenAction::Submit(UserOperation::Claim)
        );
        lock.clear_password();
        assert!(lock.credentials().1.is_empty());
    }

    #[test]
    fn failed_login_is_redrawn_with_bounded_retries() {
        let mut lock = LockScreen::new();
        lock.apply_status(UserStatus::BadCredentials);
        assert!(lock.failure());
        assert_eq!(lock.retries(), 1);
        for _ in 0..10 {
            lock.apply_status(UserStatus::BadCredentials);
        }
        assert_eq!(lock.retries(), MAX_RETRIES);
        assert!(lock.credentials().1.is_empty());
    }
}
