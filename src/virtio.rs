use core::{
    arch::asm,
    sync::atomic::{AtomicBool, AtomicU16, Ordering, compiler_fence},
};

use crate::{
    ipc::{Envelope, Message},
    memory::{Page, PhysicalMemory},
    pci::PciDevice,
    scheduler::{Runnable, TaskState},
    services::ServiceHandle,
    session::Principal,
};
use logos_core::capabilities::{Capability, CapabilityManager};

const ACKNOWLEDGE: u8 = 1;
const DRIVER: u8 = 2;
const DRIVER_OK: u8 = 4;
const FAILED: u8 = 128;
const PAGE_SIZE: usize = 4096;
// ponytail: QEMU legacy queue needs three pages; increase when a larger queue is bound.
const QUEUE_PAGES: usize = 8;
static ISR_PORT: AtomicU16 = AtomicU16::new(0);
static COMPLETE: AtomicBool = AtomicBool::new(false);

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
    queue: VirtQueue,
    queue_size: usize,
    interrupt_gsi: u32,
    state: DriverState,
    pending_page: Option<Page>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DriverState {
    Bound,
    Quiesced,
    Failed,
}

pub struct VirtQueue {
    pages: [Option<Page>; QUEUE_PAGES],
}

impl VirtQueue {
    pub fn allocate(memory: &mut PhysicalMemory, queue_size: usize) -> Option<Self> {
        let bytes = queue_size.checked_mul(16)?.checked_add(6 + queue_size * 2)?;
        let bytes = bytes.checked_add(PAGE_SIZE - 1)? / PAGE_SIZE * PAGE_SIZE;
        let bytes = bytes.checked_add(6 + queue_size * 8)?;
        let pages = bytes.div_ceil(PAGE_SIZE);
        if pages > QUEUE_PAGES {
            return None;
        }
        let mut queue = Self { pages: [const { None }; QUEUE_PAGES] };
        for index in 0..pages {
            let page = memory.allocate_owned()?;
            if index > 0 && page.address() != queue.address() + (index * PAGE_SIZE) as u64 {
                let _ = memory.release_page(page);
                let _ = queue.release(memory);
                return None;
            }
            queue.pages[index] = Some(page);
        }
        unsafe { core::ptr::write_bytes(queue.address() as *mut u8, 0, pages * PAGE_SIZE) };
        Some(queue)
    }

    pub fn address(&self) -> u64 {
        self.pages[0].as_ref().map(Page::address).unwrap_or(0)
    }

    pub fn release(self, memory: &mut PhysicalMemory) -> bool {
        // Release high-to-low so the LIFO allocator returns a contiguous queue low-to-high.
        self.pages.into_iter().rev().flatten().all(|page| memory.release_page(page))
    }
}

pub struct ServiceTask<'a> {
    service: &'a mut VirtioService,
    requests: &'a crate::ipc::Channel,
    responses: &'a crate::ipc::Channel,
    capabilities: &'a CapabilityManager,
    capability: Capability,
    principal: Principal,
    memory: &'a mut PhysicalMemory,
    pending: Option<(ServiceHandle, crate::ipc::RequestId)>,
}

impl<'a> ServiceTask<'a> {
    pub fn new(
        service: &'a mut VirtioService,
        requests: &'a crate::ipc::Channel,
        responses: &'a crate::ipc::Channel,
        capabilities: &'a CapabilityManager,
        capability: Capability,
        principal: Principal,
        memory: &'a mut PhysicalMemory,
    ) -> Self {
        Self {
            service,
            requests,
            responses,
            capabilities,
            capability,
            principal,
            memory,
            pending: None,
        }
    }
}

impl Runnable for ServiceTask<'_> {
    fn run(&mut self) -> TaskState {
        if let Some((destination, request)) = self.pending {
            if !take_completion() {
                return TaskState::Blocked(crate::scheduler::Event::VIRTIO);
            }
            self.pending = None;
            let failed = self.service.failed();
            if failed {
                crate::trace::record(crate::trace::Event::DriverFailed);
                crate::health::driver_failure(b"virtio", self.service.recover());
            }
            let reclaimed = self.service.reclaim_request(self.memory);
            let _ = self.responses.reply(
                self.capabilities,
                self.capability,
                self.principal,
                destination,
                if failed || !reclaimed { Message::Failed } else { Message::Complete },
                request,
            );
            return TaskState::Ready;
        }
        if let Some(envelope) = self.requests.receive() {
            let reply = match envelope.message {
                Message::Ping => self.service.handle(envelope),
                Message::Inflate if self.service.accepts(envelope) => {
                    if self.service.submit_inflate_one_page(self.memory) {
                        self.pending = Some((envelope.destination, envelope.request));
                        return TaskState::Blocked(crate::scheduler::Event::VIRTIO);
                    }
                    None
                }
                Message::Recover if self.service.accepts(envelope) => {
                    Some(if self.service.recover() { Message::Complete } else { Message::Failed })
                }
                Message::Cancel if self.service.accepts(envelope) => Some(Message::Failed),
                _ => None,
            };
            if let Some(reply) = reply {
                let _ = self.responses.reply(
                    self.capabilities,
                    self.capability,
                    self.principal,
                    envelope.destination,
                    reply,
                    envelope.request,
                );
            }
        }
        TaskState::Ready
    }
}

