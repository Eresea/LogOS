use core::{
    arch::{asm, global_asm},
    sync::atomic::Ordering,
};

use super::*;

#[unsafe(no_mangle)]
extern "C" fn schedule_from_interrupt(fx_context: usize, cpu: usize, vector: usize) -> usize {
    if HALTED.load(Ordering::Acquire) {
        fatal(b"LogOS vNext: halted");
    }
    let current = SCHEDULER.current_task(cpu);
    let user_fault = if matches!(vector, 6 | 13 | 14) {
        let Some(handle) = current else {
            fatal(b"LogOS vNext: fault without task");
        };
        #[cfg(feature = "qemu-proof")]
        {
            if !crate::user_mode::faulted(handle, vector) {
                fatal(b"LogOS vNext: kernel fault");
            }
            true
        }
        #[cfg(not(feature = "qemu-proof"))]
        {
            let _ = handle;
            fatal(b"LogOS vNext: user fault disabled");
        }
    } else {
        false
    };
    #[cfg(feature = "qemu-proof")]
    let mut user_fault = user_fault;
    #[cfg(feature = "qemu-proof")]
    if vector == usize::from(SWITCH_VECTOR) {
        if let Some(handle) = current {
            if crate::user_mode::is_user_task(handle)
                && !crate::user_mode::dispatch_syscall(handle, fx_context)
            {
                if !crate::user_mode::syscall_faulted(handle) {
                    fatal(b"LogOS vNext: syscall fault");
                }
                user_fault = true;
            }
        }
    }
    if vector == usize::from(TIMER_VECTOR) {
        if cpu == 0 {
            let now = TIMER_TICKS.fetch_add(1, Ordering::AcqRel) + 1;
            SCHEDULER.wake_due(now);
        }
        SCHEDULER.record_tick(cpu);
        local_tick(cpu);
        #[cfg(feature = "qemu-proof")]
        crate::proof::observe(cpu);
    }
    if vector == usize::from(KEYBOARD_VECTOR) {
        handle_keyboard_interrupt();
    }
    let local = unsafe { &*core::ptr::addr_of_mut!(CPU_LOCALS).cast::<CpuLocal>().add(cpu) };
    unsafe { write_apic(APIC_EOI, 0) };
    if let Some(current) = current {
        if !SCHEDULER.save_context(current, fx_context) {
            fatal(b"LogOS vNext: context save");
        }
        let outcome = if user_fault {
            crate::FinishState::Completed
        } else if vector == usize::from(TIMER_VECTOR) {
            crate::FinishState::Runnable
        } else {
            match local.pending_action.swap(0, Ordering::AcqRel) {
                ACTION_BLOCK => crate::FinishState::Blocked,
                ACTION_COMPLETE => crate::FinishState::Completed,
                ACTION_TIMED_BLOCK => crate::FinishState::TimedBlocked,
                _ => crate::FinishState::Runnable,
            }
        };
        if !SCHEDULER.finish(current, outcome) {
            fatal(b"LogOS vNext: context publish");
        }
    } else {
        local.idle_context.store(fx_context, Ordering::Release);
    }
    SCHEDULER.clear_current(cpu);
    unsafe {
        let local = &*core::ptr::addr_of!(CPU_LOCALS).cast::<CpuLocal>().add(cpu);
        local.current_slot.store(usize::MAX, Ordering::Release);
        local.current_generation.store(0, Ordering::Release);
    }
    let Some(next) = SCHEDULER.claim_next(cpu) else {
        #[cfg(feature = "qemu-proof")]
        crate::user_mode::prepare_kernel();
        set_task_kernel_stack(cpu, unsafe {
            (*core::ptr::addr_of!(CPU_LOCALS).cast::<CpuLocal>().add(cpu)).user_entry_stack_top
        } as usize);
        return local.idle_context.load(Ordering::Acquire);
    };
    local.current_slot.store(next.slot(), Ordering::Release);
    local.current_generation.store(next.generation(), Ordering::Release);
    local.scheduler_cursor.store(SCHEDULER.cursor(cpu).unwrap_or(0), Ordering::Release);
    local.switch_count.store(SCHEDULER.switches(cpu).unwrap_or(0), Ordering::Release);
    if SCHEDULER.saved_context(next).is_none() {
        initialize_task_context(next);
    }
    let Some(stack_top) = SCHEDULER.task_stack_top(next) else {
        fatal(b"LogOS vNext: TSS task stack");
    };
    set_task_kernel_stack(cpu, stack_top);
    crate::arch::prepare_task_address_space(SCHEDULER.address_space(next).unwrap_or(0));
    SCHEDULER.saved_context(next).unwrap_or_else(|| fatal(b"LogOS vNext: no context"))
}

