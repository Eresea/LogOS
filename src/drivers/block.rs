use core::{
    arch::asm,
    sync::atomic::{AtomicBool, AtomicU16, Ordering, compiler_fence},
};

use crate::{
    memory::{Contiguous, Page, PhysicalMemory},
    pci::PciDevice,
};
use logos_abi::{BlockInfo, BlockOperation, BlockRequest, PersistenceStatus};

const ACKNOWLEDGE: u8 = 1;
const DRIVER: u8 = 2;
const DRIVER_OK: u8 = 4;
const FAILED: u8 = 128;
const NEXT: u16 = 1;
const DEVICE_WRITE: u16 = 2;
const FEATURE_FLUSH: u32 = 1 << 9;
const REQUEST_FLUSH: u32 = 4;
const QUEUE_DEPTH: usize = 8;
const QUEUE_PAGES: usize = 2;
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

#[repr(C)]
struct Header {
    kind: u32,
    reserved: u32,
    sector: u64,
}

pub struct Device {
    base: u16,
    queue: Contiguous,
    queue_size: usize,
    info: BlockInfo,
    pending: Option<Page>,
    resets: u32,
    timeouts: u32,
    last_recovery_failed: bool,
}

impl Device {
    pub fn bind(
        device: PciDevice,
        interrupt_gsi: u32,
        memory: &mut PhysicalMemory,
    ) -> Option<Self> {
        let bar = device.bar(0);
        if bar & 1 == 0 {
            return None;
        }
        let base = (bar & !3) as u16;
        unsafe {
            outb(base + 0x12, 0);
            outb(base + 0x12, ACKNOWLEDGE);
            if inl(base) & FEATURE_FLUSH == 0 {
                return None;
            }
            outl(base + 0x04, FEATURE_FLUSH);
            outb(base + 0x12, ACKNOWLEDGE | DRIVER);
            outw(base + 0x0e, 0);
        }
        let queue_size = usize::from(unsafe { inw(base + 0x0c) }).min(QUEUE_DEPTH);
        if queue_size < 3 {
            return None;
        }
        let queue = memory.allocate_contiguous(QUEUE_PAGES)?;
        unsafe { core::ptr::write_bytes(queue.address() as *mut u8, 0, QUEUE_PAGES * 4096) };
        let Ok(pfn) = u32::try_from(queue.address() >> 12) else {
            let _ = queue.release(memory);
            return None;
        };
        unsafe {
            outl(base + 0x08, pfn);
            outb(base + 0x12, ACKNOWLEDGE | DRIVER | DRIVER_OK);
        }
        let blocks =
            u64::from(unsafe { inl(base + 0x14) }) | (u64::from(unsafe { inl(base + 0x18) }) << 32);
        let info = BlockInfo {
            logical_block_size: 512,
            blocks,
            max_transfer_blocks: (logos_abi::PAGE_SIZE / 512) as u32,
        };
        if !info.valid() || !crate::interrupts::route_virtio(interrupt_gsi) {
            unsafe { outb(base + 0x12, 0) };
            let _ = queue.release(memory);
            return None;
        }
        ISR_PORT.store(base + 0x13, Ordering::Release);
        Some(Self {
            base,
            queue,
            queue_size,
            info,
            pending: None,
            resets: 0,
            timeouts: 0,
            last_recovery_failed: false,
        })
    }

    pub const fn info(&self) -> BlockInfo {
        self.info
    }

