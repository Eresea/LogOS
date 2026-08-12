//! Keyboard adapter state.  Hardware bytes stop here; terminal services only
//! receive semantic key and committed-text messages.

use crate::terminal_abi::{InputMessage, KeyCode, KeyState};

pub const MOD_SHIFT: u16 = crate::terminal_abi::MOD_SHIFT;
pub const MOD_CTRL: u16 = crate::terminal_abi::MOD_CTRL;
pub const MOD_ALT: u16 = crate::terminal_abi::MOD_ALT;
pub const MOD_CAPS_LOCK: u16 = crate::terminal_abi::MOD_CAPS_LOCK;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DecodedInput {
    pub key: InputMessage,
    pub text: Option<InputMessage>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InputDecoder {
    extended: bool,
    break_code: bool,
    modifiers: u16,
    caps_lock: bool,
}

impl InputDecoder {
    pub const fn new() -> Self {
        Self { extended: false, break_code: false, modifiers: 0, caps_lock: false }
    }

    pub const fn modifiers(&self) -> u16 {
        self.modifiers
    }

    /// Feed one PS/2 Set-2 byte. Prefix bytes produce no event.
    pub fn feed(&mut self, byte: u8) -> Option<DecodedInput> {
        match byte {
            0xe0 => {
                self.extended = true;
                None
            }
            0xf0 => {
                self.break_code = true;
                None
            }
            0xe1 => {
                self.extended = false;
                self.break_code = false;
                None
            }
            code => {
                let released = self.break_code;
                let extended = self.extended;
                self.break_code = false;
                self.extended = false;
                let key_code = map_code(code, extended)?;
                self.update_modifiers(key_code, released);
                let state = if released { KeyState::Released } else { KeyState::Pressed };
                let key = InputMessage::key(key_code, state, self.modifiers);
                let text = (!released).then(|| self.committed_text(key_code));
                Some(DecodedInput { key, text: text.flatten() })
            }
        }
    }

    fn update_modifiers(&mut self, key: KeyCode, released: bool) {
        let flag = match key {
            KeyCode::SHIFT_LEFT | KeyCode::SHIFT_RIGHT => Some(MOD_SHIFT),
            KeyCode::CTRL => Some(MOD_CTRL),
            KeyCode::ALT => Some(MOD_ALT),
            KeyCode::CAPS_LOCK => {
                if !released {
                    self.caps_lock = !self.caps_lock;
                }
                None
            }
            _ => None,
        };
        if let Some(flag) = flag {
            if released {
                self.modifiers &= !flag;
            } else {
                self.modifiers |= flag;
            }
        }
        if self.caps_lock {
            self.modifiers |= MOD_CAPS_LOCK;
        } else {
            self.modifiers &= !MOD_CAPS_LOCK;
        }
    }

    fn committed_text(&self, key: KeyCode) -> Option<InputMessage> {
        if self.modifiers & (MOD_CTRL | MOD_ALT) != 0 {
            return None;
        }
        let byte = key.character_byte()?;
        let byte = if byte.is_ascii_alphabetic() {
            let upper = self.modifiers & (MOD_SHIFT | MOD_CAPS_LOCK) != 0;
            if upper { byte.to_ascii_uppercase() } else { byte.to_ascii_lowercase() }
        } else if self.modifiers & MOD_SHIFT != 0 {
            shifted_ascii(byte)
        } else {
            byte
        };
        InputMessage::text(&[byte])
    }
}

impl Default for InputDecoder {
    fn default() -> Self {
        Self::new()
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

const fn map_code(byte: u8, extended: bool) -> Option<KeyCode> {
    if extended {
        return Some(match byte {
            0x75 => KeyCode::UP,
            0x72 => KeyCode::DOWN,
            0x6b => KeyCode::LEFT,
            0x74 => KeyCode::RIGHT,
            0x6c => KeyCode::HOME,
            0x69 => KeyCode::END,
            0x7d => KeyCode::PAGE_UP,
            0x7a => KeyCode::PAGE_DOWN,
            0x71 => KeyCode::DELETE,
            0x14 => KeyCode::CTRL,
            0x11 => KeyCode::ALT,
            _ => KeyCode::UNKNOWN,
        });
    }
    Some(match byte {
        0x76 => KeyCode::ESCAPE,
        0x5a => KeyCode::ENTER,
        0x66 => KeyCode::BACKSPACE,
        0x0d => KeyCode::TAB,
        0x12 => KeyCode::SHIFT_LEFT,
        0x59 => KeyCode::SHIFT_RIGHT,
        0x14 => KeyCode::CTRL,
        0x11 => KeyCode::ALT,
        0x58 => KeyCode::CAPS_LOCK,
        0x16 => KeyCode::character(b'1'),
        0x1e => KeyCode::character(b'2'),
        0x26 => KeyCode::character(b'3'),
        0x25 => KeyCode::character(b'4'),
        0x2e => KeyCode::character(b'5'),
        0x36 => KeyCode::character(b'6'),
        0x3d => KeyCode::character(b'7'),
        0x3e => KeyCode::character(b'8'),
        0x46 => KeyCode::character(b'9'),
        0x45 => KeyCode::character(b'0'),
        0x4e => KeyCode::character(b'-'),
        0x55 => KeyCode::character(b'='),
        0x54 => KeyCode::character(b'['),
        0x5b => KeyCode::character(b']'),
        0x4c => KeyCode::character(b';'),
        0x52 => KeyCode::character(b'\''),
        0x41 => KeyCode::character(b','),
        0x49 => KeyCode::character(b'.'),
        0x4a => KeyCode::character(b'/'),
        0x0e => KeyCode::character(b'`'),
        0x5d => KeyCode::character(b'\\'),
        0x1c => KeyCode::character(b'a'),
        0x32 => KeyCode::character(b'b'),
        0x21 => KeyCode::character(b'c'),
        0x23 => KeyCode::character(b'd'),
        0x24 => KeyCode::character(b'e'),
        0x2b => KeyCode::character(b'f'),
        0x34 => KeyCode::character(b'g'),
        0x33 => KeyCode::character(b'h'),
        0x43 => KeyCode::character(b'i'),
        0x3b => KeyCode::character(b'j'),
        0x42 => KeyCode::character(b'k'),
        0x4b => KeyCode::character(b'l'),
        0x3a => KeyCode::character(b'm'),
        0x31 => KeyCode::character(b'n'),
        0x44 => KeyCode::character(b'o'),
        0x4d => KeyCode::character(b'p'),
        0x15 => KeyCode::character(b'q'),
        0x2d => KeyCode::character(b'r'),
        0x1b => KeyCode::character(b's'),
        0x2c => KeyCode::character(b't'),
        0x3c => KeyCode::character(b'u'),
        0x2a => KeyCode::character(b'v'),
        0x1d => KeyCode::character(b'w'),
        0x22 => KeyCode::character(b'x'),
        0x35 => KeyCode::character(b'y'),
        0x1a => KeyCode::character(b'z'),
        _ => KeyCode::UNKNOWN,
    })
}

impl KeyCode {
    pub const UNKNOWN: Self = Self::Unknown;
    pub const ESCAPE: Self = Self::Escape;
    pub const ENTER: Self = Self::Enter;
    pub const BACKSPACE: Self = Self::Backspace;
    pub const TAB: Self = Self::Tab;
    pub const UP: Self = Self::Up;
    pub const DOWN: Self = Self::Down;
    pub const LEFT: Self = Self::Left;
    pub const RIGHT: Self = Self::Right;
    pub const HOME: Self = Self::Home;
    pub const END: Self = Self::End;
    pub const PAGE_UP: Self = Self::PageUp;
    pub const PAGE_DOWN: Self = Self::PageDown;
    pub const DELETE: Self = Self::Delete;
    pub const CTRL: Self = Self(0x300);
    pub const ALT: Self = Self(0x301);
    pub const CAPS_LOCK: Self = Self(0x302);
    pub const SHIFT_LEFT: Self = Self(0x303);
    pub const SHIFT_RIGHT: Self = Self(0x304);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_two_decodes_key_and_committed_text() {
        let mut decoder = InputDecoder::new();
        let event = decoder.feed(0x1c).unwrap();
        assert_eq!(event.key.code, KeyCode::character(b'a').raw());
        assert_eq!(event.text.unwrap().text_bytes(), Some(&b"a"[..]));
        assert_eq!(decoder.feed(0xf0), None);
        assert_eq!(decoder.feed(0x1c).unwrap().key.state, KeyState::Released);
    }

    #[test]
    fn modifiers_and_extended_arrows_are_semantic() {
        let mut decoder = InputDecoder::new();
        decoder.feed(0x12);
        let event = decoder.feed(0x1c).unwrap();
        assert_eq!(event.text.unwrap().text_bytes(), Some(&b"A"[..]));
        decoder.feed(0xe0);
        let arrow = decoder.feed(0x75).unwrap();
        assert_eq!(KeyCode::from_raw(arrow.key.code), KeyCode::UP);
        assert!(arrow.text.is_none());
    }
}
