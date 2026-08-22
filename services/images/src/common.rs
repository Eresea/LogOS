extern crate alloc;

use alloc::vec::Vec;
use core::alloc::{GlobalAlloc, Layout};
use core::arch::asm;
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, Ordering};
use core::{mem, ptr};

use logos_abi::{IpcCapabilityPage, IpcStatus, ServiceId};

const SERVICE_PAGE_BYTES: usize = 4096;
const ALLOCATION_MAGIC: u64 = 0x4c4f_474f_5348_4541;

#[repr(C)]
#[derive(Clone, Copy)]
struct FreeSpan {
    previous: usize,
    next: usize,
    bytes: usize,
}

const FREE_SPAN_BYTES: usize = mem::size_of::<FreeSpan>();

#[repr(C)]
struct AllocationHeader {
    magic: u64,
    start: usize,
    span: usize,
    bytes: usize,
}

const HEADER_BYTES: usize = mem::size_of::<AllocationHeader>();

struct AllocatorState {
    heap_capability: logos_abi::CapabilityHandle,
    base: usize,
    mapped_end: usize,
    quota_end: usize,
    used: usize,
    free_head: usize,
}

impl AllocatorState {
    const EMPTY: Self = Self {
        heap_capability: logos_abi::CapabilityHandle::EMPTY,
        base: 0,
        mapped_end: 0,
        quota_end: 0,
        used: 0,
        free_head: 0,
    };

    fn initialize(
        &mut self,
        capability: logos_abi::CapabilityHandle,
        base: usize,
        pages: usize,
        quota_pages: usize,
    ) -> bool {
        let Some(mapped_bytes) = pages.checked_mul(SERVICE_PAGE_BYTES) else {
            return false;
        };
        let Some(quota_bytes) = quota_pages.checked_mul(SERVICE_PAGE_BYTES) else {
            return false;
        };
        let Some(mapped_end) = base.checked_add(mapped_bytes) else {
            return false;
        };
        let Some(quota_end) = base.checked_add(quota_bytes) else {
            return false;
        };
        if !capability.is_valid()
            || base == 0
            || base & (SERVICE_PAGE_BYTES - 1) != 0
            || pages == 0
            || quota_end < mapped_end
        {
            return false;
        }
        self.heap_capability = capability;
        self.base = base;
        self.mapped_end = mapped_end;
        self.quota_end = quota_end;
        self.used = 0;
        self.free_head = 0;
        if !self.insert(base, mapped_bytes) {
            return false;
        }
        true
    }

    fn extend(&mut self, pages: usize) -> bool {
        let Some(bytes) = pages.checked_mul(SERVICE_PAGE_BYTES) else {
            return false;
        };
        let Some(end) = self.mapped_end.checked_add(bytes) else {
            return false;
        };
        if pages == 0 || end > self.quota_end || !self.insert(self.mapped_end, bytes) {
            return false;
        }
        self.mapped_end = end;
        true
    }

    fn can_shrink(&self) -> bool {
        if self.mapped_end.saturating_sub(self.base) <= SERVICE_PAGE_BYTES {
            return false;
        }
        let mut address = self.free_head;
        while address != 0 {
            let span = self.read_span(address);
            if address.saturating_add(span.bytes) == self.mapped_end
                && span.bytes >= SERVICE_PAGE_BYTES
            {
                return true;
            }
            address = span.next;
        }
        false
    }

    fn shrink(&mut self, pages: usize) -> bool {
        if pages != 1 || !self.can_shrink() {
            return false;
        }
        let mut address = self.free_head;
        while address != 0 {
            let span = self.read_span(address);
            if address.saturating_add(span.bytes) == self.mapped_end
                && span.bytes >= SERVICE_PAGE_BYTES
            {
                if span.bytes == SERVICE_PAGE_BYTES {
                    self.remove(address);
                } else {
                    self.write_span(
                        address,
                        FreeSpan { bytes: span.bytes - SERVICE_PAGE_BYTES, ..span },
                    );
                }
                self.mapped_end -= SERVICE_PAGE_BYTES;
                return true;
            }
            address = span.next;
        }
        false
    }

