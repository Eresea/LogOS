use core::{
    arch::asm,
    sync::atomic::{AtomicBool, AtomicU8, AtomicU16, Ordering},
};

use uefi::{system, table::cfg::ACPI2_GUID};

// ponytail: fixed QEMU routing table; add dynamic storage when bridge discovery needs more routes.
const ROUTES: usize = 64;
static RESET_PORT: AtomicU16 = AtomicU16::new(0);
static RESET_VALUE: AtomicU8 = AtomicU8::new(0);
static RESET_READY: AtomicBool = AtomicBool::new(false);
static POWER_PORT: AtomicU16 = AtomicU16::new(0);
static POWER_VALUE: AtomicU16 = AtomicU16::new(0);
static POWER_READY: AtomicBool = AtomicBool::new(false);

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
    routes: [Option<PciRoute>; ROUTES],
    reset: Option<Reset>,
    power: Option<Power>,
}

#[derive(Clone, Copy)]
pub struct Madt {
    pub local_apic: usize,
    pub io_apic: usize,
    pub io_apic_gsi_base: u32,
    pub topology: CpuTopology,
}

pub const MAX_CPUS: usize = 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Cpu {
    pub apic_id: u32,
    pub enabled: bool,
    pub bsp: bool,
}

impl Cpu {
    pub const fn usable(self) -> bool {
        self.enabled
    }
}

#[derive(Clone, Copy)]
pub struct CpuTopology {
    cpus: [Option<Cpu>; MAX_CPUS],
    count: usize,
    truncated: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TopologyError {
    Malformed,
}

impl CpuTopology {
    pub const fn empty() -> Self {
        Self { cpus: [None; MAX_CPUS], count: 0, truncated: false }
    }

    pub fn parse(entries: &[u8], bsp_apic_id: Option<u32>) -> Result<Self, TopologyError> {
        let mut topology = Self::empty();
        let mut offset = 0;
        while offset < entries.len() {
            let Some(&kind) = entries.get(offset) else {
                return Err(TopologyError::Malformed);
            };
            let Some(&length) = entries.get(offset + 1) else {
                return Err(TopologyError::Malformed);
            };
            let length = length as usize;
            if length < 2 || length > entries.len() - offset {
                return Err(TopologyError::Malformed);
            }
            let entry = &entries[offset..offset + length];
            let processor = match kind {
                0 if length >= 8 => {
                    let apic_id = u32::from(entry[3]);
                    let flags = u32::from_le_bytes([entry[4], entry[5], entry[6], entry[7]]);
                    Some((apic_id, flags))
                }
                9 if length >= 16 => {
                    let apic_id = u32::from_le_bytes([entry[4], entry[5], entry[6], entry[7]]);
                    let flags = u32::from_le_bytes([entry[8], entry[9], entry[10], entry[11]]);
                    Some((apic_id, flags))
                }
                0 | 9 => return Err(TopologyError::Malformed),
                _ => None,
            };
            if let Some((apic_id, flags)) = processor {
                if topology.cpus[..topology.count]
                    .iter()
                    .flatten()
                    .any(|cpu| cpu.apic_id == apic_id)
                {
                    return Err(TopologyError::Malformed);
                }
                let cpu =
                    Cpu { apic_id, enabled: flags & 0x3 != 0, bsp: bsp_apic_id == Some(apic_id) };
                if topology.count == MAX_CPUS {
                    topology.truncated = true;
                } else {
                    topology.cpus[topology.count] = Some(cpu);
                    topology.count += 1;
                }
            }
            offset += length;
        }
        Ok(topology)
    }

    pub const fn count(&self) -> usize {
        self.count
    }

    pub const fn truncated(&self) -> bool {
        self.truncated
    }

    pub fn get(&self, index: usize) -> Option<Cpu> {
        self.cpus.get(index).copied().flatten()
    }

    pub fn bsp(&self) -> Option<Cpu> {
        self.cpus[..self.count].iter().flatten().find(|cpu| cpu.bsp).copied()
    }

    pub fn usable(&self) -> impl Iterator<Item = Cpu> + '_ {
        self.cpus[..self.count].iter().flatten().copied().filter(|cpu| cpu.usable())
    }
}

#[derive(Clone, Copy)]
struct PciRoute {
    device: u8,
    pin: u8,
    gsi: u32,
    link: [u8; 4],
}

#[derive(Clone, Copy)]
struct Reset {
    port: u16,
    value: u8,
}

#[derive(Clone, Copy)]
struct Power {
    port: u16,
    value: u16,
}

impl Tables {
    pub const fn has_power(&self) -> bool {
        self.power.is_some()
    }

    pub fn pci_gsi(&self, bus: u8, device: u8, pin: u8) -> Option<u32> {
        (bus == 0).then(|| {
            self.routes
                .iter()
                .flatten()
                .find_map(|route| (route.device == device && route.pin == pin).then_some(route.gsi))
        })?
    }

