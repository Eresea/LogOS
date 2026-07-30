#![no_std]

#[cfg(target_os = "uefi")]
use core::panic::PanicInfo;

pub mod heap {
    use core::{
        alloc::{GlobalAlloc, Layout},
        ptr,
        sync::atomic::{AtomicUsize, Ordering},
    };

    pub struct PageArena {
        start: AtomicUsize,
        end: AtomicUsize,
        next: AtomicUsize,
    }

    impl PageArena {
        pub const fn new() -> Self {
            Self { start: AtomicUsize::new(0), end: AtomicUsize::new(0), next: AtomicUsize::new(0) }
        }

        /// # Safety
        /// The range must be exclusively owned, writable service memory.
        pub unsafe fn initialize(&self, start: usize, bytes: usize) -> bool {
            let Some(end) = start.checked_add(bytes) else {
                return false;
            };
            if start == 0
                || bytes < logos_abi::PAGE_SIZE
                || !start.is_multiple_of(logos_abi::PAGE_SIZE)
            {
                return false;
            }
            self.start.store(start, Ordering::Release);
            self.end.store(end, Ordering::Release);
            self.next.store(start, Ordering::Release);
            true
        }
    }

    unsafe impl GlobalAlloc for PageArena {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            let end = self.end.load(Ordering::Acquire);
            let result = self.next.fetch_update(Ordering::AcqRel, Ordering::Acquire, |next| {
                let aligned = next.checked_add(layout.align() - 1)? & !(layout.align() - 1);
                let next = aligned.checked_add(layout.size())?;
                (next <= end).then_some(next)
            });
            match result {
                Ok(previous) => {
                    let aligned = (previous + layout.align() - 1) & !(layout.align() - 1);
                    aligned as *mut u8
                }
                Err(_) => ptr::null_mut(),
            }
        }

        unsafe fn dealloc(&self, _pointer: *mut u8, _layout: Layout) {
            // ponytail: service heaps reset wholesale; add a free list when long-lived churn proves it.
        }
    }

    impl Default for PageArena {
        fn default() -> Self {
            Self::new()
        }
    }
}

#[cfg(target_os = "uefi")]
#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}

#[unsafe(export_name = "efi_main")]
pub extern "win64" fn efi_main(
    _image_handle: *const core::ffi::c_void,
    _system_table: *const core::ffi::c_void,
) -> usize {
    0
}
