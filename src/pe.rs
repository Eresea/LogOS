const PE32_PLUS: u16 = 0x20b;
const EXECUTABLE: u32 = 0x2000_0000;
const MAX_SECTIONS: usize = 8;

#[derive(Clone, Copy)]
pub struct Image {
    entry_rva: u32,
    image_size: u32,
    sections: [Option<Section>; MAX_SECTIONS],
    section_count: usize,
}

#[derive(Clone, Copy)]
pub struct Section {
    pub address: u32,
    pub size: u32,
    pub executable: bool,
    pub writable: bool,
}

impl Image {
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        if bytes.get(..2)? != b"MZ" {
            return None;
        }
        let pe = usize::try_from(read_u32(bytes, 0x3c)?).ok()?;
        if bytes.get(pe..pe.checked_add(4)?)? != b"PE\0\0" {
            return None;
        }
        let file = pe.checked_add(4)?;
        let section_count = usize::from(read_u16(bytes, file.checked_add(2)?)?);
        let optional_size = usize::from(read_u16(bytes, file.checked_add(16)?)?);
        if section_count == 0 || section_count > MAX_SECTIONS {
            return None;
        }
        let optional = file.checked_add(20)?;
        if read_u16(bytes, optional)? != PE32_PLUS {
            return None;
        }
        let entry_rva = read_u32(bytes, optional.checked_add(16)?)?;
        let image_size = read_u32(bytes, optional.checked_add(56)?)?;
        if image_size == 0
            || usize::try_from(image_size).ok()? > bytes.len()
            || entry_rva >= image_size
        {
            return None;
        }
        let table = optional.checked_add(optional_size)?;
        let mut sections = [None; MAX_SECTIONS];
        let mut entry_executable = false;
        for (index, slot) in sections.iter_mut().enumerate().take(section_count) {
            let offset = table.checked_add(index.checked_mul(40)?)?;
            let virtual_size = read_u32(bytes, offset.checked_add(8)?)?;
            let address = read_u32(bytes, offset.checked_add(12)?)?;
            let raw_size = read_u32(bytes, offset.checked_add(16)?)?;
            let characteristics = read_u32(bytes, offset.checked_add(36)?)?;
            let size = virtual_size.max(raw_size);
            let end = address.checked_add(size)?;
            if size == 0 || end > image_size {
                return None;
            }
            let executable = characteristics & EXECUTABLE != 0;
            entry_executable |= executable && entry_rva >= address && entry_rva < end;
            *slot = Some(Section {
                address,
                size,
                executable,
                writable: characteristics & 0x8000_0000 != 0,
            });
        }
        entry_executable.then_some(Self { entry_rva, image_size, sections, section_count })
    }

    pub const fn entry_rva(&self) -> u32 {
        self.entry_rva
    }

    pub const fn image_size(&self) -> u32 {
        self.image_size
    }

    pub fn sections(self) -> impl Iterator<Item = Section> {
        self.sections.into_iter().take(self.section_count).flatten()
    }
}

fn read_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    let bytes = bytes.get(offset..offset.checked_add(2)?)?;
    Some(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    let bytes = bytes.get(offset..offset.checked_add(4)?)?;
    Some(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

pub fn self_check() -> bool {
    let mut bytes = [0u8; 0x400];
    bytes[..2].copy_from_slice(b"MZ");
    bytes[0x3c..0x40].copy_from_slice(&(0x80u32).to_le_bytes());
    bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
    bytes[0x86..0x88].copy_from_slice(&(1u16).to_le_bytes());
    bytes[0x94..0x96].copy_from_slice(&(0xf0u16).to_le_bytes());
    bytes[0x98..0x9a].copy_from_slice(&PE32_PLUS.to_le_bytes());
    bytes[0xa8..0xac].copy_from_slice(&(0x200u32).to_le_bytes());
    bytes[0xd0..0xd4].copy_from_slice(&(0x400u32).to_le_bytes());
    let section = 0x188;
    bytes[section + 8..section + 12].copy_from_slice(&(0x100u32).to_le_bytes());
    bytes[section + 12..section + 16].copy_from_slice(&(0x200u32).to_le_bytes());
    bytes[section + 36..section + 40].copy_from_slice(&EXECUTABLE.to_le_bytes());
    let valid = Image::parse(&bytes).is_some_and(|image| {
        image.entry_rva() == 0x200
            && image.image_size() == 0x400
            && image
                .sections()
                .next()
                .is_some_and(|section| section.executable && !section.writable)
    });
    bytes[section + 36..section + 40].fill(0);
    valid && Image::parse(&bytes).is_none()
}
