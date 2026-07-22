use core::arch::asm;

use crate::pci::PciDevice;

pub fn reset_legacy(device: PciDevice) -> bool {
    let bar = device.bar(0);
    if bar & 1 == 0 {
        return false;
    }
    let base = (bar & !3) as u16;
    unsafe {
        outb(base + 0x12, 0);
        inb(base + 0x12) == 0
    }
}

unsafe fn outb(port: u16, value: u8) {
    unsafe { asm!("out dx, al", in("dx") port, in("al") value) };
}

unsafe fn inb(port: u16) -> u8 {
    let value: u8;
    unsafe { asm!("in al, dx", in("dx") port, out("al") value) };
    value
}