impl VirtioService {
    pub fn bind(
        device: PciDevice,
        interrupt_gsi: u32,
        handle: ServiceHandle,
        memory: &mut PhysicalMemory,
    ) -> Option<Self> {
        let bar = device.bar(0);
        if bar & 1 == 0 {
            return None;
        }
        let mut service = Self {
            handle,
            status_port: (bar & !3) as u16 + 0x12,
            notify_port: (bar & !3) as u16 + 0x10,
            queue: VirtQueue { pages: [const { None }; QUEUE_PAGES] },
            queue_size: 0,
            interrupt_gsi,
            state: DriverState::Quiesced,
            pending_page: None,
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
        let queue = VirtQueue::allocate(memory, queue_size)?;
        service.queue = queue;
        service.queue_size = queue_size;
        if !service.activate() {
            service.quiesce();
            let _ = service.queue.release(memory);
            crate::trace::record(crate::trace::Event::DriverFailed);
            crate::health::driver_failure(b"virtio", false);
            return None;
        }
        ISR_PORT.store(base + 0x13, Ordering::Release);
        crate::trace::record(crate::trace::Event::DriverBound);
        Some(service)
    }

    pub fn handle(&self, envelope: Envelope) -> Option<Message> {
        if !self.accepts(envelope) {
            return None;
        }
        match envelope.message {
            Message::Ping => Some(Message::Pong),
            Message::Pong
            | Message::Inflate
            | Message::Recover
            | Message::Cancel
            | Message::Complete
            | Message::Failed => None,
        }
    }

    fn accepts(&self, envelope: Envelope) -> bool {
        envelope.destination == self.handle
            && self.state == DriverState::Bound
            && (unsafe { inb(self.status_port) } & (DRIVER_OK | FAILED)) == DRIVER_OK
    }

    fn failed(&mut self) -> bool {
        let failed = (unsafe { inb(self.status_port) } & FAILED) != 0;
        if failed {
            self.state = DriverState::Failed;
        }
        failed
    }

    fn quiesce(&mut self) {
        unsafe { outb(self.status_port, 0) };
        self.state = DriverState::Quiesced;
        crate::trace::record(crate::trace::Event::DriverQuiesced);
    }

    pub fn release(mut self, memory: &mut PhysicalMemory) -> bool {
        // The ISR port is global interrupt state; clear it before returning the queue pages.
        ISR_PORT.store(0, Ordering::Release);
        self.quiesce();
        self.reclaim_request(memory) && self.queue.release(memory)
    }

    fn recover(&mut self) -> bool {
        self.quiesce();
        let recovered = self.activate();
        if recovered {
            crate::trace::record(crate::trace::Event::DriverRecovered);
        }
        recovered
    }

    fn activate(&mut self) -> bool {
        let base = self.status_port - 0x12;
        let Ok(pfn) = u32::try_from(self.queue.address() >> 12) else {
            self.state = DriverState::Failed;
            return false;
        };
        unsafe {
            outb(self.status_port, 0);
            outb(self.status_port, ACKNOWLEDGE);
            outl(base + 0x04, 0);
            outb(self.status_port, ACKNOWLEDGE | DRIVER);
            outw(base + 0x0e, 0);
            outl(base + 0x08, pfn);
            outb(self.status_port, ACKNOWLEDGE | DRIVER | DRIVER_OK);
        }
        let active = (unsafe { inb(self.status_port) } & DRIVER_OK) != 0
            && crate::interrupts::route_virtio(self.interrupt_gsi);
        self.state = if active { DriverState::Bound } else { DriverState::Failed };
        crate::trace::record(if active {
            crate::trace::Event::DriverBound
        } else {
            crate::trace::Event::DriverFailed
        });
        active
    }

    pub fn submit_inflate_one_page(&mut self, memory: &mut PhysicalMemory) -> bool {
        let page = match memory.allocate_owned() {
            Some(page) => page,
            None => return false,
        };
        let address = page.address();
        let pfn = match u32::try_from(address >> 12) {
            Ok(pfn) => pfn,
            Err(_) => return memory.release_page(page),
        };
        let queue = self.queue.address();
        let avail = queue + (self.queue_size * core::mem::size_of::<Descriptor>()) as u64;
        unsafe {
            (address as *mut u32).write_volatile(pfn);
            (queue as *mut Descriptor).write_volatile(Descriptor {
                address,
                length: core::mem::size_of::<u32>() as u32,
                flags: 0,
                next: 0,
            });
            ((avail + 4) as *mut u16).write_volatile(0);
            compiler_fence(Ordering::Release);
            ((avail + 2) as *mut u16).write_volatile(1);
            outw(self.notify_port, 0);
        }
        self.pending_page = Some(page);
        crate::trace::record(crate::trace::Event::VirtioSubmit);
        true
    }

    pub fn resources_reclaimed(&self) -> bool {
        self.pending_page.is_none()
    }

    fn reclaim_request(&mut self, memory: &mut PhysicalMemory) -> bool {
        self.pending_page.take().is_none_or(|page| memory.release_page(page))
    }
}

pub fn interrupt() {
    let port = ISR_PORT.load(Ordering::Acquire);
    if port != 0 && unsafe { inb(port) } & 1 != 0 {
        COMPLETE.store(true, Ordering::Release);
        crate::trace::record(crate::trace::Event::VirtioComplete);
    }
}

pub fn completion_pending() -> bool {
    COMPLETE.load(Ordering::Acquire)
}

fn take_completion() -> bool {
    COMPLETE.swap(false, Ordering::AcqRel)
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
