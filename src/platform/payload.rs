use core::{cell::UnsafeCell, mem::size_of, slice};
use logos_core::native_service::Header;
use uefi::{
    boot::{self, LoadImageSource},
    cstr16,
    proto::{
        loaded_image::LoadedImage,
        media::file::{File, FileAttribute, FileMode, RegularFile},
    },
};

const MAX_PAYLOAD: usize = 512 * 1024;
struct Buffer(UnsafeCell<[u8; MAX_PAYLOAD]>);
unsafe impl Sync for Buffer {}
static TERMINAL_PAYLOAD: Buffer = Buffer(UnsafeCell::new([0; MAX_PAYLOAD]));
static SESSIONS_PAYLOAD: Buffer = Buffer(UnsafeCell::new([0; MAX_PAYLOAD]));
static STORAGE_PAYLOAD: Buffer = Buffer(UnsafeCell::new([0; MAX_PAYLOAD]));

#[derive(Clone, Copy)]
pub struct Payload {
    base: *const u8,
    image: crate::pe::Image,
    entry_rva: u32,
}

#[derive(Clone, Copy)]
pub struct Payloads {
    pub terminal: Payload,
    pub sessions: Payload,
    pub storage: Payload,
}

impl Payload {
    pub fn entry_rva(self) -> u32 {
        self.entry_rva
    }

    pub fn sections(self) -> impl Iterator<Item = crate::pe::Section> {
        self.image.sections()
    }

    pub fn copy_page(self, rva: u32, destination: u64, mapped_base: u64) -> bool {
        let Ok(rva) = usize::try_from(rva) else {
            return false;
        };
        let Ok(image_size) = usize::try_from(self.image.image_size()) else {
            return false;
        };
        let Some(remaining) = image_size.checked_sub(rva) else {
            return false;
        };
        unsafe {
            core::ptr::write_bytes(destination as *mut u8, 0, 4096);
            core::ptr::copy_nonoverlapping(
                self.base.add(rva),
                destination as *mut u8,
                remaining.min(4096),
            );
        }
        self.relocate_page(rva as u32, destination, mapped_base)
    }

    fn relocate_page(self, page_rva: u32, destination: u64, mapped_base: u64) -> bool {
        let Some((rva, size)) = self.image.relocations() else {
            return mapped_base == self.base as u64;
        };
        let Ok(rva) = usize::try_from(rva) else {
            return false;
        };
        let Ok(size) = usize::try_from(size) else {
            return false;
        };
        let relocations = unsafe { slice::from_raw_parts(self.base.add(rva), size) };
        let mut block = 0;
        // ponytail: bounded native images are tiny; index relocations if payloads grow.
        while block < relocations.len() {
            let Some((target_page, entries, block_size)) = relocation_block(relocations, block)
            else {
                return false;
            };
            for entry in entries.chunks_exact(2) {
                let entry = u16::from_le_bytes([entry[0], entry[1]]);
                if entry >> 12 == 0 {
                    continue;
                }
                if entry >> 12 != 10 {
                    return false;
                }
                let Some(target_rva) = target_page.checked_add(u32::from(entry & 0x0fff)) else {
                    return false;
                };
                if target_rva / 4096 != page_rva / 4096 {
                    continue;
                }
                let Ok(offset) = usize::try_from(target_rva - page_rva) else {
                    return false;
                };
                if offset > 4096 - size_of::<u64>() {
                    return false;
                }
                unsafe {
                    let target = (destination as *mut u8).add(offset).cast::<u64>();
                    target.write_unaligned(
                        target
                            .read_unaligned()
                            .wrapping_add(mapped_base.wrapping_sub(self.base as u64)),
                    );
                }
            }
            block += block_size;
        }
        true
    }
}

fn relocation_block(bytes: &[u8], offset: usize) -> Option<(u32, &[u8], usize)> {
    let header = bytes.get(offset..offset.checked_add(8)?)?;
    let target_page = u32::from_le_bytes([header[0], header[1], header[2], header[3]]);
    let size =
        usize::try_from(u32::from_le_bytes([header[4], header[5], header[6], header[7]])).ok()?;
    if size < 8 || !size.is_multiple_of(2) {
        return None;
    }
    let end = offset.checked_add(size)?;
    Some((target_page, bytes.get(offset + 8..end)?, size))
}