fn local_tick(cpu: usize) {
    let ticks = SCHEDULER.ticks(cpu).unwrap_or(0);
    unsafe {
        (*core::ptr::addr_of_mut!(CPU_LOCALS).cast::<CpuLocal>().add(cpu))
            .tick_count
            .store(ticks, Ordering::Release)
    };
}

fn initialize_task_context(handle: crate::TaskHandle) {
    let Some(top) = SCHEDULER.task_stack_top(handle) else {
        fatal(b"LogOS vNext: task stack");
    };
    let fx = (top.saturating_sub(FX_STATE_SIZE + 8)) & !15;
    let gpr = fx.saturating_sub(GPR_WORDS * 8 + 8 + 24);
    unsafe {
        core::ptr::write_bytes(gpr as *mut u8, 0, GPR_WORDS * 8 + 8 + 24);
        core::ptr::write_bytes(fx as *mut u8, 0, FX_STATE_SIZE + 8);
        core::ptr::write_unaligned((fx as *mut u8).add(0) as *mut u16, 0x037f);
        core::ptr::write_unaligned((fx as *mut u8).add(24) as *mut u32, 0x1f80);
        core::ptr::write_unaligned((fx + FX_CONTEXT_POINTER) as *mut usize, gpr);
        core::ptr::write_unaligned((gpr + VECTOR_OFFSET) as *mut usize, SWITCH_VECTOR as usize);
        core::ptr::write_unaligned(
            (gpr + VECTOR_OFFSET + 8) as *mut usize,
            task_bootstrap as *const () as usize,
        );
        core::ptr::write_unaligned(
            (gpr + VECTOR_OFFSET + 16) as *mut usize,
            KERNEL_CODE_SELECTOR as usize,
        );
        core::ptr::write_unaligned((gpr + VECTOR_OFFSET + 24) as *mut usize, 0x202);
        // The save area is in push order (r15 first, rax last on restore).
        core::ptr::write_unaligned(gpr as *mut usize, top);
    }
    if !SCHEDULER.set_initial_context(handle, fx) {
        fatal(b"LogOS vNext: initial context");
    }
}

#[unsafe(no_mangle)]
extern "C" fn task_trampoline() -> ! {
    let cpu = unsafe { (*(read_gs() as *const CpuLocal)).cpu_index as usize };
    if let Some(handle) = SCHEDULER.current_task(cpu) {
        if let Some(entry) = SCHEDULER.entry(handle) {
            entry();
        }
    }
    request_switch(ACTION_COMPLETE);
    loop {
        core::hint::spin_loop();
    }
}

pub fn yield_current() {
    request_switch(ACTION_YIELD)
}

pub fn block_current() {
    request_switch(ACTION_BLOCK)
}

pub fn sleep_current_for(ticks: u64) {
    let cpu = current_cpu();
    let Some(handle) = SCHEDULER.current_task(cpu) else {
        fatal(b"LogOS vNext: sleep without task");
    };
    let now = TIMER_TICKS.load(Ordering::Acquire);
    let deadline = now.saturating_add(ticks.max(1)).min(u64::MAX - 1);
    unsafe {
        let local = &*(read_gs() as *const CpuLocal);
        asm!("cli");
        if !SCHEDULER.arm_deadline(handle, deadline) {
            fatal(b"LogOS vNext: sleep deadline");
        }
        local.pending_action.store(ACTION_TIMED_BLOCK, Ordering::Release);
        asm!("sti; int 49");
    }
}

