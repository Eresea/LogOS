use core::arch::asm;

use crate::{
    ipc::{Envelope, Message},
    memory::PhysicalMemory,
    pci::PciDevice,
    services::ServiceHandle,
};

const ACKNOWLEDGE: u8 = 1;
const DRIVER: u8 = 2;
const DRIVER_OK: u8 = 4;
const PAGE_SIZE: usize = 4096;

pub struct VirtioService {
    handle: ServiceHandle,
    status_port: u16,
}

impl VirtioService {
    pub fn bind(
        device: PciDevice,
        handle: ServiceHandle,
        memory: &mut PhysicalMemory,
    ) -> Option<Self> {
        let bar = device.bar(0);
        if bar & 1 == 0 {
            return None;
        }
        let service = Self { handle, status_port: (bar & !3) as u16 + 0x12 };
        let base = service.status_port - 0x12;
        unsafe {
            outb(service.status_port, 0);
            outb(service.status_port, ACKNOWLEDGE);
            outl(base + 0x04, 0);
            outb(service.status_port, ACKNOWLEDGE | DRIVER);
            outw(base + 0x0e, 0);
        }
        let queue_size = unsafe { inw(base + 0x0c) } as usize;
        let queue = allocate_queue(memory, queue_size)?;
        let pfn = u32::try_from(queue >> 12).ok()?;
        unsafe {
            outl(base + 0x08, pfn);
            outb(service.status_port, ACKNOWLEDGE | DRIVER | DRIVER_OK);
        }
        (unsafe { inb(service.status_port) } & DRIVER_OK != 0).then_some(service)
    }

    pub fn handle(&self, envelope: Envelope) -> Option<Message> {
        if envelope.destination != self.handle || unsafe { inb(self.status_port) } & DRIVER_OK == 0
        {
            return None;
        }
        match envelope.message {
            Message::Ping => Some(Message::Pong),
            Message::Pong => None,
        }
    }
}

fn allocate_queue(memory: &mut PhysicalMemory, queue_size: usize) -> Option<u64> {
    let bytes = queue_size.checked_mul(16)?.checked_add(6 + queue_size * 2)?;
    let bytes = bytes.checked_add(PAGE_SIZE - 1)? / PAGE_SIZE * PAGE_SIZE;
    let bytes = bytes.checked_add(6 + queue_size * 8)?;
    let pages = bytes.div_ceil(PAGE_SIZE);
    let first = memory.allocate_page()?;
    for page in 1..pages {
        (memory.allocate_page()? == first + (page * PAGE_SIZE) as u64).then_some(())?;
    }
    unsafe { core::ptr::write_bytes(first as *mut u8, 0, pages * PAGE_SIZE) };
    Some(first)
}

unsafe fn outb(port: u16, value: u8) {
    unsafe { asm!("out dx, al", in("dx") port, in("al") value) };
}

unsafe fn inb(port: u16) -> u8 {
    let value: u8;
    unsafe { asm!("in al, dx", in("dx") port, out("al") value) };
    value
}

unsafe fn outw(port: u16, value: u16) {
    unsafe { asm!("out dx, ax", in("dx") port, in("ax") value) };
}

unsafe fn inw(port: u16) -> u16 {
    let value: u16;
    unsafe { asm!("in ax, dx", in("dx") port, out("ax") value) };
    value
}

unsafe fn outl(port: u16, value: u32) {
    unsafe { asm!("out dx, eax", in("dx") port, in("eax") value) };
}
