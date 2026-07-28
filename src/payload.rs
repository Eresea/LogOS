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

pub fn stage() -> bool {
    let Ok(mut file_system) = boot::get_image_file_system(boot::image_handle()) else {
        return false;
    };
    let Ok(mut root) = file_system.open_volume() else {
        return false;
    };
    let Some(mut file) = root
        .open(cstr16!(r"\EFI\LOGOS\TERMINAL.EFI"), FileMode::Read, FileAttribute::empty())
        .ok()
        .and_then(|file| file.into_regular_file())
    else {
        return false;
    };
    let buffer = unsafe { &mut *PAYLOAD.0.get() };
    let Ok(length) = file.read(buffer) else {
        return false;
    };
    if length == 0 || length == buffer.len() {
        return false;
    }
    let Ok(image) = boot::load_image(
        boot::image_handle(),
        LoadImageSource::FromBuffer { buffer: &buffer[..length], file_path: None },
    ) else {
        return false;
    };
    let Ok(loaded) = boot::open_protocol_exclusive::<LoadedImage>(image) else {
        return false;
    };
    let (base, size) = loaded.info();
    let Ok(size) = usize::try_from(size) else {
        return false;
    };
    if base.is_null() || size < core::mem::size_of::<Header>() {
        return false;
    }
    let image = unsafe { slice::from_raw_parts(base.cast::<u8>(), size) };
    image.windows(core::mem::size_of::<Header>()).any(|bytes| {
        let header = unsafe { (bytes.as_ptr().cast::<Header>()).read_unaligned() };
        header.valid_for(NAME)
    })
}
