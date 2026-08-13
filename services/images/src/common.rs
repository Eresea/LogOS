use core::arch::asm;

use logos_abi::ServiceId;

pub fn idle() -> ! {
    loop {
        core::hint::spin_loop();
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
            options(nostack, preserves_flags),
        );
    }
}
