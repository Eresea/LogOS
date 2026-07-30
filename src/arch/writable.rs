use core::cell::UnsafeCell;

#[repr(transparent)]
pub struct Writable<T>(UnsafeCell<T>);

// ponytail: bootstrap CPU only; replace with per-CPU storage before SMP.
unsafe impl<T> Sync for Writable<T> {}

impl<T> Writable<T> {
    pub const fn new(value: T) -> Self {
        Self(UnsafeCell::new(value))
    }

    pub const fn get(&self) -> *mut T {
        self.0.get()
    }
}
