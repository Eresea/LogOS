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

pub fn present(context: u64) -> bool {
    let Some((x, y, color)) = (unsafe { logos_core::native_service::Context::pixel_at(context) })
    else {
        return false;
    };
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
        pixel.write_volatile(color[0]);
        pixel.add(1).write_volatile(color[1]);
        pixel.add(2).write_volatile(color[2]);
        pixel.add(3).write_volatile(0);
    }
    true
}

pub fn present_text(context: u64) -> bool {
    let Some(request) = (unsafe { logos_core::native_service::Context::text_at(context) }) else {
        return false;
    };
    request.text[..request.length].iter().enumerate().all(|(index, &byte)| {
        let Some(x) = usize::try_from(request.x).ok().and_then(|x| x.checked_add(index * 6)) else {
            return false;
        };
        let Some(y) = usize::try_from(request.y).ok() else {
            return false;
        };
        crate::glyph(byte).is_some_and(|glyph| {
            glyph.iter().enumerate().all(|(row, bits)| {
                (0..5).all(|column| {
                    bits & (1 << (4 - column)) == 0
                        || write_pixel(x + column, y + row, request.color)
                })
            })
        })
    })
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
