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

pub fn self_check() -> bool {
    record(Event::SelfCheck);
    latest() == Event::SelfCheck
}

pub fn message() -> &'static [u8] {
    match latest() {
        Event::Boot => b"TRACE BOOT\n",
        Event::TaskBlocked => b"TRACE TASK BLOCKED\n",
        Event::TaskWoken => b"TRACE TASK WOKEN\n",
        Event::VirtioSubmit => b"TRACE VIRTIO SUBMIT\n",
        Event::VirtioComplete => b"TRACE VIRTIO COMPLETE\n",
        Event::SelfCheck => b"TRACE SELF CHECK\n",
        Event::Empty => b"TRACE NONE\n",
    }
}
