use crate::capabilities::{Capability, CapabilityKind, CapabilityManager};

const MESSAGES: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Message {
    Ping,
}

pub struct Channel {
    messages: [Option<Message>; MESSAGES],
    head: usize,
    len: usize,
}

impl Channel {
    pub const fn new() -> Self {
        Self { messages: [None; MESSAGES], head: 0, len: 0 }
    }

    pub fn send(
        &mut self,
        capabilities: &CapabilityManager,
        capability: Capability,
        message: Message,
    ) -> bool {
        if !capabilities.allows(capability, CapabilityKind::Service) || self.len == MESSAGES {
            return false;
        }
        // ponytail: fixed queue; add blocking/wakeup semantics when services run concurrently.
        let tail = (self.head + self.len) % MESSAGES;
        self.messages[tail] = Some(message);
        self.len += 1;
        true
    }

    pub fn receive(&mut self) -> Option<Message> {
        if self.len == 0 {
            return None;
        }
        let message = self.messages[self.head].take();
        self.head = (self.head + 1) % MESSAGES;
        self.len -= 1;
        message
    }
}
