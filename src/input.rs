#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Event {
    Text(u8),
    Backspace,
    Enter,
    Escape,
}

pub struct Service;

impl Service {
    pub const fn new() -> Self {
        Self
    }

    pub fn next(&mut self) -> Option<Event> {
        crate::keyboard::poll_scancode().and_then(Self::decode)
    }

    pub fn self_check() -> bool {
        Self::decode(0x22) == Some(Event::Text(b'g'))
    }

    fn decode(scancode: u8) -> Option<Event> {
        match crate::keyboard::decode(scancode)? {
            0x08 => Some(Event::Backspace),
            b'\n' => Some(Event::Enter),
            0x1b => Some(Event::Escape),
            text => Some(Event::Text(text)),
        }
    }
}
