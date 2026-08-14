use core::arch::asm;

use logos_abi::ServiceId;

pub fn idle() -> ! {
    loop {
        core::hint::spin_loop();
    }
}

pub const WAIT_TIMEOUT_TICKS: u64 = logos_abi::SERVICE_HEARTBEAT_INTERVAL_TICKS / 2;

pub const fn ipc_read_event(endpoint: usize) -> u64 {
    logos_abi::ipc_read_event_mask(endpoint)
}

pub const fn ipc_write_event(endpoint: usize) -> u64 {
    logos_abi::ipc_write_event_mask(endpoint)
}

#[allow(dead_code)]
pub const fn keyboard_read_event() -> u64 {
    logos_abi::keyboard_read_event_mask()
}

#[inline(always)]
pub fn heartbeat(service: ServiceId) {
    unsafe {
        asm!(
            "mov eax, 10",
            "int 49",
            in("rdi") service as usize,
            lateout("rax") _,
            options(nostack, preserves_flags),
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
            options(nostack, preserves_flags),
        );
    }
    heartbeat(service);
}

#[inline(always)]
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
            options(nostack, preserves_flags),
        );
    }
}

pub fn notify_edge(mask: u64, notification: logos_abi::Notify) {
    if notification == logos_abi::Notify::Notified {
        notify(mask);
    }
}
