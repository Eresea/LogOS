use super::writable::Writable;
use core::{
    arch::{asm, global_asm},
    sync::atomic::{AtomicU64, AtomicUsize, Ordering},
};

const TIMER_VECTOR: usize = 32;
const KEYBOARD_VECTOR: usize = 33;
const VIRTIO_VECTOR: usize = 48;
const SERVICE_GATE_VECTOR: usize = 0x80;
const EXCEPTIONS: usize = 32;
const PIT_HZ: u16 = 100;
const PIT_DIVISOR: u16 = (1_193_182u32 / PIT_HZ as u32) as u16;

static TICKS: AtomicU64 = AtomicU64::new(0);
static LOCAL_APIC: AtomicUsize = AtomicUsize::new(0);
static IO_APIC: AtomicUsize = AtomicUsize::new(0);
static IO_APIC_GSI_BASE: AtomicUsize = AtomicUsize::new(0);
static IDT: Writable<[IdtEntry; 256]> = Writable::new([IdtEntry::MISSING; 256]);

unsafe extern "C" {
    fn default_interrupt();
    fn timer_interrupt();
    fn keyboard_irq();
    fn virtio_irq();
    fn user_gate();
    static exception_stub_table: [usize; EXCEPTIONS];
}

#[unsafe(no_mangle)]
extern "C" fn timer_tick() {
    TICKS.fetch_add(1, Ordering::Relaxed);
}

#[unsafe(no_mangle)]
extern "C" fn virtio_interrupt() {
    crate::drivers::virtio::interrupt();
    crate::drivers::block::interrupt();
    crate::drivers::network::interrupt();
    let local_apic = LOCAL_APIC.load(Ordering::Acquire);
    unsafe { core::ptr::write_volatile((local_apic + 0xb0) as *mut u32, 0) };
}

#[unsafe(no_mangle)]
extern "C" fn fatal_fault() {
    crate::platform::trace::record(crate::platform::trace::Event::Fault);
}

pub fn install(madt: crate::arch::acpi::Madt) -> bool {
    if madt.local_apic == 0 || madt.io_apic == 0 {
        return false;
    }
    LOCAL_APIC.store(madt.local_apic, Ordering::Release);
    IO_APIC.store(madt.io_apic, Ordering::Release);
    IO_APIC_GSI_BASE.store(madt.io_apic_gsi_base as usize, Ordering::Release);
    unsafe {
        let idt = IDT.get();
        let selector = code_selector();
        for vector in 0..256 {
            (*idt)[vector] = IdtEntry::new(default_interrupt as *const () as usize, selector);
        }
        for (vector, handler) in exception_stub_table.iter().copied().enumerate() {
            (*idt)[vector] = IdtEntry::new(handler, selector);
        }
        (*idt)[TIMER_VECTOR] = IdtEntry::new(timer_interrupt as *const () as usize, selector);
        (*idt)[KEYBOARD_VECTOR] = IdtEntry::new(keyboard_irq as *const () as usize, selector);
        (*idt)[VIRTIO_VECTOR] = IdtEntry::new(virtio_irq as *const () as usize, selector);
        (*idt)[SERVICE_GATE_VECTOR] =
            IdtEntry::new(user_gate as *const () as usize, selector).user_callable();
        load_idt(idt);
        configure_pic();
        configure_pit();
    }
    crate::drivers::keyboard::enable_interrupts()
}

pub fn route_virtio(gsi: u32) -> bool {
    let gsi = gsi as usize;
    let base = IO_APIC_GSI_BASE.load(Ordering::Acquire);
    let Some(pin) = gsi.checked_sub(base) else {
        return false;
    };
    unsafe {
        if pin > ((ioapic_read(1) >> 16) & 0xff) as usize {
            return false;
        }
        ioapic_write(0x10 + (pin as u8) * 2, VIRTIO_VECTOR as u32);
        ioapic_write(0x11 + (pin as u8) * 2, 0);
    }
    true
}