fn request_switch(action: u64) {
    unsafe {
        let local = &*(read_gs() as *const CpuLocal);
        // Keep the action publication and software interrupt adjacent. STI's
        // one-instruction interrupt shadow prevents a timer from observing a
        // half-published voluntary transition, while the pushed flags retain
        // interrupts enabled for the resumed task.
        asm!("cli");
        local.pending_action.store(action, Ordering::Release);
        asm!("sti; int 49");
    }
}

fn read_gs() -> usize {
    let value: usize;
    unsafe { asm!("mov {}, gs:0", out(reg) value, options(nomem, nostack, preserves_flags)) };
    value
}

#[cfg_attr(not(feature = "qemu-proof"), allow(dead_code))]
pub(crate) fn current_cpu() -> usize {
    unsafe { (*(read_gs() as *const CpuLocal)).cpu_index as usize }
}

pub(crate) fn current_ticks() -> u64 {
    TIMER_TICKS.load(Ordering::Acquire)
}

global_asm!(
    ".section .text.ap_trampoline,\"ax\"",
    ".code16",
    ".global ap_trampoline_start",
    "ap_trampoline_start:",
    "cli",
    "xor ax, ax",
    "mov ds, ax",
    "mov esp, 0x7000",
    "mov ebp, 0x8000",
    "mov eax, 0",
    ".global ap_cr3_patch",
    "ap_cr3_patch:",
    "mov cr3, eax",
    "lgdt [ebp + 0x830]",
    "mov eax, cr4",
    "or eax, 0x20",
    "mov cr4, eax",
    "mov ecx, 0xc0000080",
    "rdmsr",
    "or eax, 0x100",
    "wrmsr",
    "mov eax, cr0",
    "or eax, 0x80000001",
    "mov cr0, eax",
    ".byte 0x66, 0xea",
    ".long 0x8000 + (ap_trampoline_protected - ap_trampoline_start)",
    ".word 0x08",
    ".code32",
    "ap_trampoline_protected:",
    "mov ax, 0x18",
    "mov ds, ax",
    "mov es, ax",
    "mov ss, ax",
    ".byte 0xea",
    ".long 0x8000 + (ap_trampoline_long - ap_trampoline_start)",
    ".word 0x10",
    ".code64",
    "ap_trampoline_long:",
    "mov rsp, [rbp + 0x810]",
    "mov rax, [rbp + 0x818]",
    "mov rdx, rax",
    "shr rdx, 32",
    "mov ecx, 0xc0000101",
    "wrmsr",
    "mov edi, [rbp + 0x820]",
    "mov rax, [rbp + 0x808]",
    "jmp rax",
    ".global ap_trampoline_end",
    "ap_trampoline_end:",
    ".section .text",
);

global_asm!(
    ".global task_bootstrap",
    "task_bootstrap:",
    "mov rsp, r15",
    "and rsp, -16",
    "sub rsp, 40",
    "call task_trampoline",
    "1:",
    "hlt",
    "jmp 1b",
);

