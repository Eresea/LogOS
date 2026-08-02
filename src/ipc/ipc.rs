use core::{
    cell::UnsafeCell,
    sync::atomic::{AtomicBool, AtomicU16, Ordering},
};

use crate::platform::services::ServiceHandle;
use crate::platform::session::Principal;
use logos_core::capabilities::{Capability, CapabilityKind, CapabilityManager};

const MESSAGES: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Message {
    Ping,
    Pong,
    Inflate,
    Recover,
    Cancel,
    Complete,
    Failed,
}

pub type RequestId = u16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Envelope {
    pub principal: Principal,
    pub destination: ServiceHandle,
    pub message: Message,
    pub request: RequestId,
}

struct Queue {
    messages: [Option<Envelope>; MESSAGES],
    head: usize,
    len: usize,
}

pub struct Channel {
    queue: UnsafeCell<Queue>,
    locked: AtomicBool,
    next_request: AtomicU16,
}

unsafe impl Sync for Channel {}

impl Channel {
    pub const fn new() -> Self {
        Self {
            queue: UnsafeCell::new(Queue { messages: [None; MESSAGES], head: 0, len: 0 }),
            locked: AtomicBool::new(false),
            next_request: AtomicU16::new(1),
        }
    }

    pub fn send(
        &self,
        capabilities: &CapabilityManager,
        capability: Capability,
        principal: Principal,
        destination: ServiceHandle,
        message: Message,
    ) -> Option<RequestId> {
        if !capabilities.allows(capability, CapabilityKind::Service) {
            return None;
        }
        let request = self.next_request.fetch_add(1, Ordering::Relaxed);
        self.enqueue(Envelope { principal, destination, message, request }).then_some(request)
    }

    pub fn reply(
        &self,
        capabilities: &CapabilityManager,
        capability: Capability,
        principal: Principal,
        destination: ServiceHandle,
        message: Message,
        request: RequestId,
    ) -> bool {
        capabilities.allows(capability, CapabilityKind::Service)
            && self.enqueue(Envelope { principal, destination, message, request })
    }

    pub fn receive(&self) -> Option<Envelope> {
        self.access(|queue| {
            if queue.len == 0 {
                return None;
            }
            let message = queue.messages[queue.head].take();
            queue.head = (queue.head + 1) % MESSAGES;
            queue.len -= 1;
            message
        })
    }

    fn enqueue(&self, envelope: Envelope) -> bool {
        self.access(|queue| {
            if queue.len == MESSAGES {
                return false;
            }
            let tail = (queue.head + queue.len) % MESSAGES;
            queue.messages[tail] = Some(envelope);
            queue.len += 1;
            true
        })
    }

    fn access<T>(&self, action: impl FnOnce(&mut Queue) -> T) -> T {
        let flags: u64;
        unsafe {
            core::arch::asm!("pushfq", "pop {}", out(reg) flags);
            core::arch::asm!("cli");
        }
        while self.locked.swap(true, Ordering::Acquire) {
            core::hint::spin_loop();
        }
        let result = action(unsafe { &mut *self.queue.get() });
        self.locked.store(false, Ordering::Release);
        if flags & (1 << 9) != 0 {
            unsafe { core::arch::asm!("sti") };
        }
        result
    }
}

pub fn self_check() -> bool {
    let channel = Channel::new();
    let envelope = Envelope {
        principal: Principal::process(7),
        destination: ServiceHandle::self_check(),
        message: Message::Ping,
        request: 7,
    };
    (0..MESSAGES).all(|_| channel.enqueue(envelope))
        && !channel.enqueue(envelope)
        && channel.receive().is_some_and(|message| message.request == envelope.request)
}