    fn allocate(&mut self, layout: Layout) -> *mut u8 {
        if layout.size() == 0 || layout.size() > self.quota_end.saturating_sub(self.base) {
            return ptr::null_mut();
        }
        let mut address = self.free_head;
        while address != 0 {
            let block = self.read_span(address);
            let Some(header_end) = address.checked_add(HEADER_BYTES) else {
                address = block.next;
                continue;
            };
            let Some(pointer) = align_up(header_end, layout.align()) else {
                address = block.next;
                continue;
            };
            let Some(end) = pointer.checked_add(layout.size()) else {
                address = block.next;
                continue;
            };
            let Some(block_end) = address.checked_add(block.bytes) else {
                address = block.next;
                continue;
            };
            if end > block_end || self.used > self.quota_end.saturating_sub(layout.size()) {
                address = block.next;
                continue;
            }
            let prefix = pointer - HEADER_BYTES - address;
            let suffix = block_end - end;
            let allocation_start =
                if prefix >= FREE_SPAN_BYTES { pointer - HEADER_BYTES } else { address };
            let allocation_end = if suffix >= FREE_SPAN_BYTES { end } else { block_end };
            self.remove(address);
            if prefix >= FREE_SPAN_BYTES {
                self.insert(address, prefix);
            }
            if suffix >= FREE_SPAN_BYTES {
                self.insert(end, suffix);
            }
            let header = AllocationHeader {
                magic: ALLOCATION_MAGIC,
                start: allocation_start,
                span: allocation_end - allocation_start,
                bytes: layout.size(),
            };
            unsafe {
                ptr::write_unaligned((pointer - HEADER_BYTES) as *mut AllocationHeader, header)
            };
            self.used = self.used.saturating_add(layout.size());
            return pointer as *mut u8;
        }
        ptr::null_mut()
    }

    unsafe fn deallocate(&mut self, pointer: *mut u8) {
        if pointer.is_null() || (pointer as usize) < HEADER_BYTES {
            return;
        }
        let header_pointer = (pointer as usize - HEADER_BYTES) as *const AllocationHeader;
        let header = unsafe { ptr::read_unaligned(header_pointer) };
        let Some(end) = header.start.checked_add(header.span) else {
            return;
        };
        if header.magic != ALLOCATION_MAGIC
            || header.start < self.base
            || end > self.mapped_end
            || header.bytes == 0
            || header.bytes > header.span
        {
            return;
        }
        if self.insert(header.start, header.span) {
            self.used = self.used.saturating_sub(header.bytes);
        }
    }

    fn read_span(&self, address: usize) -> FreeSpan {
        unsafe { ptr::read_unaligned(address as *const FreeSpan) }
    }

    fn write_span(&self, address: usize, span: FreeSpan) {
        unsafe { ptr::write_unaligned(address as *mut FreeSpan, span) };
    }

    fn remove(&mut self, address: usize) {
        let span = self.read_span(address);
        if span.previous == 0 {
            self.free_head = span.next;
        } else {
            let mut previous = self.read_span(span.previous);
            previous.next = span.next;
            self.write_span(span.previous, previous);
        }
        if span.next != 0 {
            let mut next = self.read_span(span.next);
            next.previous = span.previous;
            self.write_span(span.next, next);
        }
    }

    fn insert(&mut self, start: usize, bytes: usize) -> bool {
        if bytes == 0 {
            return true;
        }
        if bytes < FREE_SPAN_BYTES {
            return false;
        }
        let Some(end) = start.checked_add(bytes) else {
            return false;
        };

        let mut previous_address = 0;
        let mut next_address = self.free_head;
        while next_address != 0 && next_address < start {
            previous_address = next_address;
            next_address = self.read_span(next_address).next;
        }
        if previous_address != 0 {
            let previous = self.read_span(previous_address);
            let Some(previous_end) = previous_address.checked_add(previous.bytes) else {
                return false;
            };
            if previous_end > start {
                return false;
            }
        }
        if next_address != 0 && end > next_address {
            return false;
        }

        let merge_previous = previous_address != 0
            && previous_address.checked_add(self.read_span(previous_address).bytes) == Some(start);
        let merge_next = next_address != 0 && end == next_address;
        let next_after = if merge_next { self.read_span(next_address).next } else { next_address };

        if merge_previous {
            let mut previous = self.read_span(previous_address);
            let Some(mut merged_bytes) = previous.bytes.checked_add(bytes) else {
                return false;
            };
            if merge_next {
                let Some(total) = merged_bytes.checked_add(self.read_span(next_address).bytes)
                else {
                    return false;
                };
                merged_bytes = total;
            }
            previous.bytes = merged_bytes;
            previous.next = next_after;
            self.write_span(previous_address, previous);
            if next_after != 0 {
                let mut next = self.read_span(next_after);
                next.previous = previous_address;
                self.write_span(next_after, next);
            }
            return true;
        }

        let mut merged_bytes = bytes;
        if merge_next {
            let Some(total) = merged_bytes.checked_add(self.read_span(next_address).bytes) else {
                return false;
            };
            merged_bytes = total;
        }
        self.write_span(
            start,
            FreeSpan { previous: previous_address, next: next_after, bytes: merged_bytes },
        );
        if previous_address == 0 {
            self.free_head = start;
        } else {
            let mut previous = self.read_span(previous_address);
            previous.next = start;
            self.write_span(previous_address, previous);
        }
        if next_after != 0 {
            let mut next = self.read_span(next_after);
            next.previous = start;
            self.write_span(next_after, next);
        }
        true
    }

