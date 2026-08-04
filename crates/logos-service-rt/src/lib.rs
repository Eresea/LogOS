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
    BlockPage, ControlPage, DisplayPage, EffectPage, Header, InputPage, MAX_TEXT, NetworkPages,
    ProtocolVersion, SessionClientPage, SessionServerPage, SessionStatus,
};

pub type EntryControlPage = *mut ControlPage;
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

#[cfg(target_arch = "x86_64")]
pub fn debug(message: &[u8]) {
    for &byte in message {
        unsafe { core::arch::asm!("out dx, al", in("dx") 0xe9u16, in("al") byte) };
    }
}

#[cfg(not(target_arch = "x86_64"))]
pub fn debug(_: &[u8]) {}

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

pub struct ServiceContext {
    raw: NonNull<ControlPage>,
    next_session_id: u32,
}

/// # Safety
/// The pointer must be the aligned, live context supplied by the kernel entry ABI.
pub fn entry(context: EntryControlPage, service: fn(&mut ServiceContext) -> !) -> ! {
    let Some(raw) = NonNull::new(context) else { spin() };
    if !raw.as_ptr().is_aligned() {
        spin();
    }
    ACTIVE_CONTEXT.store(raw.as_ptr() as usize, Ordering::Release);
    let mut context = ServiceContext { raw, next_session_id: 1 };
    service(&mut context)
}

impl ServiceContext {
    fn raw_address(&self) -> u64 {
        self.raw.as_ptr() as u64
    }

    fn endpoint_page(&self, operation: u32) -> Option<(u64, u32)> {
        let raw = self.raw();
        let page = match operation {
            native_service::READ_INPUT => raw.input_page,
            native_service::PRESENT_PIXEL
            | native_service::PRESENT_TEXT
            | native_service::CLEAR_DISPLAY => raw.display_page,
            _ => 0,
        };
        (page != 0 && raw.generation != 0).then_some((page, raw.generation))
    }

    fn session_client_page(&self) -> Option<(u64, u32)> {
        let raw = self.raw();
        (raw.session_client_page != 0 && raw.generation != 0)
            .then_some((raw.session_client_page, raw.generation))
    }

    fn session_server_page(&self) -> Option<(u64, u32)> {
        let raw = self.raw();
        (raw.session_server_page != 0 && raw.generation != 0)
            .then_some((raw.session_server_page, raw.generation))
    }

    fn effect_page(&self) -> Option<(u64, u32)> {
        let raw = self.raw();
        (raw.effect_page != 0 && raw.generation != 0).then_some((raw.effect_page, raw.generation))
    }

    fn raw(&self) -> &ControlPage {
        // SAFETY: `entry` validates the pointer and the kernel owns the mapping for the task.
        unsafe { self.raw.as_ref() }
    }

    fn raw_mut(&mut self) -> &mut ControlPage {
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
        let Some((page, generation)) = self.endpoint_page(native_service::READ_INPUT) else {
            return false;
        };
        (unsafe { InputPage::wait_at(page, generation) }) && self.invoke(native_service::READ_INPUT)
    }

    pub fn wait_for_request(&mut self) -> bool {
        self.invoke(native_service::READ_INPUT)
    }

    pub fn complete(&mut self) -> bool {
        self.invoke(native_service::COMPLETE)
    }

    pub fn acknowledged(&self) -> bool {
        self.raw().status == ACKNOWLEDGED
    }

    pub fn input_byte(&self) -> Option<u8> {
        let (page, generation) = self.endpoint_page(native_service::READ_INPUT)?;
        unsafe { InputPage::take_at(page, generation) }
    }

    pub fn clear_display(&mut self) -> bool {
        let Some((page, generation)) = self.endpoint_page(native_service::CLEAR_DISPLAY) else {
            return false;
        };
        (unsafe { DisplayPage::request_clear_at(page, generation) })
            && self.invoke(native_service::CLEAR_DISPLAY)
            && unsafe { DisplayPage::finish_at(page, generation) }
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
        if let Some((page, generation)) = self.endpoint_page(native_service::PRESENT_TEXT) {
            (unsafe { DisplayPage::request_text_at(page, generation, x, y, color, text) })
                && self.invoke(native_service::PRESENT_TEXT)
                && unsafe { DisplayPage::finish_at(page, generation) }
        } else {
            false
        }
    }

