#[derive(Clone, Copy, PartialEq, Eq)]
pub struct PhysicalKey(pub u8);

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LogicalKey {
    Text(u8),
    Backspace,
    Delete,
    Enter,
    Escape,
    Left,
    Right,
    Home,
    End,
    Up,
    Down,
    Unknown,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum State {
    Press,
    Release,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Modifiers(u8);

impl Modifiers {
    const SHIFT: u8 = 1;
    const CONTROL: u8 = 2;

    pub const fn none() -> Self {
        Self(0)
    }

    fn shift(self) -> bool {
        self.0 & Self::SHIFT != 0
    }

    fn control(self) -> bool {
        self.0 & Self::CONTROL != 0
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Event {
    Key { physical: PhysicalKey, logical: LogicalKey, state: State, modifiers: Modifiers },
    Repeat { physical: PhysicalKey, logical: LogicalKey, modifiers: Modifiers },
}

impl Event {
    pub fn text(self) -> Option<u8> {
        match self {
            Self::Key { logical: LogicalKey::Text(text), state: State::Press, .. }
            | Self::Repeat { logical: LogicalKey::Text(text), .. } => Some(text),
            _ => None,
        }
    }

    pub fn is_enter(self) -> bool {
        matches!(self, Self::Key { logical: LogicalKey::Enter, state: State::Press, .. })
    }

    pub fn pressed(self) -> Option<(LogicalKey, Modifiers)> {
        match self {
            Self::Key { logical, state: State::Press, modifiers, .. } => Some((logical, modifiers)),
            Self::Repeat { logical, modifiers, .. } => Some((logical, modifiers)),
            _ => None,
        }
    }

    pub fn control(self) -> bool {
        self.pressed().is_some_and(|(_, modifiers)| modifiers.control())
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    Qwerty,
    Azerty,
}

pub struct Service {
    layout: Layout,
    modifiers: Modifiers,
    held: Option<(PhysicalKey, LogicalKey, Modifiers)>,
    repeat_at: u64,
    extended: bool,
}

impl Default for Service {
    fn default() -> Self {
        Self::new()
    }
}

impl Service {
    pub const fn new() -> Self {
        Self {
            layout: Layout::Qwerty,
            modifiers: Modifiers::none(),
            held: None,
            repeat_at: 0,
            extended: false,
        }
    }

    pub fn set_layout(&mut self, layout: Layout) {
        self.layout = layout;
    }

    pub fn next(
        &mut self,
        now: u64,
        mut poll_scancode: impl FnMut() -> Option<u8>,
    ) -> Option<Event> {
        while let Some(scancode) = poll_scancode() {
            if scancode == 0xe0 {
                self.extended = true;
                continue;
            }
            let extended = self.extended;
            self.extended = false;
            return Some(self.decode(scancode, extended, now));
        }
        let (physical, logical, modifiers) = self.held?;
        if now < self.repeat_at {
            return None;
        }
        self.repeat_at = now.wrapping_add(5);
        Some(Event::Repeat { physical, logical, modifiers })
    }

    pub fn self_check() -> bool {
        let mut input = Self::new();
        let qwerty = input.decode(0x10, false, 0).text() == Some(b'q')
            && input.decode(0x2c, false, 0).text() == Some(b'z');
        input.set_layout(Layout::Azerty);
        let azerty = input.decode(0x10, false, 0).text() == Some(b'a')
            && input.decode(0x11, false, 0).text() == Some(b'z')
            && input.decode(0x2c, false, 0).text() == Some(b'w')
            && input.decode(0x27, false, 0).text() == Some(b'm');
        let azerty_number = input.decode(0x2a, false, 0).text().is_none()
            && input.decode(0x02, false, 0).text() == Some(b'1')
            && input.decode(0x03, false, 0).text() == Some(b'2')
            && input.decode(0x0b, false, 0).text() == Some(b'0')
            && input.decode(0xaa, false, 0).text().is_none();
        let pressed =
            matches!(input.decode(0x2a, false, 0), Event::Key { state: State::Press, .. });
        let released =
            matches!(input.decode(0xaa, false, 0), Event::Key { state: State::Release, .. });
        let left =
            matches!(input.decode(0x4b, true, 0), Event::Key { logical: LogicalKey::Left, .. });
        let up = matches!(input.decode(0x48, true, 0), Event::Key { logical: LogicalKey::Up, .. });
        let command_keys = input.decode(0x20, false, 0).text() == Some(b'd')
            && input.decode(0x27, false, 0).text() == Some(b'm')
            && input.decode(0x11, false, 0).text() == Some(b'z');
        qwerty
            && azerty
            && azerty_number
            && pressed
            && released
            && left
            && up
            && command_keys
            && input.next(1, || None).is_none()
            && matches!(input.next(25, || None), Some(Event::Repeat { .. }))
    }

    fn decode(&mut self, scancode: u8, extended: bool, now: u64) -> Event {
        let state = if scancode & 0x80 == 0 { State::Press } else { State::Release };
        let physical = PhysicalKey(scancode & 0x7f);
        if physical.0 == 0x2a || physical.0 == 0x36 {
            self.modifiers = Modifiers(if state == State::Press {
                self.modifiers.0 | Modifiers::SHIFT
            } else {
                self.modifiers.0 & !Modifiers::SHIFT
            });
        }
        if physical.0 == 0x1d {
            self.modifiers = Modifiers(if state == State::Press {
                self.modifiers.0 | Modifiers::CONTROL
            } else {
                self.modifiers.0 & !Modifiers::CONTROL
            });
        }
        let logical = self.logical(physical, extended);
        let modifiers = self.modifiers;
        if state == State::Press && !matches!(logical, LogicalKey::Unknown) {
            self.held = Some((physical, logical, modifiers));
            self.repeat_at = now.wrapping_add(25);
        } else if state == State::Release {
            self.held = None;
            self.repeat_at = 0;
        }
        Event::Key { physical, logical, state, modifiers }
    }

    fn logical(&self, physical: PhysicalKey, extended: bool) -> LogicalKey {
        if extended {
            return match physical.0 {
                0x4b => LogicalKey::Left,
                0x4d => LogicalKey::Right,
                0x47 => LogicalKey::Home,
                0x4f => LogicalKey::End,
                0x53 => LogicalKey::Delete,
                0x48 => LogicalKey::Up,
                0x50 => LogicalKey::Down,
                _ => LogicalKey::Unknown,
            };
        }
        let text = match (self.layout, physical.0) {
            (_, 0x01) => return LogicalKey::Escape,
            (_, 0x0e) => return LogicalKey::Backspace,
            (_, 0x1c) => return LogicalKey::Enter,
            (_, 0x39) => (b' ', b' '),
            (Layout::Qwerty, 0x02) => (b'1', b'!'),
            (Layout::Qwerty, 0x03) => (b'2', b'@'),
            (Layout::Qwerty, 0x04) => (b'3', b'#'),
            (Layout::Qwerty, 0x05) => (b'4', b'$'),
            (Layout::Qwerty, 0x06) => (b'5', b'%'),
            (Layout::Qwerty, 0x07) => (b'6', b'^'),
            (Layout::Qwerty, 0x08) => (b'7', b'&'),
            (Layout::Qwerty, 0x09) => (b'8', b'*'),
            (Layout::Qwerty, 0x0a) => (b'9', b'('),
            (Layout::Qwerty, 0x0b) => (b'0', b')'),
            (Layout::Azerty, 0x02) => (b'&', b'1'),
            (Layout::Azerty, 0x03) => (0, b'2'),
            (Layout::Azerty, 0x04) => (b'"', b'3'),
            (Layout::Azerty, 0x05) => (b'\'', b'4'),
            (Layout::Azerty, 0x06) => (b'(', b'5'),
            (Layout::Azerty, 0x07) => (b'-', b'6'),
            (Layout::Azerty, 0x08) => (0, b'7'),
            (Layout::Azerty, 0x09) => (b'_', b'8'),
            (Layout::Azerty, 0x0a) => (0, b'9'),
            (Layout::Azerty, 0x0b) => (0, b'0'),
            (Layout::Azerty, 0x1a) => (b'^', b'^'),
            (Layout::Azerty, 0x1b) => (b'$', b'$'),
            (Layout::Azerty, 0x32) => (b',', b'?'),
            (Layout::Azerty, 0x33) => (b';', b'.'),
            (Layout::Azerty, 0x34) => (b':', b'/'),
            (Layout::Azerty, 0x35) => (b'!', b'!'),
            (Layout::Qwerty, 0x1a) => (b'[', b'{'),
            (Layout::Qwerty, 0x1b) => (b']', b'}'),
            (Layout::Qwerty, 0x27) => (b';', b':'),
            (Layout::Qwerty, 0x28) => (b'\'', b'"'),
            (Layout::Qwerty, 0x29) => (b'`', b'~'),
            (Layout::Qwerty, 0x2b) => (b'\\', b'|'),
            (Layout::Qwerty, 0x33) => (b',', b'<'),
            (Layout::Qwerty, 0x34) => (b'.', b'>'),
            (Layout::Qwerty, 0x35) => (b'/', b'?'),
            (Layout::Qwerty, 0x10) => (b'q', b'Q'),
            (Layout::Qwerty, 0x11) => (b'w', b'W'),
            (Layout::Qwerty, 0x12) => (b'e', b'E'),
            (Layout::Qwerty, 0x13) => (b'r', b'R'),
            (Layout::Qwerty, 0x14) => (b't', b'T'),
            (Layout::Qwerty, 0x15) => (b'y', b'Y'),
            (Layout::Qwerty, 0x16) => (b'u', b'U'),
            (Layout::Qwerty, 0x17) => (b'i', b'I'),
            (Layout::Qwerty, 0x18) => (b'o', b'O'),
            (Layout::Qwerty, 0x19) => (b'p', b'P'),
            (Layout::Qwerty, 0x1e) => (b'a', b'A'),
            (Layout::Qwerty, 0x1f) => (b's', b'S'),
            (Layout::Qwerty, 0x20) => (b'd', b'D'),
            (Layout::Qwerty, 0x21) => (b'f', b'F'),
            (Layout::Qwerty, 0x22) => (b'g', b'G'),
            (Layout::Qwerty, 0x23) => (b'h', b'H'),
            (Layout::Qwerty, 0x24) => (b'j', b'J'),
            (Layout::Qwerty, 0x25) => (b'k', b'K'),
            (Layout::Qwerty, 0x26) => (b'l', b'L'),
            (Layout::Qwerty, 0x2c) => (b'z', b'Z'),
            (Layout::Qwerty, 0x2d) => (b'x', b'X'),
            (Layout::Qwerty, 0x2e) => (b'c', b'C'),
            (Layout::Qwerty, 0x2f) => (b'v', b'V'),
            (Layout::Qwerty, 0x30) => (b'b', b'B'),
            (Layout::Qwerty, 0x31) => (b'n', b'N'),
            (Layout::Qwerty, 0x32) => (b'm', b'M'),
            (Layout::Azerty, 0x10) => (b'a', b'A'),
            (Layout::Azerty, 0x11) => (b'z', b'Z'),
            (Layout::Azerty, 0x12) => (b'e', b'E'),
            (Layout::Azerty, 0x13) => (b'r', b'R'),
            (Layout::Azerty, 0x14) => (b't', b'T'),
            (Layout::Azerty, 0x15) => (b'y', b'Y'),
            (Layout::Azerty, 0x16) => (b'u', b'U'),
            (Layout::Azerty, 0x17) => (b'i', b'I'),
            (Layout::Azerty, 0x18) => (b'o', b'O'),
            (Layout::Azerty, 0x19) => (b'p', b'P'),
            (Layout::Azerty, 0x1e) => (b'q', b'Q'),
            (Layout::Azerty, 0x1f) => (b's', b'S'),
            (Layout::Azerty, 0x20) => (b'd', b'D'),
            (Layout::Azerty, 0x21) => (b'f', b'F'),
            (Layout::Azerty, 0x22) => (b'g', b'G'),
            (Layout::Azerty, 0x23) => (b'h', b'H'),
            (Layout::Azerty, 0x24) => (b'j', b'J'),
            (Layout::Azerty, 0x25) => (b'k', b'K'),
            (Layout::Azerty, 0x26) => (b'l', b'L'),
            (Layout::Azerty, 0x27) => (b'm', b'M'),
            (Layout::Azerty, 0x2c) => (b'w', b'W'),
            (Layout::Azerty, 0x2d) => (b'x', b'X'),
            (Layout::Azerty, 0x2e) => (b'c', b'C'),
            (Layout::Azerty, 0x2f) => (b'v', b'V'),
            (Layout::Azerty, 0x30) => (b'b', b'B'),
            (Layout::Azerty, 0x31) => (b'n', b'N'),
            _ => return LogicalKey::Unknown,
        };
        let text = if self.modifiers.shift() { text.1 } else { text.0 };
        if text == 0 { LogicalKey::Unknown } else { LogicalKey::Text(text) }
    }
}
