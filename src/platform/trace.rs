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
    DriverBound,
    DriverQuiesced,
    DriverRecovered,
    DriverFailed,
    Fault,
    SelfCheck,
}

#[derive(Clone, Copy)]
pub struct Snapshot {
    events: [Event; EVENTS],
    len: usize,
    first_cursor: u64,
    next_cursor: u64,
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
        let next_cursor = head as u64;
        Snapshot { events, len, first_cursor: next_cursor.saturating_sub(len as u64), next_cursor }
    }
}

impl Snapshot {
    pub fn events(&self) -> &[Event] {
        &self.events[..self.len]
    }

    pub const fn cursor_range(&self) -> (u64, u64) {
        (self.first_cursor, self.next_cursor)
    }

    pub fn since(&self, cursor: u64, output: &mut [Event; EVENTS]) -> (u64, usize, bool) {
        let gap = cursor < self.first_cursor;
        let start = cursor.max(self.first_cursor).min(self.next_cursor);
        let count = (self.next_cursor - start).min(EVENTS as u64) as usize;
        output[..count]
            .copy_from_slice(&self.events[(start - self.first_cursor) as usize..][..count]);
        (self.next_cursor, count, gap)
    }
}

pub fn self_check() -> bool {
    record(Event::SelfCheck);
    let snapshot = snapshot();
    let mut output = [Event::Empty; EVENTS];
    let (next, count, gap) = snapshot.since(snapshot.cursor_range().0, &mut output);
    latest() == Event::SelfCheck
        && snapshot.events().last() == Some(&Event::SelfCheck)
        && next == snapshot.cursor_range().1
        && count != 0
        && !gap
}

pub fn message(event: Event) -> &'static [u8] {
    match event {
        Event::Boot => b"TRACE BOOT\n",
        Event::TaskBlocked => b"TRACE TASK BLOCKED\n",
        Event::TaskWoken => b"TRACE TASK WOKEN\n",
        Event::VirtioSubmit => b"TRACE VIRTIO SUBMIT\n",
        Event::VirtioComplete => b"TRACE VIRTIO COMPLETE\n",
        Event::DriverBound => b"TRACE DRIVER BOUND\n",
        Event::DriverQuiesced => b"TRACE DRIVER QUIESCED\n",
        Event::DriverRecovered => b"TRACE DRIVER RECOVERED\n",
        Event::DriverFailed => b"TRACE DRIVER FAILED\n",
        Event::Fault => b"TRACE FAULT\n",
        Event::SelfCheck => b"TRACE SELF CHECK\n",
        Event::Empty => b"TRACE NONE\n",
    }
}
