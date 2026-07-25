#[derive(Clone, Copy, PartialEq, Eq)]
pub struct PhysicalKey(pub u8);

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LogicalKey {
    Text(u8),
    Backspace,
    Enter,
    Escape,
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

    pub const fn none() -> Self {
        Self(0)
    }

    fn shift(self) -> bool {
        self.0 & Self::SHIFT != 0
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

    pub fn is_backspace(self) -> bool {
        matches!(self, Self::Key { logical: LogicalKey::Backspace, state: State::Press, .. })
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
    repeat_budget: u8,
}

impl Service {
    pub const fn new() -> Self {
        Self { layout: Layout::Qwerty, modifiers: Modifiers::none(), held: None, repeat_budget: 0 }
    }

    pub fn set_layout(&mut self, layout: Layout) {
        self.layout = layout;
    }

    pub fn next(&mut self) -> Option<Event> {
        if let Some(scancode) = crate::keyboard::poll_scancode() {
            return Some(self.decode(scancode));
        }
        let (physical, logical, modifiers) = self.held?;
        if self.repeat_budget == 0 {
            return None;
        }
        self.repeat_budget -= 1;
        Some(Event::Repeat { physical, logical, modifiers })
    }

    pub fn self_check() -> bool {
        let mut input = Self::new();
        let qwerty = input.decode(0x10).text() == Some(b'q');
        input.set_layout(Layout::Azerty);
        let azerty = input.decode(0x10).text() == Some(b'a');
        let pressed = matches!(input.decode(0x2a), Event::Key { state: State::Press, .. });
        let released = matches!(input.decode(0xaa), Event::Key { state: State::Release, .. });
        qwerty && azerty && pressed && released
    }

    fn decode(&mut self, scancode: u8) -> Event {
        let state = if scancode & 0x80 == 0 { State::Press } else { State::Release };
        let physical = PhysicalKey(scancode & 0x7f);
        if physical.0 == 0x2a || physical.0 == 0x36 {
            self.modifiers = Modifiers(if state == State::Press { Modifiers::SHIFT } else { 0 });
        }
        let logical = self.logical(physical);
        let modifiers = self.modifiers;
        if state == State::Press && !matches!(logical, LogicalKey::Unknown) {
            self.held = Some((physical, logical, modifiers));
            self.repeat_budget = 1;
        } else if state == State::Release {
            self.held = None;
            self.repeat_budget = 0;
        }
        Event::Key { physical, logical, state, modifiers }
    }

    fn logical(&self, physical: PhysicalKey) -> LogicalKey {
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
            (_, 0x13) => b'r',
            (_, 0x14) => b't',
            (_, 0x17) => b'i',
            (_, 0x18) => b'o',
            (_, 0x19) => b'p',
            (_, 0x1f) => b's',
            (_, 0x21) => b'f',
            (_, 0x22) => b'g',
            (_, 0x23) => b'h',
            (_, 0x26) => b'l',
            (_, 0x2d) => b'x',
            (_, 0x2e) => b'c',
            (_, 0x2f) => b'v',
            (_, 0x31) => b'n',
            (_, 0x02) => b'1',
            (_, 0x0b) => b'0',
            _ => return LogicalKey::Unknown,
        };
        LogicalKey::Text(if self.modifiers.shift() { text.to_ascii_uppercase() } else { text })
    }
}
