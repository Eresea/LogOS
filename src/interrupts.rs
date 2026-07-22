use core::{
    arch::{asm, global_asm},
    sync::atomic::{AtomicU64, Ordering},
};

const TIMER_VECTOR: usize = 32;
const KEYBOARD_VECTOR: usize = 33;
const EXCEPTIONS: usize = 32;
const PIT_HZ: u16 = 100;
const PIT_DIVISOR: u16 = (1_193_182u32 / PIT_HZ as u32) as u16;

static TICKS: AtomicU64 = AtomicU64::new(0);
static mut IDT: [IdtEntry; 256] = [IdtEntry::MISSING; 256];

unsafe extern "C" {
    fn default_interrupt();
    fn fatal_interrupt();
    fn timer_interrupt();
    fn keyboard_irq();
}

#[unsafe(no_mangle)]
extern "C" fn timer_tick() {
    TICKS.fetch_add(1, Ordering::Relaxed);
}

pub fn install() -> bool {
    unsafe {
        let idt = core::ptr::addr_of_mut!(IDT);
        let selector = code_selector();
        for vector in 0..256 {
            (*idt)[vector] = IdtEntry::new(default_interrupt as *const () as usize, selector);
        }
        for vector in 0..EXCEPTIONS {
            (*idt)[vector] = IdtEntry::new(fatal_interrupt as *const () as usize, selector);
        }
        (*idt)[TIMER_VECTOR] = IdtEntry::new(timer_interrupt as *const () as usize, selector);
        (*idt)[KEYBOARD_VECTOR] = IdtEntry::new(keyboard_irq as *const () as usize, selector);
        load_idt(idt);
        configure_pic();
        configure_pit();
    }
    crate::keyboard::enable_interrupts()
}

pub fn enable() {
    unsafe { asm!("sti") };
}

pub fn wait_for_tick() {
    let tick = TICKS.load(Ordering::Acquire);
    while TICKS.load(Ordering::Acquire) == tick {
        unsafe { asm!("hlt") };
    }
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

global_asm!(
    ".global default_interrupt",
    "default_interrupt:",
    "iretq",
    ".global fatal_interrupt",
    "fatal_interrupt:",
    "cli",
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
);
