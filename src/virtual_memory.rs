use core::{arch::asm, ptr};

use crate::memory::PhysicalMemory;

const ENTRIES: usize = 512;
const PRESENT_WRITABLE: u64 = 0b11;
const ADDRESS_MASK: u64 = 0x000f_ffff_ffff_f000;

pub fn install(physical: &mut PhysicalMemory) -> Option<u64> {
    let pml4 = physical.allocate_page()?;
    let pdpt = physical.allocate_page()?;
    let pd = physical.allocate_page()?;
    let pt = physical.allocate_page()?;
    let mapped_page = physical.allocate_page()?;

    unsafe {
        ptr::copy_nonoverlapping(read_cr3() as *const u64, pml4 as *mut u64, ENTRIES);
        ptr::write_bytes(pdpt as *mut u8, 0, 4096);
        ptr::write_bytes(pd as *mut u8, 0, 4096);
        ptr::write_bytes(pt as *mut u8, 0, 4096);

        let pml4 = pml4 as *mut u64;
        let slot = (256..ENTRIES).find(|&index| pml4.add(index).read() == 0)?;
        pml4.add(slot).write(pdpt | PRESENT_WRITABLE);
        (pdpt as *mut u64).write(pd | PRESENT_WRITABLE);
        (pd as *mut u64).write(pt | PRESENT_WRITABLE);
        (pt as *mut u64).write(mapped_page | PRESENT_WRITABLE);
        write_cr3(pml4 as u64);
        Some(canonical_address(slot))
    }
}

pub unsafe fn verify(address: u64) -> bool {
    let value = 0x004c_4f47_4f53_u64;
    let page = address as *mut u64;
    unsafe {
        page.write_volatile(value);
        page.read_volatile() == value
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
