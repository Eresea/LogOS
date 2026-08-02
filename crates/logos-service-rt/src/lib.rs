#![no_std]

use core::{
    arch::asm,
    mem::MaybeUninit,
    ptr::NonNull,
    sync::atomic::{AtomicUsize, Ordering},
};

#[cfg(target_os = "uefi")]
use core::panic::PanicInfo;

use logos_abi::service as native_service;
pub use logos_abi::service::{
    BlockPage, Context as RawContext, Header, MAX_TEXT, NetworkPages, ProtocolVersion,
};

pub type EntryContext = *mut RawContext;
static ACTIVE_CONTEXT: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SharedPage {
    pub handle: logos_abi::PageHandle,
    pub address: u64,
}

pub const ACKNOWLEDGED: u32 = native_service::ACKNOWLEDGED;
pub const STORAGE_FORMATTED: u32 = native_service::STORAGE_FORMATTED;
pub const STORAGE_RECOVERED: u32 = native_service::STORAGE_RECOVERED;
pub const STORAGE_RECOVERED_INCOMPLETE: u32 = native_service::STORAGE_RECOVERED_INCOMPLETE;
pub const STORAGE_CORRUPT: u32 = native_service::STORAGE_CORRUPT;
pub const STORAGE_IO_FAILED: u32 = native_service::STORAGE_IO_FAILED;
pub const STORAGE_UNAVAILABLE: u32 = native_service::STORAGE_UNAVAILABLE;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BlockError {
    Invalid,
    Io,
    TimedOut,
    Corrupt,
    Full,
    NotFound,
}

#[derive(Clone, Copy)]
pub struct TextReply {
    pub text: [u8; MAX_TEXT],
    pub length: usize,
}

pub struct Context {
    raw: NonNull<RawContext>,
}

/// # Safety
/// The pointer must be the aligned, live context supplied by the kernel entry ABI.
pub fn entry(context: EntryContext, service: fn(&mut Context) -> !) -> ! {
    let Some(raw) = NonNull::new(context) else { spin() };
    if !raw.as_ptr().is_aligned() {
        spin();
    }
    ACTIVE_CONTEXT.store(raw.as_ptr() as usize, Ordering::Release);
    let mut context = Context { raw };
    service(&mut context)
}

impl Context {
    fn raw_address(&self) -> u64 {
        self.raw.as_ptr() as u64
    }

    fn raw(&self) -> &RawContext {
        // SAFETY: `entry` validates the pointer and the kernel owns the mapping for the task.
        unsafe { self.raw.as_ref() }
    }

    fn raw_mut(&mut self) -> &mut RawContext {
        // SAFETY: `entry` validates the pointer and this service is single-threaded.
        unsafe { self.raw.as_mut() }
    }

    fn invoke(&mut self, operation: u32) -> bool {
        self.raw_mut().operation = operation;
        #[cfg(target_arch = "x86_64")]
        unsafe {
            asm!("int 0x80", options(nostack, preserves_flags));
            true
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            false
        }
    }

    pub fn ready(&mut self) -> bool {
        self.invoke(native_service::READY)
    }

    pub fn wait_for_input(&mut self) -> bool {
        self.invoke(native_service::READ_INPUT)
    }

    pub fn complete(&mut self) -> bool {
        self.invoke(native_service::COMPLETE)
    }

    pub fn acknowledged(&self) -> bool {
        self.raw().status == ACKNOWLEDGED
    }

    pub fn input(&self) -> u32 {
        self.raw().input
    }

    pub fn input_byte(&self) -> Option<u8> {
        u8::try_from(self.input()).ok()
    }

    pub fn clear_display(&mut self) -> bool {
        self.invoke(native_service::CLEAR_DISPLAY)
    }

    pub fn present_text(
        &mut self,
        x: u32,
        y: u32,
        color: logos_abi::DisplayColor,
        text: &[u8],
    ) -> bool {
        if text.len() > MAX_TEXT {
            return false;
        }
        let raw = self.raw_mut();
        raw.text = [0; MAX_TEXT];
        raw.text[..text.len()].copy_from_slice(text);
        raw.x = x;
        raw.y = y;
        raw.color = color.wire();
        raw.text_length = text.len() as u32;
        self.invoke(native_service::PRESENT_TEXT)
    }

