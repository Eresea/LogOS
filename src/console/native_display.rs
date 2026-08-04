use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

static FRAMEBUFFER: AtomicU64 = AtomicU64::new(0);
static FRAMEBUFFER_SIZE: AtomicUsize = AtomicUsize::new(0);
static WIDTH: AtomicUsize = AtomicUsize::new(0);
static HEIGHT: AtomicUsize = AtomicUsize::new(0);
static STRIDE: AtomicUsize = AtomicUsize::new(0);

pub fn install(
    framebuffer: *mut u8,
    framebuffer_size: usize,
    width: usize,
    height: usize,
    stride: usize,
) -> bool {
    if framebuffer.is_null()
        || width == 0
        || height == 0
        || stride < width
        || stride
            .checked_mul(height)
            .and_then(|pixels| pixels.checked_mul(4))
            .is_none_or(|size| size > framebuffer_size)
    {
        return false;
    }
    FRAMEBUFFER.store(framebuffer as u64, Ordering::Release);
    FRAMEBUFFER_SIZE.store(framebuffer_size, Ordering::Release);
    WIDTH.store(width, Ordering::Release);
    HEIGHT.store(height, Ordering::Release);
    STRIDE.store(stride, Ordering::Release);
    true
}

pub fn present(page: u64, generation: u32) -> bool {
    let Some(request) =
        (unsafe { logos_core::native_service::DisplayPage::request_at(page, generation) })
    else {
        return false;
    };
    if request.operation != logos_abi::service::PRESENT_PIXEL {
        return false;
    }
    let (x, y, color) = (request.x, request.y, request.color);
    let (x, y) = (x as usize, y as usize);
    let Some(offset) = y
        .checked_mul(STRIDE.load(Ordering::Acquire))
        .and_then(|row| row.checked_add(x))
        .and_then(|pixel| pixel.checked_mul(4))
    else {
        return false;
    };
    if x >= WIDTH.load(Ordering::Acquire)
        || y >= HEIGHT.load(Ordering::Acquire)
        || offset.checked_add(3).is_none_or(|end| end >= FRAMEBUFFER_SIZE.load(Ordering::Acquire))
    {
        return false;
    }
    let framebuffer = FRAMEBUFFER.load(Ordering::Acquire) as *mut u8;
    if framebuffer.is_null() {
        return false;
    }
    unsafe {
        let pixel = framebuffer.add(offset);
        let color = color.rgb();
        pixel.write_volatile(color[0]);
        pixel.add(1).write_volatile(color[1]);
        pixel.add(2).write_volatile(color[2]);
        pixel.add(3).write_volatile(0);
    }
    unsafe { logos_core::native_service::DisplayPage::complete_at(page, generation) }
}

pub fn present_text(page: u64, generation: u32) -> bool {
    let Some(request) =
        (unsafe { logos_core::native_service::DisplayPage::request_at(page, generation) })
    else {
        return false;
    };
    if request.operation != logos_abi::service::PRESENT_TEXT {
        return false;
    }
    let rendered = request.text[..request.length].iter().enumerate().all(|(index, &byte)| {
        let Some(x) = usize::try_from(request.x).ok().and_then(|x| x.checked_add(index * 8)) else {
            return false;
        };
        let Some(y) = usize::try_from(request.y).ok() else {
            return false;
        };
        logos_terminal::text::glyph(byte).iter().enumerate().all(|(row, bits)| {
            (0..8).all(|column| {
                bits & (1 << (7 - column)) == 0
                    || write_pixel(x + column, y + row, request.color.rgb())
            })
        })
    });
    rendered && unsafe { logos_core::native_service::DisplayPage::complete_at(page, generation) }
}

pub fn clear(page: u64, generation: u32) -> bool {
    let Some(request) =
        (unsafe { logos_core::native_service::DisplayPage::request_at(page, generation) })
    else {
        return false;
    };
    if request.operation != logos_abi::service::CLEAR_DISPLAY {
        return false;
    }
    let framebuffer = FRAMEBUFFER.load(Ordering::Acquire) as *mut u8;
    if framebuffer.is_null() {
        return false;
    }
    unsafe { core::ptr::write_bytes(framebuffer, 0, FRAMEBUFFER_SIZE.load(Ordering::Acquire)) };
    unsafe { logos_core::native_service::DisplayPage::complete_at(page, generation) }
}

pub fn handle(page: u64, generation: u32) -> bool {
    let Some(operation) =
        (unsafe { logos_core::native_service::DisplayPage::request_at(page, generation) })
            .map(|request| request.operation)
    else {
        return false;
    };
    match operation {
        logos_abi::service::PRESENT_PIXEL => present(page, generation),
        logos_abi::service::PRESENT_TEXT => present_text(page, generation),
        logos_abi::service::CLEAR_DISPLAY => clear(page, generation),
        _ => false,
    }
}

pub fn matches(x: usize, y: usize, color: [u8; 3]) -> bool {
    if x >= WIDTH.load(Ordering::Acquire) || y >= HEIGHT.load(Ordering::Acquire) {
        return false;
    }
    let Some(offset) = y
        .checked_mul(STRIDE.load(Ordering::Acquire))
        .and_then(|row| row.checked_add(x))
        .and_then(|pixel| pixel.checked_mul(4))
    else {
        return false;
    };
    let framebuffer = FRAMEBUFFER.load(Ordering::Acquire) as *const u8;
    !framebuffer.is_null()
        && unsafe {
            let pixel = framebuffer.add(offset);
            [pixel.read_volatile(), pixel.add(1).read_volatile(), pixel.add(2).read_volatile()]
                == color
        }
}

fn write_pixel(x: usize, y: usize, color: [u8; 3]) -> bool {
    let Some(offset) = y
        .checked_mul(STRIDE.load(Ordering::Acquire))
        .and_then(|row| row.checked_add(x))
        .and_then(|pixel| pixel.checked_mul(4))
    else {
        return false;
    };
    if x >= WIDTH.load(Ordering::Acquire)
        || y >= HEIGHT.load(Ordering::Acquire)
        || offset.checked_add(3).is_none_or(|end| end >= FRAMEBUFFER_SIZE.load(Ordering::Acquire))
    {
        return false;
    }
    let framebuffer = FRAMEBUFFER.load(Ordering::Acquire) as *mut u8;
    if framebuffer.is_null() {
        return false;
    }
    unsafe {
        let pixel = framebuffer.add(offset);
        pixel.write_volatile(color[0]);
        pixel.add(1).write_volatile(color[1]);
        pixel.add(2).write_volatile(color[2]);
        pixel.add(3).write_volatile(0);
    }
    true
}
