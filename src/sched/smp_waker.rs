use core::{
    sync::atomic::{AtomicPtr, AtomicU32, AtomicUsize, Ordering, fence},
    task::{RawWaker, RawWakerVTable, Waker},
};

use super::{SmpScheduler, TaskHandle};

pub(super) struct WakerToken {
    references: AtomicUsize,
    scheduler: AtomicPtr<()>,
    handle: AtomicU32,
}

impl WakerToken {
    pub(super) const fn new() -> Self {
        Self {
            references: AtomicUsize::new(0),
            scheduler: AtomicPtr::new(core::ptr::null_mut()),
            handle: AtomicU32::new(0),
        }
    }

    pub(super) fn reserve(&self, scheduler: *mut (), handle: TaskHandle) -> bool {
        if self.references.compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire).is_err() {
            return false;
        }
        self.scheduler.store(scheduler, Ordering::Relaxed);
        self.handle.store(handle.raw(), Ordering::Relaxed);
        true
    }

    fn acquire(&self) {
        debug_assert!(self.references.load(Ordering::Acquire) != 0);
        self.references.fetch_add(1, Ordering::Relaxed);
    }

    pub(super) fn release(&self) {
        let previous = self.references.fetch_sub(1, Ordering::Release);
        debug_assert!(previous != 0);
        if previous == 1 {
            fence(Ordering::Acquire);
        }
    }
}

pub(super) fn from_token(token: &WakerToken) -> Waker {
    token.acquire();
    let raw = RawWaker::new(token as *const WakerToken as *const (), &WAKER_VTABLE);
    unsafe { Waker::from_raw(raw) }
}

unsafe fn clone(data: *const ()) -> RawWaker {
    let token = unsafe { &*(data as *const WakerToken) };
    token.acquire();
    RawWaker::new(data, &WAKER_VTABLE)
}

unsafe fn wake(data: *const ()) {
    let token = unsafe { &*(data as *const WakerToken) };
    dispatch(token);
    token.release();
}

unsafe fn wake_by_ref(data: *const ()) {
    let token = unsafe { &*(data as *const WakerToken) };
    dispatch(token);
}

unsafe fn drop(data: *const ()) {
    let token = unsafe { &*(data as *const WakerToken) };
    token.release();
}

fn dispatch(token: &WakerToken) {
    let scheduler = token.scheduler.load(Ordering::Acquire);
    if !scheduler.is_null() {
        let scheduler = scheduler.cast::<SmpScheduler<'static>>();
        let handle = TaskHandle::from_raw(token.handle.load(Ordering::Acquire));
        unsafe { (&*scheduler).wake(handle) };
    }
}

static WAKER_VTABLE: RawWakerVTable = RawWakerVTable::new(clone, wake, wake_by_ref, drop);
