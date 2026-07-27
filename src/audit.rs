use crate::session::Principal;

const EVENTS: usize = 8;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    SecretWrite,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Event {
    pub principal: Principal,
    pub effect: Effect,
}

pub struct Log {
    events: [Option<Event>; EVENTS],
    len: usize,
}

impl Log {
    pub const fn new() -> Self {
        Self { events: [None; EVENTS], len: 0 }
    }

    pub fn record(&mut self, event: Event) -> bool {
        if self.len == EVENTS {
            return false;
        }
        self.events[self.len] = Some(event);
        self.len += 1;
        true
    }

    pub fn latest(&self) -> Option<Event> {
        self.len.checked_sub(1).and_then(|index| self.events[index])
    }
}

pub fn self_check() -> bool {
    let mut log = Log::new();
    let event = Event { principal: Principal::LOCAL, effect: Effect::SecretWrite };
    log.record(event) && log.latest() == Some(event)
}
