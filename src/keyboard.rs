use core::arch::asm;

pub fn poll() -> Option<u8> {
    if unsafe { inb(0x64) } & 1 == 0 {
        return None;
    }
    let scancode = unsafe { inb(0x60) };
    (scancode & 0x80 == 0).then(|| decode(scancode)).flatten()
}

pub fn self_check() -> bool {
    (unsafe { inb(0x64) } & 0xc0) == 0
}

unsafe fn inb(port: u16) -> u8 {
    let value: u8;
    unsafe { asm!("in al, dx", in("dx") port, out("al") value) };
    value
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
        0x26 => Some(b'l'),
        0x2d => Some(b'x'),
        0x2e => Some(b'c'),
        0x2f => Some(b'v'),
        0x31 => Some(b'n'),
        _ => None,
    }
}
