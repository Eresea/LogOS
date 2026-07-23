use core::{
    cell::UnsafeCell,
    sync::atomic::{AtomicUsize, Ordering},
};

const EVENTS: usize = 64;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Event {
    Empty,
    Boot,
    TaskBlocked,
    TaskWoken,
    VirtioSubmit,
    VirtioComplete,
    SelfCheck,
}

#[derive(Clone, Copy)]
pub struct Snapshot {
    events: [Event; EVENTS],
    len: usize,
}

struct Ring(UnsafeCell<[Event; EVENTS]>);

unsafe impl Sync for Ring {}

static RING: Ring = Ring(UnsafeCell::new([Event::Empty; EVENTS]));
static HEAD: AtomicUsize = AtomicUsize::new(0);

pub fn record(event: Event) {
    let flags: u64;
    unsafe {
        core::arch::asm!("pushfq", "pop {}", out(reg) flags);
        core::arch::asm!("cli");
        let head = HEAD.load(Ordering::Relaxed);
        (*RING.0.get())[head % EVENTS] = event;
        HEAD.store(head.wrapping_add(1), Ordering::Release);
        if flags & (1 << 9) != 0 {
            core::arch::asm!("sti");
        }
    }
}

pub fn latest() -> Event {
    let head = HEAD.load(Ordering::Acquire);
    if head == 0 { Event::Empty } else { unsafe { (*RING.0.get())[(head - 1) % EVENTS] } }
}

pub fn snapshot() -> Snapshot {
    let flags: u64;
    unsafe {
        core::arch::asm!("pushfq", "pop {}", out(reg) flags);
        core::arch::asm!("cli");
        let head = HEAD.load(Ordering::Acquire);
        let len = head.min(EVENTS);
        let mut events = [Event::Empty; EVENTS];
        for (index, event) in events[..len].iter_mut().enumerate() {
            *event = (*RING.0.get())[(head - len + index) % EVENTS];
        }
        if flags & (1 << 9) != 0 {
            core::arch::asm!("sti");
        }
        Snapshot { events, len }
    }
}

impl Snapshot {
    pub fn events(&self) -> &[Event] {
        &self.events[..self.len]
    }
}

pub fn self_check() -> bool {
    record(Event::SelfCheck);
    latest() == Event::SelfCheck && snapshot().events().last() == Some(&Event::SelfCheck)
}

pub fn message(event: Event) -> &'static [u8] {
    match event {
        Event::Boot => b"TRACE BOOT\n",
        Event::TaskBlocked => b"TRACE TASK BLOCKED\n",
        Event::TaskWoken => b"TRACE TASK WOKEN\n",
        Event::VirtioSubmit => b"TRACE VIRTIO SUBMIT\n",
        Event::VirtioComplete => b"TRACE VIRTIO COMPLETE\n",
        Event::SelfCheck => b"TRACE SELF CHECK\n",
        Event::Empty => b"TRACE NONE\n",
    }
}
