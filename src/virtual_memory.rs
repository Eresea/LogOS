use core::{arch::asm, ptr};

use crate::memory::{Page, PhysicalMemory};

const ENTRIES: usize = 512;
const PRESENT_WRITABLE: u64 = 0b11;
const ADDRESS_MASK: u64 = 0x000f_ffff_ffff_f000;

pub struct Mapping {
    address: u64,
    previous_cr3: u64,
    pml4: Page,
    pdpt: Page,
    pd: Page,
    pt: Page,
    mapped: Page,
}

pub fn install(physical: &mut PhysicalMemory) -> Option<Mapping> {
    let pml4 = physical.allocate_owned()?;
    let pdpt = physical.allocate_owned()?;
    let pd = physical.allocate_owned()?;
    let pt = physical.allocate_owned()?;
    let mapped = physical.allocate_owned()?;
    let previous_cr3 = unsafe { read_cr3() };
    let pml4_address = pml4.address();
    let pdpt_address = pdpt.address();
    let pd_address = pd.address();
    let pt_address = pt.address();
    let mapped_address = mapped.address();

    unsafe {
        ptr::copy_nonoverlapping(previous_cr3 as *const u64, pml4_address as *mut u64, ENTRIES);
        ptr::write_bytes(pdpt_address as *mut u8, 0, 4096);
        ptr::write_bytes(pd_address as *mut u8, 0, 4096);
        ptr::write_bytes(pt_address as *mut u8, 0, 4096);

        let pml4_table = pml4_address as *mut u64;
        let slot = (256..ENTRIES).find(|&index| pml4_table.add(index).read() == 0)?;
        pml4_table.add(slot).write(pdpt_address | PRESENT_WRITABLE);
        (pdpt_address as *mut u64).write(pd_address | PRESENT_WRITABLE);
        (pd_address as *mut u64).write(pt_address | PRESENT_WRITABLE);
        (pt_address as *mut u64).write(mapped_address | PRESENT_WRITABLE);
        write_cr3(pml4_address);
        Some(Mapping { address: canonical_address(slot), previous_cr3, pml4, pdpt, pd, pt, mapped })
    }
}

pub unsafe fn verify(mapping: &Mapping) -> bool {
    let value = 0x004c_4f47_4f53_u64;
    let page = mapping.address as *mut u64;
    unsafe {
        page.write_volatile(value);
        page.read_volatile() == value
    }
}

impl Mapping {
    pub fn release(self, physical: &mut PhysicalMemory) -> bool {
        unsafe {
            let pml4 = self.pml4.address() as *mut u64;
            let slot = (self.address >> 39) as usize & 0x1ff;
            pml4.add(slot).write(0);
            write_cr3(self.previous_cr3);
        }
        physical.release_page(self.mapped)
            && physical.release_page(self.pt)
            && physical.release_page(self.pd)
            && physical.release_page(self.pdpt)
            && physical.release_page(self.pml4)
    }
}

unsafe fn read_cr3() -> u64 {
    let value: u64;
    unsafe { asm!("mov {}, cr3", out(reg) value) };
    value & ADDRESS_MASK
}

unsafe fn write_cr3(value: u64) {
    unsafe { asm!("mov cr3, {}", in(reg) value) };
}

const fn canonical_address(pml4_index: usize) -> u64 {
    ((pml4_index as u64) << 39) | 0xffff_0000_0000_0000
}
