#![no_std]

#[cfg(test)]
extern crate std;

use logos_abi::{
    InputMessage, KeyCode, KeyState, MAX_TEXT_BYTES, MOD_SHIFT, MessageKind, UserOperation,
    UserStatus,
};
use logos_ui_forms::{BoundedText, Control, FormState, ValidationError};

pub const MAX_FIELD_BYTES: usize = 32;
pub const MAX_RETRIES: u8 = 3;

pub type LoginText = BoundedText<MAX_FIELD_BYTES>;

pub struct LoginControls {
    pub username: Control<LoginText>,
    pub password: Control<LoginText>,
    pub confirm_password: Control<LoginText>,
}

pub struct LoginForm {
    pub controls: LoginControls,
    state: FormState,
    claim: bool,
}

impl LoginForm {
    pub const fn new() -> Self {
        Self {
            controls: LoginControls {
                username: Control::new(LoginText::new()),
                password: Control::new(LoginText::new()),
                confirm_password: Control::new(LoginText::new()),
            },
            state: FormState::new(),
            claim: false,
        }
    }

    pub const fn valid(&self) -> bool {
        self.state.valid()
    }

    pub const fn dirty(&self) -> bool {
        self.state.dirty()
    }

    pub const fn touched(&self) -> bool {
        self.state.touched()
    }

    pub const fn submitting(&self) -> bool {
        self.state.submitting()
    }

    pub const fn can_submit(&self) -> bool {
        self.state.can_submit()
    }

    pub const fn is_claim(&self) -> bool {
        self.claim
    }

    pub const fn errors(&self) -> &logos_ui_forms::ValidationErrors {
        self.state.errors()
    }

    pub fn revalidate(&mut self) {
        let username_empty = self.controls.username.value_ref().is_empty();
        let password_empty = self.controls.password.value_ref().is_empty();
        let confirmation_empty = self.controls.confirm_password.value_ref().is_empty();
        let confirmation_matches =
            self.controls.confirm_password.value_ref() == self.controls.password.value_ref();
        self.controls.username.clear_errors();
        self.controls.password.clear_errors();
        self.controls.confirm_password.clear_errors();
        if username_empty {
            let _ = self.controls.username.add_error(ValidationError::Required);
        }
        if password_empty {
            let _ = self.controls.password.add_error(ValidationError::Required);
        }
        if self.claim && confirmation_empty {
            let _ = self.controls.confirm_password.add_error(ValidationError::Required);
        } else if self.claim && !confirmation_matches {
            let _ = self.controls.confirm_password.add_error(ValidationError::Mismatch);
        }
        self.controls.username.set_valid(!username_empty);
        self.controls.password.set_valid(!password_empty);
        self.controls
            .confirm_password
            .set_valid(!self.claim || (!confirmation_empty && confirmation_matches));
        self.state.set_valid(
            !username_empty
                && !password_empty
                && (!self.claim || (!confirmation_empty && confirmation_matches)),
        );
        self.state.set_dirty(
            self.controls.username.dirty()
                || self.controls.password.dirty()
                || self.controls.confirm_password.dirty(),
        );
    }

    pub fn set_claim_mode(&mut self, claim: bool) {
        self.claim = claim;
        self.revalidate();
    }

    pub fn begin_submission(&mut self) -> bool {
        if self.submitting() {
            return false;
        }
        self.revalidate();
        self.state.set_touched(true);
        self.controls.username.set_touched(true);
        self.controls.password.set_touched(true);
        self.controls.confirm_password.set_touched(true);
        if !self.can_submit() {
            return false;
        }
        self.state.set_submitting(true);
        true
    }

    pub fn complete_submission(&mut self) {
        self.state.set_submitting(false);
    }

    pub fn clear_password(&mut self) {
        self.controls.password.value_mut().clear();
        self.controls.password.mark_changed();
        self.controls.confirm_password.value_mut().clear();
        self.controls.confirm_password.mark_changed();
        self.revalidate();
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }
}