    pub fn syscall(&mut self, syscall: logos_abi::Syscall, argument: &[u8]) -> Option<TextReply> {
        if argument.len() > MAX_TEXT {
            return None;
        }
        let (page, generation) = self.session_client_page()?;
        let id = self.next_session_id;
        self.next_session_id = id.checked_add(1)?;
        let mut bytes = [0; MAX_TEXT];
        bytes[..argument.len()].copy_from_slice(argument);
        let request = logos_abi::SessionRequest::new(syscall, bytes, argument.len());
        if !(unsafe { SessionClientPage::request_at(page, generation, generation, id, request) })
            || !self.invoke(native_service::SYSCALL)
        {
            return None;
        }
        let response = unsafe { SessionClientPage::finish_at(page, generation, generation, id) }?;
        Some(TextReply { text: response.reply.text, length: response.reply.length })
    }

    pub fn wait_for_session(&mut self) -> bool {
        let Some((page, generation)) = self.session_server_page() else { return false };
        (unsafe { SessionServerPage::wait_at(page, generation, generation) })
            && self.invoke(native_service::READ_INPUT)
    }

    pub fn session_request(&self) -> Option<native_service::SessionServerRequest> {
        let (page, generation) = self.session_server_page()?;
        unsafe { SessionServerPage::take_at(page, generation, generation) }
    }

    pub fn session_effect(
        &mut self,
        id: u32,
        effect: logos_abi::Effect,
        argument: &[u8],
    ) -> Option<logos_abi::EffectResult> {
        if argument.len() > MAX_TEXT {
            return None;
        }
        let (page, generation) = self.effect_page()?;
        let mut bytes = [0; MAX_TEXT];
        bytes[..argument.len()].copy_from_slice(argument);
        let request = logos_abi::EffectRequest::new(effect, bytes, argument.len());
        if !(unsafe { EffectPage::request_at(page, generation, generation, id, request) })
            || !self.invoke(native_service::SESSION_EFFECT)
        {
            return None;
        }
        unsafe { EffectPage::finish_at(page, generation, generation, id) }
            .map(|response| response.reply.result)
    }

    pub fn session_reply(&mut self, id: u32, status: SessionStatus, reply: &[u8]) -> bool {
        if reply.len() > MAX_TEXT {
            return false;
        }
        let Some((page, generation)) = self.session_server_page() else { return false };
        let Some(reply) = logos_abi::SessionReply::from_bytes(reply) else { return false };
        (unsafe { SessionServerPage::reply_at(page, generation, generation, id, status, reply) })
            && self.invoke(native_service::SESSION_REPLY)
    }

    pub fn store(&mut self, request: logos_abi::StoreRequest) -> Option<logos_abi::StoreReply> {
        if !request.valid()
            || unsafe { !ControlPage::request_store_at(self.raw_address(), request) }
            || !self.invoke(native_service::STORE_REQUEST)
        {
            return None;
        }
        unsafe { ControlPage::store_reply_at(self.raw_address(), request.id) }
    }

    pub fn store_request(&self) -> Option<logos_abi::StoreRequest> {
        unsafe { ControlPage::store_at(self.raw_address()) }
    }

    pub fn store_reply(&mut self, reply: logos_abi::StoreReply) -> bool {
        let valid = unsafe { ControlPage::reply_store_at(self.raw_address(), reply) };
        valid && self.invoke(native_service::STORE_REPLY)
    }

    pub fn shared_page(&self) -> Option<SharedPage> {
        let handle = unsafe { ControlPage::shared_page_at(self.raw_address()) }?;
        let address = self.raw_address().checked_sub(logos_abi::PAGE_SIZE as u64)?;
        Some(SharedPage { handle, address })
    }

    pub fn block_client(&self) -> Option<BlockClient> {
        let page = unsafe { ControlPage::block_page_at(self.raw_address()) }?;
        Some(BlockClient { context: self.raw_address(), page, next_id: 1 })
    }

