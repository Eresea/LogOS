#![allow(dead_code)]

use core::{
    arch::asm,
    sync::atomic::{AtomicBool, AtomicU16, Ordering, compiler_fence},
};

use logos_core::capabilities::CapabilityKind;

use crate::{
    arch::pci::PciDevice,
    mm::memory::{Page, PhysicalMemory},
};

const ACKNOWLEDGE: u8 = 1;
const DRIVER: u8 = 2;
const DRIVER_OK: u8 = 4;
const FAILED: u8 = 128;
const FEATURE_MAC: u32 = 1 << 5;
const VIRTIO_HEADER: usize = 10;
const RX_BUFFERS: usize = 4;
const MTU: usize = 1500;
const MAX_FRAME: usize = 1514;
static ISR_PORT: AtomicU16 = AtomicU16::new(0);
static COMPLETE: AtomicBool = AtomicBool::new(false);

const NETWORK: crate::drivers::device::DriverManifest = crate::drivers::device::DriverManifest {
    interface: crate::drivers::device::Interface::new(crate::drivers::device::Class::Network),
    vendor_id: 0x1af4,
    device_id: 0x1000,
    capabilities: &[CapabilityKind::Memory],
};

pub fn discover(devices: &crate::arch::pci::PciDevices) -> Option<PciDevice> {
    crate::drivers::device::bind(&[NETWORK], NETWORK.vendor_id, NETWORK.device_id)
        .and_then(|manifest| devices.find_class(manifest.vendor_id, manifest.device_id, 0x02, 0x00))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Info {
    pub mac: [u8; 6],
    pub mtu: u16,
    pub generation: u16,
}

pub struct Device {
    base: u16,
    rx_queue: super::virtio::VirtQueue,
    tx_queue: super::virtio::VirtQueue,
    queue_size: usize,
    tx_queue_size: usize,
    rx_pages: [Page; RX_BUFFERS],
    tx_page: Page,
    rx_available: u16,
    rx_used: u16,
    tx_available: u16,
    tx_used: u16,
    tx_pending: bool,
    info: Info,
}

impl Device {
    pub fn bind(
        device: PciDevice,
        interrupt_gsi: u32,
        memory: &mut PhysicalMemory,
    ) -> Option<Self> {
        if !device.enable_bus_master() {
            return None;
        }
        let bar = device.bar(0);
        if bar & 1 == 0 {
            return None;
        }
        let base = u16::try_from(bar & !3).ok()?;
        unsafe {
            outb(base + 0x12, 0);
            outb(base + 0x12, ACKNOWLEDGE);
            outb(base + 0x12, ACKNOWLEDGE | DRIVER);
            if inl(base) & FEATURE_MAC == 0 {
                outb(base + 0x12, 0);
                return None;
            }
            outl(base + 0x04, FEATURE_MAC);
            outw(base + 0x0e, 0);
        }
        let rx_queue_size = usize::from(unsafe { inw(base + 0x0c) });
        if !(RX_BUFFERS..=256).contains(&rx_queue_size) {
            return None;
        }
        let rx_queue = super::virtio::VirtQueue::allocate(memory, rx_queue_size)?;
        unsafe { outl(base + 0x08, u32::try_from(rx_queue.address() >> 12).ok()?) };
        unsafe { outw(base + 0x0e, 1) };
        let tx_queue_size = usize::from(unsafe { inw(base + 0x0c) });
        if !(1..=256).contains(&tx_queue_size) {
            let _ = rx_queue.release(memory);
            return None;
        }
        let tx_queue = match super::virtio::VirtQueue::allocate(memory, tx_queue_size) {
            Some(queue) => queue,
            None => {
                let _ = rx_queue.release(memory);
                return None;
            }
        };
        let Some(tx_pfn) = u32::try_from(tx_queue.address() >> 12).ok() else {
            let _ = tx_queue.release(memory);
            let _ = rx_queue.release(memory);
            return None;
        };
        unsafe { outl(base + 0x08, tx_pfn) };
        let Some(rx_pages) = allocate_pages::<RX_BUFFERS>(memory) else {
            let _ = tx_queue.release(memory);
            let _ = rx_queue.release(memory);
            return None;
        };
        let Some(tx_page) = memory.allocate_owned() else {
            release_pages(rx_pages, memory);
            let _ = tx_queue.release(memory);
            let _ = rx_queue.release(memory);
            return None;
        };
        let mut mac = [0; 6];
        let mut stable = [0; 6];
        for (index, byte) in mac.iter_mut().enumerate() {
            *byte = unsafe { inb(base + 0x14 + index as u16) };
        }
        for (index, byte) in stable.iter_mut().enumerate() {
            *byte = unsafe { inb(base + 0x14 + index as u16) };
        }
        if mac != stable || mac == [0; 6] || mac == [0xff; 6] {
            let _ = tx_queue.release(memory);
            let _ = rx_queue.release(memory);
            return None;
        }
        let mut network = Self {
            base,
            rx_queue,
            tx_queue,
            queue_size: rx_queue_size,
            tx_queue_size,
            rx_pages,
            tx_page,
            rx_available: 0,
            rx_used: 0,
            tx_available: 0,
            tx_used: 0,
            tx_pending: false,
            info: Info { mac, mtu: MTU as u16, generation: 1 },
        };
        if !network.post_rx() {
            let _ = network.release(memory);
            return None;
        }
        unsafe { outb(base + 0x12, ACKNOWLEDGE | DRIVER | DRIVER_OK) };
        if unsafe { inb(base + 0x12) } & (DRIVER_OK | FAILED) != DRIVER_OK
            || !crate::arch::interrupts::route_virtio(interrupt_gsi)
        {
            crate::debug::write_line(b"LogOS: network device activation failed");
            let _ = network.release(memory);
            return None;
        }
        ISR_PORT.store(base + 0x13, Ordering::Release);
        Some(network)
    }

    pub const fn info(&self) -> Info {
        self.info
    }

    pub fn transmit(&mut self, frame: &[u8]) -> Result<(), NetworkError> {
        if self.tx_pending {
            return Err(NetworkError::Busy);
        }
        if !(60..=MAX_FRAME).contains(&frame.len()) {
            return Err(NetworkError::Length);
        }
        unsafe {
            core::ptr::write_bytes(self.tx_page.address() as *mut u8, 0, VIRTIO_HEADER);
            core::ptr::copy_nonoverlapping(
                frame.as_ptr(),
                (self.tx_page.address() as *mut u8).add(VIRTIO_HEADER),
                frame.len(),
            );
            let descriptor = self.tx_queue.address() as *mut Descriptor;
            descriptor.write_volatile(Descriptor {
                address: self.tx_page.address(),
                length: (VIRTIO_HEADER + frame.len()) as u32,
                flags: 0,
                next: 0,
            });
            let avail = available_address(self.tx_queue.address(), self.tx_queue_size);
            ((avail + 4) as *mut u16).write_volatile(0);
            compiler_fence(Ordering::Release);
            ((avail + 2) as *mut u16).write_volatile(self.tx_available.wrapping_add(1));
            outw(self.base + 0x10, 1);
        }
        self.tx_available = self.tx_available.wrapping_add(1);
        self.tx_pending = true;
        Ok(())
    }

    pub fn complete_transmit(&mut self) -> Result<Option<()>, NetworkError> {
        if !self.tx_pending {
            return Ok(None);
        }
        let used = self.used_index(self.tx_queue.address(), self.tx_queue_size);
        if used == self.tx_used && !COMPLETE.swap(false, Ordering::AcqRel) {
            return Ok(None);
        }
        if used.wrapping_sub(self.tx_used) != 1 {
            return Err(NetworkError::Device);
        }
        let entry = used_entry(self.tx_queue.address(), self.tx_queue_size, self.tx_used);
        let id = unsafe { (entry as *const u32).read_volatile() };
        if id != 0 {
            return Err(NetworkError::Device);
        }
        self.tx_used = used;
        self.tx_pending = false;
        Ok(Some(()))
    }

    pub fn receive(&mut self, output: &mut [u8]) -> Result<Option<usize>, NetworkError> {
        let used = self.used_index(self.rx_queue.address(), self.queue_size);
        if used == self.rx_used {
            COMPLETE.swap(false, Ordering::AcqRel);
            return Ok(None);
        }
        let progress = used.wrapping_sub(self.rx_used);
        if progress == 0 || progress > RX_BUFFERS as u16 {
            return Err(NetworkError::Device);
        }
        let entry = used_entry(self.rx_queue.address(), self.queue_size, self.rx_used);
        let id = unsafe { (entry as *const u32).read_volatile() } as usize;
        let length = unsafe { (entry as *const u32).add(1).read_volatile() } as usize;
        if id >= RX_BUFFERS || !(VIRTIO_HEADER..=VIRTIO_HEADER + MAX_FRAME).contains(&length) {
            return Err(NetworkError::Device);
        }
        let frame_length = length - VIRTIO_HEADER;
        if output.len() < frame_length {
            return Err(NetworkError::Length);
        }
        unsafe {
            core::ptr::copy_nonoverlapping(
                (self.rx_pages[id].address() as *const u8).add(VIRTIO_HEADER),
                output.as_mut_ptr(),
                frame_length,
            );
        }
        self.rx_used = self.rx_used.wrapping_add(1);
        self.post_rx_buffer(id)?;
        Ok(Some(frame_length))
    }

    pub fn reset(&mut self) -> bool {
        unsafe { outb(self.base + 0x12, 0) };
        unsafe {
            core::ptr::write_bytes(
                self.rx_queue.address() as *mut u8,
                0,
                self.rx_queue_size_bytes(),
            );
            core::ptr::write_bytes(
                self.tx_queue.address() as *mut u8,
                0,
                self.tx_queue_size_bytes(),
            );
        }
        self.rx_available = 0;
        self.rx_used = 0;
        self.tx_available = 0;
        self.tx_used = 0;
        self.tx_pending = false;
        self.info.generation = self.info.generation.wrapping_add(1).max(1);
        unsafe {
            outb(self.base + 0x12, ACKNOWLEDGE);
            outb(self.base + 0x12, ACKNOWLEDGE | DRIVER);
            outl(self.base + 0x04, FEATURE_MAC);
            outw(self.base + 0x0e, 0);
            outl(self.base + 0x08, (self.rx_queue.address() >> 12) as u32);
            outw(self.base + 0x0e, 1);
            outl(self.base + 0x08, (self.tx_queue.address() >> 12) as u32);
        }
        if !self.post_rx() {
            return false;
        }
        unsafe { outb(self.base + 0x12, ACKNOWLEDGE | DRIVER | DRIVER_OK) };
        let status = unsafe { inb(self.base + 0x12) };
        status & (DRIVER_OK | FAILED) == DRIVER_OK
    }

    pub fn release(self, memory: &mut PhysicalMemory) -> bool {
        ISR_PORT.store(0, Ordering::Release);
        unsafe { outb(self.base + 0x12, 0) };
        let pages = self.rx_pages;
        let tx = self.tx_page;
        release_pages(pages, memory)
            && memory.release_page(tx)
            && self.rx_queue.release(memory)
            && self.tx_queue.release(memory)
    }

    fn post_rx(&mut self) -> bool {
        for index in 0..RX_BUFFERS {
            if self.post_rx_buffer(index).is_err() {
                return false;
            }
        }
        true
    }

    fn rx_queue_size_bytes(&self) -> usize {
        self.queue_size * core::mem::size_of::<Descriptor>() + 4096
    }

    fn tx_queue_size_bytes(&self) -> usize {
        self.tx_queue_size * core::mem::size_of::<Descriptor>() + 4096
    }

    fn post_rx_buffer(&mut self, index: usize) -> Result<(), NetworkError> {
        if index >= RX_BUFFERS || index >= self.queue_size {
            return Err(NetworkError::Device);
        }
        unsafe {
            outw(self.base + 0x0e, 0);
            core::ptr::write_bytes(self.rx_pages[index].address() as *mut u8, 0, 4096);
            let descriptor = self.rx_queue.address() as *mut Descriptor;
            descriptor.add(index).write_volatile(Descriptor {
                address: self.rx_pages[index].address(),
                length: 4096,
                flags: 2,
                next: 0,
            });
            let avail = available_address(self.rx_queue.address(), self.queue_size);
            let slot = usize::from(self.rx_available) % self.queue_size;
            ((avail + 4 + slot as u64 * 2) as *mut u16).write_volatile(index as u16);
            compiler_fence(Ordering::Release);
            self.rx_available = self.rx_available.wrapping_add(1);
            ((avail + 2) as *mut u16).write_volatile(self.rx_available);
            outw(self.base + 0x10, 0);
        }
        Ok(())
    }

    fn used_index(&self, queue: u64, size: usize) -> u16 {
        unsafe { ((used_address(queue, size) + 2) as *const u16).read_volatile() }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NetworkError {
    Busy,
    Length,
    Device,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Descriptor {
    address: u64,
    length: u32,
    flags: u16,
    next: u16,
}

fn available_address(queue: u64, size: usize) -> u64 {
    queue + (size * core::mem::size_of::<Descriptor>()) as u64
}

fn used_address(queue: u64, size: usize) -> u64 {
    (available_address(queue, size) + 6 + size as u64 * 2 + 4095) & !4095
}

fn used_entry(queue: u64, size: usize, index: u16) -> u64 {
    used_address(queue, size) + 4 + (usize::from(index) % size * 8) as u64
}

fn allocate_pages<const N: usize>(memory: &mut PhysicalMemory) -> Option<[Page; N]> {
    let mut pages = [const { None }; N];
    for page in &mut pages {
        *page = memory.allocate_owned();
    }
    if pages.iter().any(Option::is_none) {
        for page in pages.into_iter().flatten() {
            let _ = memory.release_page(page);
        }
        return None;
    }
    Some(pages.map(Option::unwrap))
}

fn release_pages<const N: usize>(pages: [Page; N], memory: &mut PhysicalMemory) -> bool {
    pages.into_iter().all(|page| memory.release_page(page))
}

pub fn interrupt() {
    let port = ISR_PORT.load(Ordering::Acquire);
    if port != 0 && unsafe { inb(port) } & 1 != 0 {
        COMPLETE.store(true, Ordering::Release);
    }
}

pub fn self_check() -> bool {
    NETWORK.interface.class() == crate::drivers::device::Class::Network
        && NETWORK.vendor_id == 0x1af4
        && NETWORK.device_id == 0x1000
        && Info { mac: [0; 6], mtu: MTU as u16, generation: 1 }.mtu == 1500
}

unsafe fn outb(port: u16, value: u8) {
    unsafe { asm!("out dx, al", in("dx") port, in("al") value) };
}
unsafe fn inb(port: u16) -> u8 {
    let value;
    unsafe { asm!("in al, dx", in("dx") port, out("al") value) };
    value
}
unsafe fn outw(port: u16, value: u16) {
    unsafe { asm!("out dx, ax", in("dx") port, in("ax") value) };
}
unsafe fn inw(port: u16) -> u16 {
    let value;
    unsafe { asm!("in ax, dx", in("dx") port, out("ax") value) };
    value
}
unsafe fn outl(port: u16, value: u32) {
    unsafe { asm!("out dx, eax", in("dx") port, in("eax") value) };
}
unsafe fn inl(port: u16) -> u32 {
    let value;
    unsafe { asm!("in eax, dx", in("dx") port, out("eax") value) };
    value
}