pub fn enable() {
    unsafe { asm!("sti") };
}

pub fn ticks() -> u64 {
    TICKS.load(Ordering::Acquire)
}

pub fn wait_for_tick() {
    let tick = TICKS.load(Ordering::Acquire);
    while TICKS.load(Ordering::Acquire) == tick {
        unsafe { asm!("hlt") };
    }
}

pub fn wait_for_virtio() {
    wait_for_tick();
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct IdtEntry {
    offset_low: u16,
    selector: u16,
    ist: u8,
    attributes: u8,
    offset_middle: u16,
    offset_high: u32,
    reserved: u32,
}

impl IdtEntry {
    const MISSING: Self = Self {
        offset_low: 0,
        selector: 0,
        ist: 0,
        attributes: 0,
        offset_middle: 0,
        offset_high: 0,
        reserved: 0,
    };

    fn new(handler: usize, selector: u16) -> Self {
        Self {
            offset_low: handler as u16,
            selector,
            ist: 0,
            attributes: 0x8e,
            offset_middle: (handler >> 16) as u16,
            offset_high: (handler >> 32) as u32,
            reserved: 0,
        }
    }

    fn user_callable(mut self) -> Self {
        self.attributes = 0xee;
        self
    }
}

#[repr(C, packed)]
struct IdtPointer {
    limit: u16,
    base: u64,
}

unsafe fn code_selector() -> u16 {
    let selector: u16;
    unsafe { asm!("mov {0:x}, cs", out(reg) selector) };
    selector
}

unsafe fn load_idt(idt: *mut [IdtEntry; 256]) {
    let pointer = IdtPointer {
        limit: (core::mem::size_of::<[IdtEntry; 256]>() - 1) as u16,
        base: idt as u64,
    };
    unsafe { asm!("lidt [{}]", in(reg) &pointer) };
}

unsafe fn configure_pic() {
    unsafe {
        outb(0x20, 0x11);
        outb(0xa0, 0x11);
        outb(0x21, 0x20);
        outb(0xa1, 0x28);
        outb(0x21, 0x04);
        outb(0xa1, 0x02);
        outb(0x21, 0x01);
        outb(0xa1, 0x01);
        outb(0x21, 0xfc);
        outb(0xa1, 0xff);
    }
}

unsafe fn configure_pit() {
    unsafe {
        outb(0x43, 0x36);
        outb(0x40, PIT_DIVISOR as u8);
        outb(0x40, (PIT_DIVISOR >> 8) as u8);
    }
}

unsafe fn outb(port: u16, value: u8) {
    unsafe { asm!("out dx, al", in("dx") port, in("al") value) };
}

unsafe fn ioapic_write(register: u8, value: u32) {
    let select = IO_APIC.load(Ordering::Acquire) as *mut u32;
    unsafe {
        select.write_volatile(register as u32);
        select.add(4).write_volatile(value);
    }
}

unsafe fn ioapic_read(register: u8) -> u32 {
    let select = IO_APIC.load(Ordering::Acquire) as *mut u32;
    unsafe {
        select.write_volatile(register as u32);
        select.add(4).read_volatile()
    }
}

global_asm!(
    ".global default_interrupt",
    "default_interrupt:",
    "iretq",
    ".global exception_stub_table",
    "exception_stub_table:",
    ".quad exception_0, exception_1, exception_2, exception_3",
    ".quad exception_4, exception_5, exception_6, exception_7",
    ".quad exception_8, exception_9, exception_10, exception_11",
    ".quad exception_12, exception_13, exception_14, exception_15",
    ".quad exception_16, exception_17, exception_18, exception_19",
    ".quad exception_20, exception_21, exception_22, exception_23",
    ".quad exception_24, exception_25, exception_26, exception_27",
    ".quad exception_28, exception_29, exception_30, exception_31",
    "exception_0: push 0; push 0; jmp exception_common",
    "exception_1: push 0; push 1; jmp exception_common",
    "exception_2: push 0; push 2; jmp exception_common",
    "exception_3: push 0; push 3; jmp exception_common",
    "exception_4: push 0; push 4; jmp exception_common",
    "exception_5: push 0; push 5; jmp exception_common",
    "exception_6: push 0; push 6; jmp exception_common",
    "exception_7: push 0; push 7; jmp exception_common",
    "exception_8: push 8; jmp exception_common",
    "exception_9: push 0; push 9; jmp exception_common",
    "exception_10: push 10; jmp exception_common",
    "exception_11: push 11; jmp exception_common",
    "exception_12: push 12; jmp exception_common",
    "exception_13: push 13; jmp exception_common",
    "exception_14: push 14; jmp exception_common",
    "exception_15: push 0; push 15; jmp exception_common",
    "exception_16: push 0; push 16; jmp exception_common",
    "exception_17: push 17; jmp exception_common",
    "exception_18: push 0; push 18; jmp exception_common",
    "exception_19: push 0; push 19; jmp exception_common",
    "exception_20: push 0; push 20; jmp exception_common",
    "exception_21: push 21; jmp exception_common",
    "exception_22: push 0; push 22; jmp exception_common",
    "exception_23: push 0; push 23; jmp exception_common",
    "exception_24: push 0; push 24; jmp exception_common",
    "exception_25: push 0; push 25; jmp exception_common",
    "exception_26: push 0; push 26; jmp exception_common",
    "exception_27: push 0; push 27; jmp exception_common",
    "exception_28: push 0; push 28; jmp exception_common",
    "exception_29: push 29; jmp exception_common",
    "exception_30: push 30; jmp exception_common",
    "exception_31: push 0; push 31; jmp exception_common",
    "exception_common:",
    "cli",
    "mov rax, [rsp + 24]",
    "and eax, 3",
    "cmp eax, 3",
    "jne fatal_interrupt",
    "mov rcx, [rsp]",
    "mov rdx, [rsp + 8]",
    "mov r8, [rsp + 16]",
    "mov rax, cr2",
    "mov r9, rax",
    "sub rsp, 40",
    "call user_fault",
    "add rsp, 40",
    "jmp user_fault_exit",
    "fatal_interrupt:",
    "sub rsp, 40",
    "call fatal_fault",
    "add rsp, 40",
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
    "mov al, ' ' ",
    "out dx, al",
    "mov al, 'H'",
    "out dx, al",
    "mov al, 'A'",
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
    ".global timer_interrupt",
    "timer_interrupt:",
    "push rax",
    "push rcx",
    "push rdx",
    "push r8",
    "push r9",
    "push r10",
    "push r11",
    "sub rsp, 40",
    "call timer_tick",
    "add rsp, 40",
    "mov al, 0x20",
    "out 0x20, al",
    "pop r11",
    "pop r10",
    "pop r9",
    "pop r8",
    "pop rdx",
    "pop rcx",
    "pop rax",
    "iretq",
    ".global keyboard_irq",
    "keyboard_irq:",
    "push rax",
    "push rcx",
    "push rdx",
    "push r8",
    "push r9",
    "push r10",
    "push r11",
    "sub rsp, 40",
    "call keyboard_interrupt",
    "add rsp, 40",
    "mov al, 0x20",
    "out 0x20, al",
    "pop r11",
    "pop r10",
    "pop r9",
    "pop r8",
    "pop rdx",
    "pop rcx",
    "pop rax",
    "iretq",
    ".global virtio_irq",
    "virtio_irq:",
    "push rax",
    "push rcx",
    "push rdx",
    "push r8",
    "push r9",
    "push r10",
    "push r11",
    "sub rsp, 40",
    "call virtio_interrupt",
    "add rsp, 40",
    "pop r11",
    "pop r10",
    "pop r9",
    "pop r8",
    "pop rdx",
    "pop rcx",
    "pop rax",
    "iretq",
);