    pub fn syscall(&mut self, syscall: logos_abi::Syscall, argument: &[u8]) -> Option<TextReply> {
        if argument.len() > MAX_TEXT {
            return None;
        }
        let raw = self.raw_mut();
        raw.x = syscall as u32;
        raw.text = [0; MAX_TEXT];
        raw.text[..argument.len()].copy_from_slice(argument);
        raw.text_length = argument.len() as u32;
        if !self.invoke(native_service::SYSCALL) {
            return None;
        }
        let response = unsafe { RawContext::response_at(self.raw_address()) }?;
        Some(TextReply { text: response.text, length: response.length })
    }

    pub fn session_request(&self) -> Option<logos_abi::SessionRequest> {
        unsafe { RawContext::syscall_at(self.raw_address()) }
    }

    pub fn session_effect(&mut self, effect: logos_abi::Effect) -> Option<logos_abi::EffectResult> {
        self.raw_mut().x = effect as u32;
        if !self.invoke(native_service::SESSION_EFFECT) {
            return None;
        }
        logos_abi::EffectResult::from_wire(self.raw().x)
    }

    pub fn session_reply(&mut self, reply: &[u8]) -> bool {
        if reply.len() > MAX_TEXT {
            return false;
        }
        let raw = self.raw_mut();
        raw.text = [0; MAX_TEXT];
        raw.text[..reply.len()].copy_from_slice(reply);
        raw.text_length = reply.len() as u32;
        self.invoke(native_service::SESSION_REPLY)
    }

    pub fn store(&mut self, request: logos_abi::StoreRequest) -> Option<logos_abi::StoreReply> {
        if !request.valid()
            || unsafe { !RawContext::request_store_at(self.raw_address(), request) }
            || !self.invoke(native_service::STORE_REQUEST)
        {
            return None;
        }
        unsafe { RawContext::store_reply_at(self.raw_address(), request.id) }
    }

    pub fn store_request(&self) -> Option<logos_abi::StoreRequest> {
        unsafe { RawContext::store_at(self.raw_address()) }
    }

    pub fn store_reply(&mut self, reply: logos_abi::StoreReply) -> bool {
        let valid = unsafe { RawContext::reply_store_at(self.raw_address(), reply) };
        valid && self.invoke(native_service::STORE_REPLY)
    }

    pub fn shared_page(&self) -> Option<SharedPage> {
        let handle = unsafe { RawContext::shared_page_at(self.raw_address()) }?;
        let address = self.raw_address().checked_sub(logos_abi::PAGE_SIZE as u64)?;
        Some(SharedPage { handle, address })
    }

    pub fn block_client(&self) -> Option<BlockClient> {
        let page = unsafe { RawContext::block_page_at(self.raw_address()) }?;
        Some(BlockClient { context: self.raw_address(), page, next_id: 1 })
    }

    pub fn network_wait(&mut self, deadline: u64) -> bool {
        (unsafe { RawContext::network_wait_at(self.raw_address(), deadline) })
            && self.invoke(native_service::NETWORK_WAIT)
    }

    pub fn network_pages(&self) -> Option<NetworkPages> {
        unsafe { RawContext::network_pages_at(self.raw_address()) }
    }

    pub fn network_request(&self) -> Option<logos_abi::NetworkRequest> {
        unsafe { RawContext::network_at(self.raw_address()) }
    }

    pub fn request_network(&mut self, request: logos_abi::NetworkRequest) -> bool {
        (unsafe { RawContext::request_network_at(self.raw_address(), request) })
            && self.invoke(native_service::NETWORK_REQUEST)
    }

    pub fn network_reply(&mut self, reply: logos_abi::NetworkReply) -> bool {
        (unsafe { RawContext::reply_network_at(self.raw_address(), reply) })
            && self.invoke(native_service::NETWORK_REPLY)
    }

    pub fn network_device_request(&mut self, request: logos_abi::NetworkDeviceRequest) -> bool {
        (unsafe { RawContext::request_network_device_at(self.raw_address(), request) })
            && self.invoke(native_service::NETWORK_DEVICE_REQUEST)
    }

    pub fn network_device_reply(&self, expected_id: u32) -> Option<logos_abi::NetworkDeviceReply> {
        unsafe { RawContext::network_device_reply_at(self.raw_address(), expected_id) }
    }

