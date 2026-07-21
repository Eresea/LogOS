use core::arch::asm;

pub fn write(message: &[u8]) {
    for &byte in message {
        unsafe { asm!("out dx, al", in("dx") 0xe9u16, in("al") byte) };
    }
}

pub fn write_line(message: &[u8]) {
    write(message);
    write(b"\r\n");
}