    pub fn network_wait(&mut self, deadline: u64) -> bool {
        (unsafe { ControlPage::network_wait_at(self.raw_address(), deadline) })
            && self.invoke(native_service::NETWORK_WAIT)
    }

    pub fn network_pages(&self) -> Option<NetworkPages> {
        unsafe { ControlPage::network_pages_at(self.raw_address()) }
    }

    pub fn network_request(&self) -> Option<logos_abi::NetworkRequest> {
        unsafe { ControlPage::network_at(self.raw_address()) }
    }

    pub fn network_response(&self, expected_id: u32) -> Option<logos_abi::NetworkReply> {
        unsafe { ControlPage::network_reply_at(self.raw_address(), expected_id) }
    }

    pub fn network_owner(&self) -> Option<u64> {
        unsafe { ControlPage::network_owner_at(self.raw_address()) }
    }

    pub fn request_network(&mut self, request: logos_abi::NetworkRequest) -> bool {
        (unsafe { ControlPage::request_network_at(self.raw_address(), request) })
            && self.invoke(native_service::NETWORK_REQUEST)
    }

    pub fn network_reply(&mut self, reply: logos_abi::NetworkReply) -> bool {
        (unsafe { ControlPage::reply_network_at(self.raw_address(), reply) })
            && self.invoke(native_service::NETWORK_REPLY)
    }

    pub fn network_reply_after_device(
        &mut self,
        request: logos_abi::NetworkRequest,
        reply: logos_abi::NetworkReply,
    ) -> bool {
        (unsafe { ControlPage::reply_network_after_device_at(self.raw_address(), request, reply) })
            && self.invoke(native_service::NETWORK_REPLY)
    }

    pub fn network_reply_after_event(
        &mut self,
        request: logos_abi::NetworkRequest,
        reply: logos_abi::NetworkReply,
    ) -> bool {
        (unsafe { ControlPage::reply_network_after_event_at(self.raw_address(), request, reply) })
            && self.invoke(native_service::NETWORK_REPLY)
    }

    pub fn network_device_request(&mut self, request: logos_abi::NetworkDeviceRequest) -> bool {
        (unsafe { ControlPage::request_network_device_at(self.raw_address(), request) })
            && self.invoke(native_service::NETWORK_DEVICE_REQUEST)
    }

    pub fn network_device_reply(&self, expected_id: u32) -> Option<logos_abi::NetworkDeviceReply> {
        unsafe { ControlPage::network_device_reply_at(self.raw_address(), expected_id) }
    }

    pub fn network_event(&self) -> Option<logos_abi::NetworkEvent> {
        unsafe { ControlPage::network_event_at(self.raw_address()) }
    }

    pub fn remote_gate_request(&self) -> Option<native_service::RemoteGateRequest> {
        unsafe { ControlPage::remote_gate_at(self.raw_address()) }
    }

    pub fn remote_gate_reply(&self, expected_id: u32) -> Option<native_service::RemoteGateReply> {
        unsafe { ControlPage::remote_gate_reply_at(self.raw_address(), expected_id) }
    }

    pub fn request_remote_gate(&mut self, request: native_service::RemoteGateRequest) -> bool {
        (unsafe { ControlPage::request_remote_gate_at(self.raw_address(), request) })
            && self.invoke(native_service::REMOTE_GATE)
    }

    pub fn reply_remote_gate(&mut self, reply: native_service::RemoteGateReply) -> bool {
        (unsafe { ControlPage::reply_remote_gate_at(self.raw_address(), reply) })
            && self.invoke(native_service::REMOTE_GATE)
    }

    pub fn storage_status(&self) -> Option<u32> {
        unsafe { ControlPage::storage_status_at(self.raw_address()) }
    }

    pub fn set_storage_status(&mut self, status: u32) {
        self.raw_mut().slot0 = status;
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
        if !request.valid_shape()
            || unsafe { !ControlPage::request_block_at(self.context, request) }
        {
            return Err(BlockError::Invalid);
        }
        #[cfg(target_arch = "x86_64")]
        unsafe {
            asm!("int 0x80", options(nostack, preserves_flags));
        }
        let reply =
            unsafe { ControlPage::block_reply_at(self.context, id) }.ok_or(BlockError::Io)?;
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
            let raw = address as *mut ControlPage;
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
