use core::{
    arch::asm,
    cell::UnsafeCell,
    sync::atomic::{AtomicUsize, Ordering},
};

const SCANCODES: usize = 16;

struct ScancodeBuffer(UnsafeCell<[u8; SCANCODES]>);

unsafe impl Sync for ScancodeBuffer {}

static SCANCODES_BUFFER: ScancodeBuffer = ScancodeBuffer(UnsafeCell::new([0; SCANCODES]));
static HEAD: AtomicUsize = AtomicUsize::new(0);
static TAIL: AtomicUsize = AtomicUsize::new(0);

pub fn poll() -> Option<u8> {
    if let Some(scancode) = pop() {
        return decode(scancode);
    }
    if unsafe { inb(0x64) } & 1 == 0 {
        return None;
    }
    let scancode = unsafe { inb(0x60) };
    (scancode & 0x80 == 0).then(|| decode(scancode)).flatten()
}

pub fn self_check() -> bool {
    (unsafe { inb(0x64) } & 0xc0) == 0
}

pub fn enable_interrupts() -> bool {
    if !wait_write() {
        return false;
    }
    unsafe { outb(0x64, 0x20) };
    if !wait_read() {
        return false;
    }
    let command = unsafe { inb(0x60) };
    if !wait_write() {
        return false;
    }
    unsafe { outb(0x64, 0x60) };
    if !wait_write() {
        return false;
    }
    unsafe { outb(0x60, command | 1) };
    true
}

#[unsafe(no_mangle)]
extern "C" fn keyboard_interrupt() {
    if unsafe { inb(0x64) } & 1 != 0 {
        push(unsafe { inb(0x60) });
    }
}

fn push(scancode: u8) {
    let head = HEAD.load(Ordering::Relaxed);
    let next = (head + 1) % SCANCODES;
    if next == TAIL.load(Ordering::Acquire) {
        return;
    }
    unsafe { (*SCANCODES_BUFFER.0.get())[head] = scancode };
    HEAD.store(next, Ordering::Release);
}

fn pop() -> Option<u8> {
    let tail = TAIL.load(Ordering::Relaxed);
    if tail == HEAD.load(Ordering::Acquire) {
        return None;
    }
    let scancode = unsafe { (*SCANCODES_BUFFER.0.get())[tail] };
    TAIL.store((tail + 1) % SCANCODES, Ordering::Release);
    Some(scancode)
}

fn wait_read() -> bool {
    for _ in 0..10_000 {
        if unsafe { inb(0x64) } & 1 != 0 {
            return true;
        }
    }
    false
}

fn wait_write() -> bool {
    for _ in 0..10_000 {
        if unsafe { inb(0x64) } & 2 == 0 {
            return true;
        }
    }
    false
}

unsafe fn inb(port: u16) -> u8 {
    let value: u8;
    unsafe { asm!("in al, dx", in("dx") port, out("al") value) };
    value
}

unsafe fn outb(port: u16, value: u8) {
    unsafe { asm!("out dx, al", in("dx") port, in("al") value) };
}

fn decode(scancode: u8) -> Option<u8> {
    match scancode {
        0x01 => Some(0x1b),
        0x0e => Some(0x08),
        0x1c => Some(b'\n'),
        0x39 => Some(b' '),
        0x02 => Some(b'1'),
        0x0b => Some(b'0'),
        0x12 => Some(b'e'),
        0x13 => Some(b'r'),
        0x14 => Some(b't'),
        0x17 => Some(b'i'),
        0x18 => Some(b'o'),
        0x19 => Some(b'p'),
        0x1e => Some(b'a'),
        0x1f => Some(b's'),
        0x23 => Some(b'h'),
        0x21 => Some(b'f'),
        0x26 => Some(b'l'),
        0x2d => Some(b'x'),
        0x2e => Some(b'c'),
        0x2f => Some(b'v'),
        0x31 => Some(b'n'),
        _ => None,
    }
}
