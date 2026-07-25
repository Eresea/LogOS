pub struct Service {
    framebuffer: *mut u8,
    framebuffer_size: usize,
    width: usize,
    height: usize,
    stride: usize,
}

impl Service {
    pub fn new(
        framebuffer: *mut u8,
        framebuffer_size: usize,
        width: usize,
        height: usize,
        stride: usize,
    ) -> Option<Self> {
        (!framebuffer.is_null()
            && width > 0
            && height > 0
            && stride >= width
            && stride.checked_mul(height)?.checked_mul(4)? <= framebuffer_size)
            .then_some(Self { framebuffer, framebuffer_size, width, height, stride })
    }

    pub fn present(&mut self, x: usize, y: usize, color: [u8; 3]) -> bool {
        let Some(offset) = y
            .checked_mul(self.stride)
            .and_then(|row| row.checked_add(x))
            .and_then(|pixel| pixel.checked_mul(4))
        else {
            return false;
        };
        let Some(end) = offset.checked_add(3) else {
            return false;
        };
        if x >= self.width || y >= self.height || end >= self.framebuffer_size {
            return false;
        }
        let pixel = unsafe { self.framebuffer.add(offset) };
        unsafe {
            pixel.write_volatile(color[0]);
            pixel.add(1).write_volatile(color[1]);
            pixel.add(2).write_volatile(color[2]);
        }
        true
    }

    pub const fn dimensions(&self) -> (usize, usize) {
        (self.width, self.height)
    }

    pub fn self_check() -> bool {
        Self::new(core::ptr::dangling_mut(), 4, 1, 1, 1).is_some()
            && Self::new(core::ptr::dangling_mut(), 3, 1, 1, 1).is_none()
    }
}
