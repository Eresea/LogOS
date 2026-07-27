use core::{arch::asm, ptr};

use crate::memory::{Page, PhysicalMemory};

const ENTRIES: usize = 512;
const PRESENT: u64 = 1;
const WRITABLE: u64 = 1 << 1;
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

#[derive(Clone, Copy)]
pub enum Permission {
    ReadOnly,
    ReadWrite,
}

pub fn install(physical: &mut PhysicalMemory, permission: Permission) -> Option<Mapping> {
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
    let Some(mapped) = physical.allocate_owned() else {
        let _ = physical.release_page(pt);
        let _ = physical.release_page(pd);
        let _ = physical.release_page(pdpt);
        let _ = physical.release_page(pml4);
        return None;
    };
    let previous_cr3 = unsafe { read_cr3() };
    let pml4_address = pml4.address();
    let pdpt_address = pdpt.address();
    let pd_address = pd.address();
    let pt_address = pt.address();
    let mapped_address = mapped.address();

    // SAFETY: the active UEFI map identity-addresses these conventional pages; its copied PML4
    // entries preserve that access until `Mapping::release` restores the original CR3.
    unsafe {
        ptr::copy_nonoverlapping(previous_cr3 as *const u64, pml4_address as *mut u64, ENTRIES);
        ptr::write_bytes(pdpt_address as *mut u8, 0, 4096);
        ptr::write_bytes(pd_address as *mut u8, 0, 4096);
        ptr::write_bytes(pt_address as *mut u8, 0, 4096);

        let pml4_table = pml4_address as *mut u64;
        let Some(slot) = (256..ENTRIES).find(|&index| pml4_table.add(index).read() == 0) else {
            let _ = physical.release_page(mapped);
            let _ = physical.release_page(pt);
            let _ = physical.release_page(pd);
            let _ = physical.release_page(pdpt);
            let _ = physical.release_page(pml4);
            return None;
        };
        pml4_table.add(slot).write(pdpt_address | PRESENT | WRITABLE);
        (pdpt_address as *mut u64).write(pd_address | PRESENT | WRITABLE);
        (pd_address as *mut u64).write(pt_address | PRESENT | WRITABLE);
        (pt_address as *mut u64).write(
            mapped_address
                | PRESENT
                | if matches!(permission, Permission::ReadWrite) { WRITABLE } else { 0 },
        );
        write_cr3(pml4_address);
        Some(Mapping { address: canonical_address(slot), previous_cr3, pml4, pdpt, pd, pt, mapped })
    }
}

pub unsafe fn verify(mapping: &Mapping) -> bool {
    let value = 0x004c_4f47_4f53_u64;
    let page = mapping.address as *mut u64;
    // SAFETY: `install` created this canonical mapping and it remains live until `release`.
    unsafe {
        page.write_volatile(value);
        page.read_volatile() == value
    }
}

impl Mapping {
    pub fn is_writable(&self) -> bool {
        // SAFETY: the bootstrap map keeps the page-table page accessible while this mapping lives.
        unsafe { (self.pt.address() as *const u64).read_volatile() & WRITABLE != 0 }
    }

    pub fn release(self, physical: &mut PhysicalMemory) -> bool {
        // SAFETY: the bootstrap map keeps the PML4 accessible; restoring CR3 precedes freeing it.
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