    pub fn install_reset(&self) {
        let Some(reset) = self.reset else { return };
        RESET_PORT.store(reset.port, Ordering::Release);
        RESET_VALUE.store(reset.value, Ordering::Release);
        RESET_READY.store(true, Ordering::Release);
        if let Some(power) = self.power {
            POWER_PORT.store(power.port, Ordering::Release);
            POWER_VALUE.store(power.value, Ordering::Release);
            POWER_READY.store(true, Ordering::Release);
        }
    }
}

pub fn reset() -> bool {
    if !RESET_READY.load(Ordering::Acquire) {
        return false;
    }
    unsafe {
        asm!("out dx, al", in("dx") RESET_PORT.load(Ordering::Acquire), in("al") RESET_VALUE.load(Ordering::Acquire));
    }
    true
}

pub fn power_off() -> bool {
    if !POWER_READY.load(Ordering::Acquire) {
        return false;
    }
    unsafe {
        asm!("out dx, ax", in("dx") POWER_PORT.load(Ordering::Acquire), in("ax") POWER_VALUE.load(Ordering::Acquire));
    }
    true
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
        let mut routes = [None; ROUTES];
        let mut reset = None;
        let mut power = None;
        for index in 0..entries {
            let address = unsafe {
                (xsdt.cast::<u8>().add(core::mem::size_of::<Header>() + index * 8) as *const u64)
                    .read_unaligned()
            };
            let table = address as *const Header;
            if table_header(table, *b"APIC").is_some() {
                madt = parse_madt(table);
            } else if table_header(table, *b"FACP").is_some() {
                routes = parse_pci_routes(table);
                reset = parse_reset(table);
                power = parse_power(table);
            } else if table_header(table, *b"SSDT").is_some() {
                let ssdt_routes = parse_aml_routes(table);
                if ssdt_routes.iter().any(Option::is_some) {
                    routes = ssdt_routes;
                }
            }
        }
        Some(Tables { xsdt: rsdp.xsdt, madt, routes, reset, power })
    })
}

fn parse_reset(fadt: *const Header) -> Option<Reset> {
    let header = table_header(fadt, *b"FACP")?;
    (header.length >= 129).then_some(())?;
    let bytes = fadt.cast::<u8>();
    let space = unsafe { bytes.add(116).read() };
    let address = unsafe { bytes.add(120).cast::<u64>().read_unaligned() };
    let value = unsafe { bytes.add(128).read() };
    (space == 1).then_some(Reset { port: u16::try_from(address).ok()?, value })
}

fn parse_power(fadt: *const Header) -> Option<Power> {
    let header = table_header(fadt, *b"FACP")?;
    (header.length >= 90).then_some(())?;
    let bytes = fadt.cast::<u8>();
    let port = unsafe { bytes.add(64).cast::<u32>().read_unaligned() };
    let dsdt = unsafe { bytes.add(40).cast::<u32>().read_unaligned() } as *const Header;
    let dsdt_header = table_header(dsdt, *b"DSDT")?;
    let aml =
        unsafe { core::slice::from_raw_parts(dsdt.cast::<u8>(), dsdt_header.length as usize) };
    let body = &aml[core::mem::size_of::<Header>()..];
    let name = body.windows(4).position(|window| window == b"_S5_")?;
    let package = body.get(name + 4..)?.iter().position(|&byte| byte == 0x12)? + name + 4;
    let (length, package_header) = package_length(body.get(package + 1..)?)?;
    let values = body.get(package + 1 + package_header..package + 1 + package_header + length)?;
    let (sleep, used) = aml_integer(values.get(1..)?)?;
    let (sleep_b, _) = aml_integer(values.get(used + 1..)?)?;
    (sleep == sleep_b).then_some(Power {
        port: u16::try_from(port).ok()?,
        value: ((sleep as u16) << 10) | (1 << 13),
    })
}

fn parse_pci_routes(fadt: *const Header) -> [Option<PciRoute>; ROUTES] {
    let routes = [None; ROUTES];
    let Some(header) = table_header(fadt, *b"FACP") else {
        return routes;
    };
    if header.length < 44 {
        return routes;
    }
    let dsdt = unsafe { fadt.cast::<u8>().add(40).cast::<u32>().read_unaligned() } as *const Header;
    if table_header(dsdt, *b"DSDT").is_none() {
        return routes;
    }
    parse_aml_routes(dsdt)
}

