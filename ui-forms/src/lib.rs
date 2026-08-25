#![no_std]

#[cfg(test)]
extern crate std;

pub const MAX_VALIDATION_ERRORS: usize = 4;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ValidationError {
    Required,
    MinLength(u8),
    Mismatch,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ValidationErrors {
    entries: [ValidationError; MAX_VALIDATION_ERRORS],
    len: u8,
}

impl ValidationErrors {
    pub const fn new() -> Self {
        Self { entries: [ValidationError::Required; MAX_VALIDATION_ERRORS], len: 0 }
    }

    pub fn clear(&mut self) {
        self.len = 0;
    }

    pub fn push(&mut self, error: ValidationError) -> bool {
        if usize::from(self.len) == MAX_VALIDATION_ERRORS {
            return false;
        }
        self.entries[usize::from(self.len)] = error;
        self.len += 1;
        true
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub const fn len(&self) -> usize {
        self.len as usize
    }

    pub fn contains(&self, error: ValidationError) -> bool {
        self.entries[..self.len as usize].contains(&error)
    }
}

impl Default for ValidationErrors {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Control<T: Copy + PartialEq> {
    value: T,
    disabled: bool,
    readonly: bool,
    valid: bool,
    dirty: bool,
    touched: bool,
    focused: bool,
    generation: u32,
    errors: ValidationErrors,
}

impl<T: Copy + PartialEq> Control<T> {
    pub const fn new(value: T) -> Self {
        Self {
            value,
            disabled: false,
            readonly: false,
            valid: false,
            dirty: false,
            touched: false,
            focused: false,
            generation: 1,
            errors: ValidationErrors::new(),
        }
    }

    pub const fn value(&self) -> T {
        self.value
    }

    pub const fn value_ref(&self) -> &T {
        &self.value
    }

    pub fn set(&mut self, value: T) -> bool {
        if self.value == value {
            return false;
        }
        self.value = value;
        self.dirty = true;
        self.bump_generation();
        true
    }

    pub fn set_user(&mut self, value: T) -> bool {
        if self.disabled || self.readonly {
            return false;
        }
        self.set(value)
    }

    pub fn value_mut(&mut self) -> &mut T {
        &mut self.value
    }

    pub fn mark_changed(&mut self) {
        self.dirty = true;
        self.bump_generation();
    }

    pub const fn generation(&self) -> u32 {
        self.generation
    }

    pub const fn disabled(&self) -> bool {
        self.disabled
    }

    pub fn set_disabled(&mut self, disabled: bool) {
        self.disabled = disabled;
        if disabled {
            self.valid = true;
        }
    }

    pub const fn readonly(&self) -> bool {
        self.readonly
    }

    pub fn set_readonly(&mut self, readonly: bool) {
        self.readonly = readonly;
    }

    pub const fn valid(&self) -> bool {
        self.valid
    }

    pub fn set_valid(&mut self, valid: bool) {
        self.valid = valid;
    }

    pub const fn dirty(&self) -> bool {
        self.dirty
    }

    pub const fn touched(&self) -> bool {
        self.touched
    }

    pub fn set_touched(&mut self, touched: bool) {
        self.touched = touched;
    }

    pub const fn focused(&self) -> bool {
        self.focused
    }

    pub fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
    }

    pub const fn errors(&self) -> &ValidationErrors {
        &self.errors
    }

    pub fn clear_errors(&mut self) {
        self.errors.clear();
    }

    pub fn add_error(&mut self, error: ValidationError) -> bool {
        self.errors.push(error)
    }

    fn bump_generation(&mut self) {
        self.generation = self.generation.wrapping_add(1).max(1);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FormState {
    valid: bool,
    dirty: bool,
    touched: bool,
    submitting: bool,
    errors: ValidationErrors,
}

impl Default for FormState {
    fn default() -> Self {
        Self::new()
    }
}

impl FormState {
    pub const fn new() -> Self {
        Self {
            valid: false,
            dirty: false,
            touched: false,
            submitting: false,
            errors: ValidationErrors::new(),
        }
    }

    pub const fn valid(&self) -> bool {
        self.valid
    }

    pub fn set_valid(&mut self, valid: bool) {
        self.valid = valid;
    }

    pub const fn dirty(&self) -> bool {
        self.dirty
    }

    pub fn set_dirty(&mut self, dirty: bool) {
        self.dirty = dirty;
    }

    pub const fn touched(&self) -> bool {
        self.touched
    }

    pub fn set_touched(&mut self, touched: bool) {
        self.touched = touched;
    }

    pub const fn submitting(&self) -> bool {
        self.submitting
    }

    pub const fn can_submit(&self) -> bool {
        self.valid && !self.submitting
    }

    pub fn set_submitting(&mut self, submitting: bool) {
        self.submitting = submitting;
    }

    pub const fn errors(&self) -> &ValidationErrors {
        &self.errors
    }

    pub fn clear_errors(&mut self) {
        self.errors.clear();
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BoundedText<const N: usize> {
    bytes: [u8; N],
    len: u8,
}

impl<const N: usize> BoundedText<N> {
    pub const fn new() -> Self {
        Self { bytes: [0; N], len: 0 }
    }

    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() > N {
            return None;
        }
        let mut value = Self::new();
        value.bytes[..bytes.len()].copy_from_slice(bytes);
        value.len = bytes.len() as u8;
        Some(value)
    }

    pub fn push(&mut self, byte: u8) -> bool {
        if usize::from(self.len) == N || !byte.is_ascii_graphic() {
            return false;
        }
        self.bytes[usize::from(self.len)] = byte;
        self.len += 1;
        true
    }

    pub fn pop(&mut self) -> bool {
        if self.len != 0 {
            self.len -= 1;
            self.bytes[usize::from(self.len)] = 0;
            true
        } else {
            false
        }
    }

    pub fn clear(&mut self) {
        self.bytes.fill(0);
        self.len = 0;
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len as usize]
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl<const N: usize> Default for BoundedText<N> {
    fn default() -> Self {
        Self::new()
    }
}

const _: () = assert!(core::mem::size_of::<ValidationErrors>() <= 16);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_tracks_bounded_mutations_and_generation() {
        let empty = BoundedText::<8>::new();
        let mut control = Control::new(empty);
        let first_generation = control.generation();
        let mut value = control.value();
        assert!(value.push(b'a'));
        assert!(control.set_user(value));
        assert!(control.dirty());
        assert!(control.generation() != first_generation);
        assert_eq!(control.value().as_bytes(), b"a");
    }

    #[test]
    fn disabled_and_readonly_controls_reject_user_updates() {
        let mut control = Control::new(BoundedText::<8>::new());
        control.set_disabled(true);
        let mut value = control.value();
        assert!(value.push(b'a'));
        assert!(!control.set_user(value));
        control.set_disabled(false);
        control.set_readonly(true);
        assert!(!control.set_user(value));
    }

    #[test]
    fn validation_errors_are_fixed_and_bounded() {
        let mut errors = ValidationErrors::new();
        for _ in 0..MAX_VALIDATION_ERRORS {
            assert!(errors.push(ValidationError::Required));
        }
        assert!(!errors.push(ValidationError::Required));
        assert_eq!(errors.len(), MAX_VALIDATION_ERRORS);
        assert!(errors.contains(ValidationError::Required));
    }
}
