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
    pub madt: Option<Madt>,
}

#[derive(Clone, Copy)]
pub struct Madt {
    pub local_apic: usize,
    pub io_apic: usize,
    pub io_apic_gsi_base: u32,
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
        let xsdt = rsdp.xsdt as *const Header;
        let header = table_header(xsdt, *b"XSDT")?;
        let entries = ((header.length as usize) - core::mem::size_of::<Header>()) / 8;
        let mut madt = None;
        for index in 0..entries {
            let address = unsafe {
                (xsdt.cast::<u8>().add(core::mem::size_of::<Header>() + index * 8) as *const u64)
                    .read_unaligned()
            };
            let table = address as *const Header;
            if table_header(table, *b"APIC").is_some() {
                madt = parse_madt(table);
                break;
            }
        }
        Some(Tables { xsdt: rsdp.xsdt, madt })
    })
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct Header {
    signature: [u8; 4],
    length: u32,
    revision: u8,
    checksum: u8,
    oem_id: [u8; 6],
    oem_table_id: [u8; 8],
    oem_revision: u32,
    creator_id: u32,
    creator_revision: u32,
}

fn table_header(table: *const Header, signature: [u8; 4]) -> Option<Header> {
    let header = unsafe { table.read_unaligned() };
    let length = header.length;
    (header.signature == signature
        && (core::mem::size_of::<Header>() as u32..=1_048_576).contains(&length)
        && checksum(table.cast(), length as usize))
    .then_some(header)
}

fn parse_madt(table: *const Header) -> Option<Madt> {
    let header = table_header(table, *b"APIC")?;
    let bytes = table.cast::<u8>();
    let local_apic =
        unsafe { bytes.add(core::mem::size_of::<Header>()).cast::<u32>().read_unaligned() };
    let mut offset = core::mem::size_of::<Header>() + 8;
    while offset + 2 <= header.length as usize {
        let kind = unsafe { bytes.add(offset).read() };
        let length = unsafe { bytes.add(offset + 1).read() } as usize;
        if length < 2 || offset + length > header.length as usize {
            return None;
        }
        if kind == 1 && length >= 12 {
            let io_apic = unsafe { bytes.add(offset + 4).cast::<u32>().read_unaligned() };
            let io_apic_gsi_base = unsafe { bytes.add(offset + 8).cast::<u32>().read_unaligned() };
            return Some(Madt {
                local_apic: local_apic as usize,
                io_apic: io_apic as usize,
                io_apic_gsi_base,
            });
        }
        offset += length;
    }
    None
}

fn checksum(bytes: *const u8, length: usize) -> bool {
    let mut sum = 0u8;
    for index in 0..length {
        sum = sum.wrapping_add(unsafe { bytes.add(index).read() });
    }
    sum == 0
}
