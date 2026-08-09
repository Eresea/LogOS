use core::{
    arch::asm,
    cell::UnsafeCell,
    sync::atomic::{AtomicUsize, Ordering},
};

const LOG_EVENTS: usize = 32;
const LOG_BYTES: usize = 160;

struct LogRing(UnsafeCell<[[u8; LOG_BYTES]; LOG_EVENTS]>);
unsafe impl Sync for LogRing {}

static LOG_RING: LogRing = LogRing(UnsafeCell::new([[0; LOG_BYTES]; LOG_EVENTS]));
static LOG_LENGTHS: LogLengths = LogLengths(UnsafeCell::new([0; LOG_EVENTS]));
static LOG_HEAD: AtomicUsize = AtomicUsize::new(0);

struct LogLengths(UnsafeCell<[u16; LOG_EVENTS]>);
unsafe impl Sync for LogLengths {}

#[derive(Clone, Copy)]
#[allow(dead_code)]
pub struct Snapshot {
    pub lines: [[u8; LOG_BYTES]; LOG_EVENTS],
    pub lengths: [u16; LOG_EVENTS],
    pub len: usize,
    pub first_cursor: u64,
    pub next_cursor: u64,
}

pub fn write(message: &[u8]) {
    for &byte in message {
        unsafe { asm!("out dx, al", in("dx") 0xe9u16, in("al") byte) };
    }
}

pub fn write_line(message: &[u8]) {
    write(message);
    write(b"\r\n");
    let head = LOG_HEAD.fetch_add(1, Ordering::AcqRel);
    let index = head % LOG_EVENTS;
    let length = message.len().min(LOG_BYTES);
    let flags: u64;
    unsafe {
        asm!("pushfq", "pop {}", out(reg) flags);
        asm!("cli");
        (*LOG_RING.0.get())[index].fill(0);
        (&mut (*LOG_RING.0.get())[index])[..length].copy_from_slice(&message[..length]);
        (*LOG_LENGTHS.0.get())[index] = length as u16;
        if flags & (1 << 9) != 0 {
            asm!("sti");
        }
    }
}

pub fn write_hex_u64_line(prefix: &[u8], value: u64) {
    let mut line = [0; LOG_BYTES];
    let prefix_length = prefix.len().min(LOG_BYTES.saturating_sub(16));
    line[..prefix_length].copy_from_slice(&prefix[..prefix_length]);
    for (index, shift) in (0..16).zip((0..64).step_by(4).rev()) {
        line[prefix_length + index] = b"0123456789abcdef"[((value >> shift) & 0xf) as usize];
    }
    write_line(&line[..prefix_length + 16]);
}

#[allow(dead_code)]
pub fn snapshot() -> Snapshot {
    let flags: u64;
    unsafe {
        asm!("pushfq", "pop {}", out(reg) flags);
        asm!("cli");
        let head = LOG_HEAD.load(Ordering::Acquire);
        let len = head.min(LOG_EVENTS);
        let mut lines = [[0; LOG_BYTES]; LOG_EVENTS];
        let mut lengths = [0; LOG_EVENTS];
        for (index, (line, length)) in
            lines[..len].iter_mut().zip(lengths[..len].iter_mut()).enumerate()
        {
            let source = (head - len + index) % LOG_EVENTS;
            *line = (*LOG_RING.0.get())[source];
            *length = (*LOG_LENGTHS.0.get())[source];
        }
        if flags & (1 << 9) != 0 {
            asm!("sti");
        }
        Snapshot {
            lines,
            lengths,
            len,
            first_cursor: (head - len) as u64,
            next_cursor: head as u64,
        }
    }
}

pub fn since(cursor: u64, output: &mut [u8; LOG_BYTES]) -> (u64, usize, bool) {
    let snapshot = snapshot();
    let gap = cursor < snapshot.first_cursor;
    let start = cursor.max(snapshot.first_cursor).min(snapshot.next_cursor);
    let index = (start - snapshot.first_cursor) as usize;
    if index >= snapshot.len {
        return (snapshot.next_cursor, 0, gap);
    }
    let length = usize::from(snapshot.lengths[index]).min(output.len());
    output[..length].copy_from_slice(&snapshot.lines[index][..length]);
    (start.saturating_add(1), length, gap)
}
