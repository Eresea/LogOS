#![no_main]
#![no_std]

mod capabilities;
mod console;
mod debug;
mod health;
mod interrupts;
mod ipc;
mod keyboard;
mod memory;
mod pci;
mod scheduler;
mod services;
mod trace;
mod virtio;
mod virtual_memory;

use uefi::{
    boot,
    mem::memory_map::MemoryMap,
    prelude::*,
    proto::console::gop::{BltOp, BltPixel, GraphicsOutput},
};

const BACKGROUND: BltPixel = BltPixel::new(12, 18, 30);
const ACCENT: BltPixel = BltPixel::new(61, 220, 151);

#[entry]
fn main() -> Status {
    debug::write_line(b"LogOS: kernel entered");
    let boot_info = match draw_boot_screen() {
        Ok(info) => info,
        Err(_) => return Status::DEVICE_ERROR,
    };
    debug::write_line(b"LogOS: leaving UEFI boot services");

    let memory_map = unsafe { boot::exit_boot_services(None) };
    kernel_main(boot_info, memory_map)
}

struct BootInfo {
    framebuffer: *mut u8,
    framebuffer_size: usize,
    resolution: (usize, usize),
    stride: usize,
}

fn draw_boot_screen() -> uefi::Result<BootInfo> {
    let graphics_handle = boot::get_handle_for_protocol::<GraphicsOutput>()?;
    let mut gop = boot::open_protocol_exclusive::<GraphicsOutput>(graphics_handle)?;
    let mode = gop.current_mode_info();
    let (width, height) = mode.resolution();
    let mut terminal = Terminal::new(&mut gop, width, height);
    terminal.reset()?;
    terminal.write(b"KERNEL\n")?;
    let mut framebuffer = gop.frame_buffer();
    Ok(BootInfo {
        framebuffer: framebuffer.as_mut_ptr(),
        framebuffer_size: framebuffer.size(),
        resolution: (width, height),
        stride: mode.stride(),
    })
}

