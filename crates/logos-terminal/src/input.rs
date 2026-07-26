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
        let qwerty = input.decode(0x10, false, 0).text() == Some(b'q');
        input.set_layout(Layout::Azerty);
        let azerty = input.decode(0x10, false, 0).text() == Some(b'a');
        let pressed =
            matches!(input.decode(0x2a, false, 0), Event::Key { state: State::Press, .. });
        let released =
            matches!(input.decode(0xaa, false, 0), Event::Key { state: State::Release, .. });
        let left =
            matches!(input.decode(0x4b, true, 0), Event::Key { logical: LogicalKey::Left, .. });
        let up = matches!(input.decode(0x48, true, 0), Event::Key { logical: LogicalKey::Up, .. });
        let command_keys = input.decode(0x20, false, 0).text() == Some(b'd')
            && input.decode(0x32, false, 0).text() == Some(b'm');
        qwerty
            && azerty
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
            (_, 0x39) => b' ',
            (Layout::Qwerty, 0x10) => b'q',
            (Layout::Azerty, 0x10) => b'a',
            (Layout::Qwerty, 0x11) => b'w',
            (Layout::Azerty, 0x11) => b'z',
            (Layout::Qwerty, 0x1e) => b'a',
            (Layout::Azerty, 0x1e) => b'q',
            (_, 0x12) => b'e',
            (_, 0x15) => b'y',
            (_, 0x16) => b'u',
            (_, 0x13) => b'r',
            (_, 0x14) => b't',
            (_, 0x17) => b'i',
            (_, 0x18) => b'o',
            (_, 0x19) => b'p',
            (_, 0x1f) => b's',
            (_, 0x20) => b'd',
            (_, 0x21) => b'f',
            (_, 0x22) => b'g',
            (_, 0x23) => b'h',
            (_, 0x26) => b'l',
            (_, 0x30) => b'b',
            (_, 0x2d) => b'x',
            (_, 0x2e) => b'c',
            (_, 0x2f) => b'v',
            (_, 0x31) => b'n',
            (_, 0x32) => b'm',
            (_, 0x02) => b'1',
            (_, 0x0b) => b'0',
            _ => return LogicalKey::Unknown,
        };
        LogicalKey::Text(if self.modifiers.shift() { text.to_ascii_uppercase() } else { text })
    }
}
