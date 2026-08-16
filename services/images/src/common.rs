use core::arch::asm;
use core::{mem, ptr};

use logos_abi::{IpcCapabilityPage, IpcStatus, ServiceId};

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
        *response = unsafe {
            ptr::read_unaligned(logos_abi::IPC_STAGING_BASE as *const logos_abi::ManagerResponse)
        };
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
