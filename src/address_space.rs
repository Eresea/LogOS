use core::{arch::asm, ptr};

use crate::memory::{Page, PhysicalMemory};

const ENTRIES: usize = 512;
const PAGE_SIZE: u64 = 4096;
const PRESENT: u64 = 1;
const WRITABLE: u64 = 1 << 1;
const USER: u64 = 1 << 2;
const ADDRESS_MASK: u64 = 0x000f_ffff_ffff_f000;

pub struct AddressSpace {
    pml4: Page,
    pdpt: Page,
    pd: Page,
    pt: Page,
    code: Page,
    stack: Page,
    code_address: u64,
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
        let Some(code) = physical.allocate_owned() else {
            let _ = physical.release_page(pt);
            let _ = physical.release_page(pd);
            let _ = physical.release_page(pdpt);
            let _ = physical.release_page(pml4);
            return None;
        };
        let Some(stack) = physical.allocate_owned() else {
            let _ = physical.release_page(code);
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
        let code_address = code.address();
        let stack_address = stack.address();
        unsafe {
            ptr::copy_nonoverlapping(read_cr3() as *const u64, pml4_address as *mut u64, ENTRIES);
            ptr::write_bytes(pdpt_address as *mut u8, 0, PAGE_SIZE as usize);
            ptr::write_bytes(pd_address as *mut u8, 0, PAGE_SIZE as usize);
            ptr::write_bytes(pt_address as *mut u8, 0, PAGE_SIZE as usize);
            let pml4_table = pml4_address as *mut u64;
            let Some(slot) = (0..ENTRIES).find(|&index| pml4_table.add(index).read() == 0) else {
                let _ = physical.release_page(stack);
                let _ = physical.release_page(code);
                let _ = physical.release_page(pt);
                let _ = physical.release_page(pd);
                let _ = physical.release_page(pdpt);
                let _ = physical.release_page(pml4);
                return None;
            };
            pml4_table.add(slot).write(pdpt_address | PRESENT | WRITABLE | USER);
            (pdpt_address as *mut u64).write(pd_address | PRESENT | WRITABLE | USER);
            (pd_address as *mut u64).write(pt_address | PRESENT | WRITABLE | USER);
            (pt_address as *mut u64).write(code_address | PRESENT | USER);
            (pt_address as *mut u64).add(1).write(stack_address | PRESENT | WRITABLE | USER);
            Some(Self { pml4, pdpt, pd, pt, code, stack, code_address: canonical_address(slot) })
        }
    }

    pub fn code_address(&self) -> u64 {
        self.code_address
    }

    pub fn stack_top(&self) -> u64 {
        self.code_address + PAGE_SIZE * 2
    }

    pub fn verifies_isolation(&self) -> bool {
        unsafe {
            let pml4 = self.pml4.address() as *const u64;
            let slot = (self.code_address >> 39) as usize & 0x1ff;
            let entry = pml4.add(slot).read_volatile();
            entry & (PRESENT | USER) == PRESENT | USER
                && (self.pt.address() as *const u64).read_volatile() & (PRESENT | WRITABLE | USER)
                    == PRESENT | USER
                && (self.pt.address() as *const u64).add(1).read_volatile()
                    & (PRESENT | WRITABLE | USER)
                    == PRESENT | WRITABLE | USER
                && (0..ENTRIES)
                    .filter(|&index| index != slot)
                    .all(|index| pml4.add(index).read_volatile() & USER == 0)
        }
    }

    pub fn release(self, physical: &mut PhysicalMemory) -> bool {
        let stack = physical.release_page(self.stack);
        let code = physical.release_page(self.code);
        let pt = physical.release_page(self.pt);
        let pd = physical.release_page(self.pd);
        let pdpt = physical.release_page(self.pdpt);
        let pml4 = physical.release_page(self.pml4);
        stack && code && pt && pd && pdpt && pml4
    }
}

unsafe fn read_cr3() -> u64 {
    let value: u64;
    unsafe { asm!("mov {}, cr3", out(reg) value) };
    value & ADDRESS_MASK
}

const fn canonical_address(pml4_index: usize) -> u64 {
    (pml4_index as u64) << 39
}
