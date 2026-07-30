use core::{
    arch::{asm, x86_64::__cpuid},
    ptr,
};

use crate::memory::{Page, PhysicalMemory};

const ENTRIES: usize = 512;
const PAGE_SIZE: u64 = 4096;
const PRESENT: u64 = 1;
const WRITABLE: u64 = 1 << 1;
const USER: u64 = 1 << 2;
const NO_EXECUTE: u64 = 1 << 63;
const ADDRESS_MASK: u64 = 0x000f_ffff_ffff_f000;

pub struct AddressSpace {
    pml4: Page,
    pdpt: Page,
    pd: Page,
    pt: Page,
    stack_lower: Page,
    stack: Page,
    mapped: [Option<Page>; ENTRIES],
    base: u64,
}

impl AddressSpace {
    pub fn new(physical: &mut PhysicalMemory) -> Option<Self> {
        let pml4 = physical.allocate_owned()?;
        let Some(pdpt) = physical.allocate_owned() else {
            let _ = physical.release_page(pml4);
            return None;
        };
        let Some(pd) = physical.allocate_owned() else {
            let _ = physical.release_page(pdpt);
            let _ = physical.release_page(pml4);
            return None;
        };
        let Some(pt) = physical.allocate_owned() else {
            let _ = physical.release_page(pd);
            let _ = physical.release_page(pdpt);
            let _ = physical.release_page(pml4);
            return None;
        };
        let Some(stack_lower) = physical.allocate_owned() else {
            let _ = physical.release_page(pt);
            let _ = physical.release_page(pd);
            let _ = physical.release_page(pdpt);
            let _ = physical.release_page(pml4);
            return None;
        };
        let Some(stack) = physical.allocate_owned() else {
            let _ = physical.release_page(stack_lower);
            let _ = physical.release_page(pt);
            let _ = physical.release_page(pd);
            let _ = physical.release_page(pdpt);
            let _ = physical.release_page(pml4);
            return None;
        };
        let pml4_address = pml4.address();
        let pdpt_address = pdpt.address();
        let pd_address = pd.address();
        let pt_address = pt.address();
        let stack_address = stack.address();
        unsafe {
            ptr::copy_nonoverlapping(read_cr3() as *const u64, pml4_address as *mut u64, ENTRIES);
            ptr::write_bytes(pdpt_address as *mut u8, 0, PAGE_SIZE as usize);
            ptr::write_bytes(pd_address as *mut u8, 0, PAGE_SIZE as usize);
            ptr::write_bytes(pt_address as *mut u8, 0, PAGE_SIZE as usize);
            let pml4_table = pml4_address as *mut u64;
            let Some(slot) = (256..ENTRIES).find(|&index| pml4_table.add(index).read() == 0) else {
                let _ = physical.release_page(stack);
                let _ = physical.release_page(pt);
                let _ = physical.release_page(pd);
                let _ = physical.release_page(pdpt);
                let _ = physical.release_page(pml4);
                return None;
            };
            pml4_table.add(slot).write(pdpt_address | PRESENT | WRITABLE | USER);
            (pdpt_address as *mut u64).write(pd_address | PRESENT | WRITABLE | USER);
            (pd_address as *mut u64).write(pt_address | PRESENT | WRITABLE | USER);
            (pt_address as *mut u64)
                .add(ENTRIES - 2)
                .write(stack_lower.address() | PRESENT | WRITABLE | USER | NO_EXECUTE);
            (pt_address as *mut u64)
                .add(ENTRIES - 1)
                .write(stack_address | PRESENT | WRITABLE | USER | NO_EXECUTE);
            Some(Self {
                pml4,
                pdpt,
                pd,
                pt,
                stack_lower,
                stack,
                mapped: [const { None }; ENTRIES],
                base: canonical_address(slot),
            })
        }
    }

    pub fn map_image(
        &mut self,
        physical: &mut PhysicalMemory,
        payload: crate::payload::Payload,
    ) -> Option<u64> {
        if !enable_nx() {
            return None;
        }
        for section in payload.sections() {
            let start = usize::try_from(section.address).ok()? / PAGE_SIZE as usize;
            let end_rva = section.address.checked_add(section.size)?;
            let end = usize::try_from(end_rva.checked_add(PAGE_SIZE as u32 - 1)?).ok()?
                / PAGE_SIZE as usize;
            if end >= ENTRIES - 2 {
                self.unmap_image(physical);
                return None;
            }
            for index in start..end {
                if !self.map_page(physical, payload, index, section.writable, section.executable) {
                    self.unmap_image(physical);
                    return None;
                }
            }
        }
        let entry = self.base.checked_add(u64::from(payload.entry_rva()))?;
        self.image_maps(entry).then_some(entry)
    }

    pub fn map_probe(&mut self, physical: &mut PhysicalMemory) -> Option<u64> {
        if self.mapped[0].is_some() {
            return None;
        }
        let page = physical.allocate_owned()?;
        unsafe {
            ptr::write_bytes(page.address() as *mut u8, 0, PAGE_SIZE as usize);
            (page.address() as *mut u8).write_volatile(0xcd);
            (page.address() as *mut u8).add(1).write_volatile(0x80);
        }
        let address = page.address();
        self.mapped[0] = Some(page);
        unsafe {
            (self.pt.address() as *mut u64).write_volatile(address | PRESENT | USER);
            (self.pt.address() as *mut u64)
                .add(1)
                .write_volatile(address | PRESENT | WRITABLE | USER | NO_EXECUTE);
        }
        Some(self.base)
    }