impl Default for LoginForm {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LockScreenMode {
    Claim,
    Login,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LockScreenField {
    Username,
    Password,
    ConfirmPassword,
    Submit,
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
    form: LoginForm,
    retries: u8,
    failure: bool,
}

impl LockScreen {
    pub const fn new() -> Self {
        Self {
            mode: LockScreenMode::Login,
            field: LockScreenField::Username,
            form: LoginForm::new(),
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

    pub const fn form(&self) -> &LoginForm {
        &self.form
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
        if self.form.submitting() {
            return LockScreenAction::Ignored;
        }
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
                code if code == KeyCode::BackTab
                    || (code == KeyCode::Tab && input.modifiers & MOD_SHIFT != 0) =>
                {
                    self.move_field(false);
                    LockScreenAction::Changed
                }
                code if code == KeyCode::Tab || code == KeyCode::Down => {
                    self.move_field(true);
                    LockScreenAction::Changed
                }
                code if code == KeyCode::Up => {
                    self.move_field(false);
                    LockScreenAction::Changed
                }
                code if code == KeyCode::Backspace => {
                    self.pop();
                    LockScreenAction::Changed
                }
                code if code == KeyCode::Enter => self.submit(),
                _ => LockScreenAction::Ignored,
            },
            _ => LockScreenAction::Ignored,
        }
    }

    /// Validate the active login form. Enter and a future button adapter share this path.
    pub fn submit(&mut self) -> LockScreenAction {
        if self.form.submitting() {
            return LockScreenAction::Ignored;
        }
        if self.form.begin_submission() {
            LockScreenAction::Submit(self.mode.operation())
        } else {
            LockScreenAction::Changed
        }
    }

    pub fn credentials(&self) -> (&[u8], &[u8]) {
        (
            self.form.controls.username.value_ref().as_bytes(),
            self.form.controls.password.value_ref().as_bytes(),
        )
    }

    pub fn confirmation(&self) -> &[u8] {
        self.form.controls.confirm_password.value_ref().as_bytes()
    }

    pub fn apply_status(&mut self, status: UserStatus) {
        self.form.complete_submission();
        self.form.clear_password();
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
        self.form.clear_password();
    }

    pub fn cancel_submission(&mut self) {
        self.form.complete_submission();
    }

    fn push(&mut self, byte: u8) {
        let control = match self.field {
            LockScreenField::Username => &mut self.form.controls.username,
            LockScreenField::Password => &mut self.form.controls.password,
            LockScreenField::ConfirmPassword => &mut self.form.controls.confirm_password,
            LockScreenField::Submit => return,
        };
        let mut value = control.value();
        if value.push(byte) {
            let _ = control.set_user(value);
            self.form.revalidate();
        }
    }

    fn pop(&mut self) {
        let control = match self.field {
            LockScreenField::Username => &mut self.form.controls.username,
            LockScreenField::Password => &mut self.form.controls.password,
            LockScreenField::ConfirmPassword => &mut self.form.controls.confirm_password,
            LockScreenField::Submit => return,
        };
        if control.value_mut().pop() {
            control.mark_changed();
            self.form.revalidate();
        }
    }

    fn move_field(&mut self, forward: bool) {
        self.field = if self.mode == LockScreenMode::Claim {
            match (self.field, forward) {
                (LockScreenField::Username, true) => LockScreenField::Password,
                (LockScreenField::Password, true) => LockScreenField::ConfirmPassword,
                (LockScreenField::ConfirmPassword, true) => LockScreenField::Submit,
                (LockScreenField::Submit, true) => LockScreenField::Submit,
                (LockScreenField::Submit, false) => LockScreenField::ConfirmPassword,
                (LockScreenField::ConfirmPassword, false) => LockScreenField::Password,
                (LockScreenField::Password, false) => LockScreenField::Username,
                (LockScreenField::Username, false) => LockScreenField::Username,
            }
        } else {
            match (self.field, forward) {
                (LockScreenField::Username, true) => LockScreenField::Password,
                (LockScreenField::Password, true) => LockScreenField::Submit,
                (LockScreenField::Submit, true) => LockScreenField::Submit,
                (LockScreenField::Submit, false) => LockScreenField::Password,
                (LockScreenField::Password, false) => LockScreenField::Username,
                (LockScreenField::Username, false) => LockScreenField::Username,
                (LockScreenField::ConfirmPassword, _) => LockScreenField::Password,
            }
        };
    }