fn parse_aml_routes(table: *const Header) -> [Option<PciRoute>; ROUTES] {
    let routes = [None; ROUTES];
    let Some(header) = table_header(table, *b"DSDT").or_else(|| table_header(table, *b"SSDT"))
    else {
        return routes;
    };
    let aml = unsafe { core::slice::from_raw_parts(table.cast(), header.length as usize) };
    let body = &aml[core::mem::size_of::<Header>()..];
    let Some(name) = body.windows(4).position(|window| window == b"_PRT") else {
        return routes;
    };
    for offset in name + 4..body.len() {
        if body[offset] != 0x12 {
            continue;
        }
        if let Some(found) = parse_route_package(&body[offset..]) {
            return resolve_links(body, found).unwrap_or([None; ROUTES]);
        }
    }
    if routes.iter().all(Option::is_none) {
        for returned in (name + 4..body.len().saturating_sub(1)).rev() {
            if body[returned] != 0xa4 {
                continue;
            }
            let Some((_, reference)) = aml_name(&body[returned + 1..]) else {
                continue;
            };
            for reference_offset in (1..body.len().saturating_sub(4)).rev() {
                if body[reference_offset - 1] != 0x08
                    || body.get(reference_offset..reference_offset + 4) != Some(&reference)
                {
                    continue;
                }
                for offset in reference_offset + 4..body.len() {
                    if body[offset] == 0x12
                        && let Some(found) = parse_route_package(&body[offset..])
                    {
                        return resolve_links(body, found).unwrap_or([None; ROUTES]);
                    }
                }
            }
        }
    }
    routes
}

fn resolve_links(
    aml: &[u8],
    mut routes: [Option<PciRoute>; ROUTES],
) -> Option<[Option<PciRoute>; ROUTES]> {
    for route in routes.iter_mut().flatten() {
        if route.link != [0; 4] {
            route.gsi = link_gsi(aml, route.link)?;
        }
    }
    Some(routes)
}

fn parse_route_package(bytes: &[u8]) -> Option<[Option<PciRoute>; ROUTES]> {
    let (length, header) = package_length(bytes.get(1..)?)?;
    let end = 1usize.checked_add(header)?.checked_add(length)?;
    let package = bytes.get(1 + header..end)?;
    let count = *package.first()? as usize;
    let mut routes = [None; ROUTES];
    let mut offset = 1;
    let mut found = 0;
    for _ in 0..count {
        let Some(entry) = package.get(offset..) else { break };
        let Some((entry_length, entry_header)) = package_length(entry.get(1..)?) else { break };
        if *entry.first()? != 0x12 {
            break;
        }
        let Some(entry_end) = entry_header.checked_add(entry_length) else { break };
        let Some(values) = entry.get(1 + entry_header + 1..entry_end) else { break };
        let Some(route) = parse_route(values) else { break };
        if found == routes.len() {
            break;
        }
        routes[found] = Some(route);
        found += 1;
        offset = offset.checked_add(entry_end)?;
    }
    (found > 0).then_some(routes)
}

fn parse_route(bytes: &[u8]) -> Option<PciRoute> {
    let (address, used) = aml_integer(bytes)?;
    let (pin, used_pin) = aml_integer(bytes.get(used..)?)?;
    let source_offset = used + used_pin;
    let source = *bytes.get(source_offset)?;
    let (source_end, link) = if source == 0 {
        (source_offset + 1, [0; 4])
    } else {
        aml_name(bytes.get(source_offset..)?)?
    };
    let gsi = if link == [0; 4] { aml_integer(bytes.get(source_end..)?)?.0 as u32 } else { 0 };
    Some(PciRoute { device: (address >> 16) as u8, pin: pin as u8, gsi, link })
}

fn aml_name(bytes: &[u8]) -> Option<(usize, [u8; 4])> {
    let mut offset = 0;
    while matches!(*bytes.get(offset)?, 0x5c | 0x5e) {
        offset += 1;
    }
    let segments = match *bytes.get(offset)? {
        0x2e => {
            offset += 1;
            2
        }
        0x2f => {
            let count = *bytes.get(offset + 1)? as usize;
            offset += 2;
            count
        }
        _ => 1,
    };
    let end = offset.checked_add(segments.checked_mul(4)?)?;
    let link: [u8; 4] = bytes.get(end.checked_sub(4)?..end)?.try_into().ok()?;
    Some((end, link))
}

fn link_gsi(aml: &[u8], link: [u8; 4]) -> Option<u32> {
    for offset in 0..aml.len().saturating_sub(9) {
        if aml.get(offset..offset + 4) != Some(&link) {
            continue;
        }
        let resource = &aml[offset + 4..aml.len().min(offset + 128)];
        for descriptor in resource.windows(9) {
            if descriptor[0] == 0x89 && descriptor[4] > 0 {
                return Some(u32::from_le_bytes([
                    descriptor[5],
                    descriptor[6],
                    descriptor[7],
                    descriptor[8],
                ]));
            }
        }
    }
    None
}