    #[cfg(test)]
    fn free_span_count(&self) -> usize {
        let mut count = 0;
        let mut address = self.free_head;
        while address != 0 {
            count += 1;
            address = self.read_span(address).next;
        }
        count
    }
}

fn align_up(value: usize, alignment: usize) -> Option<usize> {
    if alignment == 0 || !alignment.is_power_of_two() {
        return None;
    }
    value.checked_add(alignment - 1).map(|value| value & !(alignment - 1))
}

pub struct ServiceAllocator {
    locked: AtomicBool,
    state: UnsafeCell<AllocatorState>,
}

unsafe impl Sync for ServiceAllocator {}

impl ServiceAllocator {
    pub const fn new() -> Self {
        Self { locked: AtomicBool::new(false), state: UnsafeCell::new(AllocatorState::EMPTY) }
    }

    fn lock(&self) -> ServiceAllocatorGuard<'_> {
        while self.locked.swap(true, Ordering::Acquire) {
            core::hint::spin_loop();
        }
        ServiceAllocatorGuard { allocator: self }
    }

    pub fn initialize(
        &self,
        capability: logos_abi::CapabilityHandle,
        base: usize,
        pages: usize,
        quota_pages: usize,
    ) -> bool {
        let guard = self.lock();
        unsafe {
            (&mut *guard.allocator.state.get()).initialize(capability, base, pages, quota_pages)
        }
    }

    fn allocate_with_growth(&self, layout: Layout) -> *mut u8 {
        loop {
            let (pointer, capability) = {
                let guard = self.lock();
                let state = unsafe { &mut *guard.allocator.state.get() };
                let pointer = state.allocate(layout);
                (pointer, state.heap_capability)
            };
            if !pointer.is_null() || !capability.is_valid() || !request_heap_growth(capability) {
                return pointer;
            }
            let guard = self.lock();
            if !unsafe { (&mut *guard.allocator.state.get()).extend(1) } {
                return ptr::null_mut();
            }
        }
    }
}

struct ServiceAllocatorGuard<'a> {
    allocator: &'a ServiceAllocator,
}

impl Drop for ServiceAllocatorGuard<'_> {
    fn drop(&mut self) {
        self.allocator.locked.store(false, Ordering::Release);
    }
}

unsafe impl GlobalAlloc for ServiceAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        self.allocate_with_growth(layout)
    }

    unsafe fn dealloc(&self, pointer: *mut u8, _layout: Layout) {
        let guard = self.lock();
        let state = unsafe { &mut *guard.allocator.state.get() };
        unsafe { state.deallocate(pointer) };
        if state.can_shrink() && request_heap_shrink(state.heap_capability) {
            state.shrink(1);
        }
    }
}

#[cfg(target_os = "none")]
#[global_allocator]
static SERVICE_GLOBAL_ALLOCATOR: ServiceAllocator = ServiceAllocator::new();

pub fn init_service_allocator() {
    #[cfg(target_os = "none")]
    {
        let bootstrap = bootstrap_page();
        let _ = SERVICE_GLOBAL_ALLOCATOR.initialize(
            bootstrap.heap,
            bootstrap.heap_base as usize,
            bootstrap.heap_pages as usize,
            bootstrap.heap_quota_pages as usize,
        );
    }
}

#[inline(always)]
pub fn bootstrap_page() -> &'static logos_abi::ServiceBootstrapPage {
    unsafe { &*(logos_abi::SERVICE_BOOTSTRAP_BASE as *const logos_abi::ServiceBootstrapPage) }
}

#[allow(dead_code)]
fn capability_record_matches(
    record: logos_abi::DirectoryRecord,
    peer: logos_abi::ServiceHandle,
    rights: logos_abi::IpcRights,
    message_bytes: usize,
) -> bool {
    record.kind == logos_abi::DirectoryRecordKind::Capability
        && record.peer == peer
        && record.rights == rights as u8
        && record.message_bytes as usize == message_bytes
        && logos_abi::CapabilityHandle::from_raw(record.handle).is_some()
}