global_asm!(
    ".global default_interrupt",
    "default_interrupt:",
    "cli",
    "mov dx, 0xe9",
    "mov al, 'F'",
    "out dx, al",
    "mov al, 'A'",
    "out dx, al",
    "mov al, 'U'",
    "out dx, al",
    "mov al, 'L'",
    "out dx, al",
    "mov al, 'T'",
    "out dx, al",
    "mov al, 13",
    "out dx, al",
    "mov al, 10",
    "out dx, al",
    "1:",
    "hlt",
    "jmp 1b",
    ".global context_timer_interrupt",
    "context_timer_interrupt:",
    "push 32",
    "jmp context_common",
    ".global keyboard_interrupt",
    "keyboard_interrupt:",
    "push 33",
    "jmp context_common",
    ".global context_switch_interrupt",
    "context_switch_interrupt:",
    "push 49",
    "jmp context_common",
    ".global user_fault_no_error",
    "user_fault_no_error:",
    "push 6",
    "jmp context_common",
    ".global user_gp_fault_error",
    "user_gp_fault_error:",
    "add rsp, 8",
    "push 13",
    "jmp context_common",
    ".global user_pf_fault_error",
    "user_pf_fault_error:",
    "add rsp, 8",
    "push 14",
    "jmp context_common",
    "context_common:",
    "push rax",
    "push rcx",
    "push rdx",
    "push rbx",
    "push rbp",
    "push rsi",
    "push rdi",
    "push r8",
    "push r9",
    "push r10",
    "push r11",
    "push r12",
    "push r13",
    "push r14",
    "push r15",
    "mov r11, rsp",
    "sub rsp, 520",
    "and rsp, -16",
    "fxsave64 [rsp]",
    "mov [rsp + 512], r11",
    "mov r10, rsp",
    "mov rsp, gs:8",
    "sub rsp, 40",
    "mov rcx, r10",
    "mov rdx, gs:24",
    "mov r8, [r11 + 120]",
    "call schedule_from_interrupt",
    "add rsp, 40",
    "mov rsp, rax",
    "fxrstor64 [rsp]",
    "mov rsp, [rsp + 512]",
    "pop r15",
    "pop r14",
    "pop r13",
    "pop r12",
    "pop r11",
    "pop r10",
    "pop r9",
    "pop r8",
    "pop rdi",
    "pop rsi",
    "pop rbp",
    "pop rbx",
    "pop rdx",
    "pop rcx",
    "pop rax",
    "add rsp, 8",
    "iretq",
);

#[cfg(feature = "qemu-proof")]
global_asm!(
    ".global proof_task_a",
    "proof_task_a:",
    "mov r12, 0x1111222233334444",
    "mov r13, 0x5555666677778888",
    "mov r14, 0x9999aaaabbbbcccc",
    "mov r15, 0xddddeeeeffff0000",
    "mov rax, 0x0123456789abcdef",
    "movq xmm6, rax",
    "1:",
    "stc",
    "mov ecx, 0x1000",
    "2:",
    "pause",
    "loop 2b",
    "pushfq",
    "pop rax",
    "test al, 1",
    "jz 9f",
    "mov rax, 0x1111222233334444",
    "cmp r12, rax",
    "jne 9f",
    "mov rax, 0x5555666677778888",
    "cmp r13, rax",
    "jne 9f",
    "mov rax, 0x9999aaaabbbbcccc",
    "cmp r14, rax",
    "jne 9f",
    "mov rax, 0xddddeeeeffff0000",
    "cmp r15, rax",
    "jne 9f",
    "movq rax, xmm6",
    "mov r11, 0x0123456789abcdef",
    "cmp rax, r11",
    "jne 9f",
    "call proof_a_progress",
    "jmp 1b",
    "9:",
    "call proof_fail",
    "hlt",
    "jmp 9b",
    ".global proof_task_b",
    "proof_task_b:",
    "mov r12, 0xaaaabbbbccccdddd",
    "mov r13, 0x1111222233334444",
    "mov r14, 0x5555666677778888",
    "mov r15, 0x9999aaaabbbbcccc",
    "mov rax, 0xfedcba9876543210",
    "movq xmm6, rax",
    "3:",
    "clc",
    "mov ecx, 0x1000",
    "4:",
    "pause",
    "loop 4b",
    "pushfq",
    "pop rax",
    "test al, 1",
    "jnz 10f",
    "mov rax, 0xaaaabbbbccccdddd",
    "cmp r12, rax",
    "jne 10f",
    "mov rax, 0x1111222233334444",
    "cmp r13, rax",
    "jne 10f",
    "mov rax, 0x5555666677778888",
    "cmp r14, rax",
    "jne 10f",
    "mov rax, 0x9999aaaabbbbcccc",
    "cmp r15, rax",
    "jne 10f",
    "movq rax, xmm6",
    "mov r11, 0xfedcba9876543210",
    "cmp rax, r11",
    "jne 10f",
    "call proof_b_progress",
    "jmp 3b",
    "10:",
    "call proof_fail",
    "hlt",
    "jmp 10b",
);
