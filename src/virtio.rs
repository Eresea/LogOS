use core::arch::asm;

use crate::{
    ipc::{Envelope, Message},
    pci::PciDevice,
    services::ServiceHandle,
};

pub struct VirtioService {
    handle: ServiceHandle,
    status_port: u16,
}

impl VirtioService {
    pub fn bind(device: PciDevice, handle: ServiceHandle) -> Option<Self> {
        let bar = device.bar(0);
        if bar & 1 == 0 {
            return None;
        }
        let service = Self { handle, status_port: (bar & !3) as u16 + 0x12 };
        unsafe { outb(service.status_port, 0) };
        (unsafe { inb(service.status_port) } == 0).then_some(service)
    }

    pub fn handle(&self, envelope: Envelope) -> Option<Message> {
        if envelope.destination != self.handle || unsafe { inb(self.status_port) } != 0 {
            return None;
        }
        match envelope.message {
            Message::Ping => Some(Message::Pong),
            Message::Pong => None,
        }
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