#[allow(dead_code)]
fn capability_from_response(
    response: &logos_abi::DirectoryResponse,
    peer: logos_abi::ServiceHandle,
    rights: logos_abi::IpcRights,
    message_bytes: usize,
) -> Result<Option<logos_abi::CapabilityHandle>, logos_abi::DirectoryStatus> {
    let mut found = None;
    for record in &response.records[..response.count as usize] {
        if !capability_record_matches(*record, peer, rights, message_bytes) {
            continue;
        }
        if found.is_some() {
            return Err(logos_abi::DirectoryStatus::Malformed);
        }
        found = logos_abi::CapabilityHandle::from_raw(record.handle);
    }
    Ok(found)
}

#[allow(dead_code)]
pub fn discover_capability(
    peer: logos_abi::ServiceHandle,
    rights: logos_abi::IpcRights,
    message_bytes: usize,
) -> Result<logos_abi::CapabilityHandle, logos_abi::DirectoryStatus> {
    #[cfg(target_os = "none")]
    {
        let bootstrap = bootstrap_page();
        if !bootstrap.is_valid() || !peer.is_valid() || message_bytes == 0 {
            return Err(logos_abi::DirectoryStatus::Malformed);
        }
        let mut request =
            logos_abi::DirectoryRequest::new(logos_abi::DirectoryOperation::Capabilities, 1);
        request.subject = bootstrap.service;
        loop {
            let mut response = logos_abi::DirectoryResponse::empty(
                request.operation,
                logos_abi::DirectoryStatus::Malformed,
                request.request_id,
            );
            let status = directory_call(bootstrap.directory, &request, &mut response);
            if status != logos_abi::DirectoryStatus::Ok {
                return Err(status);
            }
            if let Some(capability) =
                capability_from_response(&response, peer, rights, message_bytes)?
            {
                return Ok(capability);
            }
            if response.flags & logos_abi::DIRECTORY_FLAG_MORE == 0 {
                return Err(logos_abi::DirectoryStatus::NotFound);
            }
            if response.cursor <= request.cursor {
                return Err(logos_abi::DirectoryStatus::Malformed);
            }
            request.cursor = response.cursor;
        }
    }
    #[cfg(not(target_os = "none"))]
    {
        let _ = (peer, rights, message_bytes);
        Err(logos_abi::DirectoryStatus::NotFound)
    }
}

fn request_heap_growth(capability: logos_abi::CapabilityHandle) -> bool {
    #[cfg(target_os = "none")]
    {
        let mut raw = logos_abi::SERVICE_HEAP_GROW_SYSCALL;
        unsafe {
            asm!(
                "int 49",
                inout("rax") raw,
                in("rdi") capability.raw() as usize,
                in("rsi") 1usize,
                options(preserves_flags),
            );
        }
        raw == logos_abi::IpcStatus::Ok as usize
    }
    #[cfg(not(target_os = "none"))]
    {
        let _ = capability;
        false
    }
}

fn request_heap_shrink(capability: logos_abi::CapabilityHandle) -> bool {
    #[cfg(target_os = "none")]
    {
        let mut raw = logos_abi::SERVICE_HEAP_SHRINK_SYSCALL;
        unsafe {
            asm!(
                "int 49",
                inout("rax") raw,
                in("rdi") capability.raw() as usize,
                in("rsi") 1usize,
                options(preserves_flags),
            );
        }
        raw == logos_abi::IpcStatus::Ok as usize
    }
    #[cfg(not(target_os = "none"))]
    {
        let _ = capability;
        false
    }
}

pub fn idle() -> ! {
    loop {
        core::hint::spin_loop();
    }
}

pub const WAIT_TIMEOUT_TICKS: u64 = logos_abi::SERVICE_HEARTBEAT_INTERVAL_TICKS / 2;

#[allow(dead_code)]
pub const fn ipc_read_event(endpoint: logos_abi::IpcEndpointId) -> u64 {
    endpoint.read_event_mask()
}

#[allow(dead_code)]
pub const fn ipc_write_event(endpoint: logos_abi::IpcEndpointId) -> u64 {
    endpoint.write_event_mask()
}

#[allow(dead_code)]
pub const fn keyboard_read_event() -> u64 {
    logos_abi::keyboard_read_event_mask()
}

