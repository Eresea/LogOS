#![no_std]

//! Keyboard adapter state.  Hardware bytes stop here; terminal services only
//! receive semantic key and committed-text messages.

#[cfg(test)]
extern crate std;

use logos_abi::{InputMessage, KeyCode, KeyState};

pub const MOD_SHIFT: u16 = logos_abi::MOD_SHIFT;
pub const MOD_CTRL: u16 = logos_abi::MOD_CTRL;
pub const MOD_ALT: u16 = logos_abi::MOD_ALT;
pub const MOD_CAPS_LOCK: u16 = logos_abi::MOD_CAPS_LOCK;
pub const MOD_NUM_LOCK: u16 = logos_abi::MOD_NUM_LOCK;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum KeyboardLayout {
    /// French AZERTY base and Shift layers, including committed UTF-8 keycaps.
    /// The bracket pair is also supported through AltGr/Ctrl+Alt.
    #[default]
    Azerty,
    /// US QWERTY physical key mapping.
    Qwerty,
}

pub const DEFAULT_KEYBOARD_LAYOUT: KeyboardLayout = KeyboardLayout::Azerty;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DecodedInput {
    pub key: InputMessage,
    pub text: Option<InputMessage>,
}

impl DecodedInput {
    /// Select the one message the terminal graph should forward for this
    /// physical input. Printable keys carry committed text; control and
    /// navigation keys carry only the semantic key event.
    pub const fn terminal_message(self) -> InputMessage {
        match self.text {
            Some(text) => text,
            None => self.key,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InputDecoder {
    extended: bool,
    break_code: bool,
    modifiers: u16,
    caps_lock: bool,
    num_lock: bool,
    layout: KeyboardLayout,
}

impl InputDecoder {
    pub const fn new() -> Self {
        Self::with_layout(DEFAULT_KEYBOARD_LAYOUT)
    }

    pub const fn with_layout(layout: KeyboardLayout) -> Self {
        Self {
            extended: false,
            break_code: false,
            modifiers: 0,
            caps_lock: false,
            num_lock: true,
            layout,
        }
    }

    pub const fn layout(&self) -> KeyboardLayout {
        self.layout
    }

    pub fn set_layout(&mut self, layout: KeyboardLayout) {
        self.layout = layout;
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
                let key_code = map_code(code, extended, self.layout, self.num_lock)?;
                self.update_modifiers(key_code, released);
                let state = if released { KeyState::Released } else { KeyState::Pressed };
                let key = InputMessage::key(key_code, state, self.modifiers);
                let text = (!released).then(|| self.committed_text(code, key_code));
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
            KeyCode::NUM_LOCK => {
                if !released {
                    self.num_lock = !self.num_lock;
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
        if self.num_lock {
            self.modifiers |= MOD_NUM_LOCK;
        } else {
            self.modifiers &= !MOD_NUM_LOCK;
        }
    }

    fn committed_text(&self, physical_code: u8, key: KeyCode) -> Option<InputMessage> {
        if let Some(bytes) = azerty_altgr_text(self.layout, physical_code, self.modifiers) {
            return InputMessage::text(bytes);
        }
        if self.modifiers & (MOD_CTRL | MOD_ALT) != 0 {
            return None;
        }
        if let Some(bytes) = azerty_text(self.layout, physical_code, self.modifiers) {
            return InputMessage::text(bytes);
        }
        if let Some(bytes) = azerty_shifted_text(self.layout, physical_code, self.modifiers) {
            return InputMessage::text(bytes);
        }
        let byte = key.character_byte()?;
        let byte = if byte.is_ascii_alphabetic() {
            let upper = (self.modifiers & MOD_SHIFT != 0) ^ (self.modifiers & MOD_CAPS_LOCK != 0);
            if upper { byte.to_ascii_uppercase() } else { byte.to_ascii_lowercase() }
        } else if self.modifiers & MOD_SHIFT != 0 {
            shifted_ascii(self.layout, physical_code, byte)
        } else {
            byte
        };
        InputMessage::text(&[byte])
    }
}

fn azerty_altgr_text(
    layout: KeyboardLayout,
    physical_code: u8,
    modifiers: u16,
) -> Option<&'static [u8]> {
    if layout != KeyboardLayout::Azerty || modifiers & MOD_ALT == 0 {
        return None;
    }
    Some(match physical_code {
        0x2e => b"[",
        0x4e => b"]",
        _ => return None,
    })
}

fn azerty_text(layout: KeyboardLayout, physical_code: u8, modifiers: u16) -> Option<&'static [u8]> {
    if layout != KeyboardLayout::Azerty || modifiers & MOD_SHIFT != 0 {
        return None;
    }
    let uppercase = modifiers & MOD_CAPS_LOCK != 0;
    Some(match (physical_code, uppercase) {
        (0x1e, false) => b"\xc3\xa9",
        (0x1e, true) => b"\xc3\x89",
        (0x3d, false) => b"\xc3\xa8",
        (0x3d, true) => b"\xc3\x88",
        (0x46, false) => b"\xc3\xa7",
        (0x46, true) => b"\xc3\x87",
        (0x45, false) => b"\xc3\xa0",
        (0x45, true) => b"\xc3\x80",
        (0x52, false) => b"\xc3\xb9",
        (0x52, true) => b"\xc3\x99",
        _ => return None,
    })
}

fn azerty_shifted_text(
    layout: KeyboardLayout,
    physical_code: u8,
    modifiers: u16,
) -> Option<&'static [u8]> {
    if layout != KeyboardLayout::Azerty || modifiers & MOD_SHIFT == 0 {
        return None;
    }
    Some(match physical_code {
        // French AZERTY number row: &é"'(-è_çà become 1 through 0.
        0x16 => b"1",
        0x1e => b"2",
        0x26 => b"3",
        0x25 => b"4",
        0x2e => b"5",
        0x36 => b"6",
        0x3d => b"7",
        0x3e => b"8",
        0x46 => b"9",
        0x45 => b"0",
        0x4e => b"\xc2\xb0",
        0x54 => b"\xc2\xa8",
        0x4a => b"\xc2\xa7",
        _ => return None,
    })
}

impl Default for InputDecoder {
    fn default() -> Self {
        Self::new()
    }
}

const fn shifted_ascii(layout: KeyboardLayout, physical_code: u8, byte: u8) -> u8 {
    if let KeyboardLayout::Azerty = layout {
        return match physical_code {
            0x4c => b'?',
            0x52 => b'%',
            0x55 => b'+',
            0x3a => b'?',
            0x41 => b'.',
            0x49 => b'/',
            0x5b => b'*',
            _ => byte,
        };
    }
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

const fn map_code(
    byte: u8,
    extended: bool,
    layout: KeyboardLayout,
    num_lock: bool,
) -> Option<KeyCode> {
    if extended {
        return Some(match byte {
            0x4a => KeyCode::character(b'/'),
            0x5a => KeyCode::ENTER,
            0x75 => KeyCode::UP,
            0x72 => KeyCode::DOWN,
            0x6b => KeyCode::LEFT,
            0x74 => KeyCode::RIGHT,
            0x6c => KeyCode::HOME,
            0x69 => KeyCode::END,
            0x7d => KeyCode::PAGE_UP,
            0x7a => KeyCode::PAGE_DOWN,
            0x71 => KeyCode::DELETE,
            0x70 => KeyCode::INSERT,
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
        0x77 => KeyCode::NUM_LOCK,
        0x70 if num_lock => KeyCode::character(b'0'),
        0x69 if num_lock => KeyCode::character(b'1'),
        0x72 if num_lock => KeyCode::character(b'2'),
        0x7a if num_lock => KeyCode::character(b'3'),
        0x6b if num_lock => KeyCode::character(b'4'),
        0x73 if num_lock => KeyCode::character(b'5'),
        0x74 if num_lock => KeyCode::character(b'6'),
        0x6c if num_lock => KeyCode::character(b'7'),
        0x75 if num_lock => KeyCode::character(b'8'),
        0x7d if num_lock => KeyCode::character(b'9'),
        0x71 if num_lock => KeyCode::character(b'.'),
        0x7c => KeyCode::character(b'*'),
        0x7b => KeyCode::character(b'-'),
        0x79 => KeyCode::character(b'+'),
        0x29 => KeyCode::character(b' '),
        code => match layout {
            KeyboardLayout::Qwerty => qwerty_code(code),
            KeyboardLayout::Azerty => azerty_code(code),
        },
    })
}

const fn qwerty_code(byte: u8) -> KeyCode {
    match byte {
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
    }
}

const fn azerty_code(byte: u8) -> KeyCode {
    // Semantic key codes remain ASCII-compatible. The text path preserves
    // supported accented, shifted, and bracket AltGr keycaps.
    match byte {
        0x16 => KeyCode::character(b'&'),
        0x1e => KeyCode::character(b'e'),
        0x26 => KeyCode::character(b'"'),
        0x25 => KeyCode::character(b'\''),
        0x2e => KeyCode::character(b'('),
        0x36 => KeyCode::character(b'-'),
        0x3d => KeyCode::character(b'e'),
        0x3e => KeyCode::character(b'_'),
        0x46 => KeyCode::character(b'c'),
        0x45 => KeyCode::character(b'a'),
        0x4e => KeyCode::character(b')'),
        0x55 => KeyCode::character(b'='),
        0x54 => KeyCode::character(b'^'),
        0x5b => KeyCode::character(b'$'),
        0x4c => KeyCode::character(b'm'),
        0x52 => KeyCode::character(b'u'),
        0x41 => KeyCode::character(b';'),
        0x49 => KeyCode::character(b':'),
        0x4a => KeyCode::character(b'!'),
        0x0e => KeyCode::character(b'`'),
        0x5d => KeyCode::character(b'*'),
        0x15 => KeyCode::character(b'a'),
        0x1d => KeyCode::character(b'z'),
        0x24 => KeyCode::character(b'e'),
        0x2d => KeyCode::character(b'r'),
        0x2c => KeyCode::character(b't'),
        0x35 => KeyCode::character(b'y'),
        0x3c => KeyCode::character(b'u'),
        0x43 => KeyCode::character(b'i'),
        0x44 => KeyCode::character(b'o'),
        0x4d => KeyCode::character(b'p'),
        0x1c => KeyCode::character(b'q'),
        0x1b => KeyCode::character(b's'),
        0x23 => KeyCode::character(b'd'),
        0x2b => KeyCode::character(b'f'),
        0x34 => KeyCode::character(b'g'),
        0x33 => KeyCode::character(b'h'),
        0x3b => KeyCode::character(b'j'),
        0x42 => KeyCode::character(b'k'),
        0x4b => KeyCode::character(b'l'),
        0x3a => KeyCode::character(b','),
        0x31 => KeyCode::character(b'n'),
        0x32 => KeyCode::character(b'b'),
        0x21 => KeyCode::character(b'c'),
        0x2a => KeyCode::character(b'v'),
        0x22 => KeyCode::character(b'x'),
        0x1a => KeyCode::character(b'w'),
        _ => KeyCode::UNKNOWN,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use logos_commands::CommandService;
    use logos_session::{MAX_LINE_BYTES, SessionService, ShellOutput};
    use logos_terminal::TerminalService;

    #[test]
    fn set_two_decodes_key_and_committed_text() {
        let mut decoder = InputDecoder::with_layout(KeyboardLayout::Qwerty);
        let event = decoder.feed(0x1c).unwrap();
        assert_eq!(event.key.code, KeyCode::character(b'a').raw());
        assert_eq!(event.text.unwrap().text_bytes(), Some(&b"a"[..]));
        assert_eq!(decoder.feed(0xf0), None);
        assert_eq!(decoder.feed(0x1c).unwrap().key.state, KeyState::Released);
    }

    #[test]
    fn azerty_is_the_default_layout() {
        let mut decoder = InputDecoder::new();
        assert_eq!(decoder.layout(), KeyboardLayout::Azerty);

        let event = decoder.feed(0x15).unwrap();
        assert_eq!(event.text.unwrap().text_bytes(), Some(&b"a"[..]));
        let event = decoder.feed(0x1c).unwrap();
        assert_eq!(event.text.unwrap().text_bytes(), Some(&b"q"[..]));
    }

    #[test]
    fn azerty_shifted_number_row_is_ascii_compatible() {
        let mut decoder = InputDecoder::new();
        decoder.feed(0x12);
        for (scancode, expected) in [
            (0x16, b"1"),
            (0x1e, b"2"),
            (0x26, b"3"),
            (0x25, b"4"),
            (0x2e, b"5"),
            (0x36, b"6"),
            (0x3d, b"7"),
            (0x3e, b"8"),
            (0x46, b"9"),
            (0x45, b"0"),
        ] {
            let event = decoder.feed(scancode).unwrap();
            assert_eq!(event.text.unwrap().text_bytes(), Some(&expected[..]));
        }
    }

    #[test]
    fn numpad_keys_follow_num_lock_and_keep_navigation() {
        let mut decoder = InputDecoder::new();

        let initial = decoder.feed(0x70).unwrap();
        assert_eq!(initial.text.unwrap().text_bytes(), Some(&b"0"[..]));
        assert_eq!(initial.key.modifiers & MOD_NUM_LOCK, MOD_NUM_LOCK);

        let num_lock = decoder.feed(0x77).unwrap();
        assert_eq!(KeyCode::from_raw(num_lock.key.code), KeyCode::NUM_LOCK);
        assert_eq!(num_lock.key.modifiers & MOD_NUM_LOCK, 0);

        let num_lock = decoder.feed(0x77).unwrap();
        assert_eq!(KeyCode::from_raw(num_lock.key.code), KeyCode::NUM_LOCK);
        assert_eq!(num_lock.key.modifiers & MOD_NUM_LOCK, MOD_NUM_LOCK);

        for (scancode, expected) in [
            (0x70, b"0"),
            (0x69, b"1"),
            (0x72, b"2"),
            (0x7a, b"3"),
            (0x6b, b"4"),
            (0x73, b"5"),
            (0x74, b"6"),
            (0x6c, b"7"),
            (0x75, b"8"),
            (0x7d, b"9"),
            (0x71, b"."),
            (0x7c, b"*"),
            (0x7b, b"-"),
            (0x79, b"+"),
        ] {
            let event = decoder.feed(scancode).unwrap();
            assert_eq!(event.text.unwrap().text_bytes(), Some(&expected[..]));
        }

        decoder.feed(0xe0);
        let slash = decoder.feed(0x4a).unwrap();
        assert_eq!(slash.text.unwrap().text_bytes(), Some(&b"/"[..]));
        decoder.feed(0xe0);
        let enter = decoder.feed(0x5a).unwrap();
        assert_eq!(KeyCode::from_raw(enter.key.code), KeyCode::ENTER);

        decoder.feed(0xf0);
        let released = decoder.feed(0x77).unwrap();
        assert_eq!(released.key.modifiers & MOD_NUM_LOCK, MOD_NUM_LOCK);
        decoder.feed(0xe0);
        let page_up = decoder.feed(0x7d).unwrap();
        assert_eq!(KeyCode::from_raw(page_up.key.code), KeyCode::PAGE_UP);
    }

    #[test]
    fn azerty_accented_keycaps_commit_utf8_text() {
        let mut decoder = InputDecoder::new();
        let event = decoder.feed(0x1e).unwrap();
        assert_eq!(event.text.unwrap().text_bytes(), Some(&b"\xc3\xa9"[..]));

        decoder.feed(0x58);
        let event = decoder.feed(0x46).unwrap();
        assert_eq!(event.text.unwrap().text_bytes(), Some(&b"\xc3\x87"[..]));
    }

    #[test]
    fn azerty_shifted_symbols_commit_their_keycap_text() {
        let mut decoder = InputDecoder::new();
        decoder.feed(0x12);

        let event = decoder.feed(0x5b).unwrap();
        assert_eq!(event.text.unwrap().text_bytes(), Some(&b"*"[..]));
        let event = decoder.feed(0x4e).unwrap();
        assert_eq!(event.text.unwrap().text_bytes(), Some(&b"\xc2\xb0"[..]));
        let event = decoder.feed(0x54).unwrap();
        assert_eq!(event.text.unwrap().text_bytes(), Some(&b"\xc2\xa8"[..]));
        let event = decoder.feed(0x4a).unwrap();
        assert_eq!(event.text.unwrap().text_bytes(), Some(&b"\xc2\xa7"[..]));
    }

    #[test]
    fn azerty_altgr_brackets_commit_for_altgr_and_ctrl_alt() {
        let mut decoder = InputDecoder::new();

        decoder.feed(0x14);
        decoder.feed(0xe0);
        decoder.feed(0x11);
        let event = decoder.feed(0x2e).unwrap();
        assert_eq!(event.text.unwrap().text_bytes(), Some(&b"["[..]));

        decoder.feed(0xe0);
        decoder.feed(0xf0);
        decoder.feed(0x11);
        decoder.feed(0xf0);
        decoder.feed(0x14);

        decoder.feed(0xe0);
        decoder.feed(0x11);
        let event = decoder.feed(0x4e).unwrap();
        assert_eq!(event.text.unwrap().text_bytes(), Some(&b"]"[..]));
    }

    #[test]
    fn modifiers_and_extended_arrows_are_semantic() {
        let mut decoder = InputDecoder::with_layout(KeyboardLayout::Qwerty);
        decoder.feed(0x12);
        let event = decoder.feed(0x1c).unwrap();
        assert_eq!(event.text.unwrap().text_bytes(), Some(&b"A"[..]));
        decoder.feed(0xe0);
        let arrow = decoder.feed(0x75).unwrap();
        assert_eq!(KeyCode::from_raw(arrow.key.code), KeyCode::UP);
        assert!(arrow.text.is_none());
    }

    #[test]
    fn shift_and_caps_lock_cancel_for_letters() {
        let mut decoder = InputDecoder::with_layout(KeyboardLayout::Qwerty);
        decoder.feed(0x58);
        decoder.feed(0x12);
        let event = decoder.feed(0x1c).unwrap();
        assert_eq!(event.text.unwrap().text_bytes(), Some(&b"a"[..]));
    }

    #[test]
    fn set_two_decodes_space() {
        let mut decoder = InputDecoder::new();
        let event = decoder.feed(0x29).unwrap();
        assert_eq!(event.text.unwrap().text_bytes(), Some(&b" "[..]));
    }

    #[test]
    fn terminal_graph_forwards_one_command_and_renders_output() {
        let mut terminal = TerminalService::new();
        let mut session = SessionService::new();
        let mut commands = CommandService::new();
        let mut command = [0; MAX_LINE_BYTES];
        let mut committed = None;

        for message in [
            InputMessage::text(b"sys.version()").unwrap(),
            InputMessage::key(KeyCode::ENTER, KeyState::Pressed, 0),
        ] {
            let Some(stream) = terminal.input(&message) else { continue };
            let Some(bytes) = stream.as_bytes() else { continue };
            let mut edit_output = ShellOutput::new();
            if let Some(length) = session.input_for_command(bytes, &mut command, &mut edit_output) {
                committed = Some(length);
            }
            terminal.session_output_bytes(edit_output.as_bytes());
        }

        let length = committed.expect("enter commits the command");
        assert_eq!(&command[..length], b"sys.version()");
        let result = commands.execute(&command[..length]);
        assert_eq!(result.as_bytes(), b"LogOS vNext 0.1.0\r\n");
        let mut output = ShellOutput::new();
        session.command_output(result.as_bytes(), &mut output);
        terminal.session_output_bytes(output.as_bytes());
        assert!(terminal.next_render().is_some());
    }
}