pub fn relocation_self_check() -> bool {
    let valid = [0, 16, 0, 0, 10, 0, 0, 0, 0, 0];
    let short = [0, 0, 0, 0, 6, 0, 0, 0];
    let odd = [0, 0, 0, 0, 9, 0, 0, 0, 0];
    relocation_block(&valid, 0).is_some()
        && relocation_block(&valid[..9], 0).is_none()
        && relocation_block(&short, 0).is_none()
        && relocation_block(&odd, 0).is_none()
}

pub fn stage() -> Option<Payloads> {
    let Ok(mut file_system) = boot::get_image_file_system(boot::image_handle()) else {
        return None;
    };
    let Ok(mut root) = file_system.open_volume() else {
        return None;
    };
    let mut terminal = root
        .open(cstr16!(r"\EFI\LOGOS\TERMINAL.EFI"), FileMode::Read, FileAttribute::empty())
        .ok()
        .and_then(|file| file.into_regular_file())?;
    let terminal_buffer = unsafe { &mut *TERMINAL_PAYLOAD.0.get() };
    let terminal = load(&mut terminal, terminal_buffer, b"terminal")?;
    let mut sessions = root
        .open(cstr16!(r"\EFI\LOGOS\SESSIONS.EFI"), FileMode::Read, FileAttribute::empty())
        .ok()
        .and_then(|file| file.into_regular_file())?;
    let sessions_buffer = unsafe { &mut *SESSIONS_PAYLOAD.0.get() };
    let sessions = load(&mut sessions, sessions_buffer, b"sessions")?;
    let mut storage = root
        .open(cstr16!(r"\EFI\LOGOS\STORAGE.EFI"), FileMode::Read, FileAttribute::empty())
        .ok()
        .and_then(|file| file.into_regular_file())?;
    let storage_buffer = unsafe { &mut *STORAGE_PAYLOAD.0.get() };
    let storage = load(&mut storage, storage_buffer, b"storage")?;
    Some(Payloads { terminal, sessions, storage })
}

fn load(file: &mut RegularFile, buffer: &mut [u8; MAX_PAYLOAD], name: &[u8]) -> Option<Payload> {
    let Ok(length) = file.read(buffer) else {
        return None;
    };
    if length == 0 || length == buffer.len() {
        return None;
    }
    let Ok(image) = boot::load_image(
        boot::image_handle(),
        LoadImageSource::FromBuffer { buffer: &buffer[..length], file_path: None },
    ) else {
        return None;
    };
    let Ok(loaded) = boot::open_protocol_exclusive::<LoadedImage>(image) else {
        return None;
    };
    let (base, size) = loaded.info();
    let Ok(size) = usize::try_from(size) else {
        return None;
    };
    if base.is_null() || size < core::mem::size_of::<Header>() {
        return None;
    }
    let image = unsafe { slice::from_raw_parts(base.cast::<u8>(), size) };
    let metadata = crate::pe::Image::parse(image)?;
    let Ok(image_size) = usize::try_from(metadata.image_size()) else {
        return None;
    };
    let header = image.windows(core::mem::size_of::<Header>()).find_map(|bytes| {
        let header = unsafe { (bytes.as_ptr().cast::<Header>()).read_unaligned() };
        header.valid_for(name).then_some(header)
    })?;
    let entry = header.entry_address();
    let base_address = base as usize;
    let entry_rva = entry.checked_sub(base_address).and_then(|rva| u32::try_from(rva).ok())?;
    (metadata.entry_rva() != 0
        && image_size <= size
        && metadata.executable_rva(entry_rva)
        && metadata.sections().all(|section| {
            section
                .address
                .checked_add(section.size)
                .is_some_and(|end| end <= metadata.image_size())
        }))
    .then_some(Payload { base: base.cast(), image: metadata, entry_rva })
}
