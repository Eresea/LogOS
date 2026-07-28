use core::{cell::UnsafeCell, slice};
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
}

impl Payload {
    pub fn entry_rva(self) -> u32 {
        self.image.entry_rva()
    }

    pub fn sections(self) -> impl Iterator<Item = crate::pe::Section> {
        self.image.sections()
    }

    pub fn copy_page(self, rva: u32, destination: u64) -> bool {
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
    (metadata.entry_rva() != 0
        && image_size <= size
        && metadata.sections().all(|section| {
            section
                .address
                .checked_add(section.size)
                .is_some_and(|end| end <= metadata.image_size())
        })
        && image.windows(core::mem::size_of::<Header>()).any(|bytes| {
            let header = unsafe { (bytes.as_ptr().cast::<Header>()).read_unaligned() };
            header.valid_for(NAME)
        }))
    .then_some(Payload { base: base.cast(), image: metadata })
}
