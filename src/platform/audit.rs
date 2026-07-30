use crate::session::Principal;

const EVENTS: usize = 8;
pub const AUDIT_BYTES: usize = 16 + EVENTS * 16;

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

    pub fn export(&self) -> [u8; AUDIT_BYTES] {
        let mut bytes = [0; AUDIT_BYTES];
        bytes[0] = self.len as u8;
        bytes[8..16].copy_from_slice(&self.next_sequence.to_le_bytes());
        for (index, event) in self.events[..self.len].iter().flatten().enumerate() {
            let start = 16 + index * 16;
            bytes[start..start + 8].copy_from_slice(&event.sequence.to_le_bytes());
            let (kind, id) = match event.principal {
                Principal::LocalUser(id) => (1, id),
                Principal::Service(id) => (2, id),
                Principal::Process(id) => (3, id),
            };
            bytes[start + 8] = kind;
            bytes[start + 9] = match event.effect {
                Effect::SecretWrite => 1,
                Effect::ApprovalGrant => 2,
                Effect::ApprovalRevoke => 3,
            };
            bytes[start + 12..start + 16].copy_from_slice(&id.to_le_bytes());
        }
        bytes
    }

    pub fn restore(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != AUDIT_BYTES || usize::from(bytes[0]) > EVENTS {
            return None;
        }
        let len = usize::from(bytes[0]);
        let mut log = Self::new();
        log.len = len;
        log.next_sequence = u64::from_le_bytes(bytes[8..16].try_into().ok()?);
        for index in 0..len {
            let start = 16 + index * 16;
            let sequence = u64::from_le_bytes(bytes[start..start + 8].try_into().ok()?);
            let id = u32::from_le_bytes(bytes[start + 12..start + 16].try_into().ok()?);
            let principal = match bytes[start + 8] {
                1 => Principal::LocalUser(id),
                2 => Principal::Service(id),
                3 => Principal::Process(id),
                _ => return None,
            };
            let effect = match bytes[start + 9] {
                1 => Effect::SecretWrite,
                2 => Effect::ApprovalGrant,
                3 => Effect::ApprovalRevoke,
                _ => return None,
            };
            log.events[index] = Some(Event { sequence, principal, effect });
        }
        (log.next_sequence > log.latest().map_or(0, |event| event.sequence)).then_some(log)
    }
}

pub fn self_check() -> bool {
    let mut log = Log::new();
    log.record(Principal::LOCAL, Effect::SecretWrite)
        && log
            .latest()
            .is_some_and(|event| event.sequence == 1 && event.principal == Principal::LOCAL)
        && Log::restore(&log.export()).is_some_and(|restored| restored.latest() == log.latest())
}