    pub fn submit(
        &mut self,
        request: BlockRequest,
        page_address: Option<u64>,
        memory: &mut PhysicalMemory,
    ) -> PersistenceStatus {
        if self.pending.is_some() || request.blocks > self.info.max_transfer_blocks {
            return PersistenceStatus::Invalid;
        }
        let Some(data_length) = request.blocks.checked_mul(512) else {
            return PersistenceStatus::Invalid;
        };
        let Some(end) = request.lba.checked_add(u64::from(request.blocks)) else {
            return PersistenceStatus::Invalid;
        };
        let kind = match request.operation {
            BlockOperation::Read => 0,
            BlockOperation::Write => 1,
            BlockOperation::Flush => REQUEST_FLUSH,
            BlockOperation::Cancel => return PersistenceStatus::Cancelled,
            BlockOperation::Reset => {
                return if self.reset() {
                    PersistenceStatus::Recovered
                } else {
                    PersistenceStatus::Io
                };
            }
        };
        if end > self.info.blocks
            || (kind != REQUEST_FLUSH
                && (request.blocks == 0 || page_address.is_none_or(|page| page == 0)))
        {
            return PersistenceStatus::Invalid;
        }
        let Some(metadata) = memory.allocate_owned() else {
            return PersistenceStatus::OutOfMemory;
        };
        let address = metadata.address();
        unsafe {
            (address as *mut Header).write_volatile(Header {
                kind,
                reserved: 0,
                sector: request.lba,
            });
            (address as *mut u8).add(16).write_volatile(0xff);
        }
        let queue = self.queue.address();
        let descriptors = queue as *mut Descriptor;
        unsafe {
            descriptors.write_volatile(Descriptor { address, length: 16, flags: NEXT, next: 1 });
            if kind == REQUEST_FLUSH {
                descriptors.add(1).write_volatile(Descriptor {
                    address: address + 16,
                    length: 1,
                    flags: DEVICE_WRITE,
                    next: 0,
                });
            } else {
                descriptors.add(1).write_volatile(Descriptor {
                    address: page_address.unwrap_or(0),
                    length: data_length,
                    flags: NEXT
                        | if request.operation == BlockOperation::Read { DEVICE_WRITE } else { 0 },
                    next: 2,
                });
                descriptors.add(2).write_volatile(Descriptor {
                    address: address + 16,
                    length: 1,
                    flags: DEVICE_WRITE,
                    next: 0,
                });
            }
            let available = queue + (self.queue_size * core::mem::size_of::<Descriptor>()) as u64;
            ((available + 4) as *mut u16).write_volatile(0);
            compiler_fence(Ordering::Release);
            ((available + 2) as *mut u16).write_volatile(1);
            outw(self.base + 0x10, 0);
        }
        self.pending = Some(metadata);
        PersistenceStatus::Complete
    }

    pub fn complete(&mut self, memory: &mut PhysicalMemory) -> Option<PersistenceStatus> {
        if !COMPLETE.swap(false, Ordering::AcqRel) {
            return None;
        }
        let page = self.pending.take()?;
        let status = unsafe { (page.address() as *const u8).add(16).read_volatile() };
        let released = memory.release_page(page);
        Some(if status == 0 && released {
            PersistenceStatus::Complete
        } else {
            PersistenceStatus::Io
        })
    }

    pub fn timeout(&mut self, memory: &mut PhysicalMemory) -> PersistenceStatus {
        self.timeouts = self.timeouts.saturating_add(1);
        let recovered = self.reset();
        let released = self.pending.take().is_none_or(|page| memory.release_page(page));
        if recovered && released { PersistenceStatus::TimedOut } else { PersistenceStatus::Io }
    }

    pub fn release(mut self, memory: &mut PhysicalMemory) -> bool {
        ISR_PORT.store(0, Ordering::Release);
        unsafe { outb(self.base + 0x12, 0) };
        let pending = self.pending.take().is_none_or(|page| memory.release_page(page));
        pending && self.queue.release(memory)
    }

    pub const fn diagnostics(&self) -> (u32, u32, bool) {
        (self.resets, self.timeouts, self.last_recovery_failed)
    }

    fn reset(&mut self) -> bool {
        self.resets = self.resets.saturating_add(1);
        let Ok(pfn) = u32::try_from(self.queue.address() >> 12) else {
            self.last_recovery_failed = true;
            return false;
        };
        unsafe {
            outb(self.base + 0x12, 0);
            outb(self.base + 0x12, ACKNOWLEDGE);
            outl(self.base + 0x04, FEATURE_FLUSH);
            outb(self.base + 0x12, ACKNOWLEDGE | DRIVER);
            outw(self.base + 0x0e, 0);
            outl(self.base + 0x08, pfn);
            outb(self.base + 0x12, ACKNOWLEDGE | DRIVER | DRIVER_OK);
        }
        self.last_recovery_failed =
            unsafe { inb(self.base + 0x12) } & (DRIVER_OK | FAILED) != DRIVER_OK;
        !self.last_recovery_failed
    }
}

pub fn interrupt() {
    let port = ISR_PORT.load(Ordering::Acquire);
    if port != 0 && unsafe { inb(port) } & 1 != 0 {
        COMPLETE.store(true, Ordering::Release);
    }
}

pub fn self_check() -> bool {
    let _submit: fn(
        &mut Device,
        BlockRequest,
        Option<u64>,
        &mut PhysicalMemory,
    ) -> PersistenceStatus = Device::submit;
    let _complete: fn(&mut Device, &mut PhysicalMemory) -> Option<PersistenceStatus> =
        Device::complete;
    let _timeout: fn(&mut Device, &mut PhysicalMemory) -> PersistenceStatus = Device::timeout;
    let _release: fn(Device, &mut PhysicalMemory) -> bool = Device::release;
    QUEUE_DEPTH == logos_abi::MAX_PERSISTENCE_OPERATIONS && QUEUE_PAGES == 2
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