fn aml_integer(bytes: &[u8]) -> Option<(u64, usize)> {
    match *bytes.first()? {
        0x00 => Some((0, 1)),
        0x01 => Some((1, 1)),
        0xff => Some((u64::MAX, 1)),
        0x0a => Some((u64::from(*bytes.get(1)?), 2)),
        0x0b => Some((u64::from(u16::from_le_bytes([*bytes.get(1)?, *bytes.get(2)?])), 3)),
        0x0c => Some((
            u64::from(u32::from_le_bytes([
                *bytes.get(1)?,
                *bytes.get(2)?,
                *bytes.get(3)?,
                *bytes.get(4)?,
            ])),
            5,
        )),
        _ => None,
    }
}

fn package_length(bytes: &[u8]) -> Option<(usize, usize)> {
    let lead = *bytes.first()?;
    let extra = (lead >> 6) as usize;
    let mut length = (lead & 0x0f) as usize;
    for index in 0..extra {
        length |= (*bytes.get(index + 1)? as usize) << (4 + index * 8);
    }
    Some((length, extra + 1))
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
    let minimum = core::mem::size_of::<Header>() + 8;
    (header.length as usize >= minimum).then_some(())?;
    let bytes = table.cast::<u8>();
    let local_apic =
        unsafe { bytes.add(core::mem::size_of::<Header>()).cast::<u32>().read_unaligned() };
    let entries = unsafe {
        core::slice::from_raw_parts(bytes.add(minimum), header.length as usize - minimum)
    };
    let topology = CpuTopology::parse(entries, current_apic_id()).ok()?;
    let mut io_apic = 0;
    let mut io_apic_gsi_base = 0;
    let mut offset = 0;
    while offset + 2 <= entries.len() {
        let entry_offset = minimum + offset;
        let kind = unsafe { bytes.add(entry_offset).read() };
        let length = unsafe { bytes.add(entry_offset + 1).read() } as usize;
        if length < 2 || offset + length > entries.len() {
            return None;
        }
        if kind == 1 && length >= 12 && io_apic == 0 {
            io_apic =
                unsafe { bytes.add(entry_offset + 4).cast::<u32>().read_unaligned() as usize };
            io_apic_gsi_base =
                unsafe { bytes.add(entry_offset + 8).cast::<u32>().read_unaligned() };
        }
        offset += length;
    }
    (io_apic != 0).then_some(Madt {
        local_apic: local_apic as usize,
        io_apic,
        io_apic_gsi_base,
        topology,
    })
}

fn current_apic_id() -> Option<u32> {
    #[cfg(target_arch = "x86_64")]
    {
        Some(unsafe { core::arch::x86_64::__cpuid(1).ebx >> 24 })
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        None
    }
}

fn checksum(bytes: *const u8, length: usize) -> bool {
    let mut sum = 0u8;
    for index in 0..length {
        sum = sum.wrapping_add(unsafe { bytes.add(index).read() });
    }
    sum == 0
}

#[cfg(test)]
mod tests {
    use super::{CpuTopology, MAX_CPUS, TopologyError};

    #[test]
    fn parses_mixed_processor_records_and_marks_bsp() {
        let entries = [
            0, 8, 0, 2, 1, 0, 0, 0, // Local APIC 2, enabled.
            0, 8, 0, 3, 0, 0, 0, 0, // Local APIC 3, disabled.
            9, 16, 0, 0, 4, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0,
        ];
        let topology = CpuTopology::parse(&entries, Some(2)).unwrap();
        assert_eq!(topology.count(), 3);
        let first = topology.get(0).unwrap();
        assert_eq!(first.apic_id, 2);
        assert!(first.enabled);
        assert!(first.bsp);
        let second = topology.get(1).unwrap();
        assert_eq!(second.apic_id, 3);
        assert!(!second.enabled);
        assert!(!second.bsp);
        let third = topology.get(2).unwrap();
        assert_eq!(third.apic_id, 4);
        assert!(third.enabled);
        assert!(!third.bsp);
        assert_eq!(topology.bsp().unwrap().apic_id, first.apic_id);
        assert_eq!(topology.usable().count(), 2);
    }

    #[test]
    fn rejects_malformed_processor_entries() {
        assert_eq!(CpuTopology::parse(&[0, 7, 0, 2, 1, 0, 0], None), Err(TopologyError::Malformed));
        assert_eq!(CpuTopology::parse(&[9, 15, 0, 0], None), Err(TopologyError::Malformed));
    }

    #[test]
    fn truncates_excess_processors_without_growing() {
        let mut entries = [0u8; (MAX_CPUS + 1) * 8];
        for (index, entry) in entries.chunks_exact_mut(8).enumerate() {
            entry[0] = 0;
            entry[1] = 8;
            entry[3] = index as u8;
            entry[4] = 1;
        }
        let topology = CpuTopology::parse(&entries, None).unwrap();
        assert_eq!(topology.count(), MAX_CPUS);
        assert!(topology.truncated());
    }
}
