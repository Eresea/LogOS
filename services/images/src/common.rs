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
        let bootstrap = unsafe {
            &*(logos_abi::SERVICE_BOOTSTRAP_BASE as *const logos_abi::ServiceBootstrapPage)
        };
        let _ = SERVICE_GLOBAL_ALLOCATOR.initialize(
            bootstrap.heap,
            bootstrap.heap_base as usize,
            bootstrap.heap_pages as usize,
            bootstrap.heap_quota_pages as usize,
        );
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
pub fn ipc_send<T: Copy>(capability_slot: usize, message: &T) -> IpcStatus {
    let Some(capability) = capability(capability_slot) else {
        return IpcStatus::Unauthorized;
    };
    let Some(expected_length) = endpoint_message_size(capability.endpoint_index()) else {
        return IpcStatus::Unauthorized;
    };
    let length = mem::size_of::<T>();
    if length != expected_length || length > logos_abi::IPC_PAGE_BYTES {
        return IpcStatus::Malformed;
    }
    unsafe {
        ptr::write_unaligned(logos_abi::IPC_STAGING_BASE as *mut T, *message);
    }
    ipc_syscall(logos_abi::IPC_SYSCALL_SEND, capability_slot, length)
}

#[inline(always)]
#[allow(dead_code)]
pub fn ipc_receive<T: Copy>(capability_slot: usize, message: &mut T) -> IpcStatus {
    let Some(capability) = capability(capability_slot) else {
        return IpcStatus::Unauthorized;
    };
    let Some(expected_length) = endpoint_message_size(capability.endpoint_index()) else {
        return IpcStatus::Unauthorized;
    };
    if mem::size_of::<T>() != expected_length || expected_length > logos_abi::IPC_PAGE_BYTES {
        return IpcStatus::Malformed;
    }
    let status = ipc_syscall(logos_abi::IPC_SYSCALL_RECEIVE, capability_slot, 0);
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
fn ipc_syscall(number: usize, capability_slot: usize, length: usize) -> IpcStatus {
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
