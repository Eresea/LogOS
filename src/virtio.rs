use core::{
    arch::asm,
    cell::Cell,
    sync::atomic::{Ordering, compiler_fence},
};

use crate::{
    ipc::{Envelope, Message},
    memory::PhysicalMemory,
    pci::PciDevice,
    scheduler::{Runnable, TaskState},
    services::ServiceHandle,
};

const ACKNOWLEDGE: u8 = 1;
const DRIVER: u8 = 2;
const DRIVER_OK: u8 = 4;
const PAGE_SIZE: usize = 4096;

#[repr(C)]
#[derive(Clone, Copy)]
struct Descriptor {
    address: u64,
    length: u32,
    flags: u16,
    next: u16,
}

pub struct VirtioService {
    handle: ServiceHandle,
    status_port: u16,
    notify_port: u16,
    queue: u64,
    queue_size: usize,
}

pub struct ServiceTask<'a> {
    service: &'a VirtioService,
    channel: &'a crate::ipc::Channel,
    memory: &'a mut PhysicalMemory,
    reply: &'a Cell<Option<Message>>,
}

impl<'a> ServiceTask<'a> {
    pub fn new(
        service: &'a VirtioService,
        channel: &'a crate::ipc::Channel,
        memory: &'a mut PhysicalMemory,
        reply: &'a Cell<Option<Message>>,
    ) -> Self {
        Self { service, channel, memory, reply }
    }
}

impl Runnable for ServiceTask<'_> {
    fn run(&mut self) -> TaskState {
        if let Some(envelope) = self.channel.receive() {
            let reply = match envelope.message {
                Message::Ping => self.service.handle(envelope),
                Message::Inflate if self.service.accepts(envelope) => {
                    self.service.inflate_one_page(self.memory).then_some(Message::Complete)
                }
                _ => None,
            };
            self.reply.set(reply);
        }
        TaskState::Ready
    }
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
        let service = Self {
            handle,
            status_port: (bar & !3) as u16 + 0x12,
            notify_port: (bar & !3) as u16 + 0x10,
            queue: 0,
            queue_size: 0,
        };
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
        let service = Self { queue, queue_size, ..service };
        (unsafe { inb(service.status_port) } & DRIVER_OK != 0).then_some(service)
    }

    pub fn handle(&self, envelope: Envelope) -> Option<Message> {
        if !self.accepts(envelope) {
            return None;
        }
        match envelope.message {
            Message::Ping => Some(Message::Pong),
            Message::Pong | Message::Inflate | Message::Complete => None,
        }
    }

    fn accepts(&self, envelope: Envelope) -> bool {
        envelope.destination == self.handle && unsafe { inb(self.status_port) } & DRIVER_OK != 0
    }

    pub fn inflate_one_page(&self, memory: &mut PhysicalMemory) -> bool {
        let page = match memory.allocate_page() {
            Some(page) => page,
            None => return false,
        };
        let pfn = match u32::try_from(page >> 12) {
            Ok(pfn) => pfn,
            Err(_) => return false,
        };
        let avail = self.queue + (self.queue_size * core::mem::size_of::<Descriptor>()) as u64;
        let used = align_up(avail + 6 + (self.queue_size * 2) as u64, PAGE_SIZE as u64);
        unsafe {
            (page as *mut u32).write_volatile(pfn);
            (self.queue as *mut Descriptor).write_volatile(Descriptor {
                address: page,
                length: core::mem::size_of::<u32>() as u32,
                flags: 0,
                next: 0,
            });
            ((avail + 4) as *mut u16).write_volatile(0);
            let used_index = (used + 2) as *const u16;
            let before = used_index.read_volatile();
            compiler_fence(Ordering::Release);
            ((avail + 2) as *mut u16).write_volatile(1);
            outw(self.notify_port, 0);
            for _ in 0..10 {
                if used_index.read_volatile() != before {
                    return true;
                }
                crate::interrupts::wait_for_tick();
            }
        }
        false
    }
}

fn align_up(value: u64, alignment: u64) -> u64 {
    (value + alignment - 1) & !(alignment - 1)
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
