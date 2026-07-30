use crate::session::Principal;

const EVENTS: usize = 8;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    SecretWrite,
    ApprovalGrant,
    ApprovalRevoke,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Event {
    pub sequence: u64,
    pub principal: Principal,
    pub effect: Effect,
}

pub struct Log {
    events: [Option<Event>; EVENTS],
    len: usize,
    next_sequence: u64,
}

impl Log {
    pub const fn new() -> Self {
        Self { events: [None; EVENTS], len: 0, next_sequence: 1 }
    }

    pub const fn can_record(&self) -> bool {
        self.len < EVENTS
    }

    pub fn record(&mut self, principal: Principal, effect: Effect) -> bool {
        let Some(slot) = self.events.get_mut(self.len) else {
            return false;
        };
        *slot = Some(Event { sequence: self.next_sequence, principal, effect });
        self.len += 1;
        self.next_sequence = self.next_sequence.wrapping_add(1);
        true
    }

    pub fn latest(&self) -> Option<Event> {
        self.len.checked_sub(1).and_then(|index| self.events[index])
    }
}

pub fn self_check() -> bool {
    let mut log = Log::new();
    log.record(Principal::LOCAL, Effect::SecretWrite)
        && log
            .latest()
            .is_some_and(|event| event.sequence == 1 && event.principal == Principal::LOCAL)
}