    pub fn network_event(&self) -> Option<logos_abi::NetworkEvent> {
        unsafe { RawContext::network_event_at(self.raw_address()) }
    }

    pub fn storage_status(&self) -> Option<u32> {
        unsafe { RawContext::storage_status_at(self.raw_address()) }
    }

    pub fn set_storage_status(&mut self, status: u32) {
        self.raw_mut().x = status;
    }

    pub fn heap_slot<T>(&self) -> Option<&'static mut MaybeUninit<T>> {
        let address = (self.raw_address() as usize).checked_sub(5 * 4096)?;
        if !address.is_multiple_of(core::mem::align_of::<T>()) {
            return None;
        }
        // SAFETY: the kernel reserves the fixed storage area below the context mapping.
        Some(unsafe { &mut *(address as *mut MaybeUninit<T>) })
    }
}

pub struct BlockClient {
    context: u64,
    page: BlockPage,
    next_id: u32,
}

impl BlockClient {
    pub fn info(&mut self) -> Result<logos_abi::BlockInfo, BlockError> {
        self.request(logos_abi::BlockOperation::Info, 0, 0, false).map(|reply| reply.info)
    }

    pub fn read_sector(&mut self, sector: usize, output: &mut [u8; 512]) -> Result<(), BlockError> {
        self.request(logos_abi::BlockOperation::Read, sector as u64, 1, true)?;
        // SAFETY: the kernel provides the page as an owned, page-aligned service mapping.
        let page = unsafe { core::slice::from_raw_parts(self.page.address as *const u8, 4096) };
        output.copy_from_slice(&page[..512]);
        Ok(())
    }

    pub fn write_sector(&mut self, sector: usize, input: &[u8; 512]) -> Result<(), BlockError> {
        // SAFETY: the kernel provides the page as an owned, writable service mapping.
        let page = unsafe { core::slice::from_raw_parts_mut(self.page.address as *mut u8, 4096) };
        page.fill(0);
        page[..512].copy_from_slice(input);
        self.request(logos_abi::BlockOperation::Write, sector as u64, 1, true).map(|_| ())
    }

    pub fn flush(&mut self) -> Result<(), BlockError> {
        self.request(logos_abi::BlockOperation::Flush, 0, 0, false).map(|_| ())
    }

    pub fn probe(&mut self) -> bool {
        let mut sector = [0; 512];
        self.info().is_ok() && self.read_sector(0, &mut sector).is_ok() && self.flush().is_ok()
    }

    fn request(
        &mut self,
        operation: logos_abi::BlockOperation,
        lba: u64,
        blocks: u32,
        page: bool,
    ) -> Result<logos_abi::BlockReply, BlockError> {
        let id = self.next_id;
        self.next_id = id.checked_add(1).ok_or(BlockError::Full)?;
        let request = logos_abi::BlockRequest {
            id,
            operation,
            lba,
            blocks,
            page: if page { self.page.handle } else { logos_abi::PageHandle(0) },
            deadline: 1_000_000,
        };
        if !request.valid_shape() || unsafe { !RawContext::request_block_at(self.context, request) }
        {
            return Err(BlockError::Invalid);
        }
        #[cfg(target_arch = "x86_64")]
        unsafe {
            asm!("int 0x80", options(nostack, preserves_flags));
        }
        let reply =
            unsafe { RawContext::block_reply_at(self.context, id) }.ok_or(BlockError::Io)?;
        match reply.status {
            logos_abi::PersistenceStatus::Complete | logos_abi::PersistenceStatus::Recovered => {
                Ok(reply)
            }
            logos_abi::PersistenceStatus::TimedOut => Err(BlockError::TimedOut),
            logos_abi::PersistenceStatus::Corrupt => Err(BlockError::Corrupt),
            logos_abi::PersistenceStatus::Full => Err(BlockError::Full),
            logos_abi::PersistenceStatus::NotFound => Err(BlockError::NotFound),
            _ => Err(BlockError::Invalid),
        }
    }
}

fn spin() -> ! {
    loop {
        core::hint::spin_loop();
    }
}

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
    let address = ACTIVE_CONTEXT.load(Ordering::Acquire);
    if address != 0 {
        unsafe {
            let raw = address as *mut RawContext;
            (*raw).operation = native_service::PANIC;
            asm!("int 0x80", options(nostack, preserves_flags));
        }
    }
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