    fn reset_fields(&mut self) {
        self.form.reset();
        self.form.set_claim_mode(self.mode == LockScreenMode::Claim);
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
        assert_eq!(lock.credentials().0, b"alice");
        assert_eq!(
            lock.input(InputMessage::key(KeyCode::Tab, KeyState::Pressed, 0)),
            LockScreenAction::Changed
        );
        assert_eq!(lock.input(InputMessage::text(b"secret").unwrap()), LockScreenAction::Changed);
        assert_eq!(lock.credentials().1, b"secret");
        assert_eq!(
            lock.input(InputMessage::key(KeyCode::Tab, KeyState::Pressed, 0)),
            LockScreenAction::Changed
        );
        assert_eq!(lock.field(), LockScreenField::ConfirmPassword);
        assert_eq!(lock.input(InputMessage::text(b"secret").unwrap()), LockScreenAction::Changed);
        assert_eq!(lock.submit(), LockScreenAction::Submit(UserOperation::Claim));
        lock.cancel_submission();
        assert_eq!(
            lock.input(InputMessage::key(KeyCode::Tab, KeyState::Pressed, MOD_SHIFT)),
            LockScreenAction::Changed
        );
        assert_eq!(
            lock.input(InputMessage::key(KeyCode::Tab, KeyState::Pressed, MOD_SHIFT)),
            LockScreenAction::Changed
        );
        assert_eq!(lock.field(), LockScreenField::Username);
        assert_eq!(lock.input(InputMessage::text(b"2").unwrap()), LockScreenAction::Changed);
        assert_eq!(lock.credentials().0, b"alice2");
        assert_eq!(
            lock.input(InputMessage::key(KeyCode::BackTab, KeyState::Pressed, 0)),
            LockScreenAction::Changed
        );
        assert_eq!(lock.field(), LockScreenField::Username);
        assert_eq!(
            lock.input(InputMessage::key(KeyCode::Down, KeyState::Pressed, 0)),
            LockScreenAction::Changed
        );
        assert_eq!(lock.field(), LockScreenField::Password);
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

    #[test]
    fn enter_validates_the_form_before_submitting() {
        let mut lock = LockScreen::new();
        lock.set_locked();
        let enter = InputMessage::key(KeyCode::Enter, KeyState::Pressed, 0);
        assert_eq!(lock.submit(), LockScreenAction::Changed);
        assert!(lock.form().controls.username.touched());
        assert!(lock.form().controls.username.errors().contains(ValidationError::Required));
        assert!(lock.form().controls.password.errors().contains(ValidationError::Required));
        assert_eq!(lock.input(enter), LockScreenAction::Changed);
        let _ = lock.input(InputMessage::text(b"alice").unwrap());
        let _ = lock.input(InputMessage::key(KeyCode::Tab, KeyState::Pressed, 0));
        assert_eq!(lock.input(enter), LockScreenAction::Changed);
        let _ = lock.input(InputMessage::text(b"secret").unwrap());
        assert!(lock.form().can_submit());
        assert_eq!(lock.input(enter), LockScreenAction::Submit(UserOperation::Login));
        assert!(lock.form().submitting());
        assert!(!lock.form().can_submit());
        assert_eq!(lock.input(enter), LockScreenAction::Ignored);
    }

    #[test]
    fn claim_requires_matching_password_confirmation() {
        let mut lock = LockScreen::new();
        lock.set_unclaimed();
        let _ = lock.input(InputMessage::text(b"admin").unwrap());
        let _ = lock.input(InputMessage::key(KeyCode::Tab, KeyState::Pressed, 0));
        let _ = lock.input(InputMessage::text(b"secret").unwrap());
        let _ = lock.input(InputMessage::key(KeyCode::Tab, KeyState::Pressed, 0));
        let _ = lock.input(InputMessage::text(b"different").unwrap());

        assert_eq!(lock.submit(), LockScreenAction::Changed);
        assert!(lock.form().controls.confirm_password.touched());
        assert!(lock.form().controls.confirm_password.errors().contains(ValidationError::Mismatch));
        assert!(!lock.form().can_submit());
    }
}