fn kernel_main(boot_info: BootInfo, memory_map: impl MemoryMap) -> ! {
    let health = health::Startup::new();
    trace::record(trace::Event::Boot);
    let framebuffer_ok = !boot_info.framebuffer.is_null()
        && boot_info.framebuffer_size > 0
        && boot_info.resolution.0 > 0
        && boot_info.resolution.1 > 0;
    health.check(b"framebuffer", framebuffer_ok);
    let Some(mut startup) = console::Startup::new(
        boot_info.framebuffer,
        boot_info.resolution.0,
        boot_info.resolution.1,
        boot_info.stride,
    ) else {
        health.fail(b"console");
    };
    startup.start();
    macro_rules! check {
        ($module:expr, $passed:expr $(,)?) => {{
            let passed = $passed;
            startup.check($module, passed);
            health.check($module, passed);
        }};
    }
    macro_rules! fail {
        ($module:expr) => {{
            startup.check($module, false);
            health.fail($module);
        }};
    }
    check!(b"debug", true);
    check!(b"framebuffer", framebuffer_ok);
    let memory_regions = memory_map.len();
    let Some(mut memory) = memory::PhysicalMemory::from_memory_map(&memory_map) else {
        fail!(b"memory");
    };
    let Some(first_page) = memory.allocate_page() else {
        fail!(b"memory");
    };
    check!(b"memory", first_page & 0xfff == 0 && memory::self_check());
    let Some(mapped_page) = virtual_memory::install(&mut memory) else {
        fail!(b"virtual memory");
    };
    let _ = (
        boot_info.framebuffer,
        boot_info.framebuffer_size,
        boot_info.resolution,
        boot_info.stride,
        memory_regions,
        first_page,
        mapped_page,
    );
    check!(b"virtual memory", unsafe { virtual_memory::verify(mapped_page) });
    let keyboard_interrupts = interrupts::install();
    interrupts::enable();
    interrupts::wait_for_tick();
    check!(b"interrupts", keyboard_interrupts);
    let mut scheduler = scheduler::Scheduler::new();
    check!(b"scheduler", scheduler::self_check());
    let mut task_a = scheduler::Task::new(task_a);
    let mut task_b = scheduler::Task::new(task_b);
    if !scheduler.spawn(&mut task_a) || !scheduler.spawn(&mut task_b) {
        fail!(b"scheduler");
    }
    while scheduler.run_next() {
        interrupts::wait_for_tick();
    }
    let mut capabilities = capabilities::CapabilityManager::new();
    let Some(debug_capability) = capabilities.grant(capabilities::CapabilityKind::Debug) else {
        fail!(b"capabilities");
    };
    check!(
        b"capabilities",
        capabilities.allows(debug_capability, capabilities::CapabilityKind::Debug)
            && capabilities.revoke(debug_capability)
            && !capabilities.allows(debug_capability, capabilities::CapabilityKind::Debug),
    );
    let devices = pci::scan();
    let Some(first_device) = devices.first() else {
        fail!(b"pci");
    };
    check!(b"pci", devices.len() > 0);
    let _ = (first_device.location(), first_device.vendor_id(), first_device.device_id());
    let Some(virtio) = devices.find(0x1af4, 0x1002) else {
        fail!(b"virtio");
    };
    let Some(service_capability) = capabilities.grant(capabilities::CapabilityKind::Service) else {
        fail!(b"capabilities");
    };
    let mut services = services::Registry::new();
    let Some(virtio_handle) =
        services.register(&capabilities, service_capability, services::Service::VirtioBalloon)
    else {
        fail!(b"services");
    };
    check!(b"services", services.resolve(services::Service::VirtioBalloon) == Some(virtio_handle),);
    let Some(virtio_service) = virtio::VirtioService::bind(virtio, virtio_handle, &mut memory)
    else {
        fail!(b"virtio");
    };
    let channel = ipc::Channel::new();
    let responses = ipc::Channel::new();
    if !channel.send(&capabilities, service_capability, virtio_handle, ipc::Message::Ping) {
        fail!(b"ipc");
    }
    let mut service_task = virtio::ServiceTask::new(
        &virtio_service,
        &channel,
        &responses,
        &capabilities,
        service_capability,
        &mut memory,
    );
    let mut service_scheduler = scheduler::Scheduler::new();
    if !service_scheduler.spawn(&mut service_task) || !service_scheduler.run_next() {
        fail!(b"scheduler");
    }
    check!(b"ipc", responses.receive().is_some_and(|reply| reply.message == ipc::Message::Pong),);
    check!(
        b"service task",
        channel.send(&capabilities, service_capability, virtio_handle, ipc::Message::Ping)
            && service_scheduler.run_next()
            && responses.receive().is_some_and(|reply| reply.message == ipc::Message::Pong),
    );
    check!(
        b"virtio",
        channel.send(&capabilities, service_capability, virtio_handle, ipc::Message::Inflate)
            && service_scheduler.run_next()
            && {
                interrupts::wait_for_virtio();
                service_scheduler.wake(0)
            }
            && service_scheduler.run_next()
            && responses.receive().is_some_and(|reply| reply.message == ipc::Message::Complete),
    );
    check!(b"console", true);
    check!(b"keyboard", keyboard::self_check());
    check!(b"trace", trace::self_check());
    let mut console = console::Shell::from_startup(
        startup,
        console::Endpoint::new(
            &channel,
            &responses,
            &capabilities,
            service_capability,
            virtio_handle,
        ),
    );
    let _ = console.start();
    health.finish();
    interrupts::disable_timer();
    console.run(|| {
        if virtio::completion_pending() {
            let _ = service_scheduler.wake(0);
        }
        let _ = service_scheduler.run_next();
    })
}

fn task_a(task: &mut scheduler::Task) -> scheduler::TaskState {
    if task.runs() == 1 {
        debug::write_line(b"LogOS: task A yielded");
        scheduler::TaskState::Ready
    } else {
        debug::write_line(b"LogOS: task A complete");
        scheduler::TaskState::Complete
    }
}

fn task_b(_: &mut scheduler::Task) -> scheduler::TaskState {
    debug::write_line(b"LogOS: task B complete");
    scheduler::TaskState::Complete
}

struct Terminal<'a> {
    gop: &'a mut GraphicsOutput,
    cursor: (usize, usize),
    width: usize,
    height: usize,
}

impl<'a> Terminal<'a> {
    const ORIGIN: (usize, usize) = (32, 136);
    const SCALE: usize = 3;

    fn new(gop: &'a mut GraphicsOutput, width: usize, height: usize) -> Self {
        Self { gop, cursor: Self::ORIGIN, width, height }
    }

    fn reset(&mut self) -> uefi::Result {
        self.fill(BACKGROUND, (0, 0), (self.width, self.height))?;
        self.fill(ACCENT, (32, 32), (self.width.saturating_sub(64), 80))?;
        self.cursor = (56, 48);
        self.write_with_color(b"LOGOS", BACKGROUND)?;
        self.cursor = Self::ORIGIN;
        Ok(())
    }

    fn write(&mut self, text: &[u8]) -> uefi::Result {
        self.write_with_color(text, ACCENT)
    }

