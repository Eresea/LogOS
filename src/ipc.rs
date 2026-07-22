use core::cell::UnsafeCell;

use crate::capabilities::{Capability, CapabilityKind, CapabilityManager};
use crate::services::ServiceHandle;

const MESSAGES: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Message {
    Ping,
    Pong,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Envelope {
    pub destination: ServiceHandle,
    pub message: Message,
}

struct Queue {
    messages: [Option<Envelope>; MESSAGES],
    head: usize,
    len: usize,
}

pub struct Channel(UnsafeCell<Queue>);

impl Channel {
    pub const fn new() -> Self {
        Self(UnsafeCell::new(Queue { messages: [None; MESSAGES], head: 0, len: 0 }))
    }

    pub fn send(
        &self,
        capabilities: &CapabilityManager,
        capability: Capability,
        destination: ServiceHandle,
        message: Message,
    ) -> bool {
        let queue = unsafe { &mut *self.0.get() };
        if !capabilities.allows(capability, CapabilityKind::Service) || queue.len == MESSAGES {
            return false;
        }
        // ponytail: cooperative-only; add synchronization when IRQs or cores send messages.
        let tail = (queue.head + queue.len) % MESSAGES;
        queue.messages[tail] = Some(Envelope { destination, message });
        queue.len += 1;
        true
    }

    pub fn receive(&self) -> Option<Envelope> {
        let queue = unsafe { &mut *self.0.get() };
        if queue.len == 0 {
            return None;
        }
        let message = queue.messages[queue.head].take();
        queue.head = (queue.head + 1) % MESSAGES;
        queue.len -= 1;
        message
    }
}