#[allow(dead_code)]
pub const fn capability_slot(
    service: ServiceId,
    endpoint: logos_abi::IpcEndpointId,
    rights: logos_abi::IpcRights,
) -> usize {
    match logos_abi::ipc_capability_slot(service, endpoint, rights) {
        Some(slot) => slot,
        None => logos_abi::MAX_IPC_CAPABILITIES,
    }
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapabilitySpec {
    pub endpoint: logos_abi::IpcEndpointId,
    pub rights: logos_abi::IpcRights,
}

#[allow(dead_code)]
pub const fn capability_spec(
    endpoint: logos_abi::IpcEndpointId,
    rights: logos_abi::IpcRights,
) -> CapabilitySpec {
    CapabilitySpec { endpoint, rights }
}

#[allow(dead_code)]
struct CapabilityCache(UnsafeCell<Option<Vec<(CapabilitySpec, logos_abi::CapabilityHandle)>>>);

unsafe impl Sync for CapabilityCache {}

#[allow(dead_code)]
static DISCOVERED_CAPABILITIES: CapabilityCache = CapabilityCache(UnsafeCell::new(None));

#[allow(dead_code)]
fn capability_peer(spec: CapabilitySpec, generation: u32) -> Option<logos_abi::ServiceHandle> {
    let core_endpoint = matches!(
        spec.endpoint,
        logos_abi::IpcEndpointId::CoreToStorage
            | logos_abi::IpcEndpointId::StorageToCore
            | logos_abi::IpcEndpointId::CoreToNetwork
            | logos_abi::IpcEndpointId::NetworkToCore
            | logos_abi::IpcEndpointId::CoreToDevice
            | logos_abi::IpcEndpointId::DeviceToCore
            | logos_abi::IpcEndpointId::CoreToStoragePackage
            | logos_abi::IpcEndpointId::StoragePackageToCore
            | logos_abi::IpcEndpointId::CoreToStorageMap
            | logos_abi::IpcEndpointId::StorageMapToCore
    );
    if core_endpoint {
        return logos_abi::ServiceHandle::new(u32::MAX, generation);
    }
    let service = match (spec.endpoint, spec.rights) {
        (logos_abi::IpcEndpointId::FetchToStorage, logos_abi::IpcRights::Send) => {
            logos_abi::ServiceId::Storage
        }
        (logos_abi::IpcEndpointId::FetchToNetwork, logos_abi::IpcRights::Send) => {
            logos_abi::ServiceId::Network
        }
        (_, logos_abi::IpcRights::Send) => spec.endpoint.consumer(),
        (_, logos_abi::IpcRights::Receive) => spec.endpoint.producer(),
    };
    logos_abi::ServiceHandle::new(service.index() as u32, generation)
}

#[allow(dead_code)]
fn discovered_capability(spec: CapabilitySpec) -> Result<logos_abi::CapabilityHandle, IpcStatus> {
    #[cfg(target_os = "none")]
    {
        let bootstrap = bootstrap_page();
        let peer =
            capability_peer(spec, bootstrap.service.generation()).ok_or(IpcStatus::Unauthorized)?;
        unsafe {
            if let Some(cache) = (&*DISCOVERED_CAPABILITIES.0.get()).as_ref() {
                if let Some((_, capability)) =
                    cache.iter().find(|(candidate, _)| *candidate == spec)
                {
                    return Ok(*capability);
                }
            }
        }
        let message_bytes =
            logos_abi::ipc_message_size(spec.endpoint.index()).ok_or(IpcStatus::Malformed)?;
        let capability = discover_capability(peer, spec.rights, message_bytes)
            .map_err(|_| IpcStatus::Unauthorized)?;
        unsafe {
            let cache = (&mut *DISCOVERED_CAPABILITIES.0.get()).get_or_insert_with(Vec::new);
            cache.try_reserve(1).map_err(|_| IpcStatus::Disconnected)?;
            cache.push((spec, capability));
        }
        Ok(capability)
    }
    #[cfg(not(target_os = "none"))]
    {
        let _ = spec;
        Err(IpcStatus::Unauthorized)
    }
}

pub trait IpcCapabilityArgument: Copy {
    fn resolve(self, message_bytes: usize) -> Result<(u64, usize), IpcStatus>;
}

impl IpcCapabilityArgument for usize {
    fn resolve(self, message_bytes: usize) -> Result<(u64, usize), IpcStatus> {
        let Some(capability) = capability(self) else {
            return Err(IpcStatus::Unauthorized);
        };
        let expected =
            endpoint_message_size(capability.endpoint_index()).ok_or(IpcStatus::Unauthorized)?;
        if message_bytes != expected {
            return Err(IpcStatus::Malformed);
        }
        Ok((self as u64, expected))
    }
}

impl IpcCapabilityArgument for CapabilitySpec {
    fn resolve(self, message_bytes: usize) -> Result<(u64, usize), IpcStatus> {
        let expected =
            logos_abi::ipc_message_size(self.endpoint.index()).ok_or(IpcStatus::Unauthorized)?;
        if message_bytes != expected {
            return Err(IpcStatus::Malformed);
        }
        Ok((discovered_capability(self)?.raw(), expected))
    }
}

#[inline(always)]
pub fn heartbeat(service: ServiceId) {
    unsafe {
        asm!(
            "mov eax, 10",
            "int 49",
            in("rdi") service as usize,
            lateout("rax") _,
            options(preserves_flags),
        );
    }
}

pub fn heartbeat_tick(ticks: &mut u16, service: ServiceId) {
    *ticks = ticks.wrapping_add(1);
    if *ticks == 1024 {
        *ticks = 0;
        heartbeat(service);
    }
}

#[cfg(feature = "qemu-proof")]
#[allow(dead_code)]
pub fn proof_line(message: &[u8]) {
    for &byte in message {
        unsafe { asm!("out dx, al", in("dx") 0xe9u16, in("al") byte) };
    }
    unsafe { asm!("out dx, al", in("dx") 0xe9u16, in("al") b'\r') };
    unsafe { asm!("out dx, al", in("dx") 0xe9u16, in("al") b'\n') };
}

#[inline(always)]
pub fn wait(mask: u64, service: ServiceId) {
    unsafe {
        asm!(
            "mov eax, 2",
            "int 49",
            in("rdi") mask as usize,
            in("rsi") WAIT_TIMEOUT_TICKS as usize,
            lateout("rax") _,
            options(preserves_flags),
        );
    }
    heartbeat(service);
}

#[inline(always)]
#[allow(dead_code)]
pub fn notify(mask: u64) {
    if mask == 0 {
        return;
    }
    unsafe {
        asm!(
            "mov eax, 3",
            "int 49",
            in("rdi") mask as usize,
            lateout("rax") _,
            options(preserves_flags),
        );
    }
}

#[allow(dead_code)]
pub fn notify_edge(mask: u64, notification: logos_abi::Notify) {
    if notification == logos_abi::Notify::Notified {
        notify(mask);
    }
}

#[inline(always)]
#[allow(dead_code)]
pub fn ipc_send<T: Copy, C: IpcCapabilityArgument>(capability: C, message: &T) -> IpcStatus {
    let length = mem::size_of::<T>();
    let (capability_raw, expected_length) = match capability.resolve(length) {
        Ok(resolved) => resolved,
        Err(status) => return status,
    };
    if length != expected_length || length > logos_abi::IPC_PAGE_BYTES {
        return IpcStatus::Malformed;
    }
    unsafe {
        ptr::write_unaligned(logos_abi::IPC_STAGING_BASE as *mut T, *message);
    }
    ipc_syscall_raw(logos_abi::IPC_SYSCALL_SEND, capability_raw, length)
}

#[inline(always)]
#[allow(dead_code)]
pub fn ipc_receive<T: Copy, C: IpcCapabilityArgument>(capability: C, message: &mut T) -> IpcStatus {
    let length = mem::size_of::<T>();
    let Ok((capability_raw, expected_length)) = capability.resolve(length) else {
        return IpcStatus::Unauthorized;
    };
    if length != expected_length || expected_length > logos_abi::IPC_PAGE_BYTES {
        return IpcStatus::Malformed;
    }
    let status = ipc_syscall_raw(logos_abi::IPC_SYSCALL_RECEIVE, capability_raw, 0);
    if status == IpcStatus::Ok {
        *message = unsafe { ptr::read_unaligned(logos_abi::IPC_STAGING_BASE as *const T) };
    }
    status
}

#[inline(always)]
#[allow(dead_code)]
pub fn ipc_send_handle<T: Copy>(capability: logos_abi::CapabilityHandle, message: &T) -> IpcStatus {
    ipc_send_raw(capability, message)
}

#[inline(always)]
#[allow(dead_code)]
pub fn ipc_receive_handle<T: Copy>(
    capability: logos_abi::CapabilityHandle,
    message: &mut T,
) -> IpcStatus {
    ipc_receive_raw(capability, message)
}

#[inline(always)]
fn ipc_send_raw<T: Copy>(capability: logos_abi::CapabilityHandle, message: &T) -> IpcStatus {
    let length = mem::size_of::<T>();
    if !capability.is_valid() || length == 0 || length > logos_abi::IPC_PAGE_BYTES {
        return IpcStatus::Malformed;
    }
    unsafe {
        ptr::write_unaligned(logos_abi::IPC_STAGING_BASE as *mut T, *message);
    }
    ipc_syscall_raw(logos_abi::IPC_SYSCALL_SEND, capability.raw(), length)
}

#[inline(always)]
fn ipc_receive_raw<T: Copy>(capability: logos_abi::CapabilityHandle, message: &mut T) -> IpcStatus {
    let length = mem::size_of::<T>();
    if !capability.is_valid() || length == 0 || length > logos_abi::IPC_PAGE_BYTES {
        return IpcStatus::Malformed;
    }
    let status = ipc_syscall_raw(logos_abi::IPC_SYSCALL_RECEIVE, capability.raw(), 0);
    if status == IpcStatus::Ok {
        *message = unsafe { ptr::read_unaligned(logos_abi::IPC_STAGING_BASE as *const T) };
    }
    status
}

#[inline(always)]
#[allow(dead_code)]
pub fn power(action: usize) -> usize {
    let mut raw = logos_abi::POWER_SYSCALL;
    unsafe {
        asm!(
            "int 49",
            inout("rax") raw,
            in("rdi") action,
            options(preserves_flags),
        );
    }
    raw
}

#[inline(always)]
#[allow(dead_code)]
pub fn manager_call(
    request: &logos_abi::ManagerRequest,
    response: &mut logos_abi::ManagerResponse,
) -> logos_abi::IpcStatus {
    unsafe {
        ptr::write_unaligned(
            logos_abi::IPC_STAGING_BASE as *mut logos_abi::ManagerRequest,
            *request,
        );
    }
    let status = manager_syscall(
        logos_abi::MANAGER_SYSCALL,
        logos_abi::MANAGER_CAPABILITY_SLOT,
        mem::size_of::<logos_abi::ManagerRequest>(),
    );
    if status == logos_abi::IpcStatus::Ok {
        let received = unsafe {
            ptr::read_unaligned(logos_abi::IPC_STAGING_BASE as *const logos_abi::ManagerResponse)
        };
        if received.request_id != request.request_id {
            return logos_abi::IpcStatus::Stale;
        }
        if !received.is_valid_for(*request) {
            return logos_abi::IpcStatus::Malformed;
        }
        *response = received;
    }
    status
}

#[inline(always)]
#[allow(dead_code)]
pub fn directory_call(
    capability: logos_abi::CapabilityHandle,
    request: &logos_abi::DirectoryRequest,
    response: &mut logos_abi::DirectoryResponse,
) -> logos_abi::DirectoryStatus {
    unsafe {
        ptr::write_unaligned(
            logos_abi::IPC_STAGING_BASE as *mut logos_abi::DirectoryRequest,
            *request,
        );
    }
    let mut raw = logos_abi::SERVICE_DIRECTORY_SYSCALL;
    unsafe {
        asm!(
            "int 49",
            inout("rax") raw,
            in("rdi") capability.raw() as usize,
            in("rsi") mem::size_of::<logos_abi::DirectoryRequest>(),
            options(preserves_flags),
        );
    }
    let status =
        logos_abi::DirectoryStatus::from_raw(raw).unwrap_or(logos_abi::DirectoryStatus::Malformed);
    if status == logos_abi::DirectoryStatus::Ok {
        let received = unsafe {
            ptr::read_unaligned(logos_abi::IPC_STAGING_BASE as *const logos_abi::DirectoryResponse)
        };
        if !received.is_valid_for(*request) {
            return logos_abi::DirectoryStatus::Malformed;
        }
        *response = received;
    }
    status
}

fn endpoint_message_size(endpoint: Option<usize>) -> Option<usize> {
    endpoint.and_then(logos_abi::ipc_message_size)
}

#[inline(always)]
#[cfg(feature = "qemu-proof")]
#[allow(dead_code)]
pub fn ipc_probe(number: usize, capability_slot: usize, length: usize) -> IpcStatus {
    ipc_syscall(number, capability_slot, length)
}

#[inline(always)]
#[allow(dead_code)]
pub fn capability(slot: usize) -> Option<logos_abi::IpcCapability> {
    if slot >= logos_abi::MAX_IPC_CAPABILITIES {
        return None;
    }
    let page = unsafe { &*(logos_abi::IPC_CAPABILITY_BASE as *const IpcCapabilityPage) };
    page.get(slot)
}

#[inline(always)]
#[allow(dead_code)]
fn ipc_syscall(number: usize, capability_slot: usize, length: usize) -> IpcStatus {
    ipc_syscall_raw(number, capability_slot as u64, length)
}

#[inline(always)]
fn ipc_syscall_raw(number: usize, capability_raw: u64, length: usize) -> IpcStatus {
    let mut raw = number;
    unsafe {
        asm!(
            "int 49",
            inout("rax") raw,
            in("rdi") capability_raw as usize,
            in("rsi") length,
            options(preserves_flags),
        );
    }
    IpcStatus::from_raw(raw).unwrap_or(IpcStatus::Malformed)
}

#[inline(always)]
fn manager_syscall(number: usize, capability_slot: usize, length: usize) -> IpcStatus {
    let mut raw = number;
    unsafe {
        asm!(
            "int 49",
            inout("rax") raw,
            in("rdi") capability_slot,
            in("rsi") length,
            options(preserves_flags),
        );
    }
    IpcStatus::from_raw(raw).unwrap_or(IpcStatus::Malformed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_response_matching_rejects_duplicates_and_wrong_shapes() {
        let peer = logos_abi::ServiceHandle::new(2, 1).unwrap();
        let capability = logos_abi::CapabilityHandle::new(7, 3).unwrap();
        let mut response = logos_abi::DirectoryResponse::empty(
            logos_abi::DirectoryOperation::Capabilities,
            logos_abi::DirectoryStatus::Ok,
            1,
        );
        response.records[0] = logos_abi::DirectoryRecord {
            kind: logos_abi::DirectoryRecordKind::Capability,
            rights: logos_abi::IpcRights::Send as u8,
            flags: 0,
            handle: capability.raw(),
            peer,
            message_bytes: 16,
            queue_capacity: 1,
            name_len: 0,
            reserved: [0; 3],
            name: [0; logos_abi::MAX_SERVICE_NAME_BYTES],
        };
        response.count = 1;
        assert_eq!(
            capability_from_response(&response, peer, logos_abi::IpcRights::Send, 16),
            Ok(Some(capability))
        );
        response.records[1] = response.records[0];
        response.count = 2;
        assert_eq!(
            capability_from_response(&response, peer, logos_abi::IpcRights::Send, 16),
            Err(logos_abi::DirectoryStatus::Malformed)
        );
    }

    #[repr(align(4096))]
    struct TestHeap([u8; SERVICE_PAGE_BYTES * 3]);

    #[test]
    fn allocator_state_grows_and_reclaims_tail_pages() {
        let mut backing = TestHeap([0; SERVICE_PAGE_BYTES * 3]);
        let capability = logos_abi::CapabilityHandle::new(2, 1).unwrap();
        let mut state = AllocatorState::EMPTY;
        assert!(state.initialize(capability, backing.0.as_mut_ptr() as usize, 1, 3));
        assert!(state.extend(2));

        let layout = Layout::from_size_align(SERVICE_PAGE_BYTES * 2, 64).unwrap();
        let pointer = state.allocate(layout);
        assert!(!pointer.is_null());
        unsafe { state.deallocate(pointer) };
        assert!(state.can_shrink());
        assert!(state.shrink(1));
        assert!(state.shrink(1));
        assert!(!state.can_shrink());
        assert_eq!(state.mapped_end - state.base, SERVICE_PAGE_BYTES);
    }

    #[test]
    fn allocator_state_enforces_quota() {
        let mut backing = TestHeap([0; SERVICE_PAGE_BYTES * 3]);
        let capability = logos_abi::CapabilityHandle::new(2, 1).unwrap();
        let mut state = AllocatorState::EMPTY;
        assert!(state.initialize(capability, backing.0.as_mut_ptr() as usize, 1, 2));
        assert!(state.extend(1));
        assert!(!state.extend(1));

        let first = Layout::from_size_align(4_000, 64).unwrap();
        let second = Layout::from_size_align(4_200, 64).unwrap();
        assert!(!state.allocate(first).is_null());
        assert!(state.allocate(second).is_null());
    }

    #[test]
    fn allocator_state_scales_intrusive_free_spans() {
        #[repr(align(4096))]
        struct LargeHeap([u8; SERVICE_PAGE_BYTES * 8]);

        let mut backing = LargeHeap([0; SERVICE_PAGE_BYTES * 8]);
        let capability = logos_abi::CapabilityHandle::new(2, 1).unwrap();
        let mut state = AllocatorState::EMPTY;
        assert!(state.initialize(capability, backing.0.as_mut_ptr() as usize, 8, 8,));

        let layout = Layout::from_size_align(64, 8).unwrap();
        let mut allocations = [ptr::null_mut(); 260];
        for allocation in &mut allocations {
            *allocation = state.allocate(layout);
            assert!(!allocation.is_null());
        }
        for allocation in allocations.iter().step_by(2) {
            unsafe { state.deallocate(*allocation) };
        }

        assert!(state.free_span_count() > 128);
        assert!(!state.allocate(layout).is_null());
    }
}