    fn write_with_color(&mut self, text: &[u8], color: BltPixel) -> uefi::Result {
        for &byte in text {
            if byte == b'\n' {
                self.newline();
            } else {
                self.draw_glyph(byte.to_ascii_uppercase(), color)?;
            }
        }
        Ok(())
    }

    fn newline(&mut self) {
        self.cursor = (Self::ORIGIN.0, self.cursor.1 + 8 * Self::SCALE);
        if self.cursor.1 + 7 * Self::SCALE > self.height {
            self.cursor = Self::ORIGIN;
        }
    }

    fn draw_glyph(&mut self, byte: u8, color: BltPixel) -> uefi::Result {
        if self.cursor.0 + 5 * Self::SCALE > self.width.saturating_sub(32) {
            self.newline();
        }
        let glyph = glyph(byte).ok_or(Status::UNSUPPORTED)?;
        for (row, &bits) in glyph.iter().enumerate() {
            for column in 0..5 {
                if bits & (1 << (4 - column)) != 0 {
                    self.fill(
                        color,
                        (self.cursor.0 + column * Self::SCALE, self.cursor.1 + row * Self::SCALE),
                        (Self::SCALE, Self::SCALE),
                    )?;
                }
            }
        }
        self.cursor.0 += 6 * Self::SCALE;
        Ok(())
    }

    fn fill(
        &mut self,
        color: BltPixel,
        dest: (usize, usize),
        dims: (usize, usize),
    ) -> uefi::Result {
        self.gop.blt(BltOp::VideoFill { color, dest, dims })
    }
}

pub(crate) fn glyph(byte: u8) -> Option<&'static [u8; 7]> {
    const A: [u8; 7] = [0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0];
    const B: [u8; 7] = [0b11110, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110, 0];
    const C: [u8; 7] = [0b01111, 0b10000, 0b10000, 0b10000, 0b10000, 0b01111, 0];
    const D: [u8; 7] = [0b11110, 0b10001, 0b10001, 0b10001, 0b10001, 0b11110, 0];
    const E: [u8; 7] = [0b11111, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111, 0];
    const F: [u8; 7] = [0b11111, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000, 0];
    const G: [u8; 7] = [0b01110, 0b10000, 0b10111, 0b10001, 0b10001, 0b01110, 0];
    const H: [u8; 7] = [0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001, 0];
    const I: [u8; 7] = [0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b11111, 0];
    const K: [u8; 7] = [0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001];
    const L: [u8; 7] = [0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111, 0];
    const M: [u8; 7] = [0b10001, 0b11011, 0b10101, 0b10001, 0b10001, 0b10001, 0];
    const N: [u8; 7] = [0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001, 0];
    const O: [u8; 7] = [0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110, 0];
    const P: [u8; 7] = [0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0];
    const R: [u8; 7] = [0b11110, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001, 0];
    const S: [u8; 7] = [0b01111, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110, 0];
    const T: [u8; 7] = [0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0];
    const U: [u8; 7] = [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110, 0];
    const V: [u8; 7] = [0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100, 0];
    const W: [u8; 7] = [0b10001, 0b10001, 0b10001, 0b10101, 0b11011, 0b10001, 0];
    const X: [u8; 7] = [0b10001, 0b01010, 0b00100, 0b00100, 0b01010, 0b10001, 0];
    const Y: [u8; 7] = [0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0b00100, 0];
    const ZERO: [u8; 7] = [0b01110, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110, 0];
    const ONE: [u8; 7] = [0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b01110, 0];
    const SPACE: [u8; 7] = [0; 7];
    const PROMPT: [u8; 7] = [0b10000, 0b01000, 0b00100, 0b00010, 0b00100, 0b01000, 0b10000];

    match byte {
        b'A' => Some(&A),
        b'B' => Some(&B),
        b'C' => Some(&C),
        b'D' => Some(&D),
        b'E' => Some(&E),
        b'F' => Some(&F),
        b'G' => Some(&G),
        b'H' => Some(&H),
        b'I' => Some(&I),
        b'K' => Some(&K),
        b'L' => Some(&L),
        b'M' => Some(&M),
        b'N' => Some(&N),
        b'O' => Some(&O),
        b'P' => Some(&P),
        b'R' => Some(&R),
        b'S' => Some(&S),
        b'T' => Some(&T),
        b'U' => Some(&U),
        b'V' => Some(&V),
        b'W' => Some(&W),
        b'X' => Some(&X),
        b'Y' => Some(&Y),
        b'0' => Some(&ZERO),
        b'1' => Some(&ONE),
        b' ' => Some(&SPACE),
        b'>' => Some(&PROMPT),
        _ => None,
    }
}
