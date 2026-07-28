use core::{cell::UnsafeCell, mem::size_of, slice};
use logos_core::native_service::Header;
use uefi::{
    boot::{self, LoadImageSource},
    cstr16,
    proto::{
        loaded_image::LoadedImage,
        media::file::{File, FileAttribute, FileMode},
    },
};

const MAX_PAYLOAD: usize = 512 * 1024;
const NAME: &[u8] = b"terminal";

struct Buffer(UnsafeCell<[u8; MAX_PAYLOAD]>);
unsafe impl Sync for Buffer {}
static PAYLOAD: Buffer = Buffer(UnsafeCell::new([0; MAX_PAYLOAD]));

#[derive(Clone, Copy)]
pub struct Payload {
    base: *const u8,
    image: crate::pe::Image,
    entry_rva: u32,
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
            let Some(header) = relocations.get(block..block + 8) else {
                return false;
            };
            let target_page = u32::from_le_bytes(header[..4].try_into().unwrap());
            let block_size = u32::from_le_bytes(header[4..].try_into().unwrap()) as usize;
            let Some(entries) = relocations.get(block + 8..block + block_size) else {
                return false;
            };
            if block_size < 8 || !block_size.is_multiple_of(2) {
                return false;
            }
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
                let offset = usize::try_from(target_rva - page_rva).unwrap();
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

pub fn stage() -> Option<Payload> {
    let Ok(mut file_system) = boot::get_image_file_system(boot::image_handle()) else {
        return None;
    };
    let Ok(mut root) = file_system.open_volume() else {
        return None;
    };
    let mut file = root
        .open(cstr16!(r"\EFI\LOGOS\TERMINAL.EFI"), FileMode::Read, FileAttribute::empty())
        .ok()
        .and_then(|file| file.into_regular_file())?;
    let buffer = unsafe { &mut *PAYLOAD.0.get() };
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
        header.valid_for(NAME).then_some(header)
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