    pub fn map_context(&mut self, physical: &mut PhysicalMemory) -> Option<(u64, u64)> {
        const CONTEXT_PAGE: usize = ENTRIES - 4;
        if self.mapped[CONTEXT_PAGE].is_some() {
            return None;
        }
        let page = physical.allocate_owned()?;
        unsafe {
            ptr::write_bytes(page.address() as *mut u8, 0, PAGE_SIZE as usize);
            (page.address() as *mut logos_core::native_service::Context)
                .write_volatile(logos_core::native_service::Context::new());
            (self.pt.address() as *mut u64)
                .add(CONTEXT_PAGE)
                .write_volatile(page.address() | PRESENT | WRITABLE | USER | NO_EXECUTE);
        }
        let address = page.address();
        self.mapped[CONTEXT_PAGE] = Some(page);
        Some((address, self.base + PAGE_SIZE * CONTEXT_PAGE as u64))
    }

    pub const fn cr3(&self) -> u64 {
        self.pml4.address()
    }

    pub fn stack_top(&self) -> u64 {
        self.base + PAGE_SIZE * ENTRIES as u64
    }

    pub fn map_kernel_stack(&mut self, address: u64) -> bool {
        if address & (PAGE_SIZE - 1) != 0 {
            return false;
        }
        unsafe {
            (self.pt.address() as *mut u64)
                .add(ENTRIES - 3)
                .write_volatile(address | PRESENT | WRITABLE | NO_EXECUTE);
        }
        true
    }

    pub fn kernel_stack_top(&self) -> u64 {
        self.base + PAGE_SIZE * (ENTRIES - 2) as u64
    }

    pub fn verifies_isolation(&self) -> bool {
        unsafe {
            let pml4 = self.pml4.address() as *const u64;
            let slot = (self.base >> 39) as usize & 0x1ff;
            let entry = pml4.add(slot).read_volatile();
            entry & (PRESENT | USER) == PRESENT | USER
                && (self.pt.address() as *const u64).add(ENTRIES - 1).read_volatile()
                    & (PRESENT | WRITABLE | USER)
                    == PRESENT | WRITABLE | USER
                && (0..ENTRIES)
                    .filter(|&index| index != slot)
                    .all(|index| pml4.add(index).read_volatile() & USER == 0)
        }
    }

    pub fn release(self, physical: &mut PhysicalMemory) -> bool {
        let mapped = self
            .mapped
            .into_iter()
            .flatten()
            .fold(true, |released, page| physical.release_page(page) && released);
        let stack = physical.release_page(self.stack);
        let stack_lower = physical.release_page(self.stack_lower);
        let pt = physical.release_page(self.pt);
        let pd = physical.release_page(self.pd);
        let pdpt = physical.release_page(self.pdpt);
        let pml4 = physical.release_page(self.pml4);
        mapped && stack && stack_lower && pt && pd && pdpt && pml4
    }

    fn map_page(
        &mut self,
        physical: &mut PhysicalMemory,
        payload: crate::payload::Payload,
        index: usize,
        writable: bool,
        executable: bool,
    ) -> bool {
        let table = self.pt.address() as *mut u64;
        let entry = unsafe { table.add(index).read_volatile() };
        if self.mapped[index].is_none() {
            let rva = match u32::try_from(index * PAGE_SIZE as usize) {
                Ok(rva) => rva,
                Err(_) => return false,
            };
            let Some(page) = physical.allocate_owned() else {
                return false;
            };
            if !payload.copy_page(rva, page.address(), self.base) {
                let _ = physical.release_page(page);
                return false;
            }
            self.mapped[index] = Some(page);
        }
        let Some(page) = self.mapped[index].as_ref() else {
            return false;
        };
        let writable = writable || entry & WRITABLE != 0;
        let executable = executable || entry & NO_EXECUTE == 0 && entry & PRESENT != 0;
        let flags = PRESENT
            | USER
            | if writable { WRITABLE } else { 0 }
            | if executable { 0 } else { NO_EXECUTE };
        unsafe { table.add(index).write_volatile(page.address() | flags) };
        true
    }

    fn unmap_image(&mut self, physical: &mut PhysicalMemory) {
        for (index, page) in self.mapped.iter_mut().enumerate() {
            if let Some(page) = page.take() {
                let _ = physical.release_page(page);
                unsafe { (self.pt.address() as *mut u64).add(index).write_volatile(0) };
            }
        }
    }

    fn image_maps(&self, entry: u64) -> bool {
        let index = ((entry - self.base) / PAGE_SIZE) as usize;
        index < ENTRIES - 1 && self.mapped[index].is_some()
    }
}

fn enable_nx() -> bool {
    if unsafe { __cpuid(0x8000_0000) }.eax < 0x8000_0001
        || unsafe { __cpuid(0x8000_0001) }.edx & (1 << 20) == 0
    {
        return false;
    }
    let low: u32;
    let high: u32;
    unsafe {
        asm!("rdmsr", in("ecx") 0xc000_0080u32, lateout("eax") low, lateout("edx") high);
        asm!("wrmsr", in("ecx") 0xc000_0080u32, in("eax") low | (1 << 11), in("edx") high);
    }
    true
}

unsafe fn read_cr3() -> u64 {
    let value: u64;
    unsafe { asm!("mov {}, cr3", out(reg) value) };
    value & ADDRESS_MASK
}

const fn canonical_address(pml4_index: usize) -> u64 {
    ((pml4_index as u64) << 39) | 0xffff_0000_0000_0000
}
