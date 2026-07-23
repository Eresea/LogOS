use uefi::{system, table::cfg::ACPI2_GUID};

#[repr(C, packed)]
struct Rsdp {
    signature: [u8; 8],
    checksum: u8,
    oem_id: [u8; 6],
    revision: u8,
    rsdt: u32,
    length: u32,
    xsdt: u64,
    extended_checksum: u8,
    reserved: [u8; 3],
}

#[derive(Clone, Copy)]
pub struct Tables {
    pub xsdt: u64,
}

pub fn discover() -> Option<Tables> {
    system::with_config_table(|tables| {
        let entry = tables.iter().find(|entry| entry.guid == ACPI2_GUID)?;
        let rsdp = unsafe { (entry.address as *const Rsdp).as_ref()? };
        if rsdp.signature != *b"RSD PTR "
            || rsdp.revision < 2
            || rsdp.length < core::mem::size_of::<Rsdp>() as u32
            || !checksum(rsdp as *const Rsdp as *const u8, 20)
            || !checksum(rsdp as *const Rsdp as *const u8, rsdp.length as usize)
            || rsdp.xsdt == 0
        {
            return None;
        }
        Some(Tables { xsdt: rsdp.xsdt })
    })
}

fn checksum(bytes: *const u8, length: usize) -> bool {
    let mut sum = 0u8;
    for index in 0..length {
        sum = sum.wrapping_add(unsafe { bytes.add(index).read() });
    }
    sum == 0
}
