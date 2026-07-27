#![no_main]
#![no_std]

mod acpi;
mod approvals;
mod audit;
mod capabilities;
mod commands;
mod console;
mod debug;
mod device;
mod entropy;
mod format;
mod health;
mod identity;
mod inference;
mod interrupts;
mod ipc;
mod keyboard;
mod memory;
mod mode;
mod pci;
mod platform;
mod resources;
mod scheduler;
mod secrets;
mod services;
mod session;
mod supervisor;
mod time;
mod trace;
mod virtual_memory;

use logos_terminal::{display, input, terminal, text};
use uefi::{boot, mem::memory_map::MemoryMap, prelude::*, proto::console::gop::GraphicsOutput};

#[entry]
fn main() -> Status {
    debug::write_line(b"LogOS: kernel entered");
    let entropy = entropy::load();
    entropy::announce(entropy);
    let machine = identity::load(entropy.as_ref());
    identity::announce(&machine);
    let wall_clock = time::wall_clock();
    time::announce(wall_clock);
    let boot_info = match boot_info() {
        Ok(info) => info,
        Err(_) => return Status::DEVICE_ERROR,
    };
    let acpi = acpi::discover();
    if let Some(tables) = acpi {
        tables.install_reset();
    }
    debug::write_line(b"LogOS: leaving UEFI boot services");

    let memory_map = unsafe { boot::exit_boot_services(None) };
    kernel_main(boot_info, memory_map, acpi, machine, wall_clock)
}

struct BootInfo {
    framebuffer: *mut u8,
    framebuffer_size: usize,
    resolution: (usize, usize),
    stride: usize,
}

fn boot_info() -> uefi::Result<BootInfo> {
    let graphics_handle = boot::get_handle_for_protocol::<GraphicsOutput>()?;
    let mut gop = boot::open_protocol_exclusive::<GraphicsOutput>(graphics_handle)?;
    let mode = gop.current_mode_info();
    let (width, height) = mode.resolution();
    let mut framebuffer = gop.frame_buffer();
    Ok(BootInfo {
        framebuffer: framebuffer.as_mut_ptr(),
        framebuffer_size: framebuffer.size(),
        resolution: (width, height),
        stride: mode.stride(),
    })
}

fn kernel_main(
    boot_info: BootInfo,
    memory_map: impl MemoryMap,
    acpi: Option<acpi::Tables>,
    machine: identity::Machine,
    wall_clock: time::WallClock,
) -> ! {
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
    macro_rules! check {
        ($module:expr, $passed:expr $(,)?) => {{
            let passed = $passed;
            health.check($module, passed);
        }};
    }
    macro_rules! fail {
        ($module:expr) => {{
            health.fail($module);
        }};
    }
    check!(b"debug", true);
    check!(
        b"machine identity",
        entropy::self_check() && identity::self_check() && machine.id() == machine.id(),
    );
    check!(b"secret store", secrets::self_check());
    check!(b"audit", audit::self_check());
    check!(b"approvals", approvals::self_check());
    check!(b"inference", inference::self_check());
    check!(b"resources", resources::self_check());
    check!(b"device interfaces", device::self_check());
    check!(b"framebuffer", framebuffer_ok);
    check!(
        b"acpi",
        acpi.is_some_and(|tables| {
            tables.xsdt != 0
                && tables.madt.is_some_and(|madt| {
                    madt.local_apic != 0 && madt.io_apic != 0 && madt.io_apic_gsi_base == 0
                })
                && tables.has_power()
        }),
    );
    let Some(madt) = acpi.and_then(|tables| tables.madt) else {
        fail!(b"acpi");
    };
    let memory_regions = memory_map.len();
    let Some(mut memory) = memory::PhysicalMemory::from_memory_map(&memory_map) else {
        fail!(b"memory");
    };
    let Some(first_page) = memory.allocate_page() else {
        fail!(b"memory");
    };
    check!(b"memory", first_page & 0xfff == 0 && memory::self_check());
    let Some(mapping) = virtual_memory::install(&mut memory, virtual_memory::Permission::ReadWrite)
    else {
        fail!(b"virtual memory");
    };
    let _ = (
        boot_info.framebuffer,
        boot_info.framebuffer_size,
        boot_info.resolution,
        boot_info.stride,
        memory_regions,
        first_page,
    );
    check!(
        b"virtual memory",
        mapping.is_writable()
            && unsafe { virtual_memory::verify(&mapping) }
            && mapping.release(&mut memory)
            && virtual_memory::install(&mut memory, virtual_memory::Permission::ReadOnly)
                .is_some_and(|mapping| !mapping.is_writable() && mapping.release(&mut memory)),
    );
    let keyboard_interrupts = interrupts::install(madt);
    interrupts::enable();
    interrupts::wait_for_tick();
    check!(b"interrupts", keyboard_interrupts);
    check!(
        b"time",
        time::self_check()
            && time::now().ticks() >= 1
            && matches!(wall_clock, time::WallClock::Unknown | time::WallClock::Untrusted { .. }),
    );
    let mut scheduler = scheduler::Scheduler::new();
    check!(b"scheduler", scheduler::self_check());
    let mut task_a = scheduler::Task::new(task_a);
    let mut task_b = scheduler::Task::new(task_b);
    if scheduler.spawn(&mut task_a).is_none() || scheduler.spawn(&mut task_b).is_none() {
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
    let Some(device) = platform::discover(&devices) else {
        fail!(b"platform device");
    };
    let Some(supervisor) = supervisor::boot_plan(supervisor::Profile::Normal).ok() else {
        fail!(b"supervisor manifest");
    };
    check!(
        b"supervisor manifest",
        supervisor::self_check() && supervisor.starts(supervisor::VIRTIO_BALLOON),
    );
    check!(b"service profiles", supervisor::profiles_self_check());
    check!(b"service dependency loss", supervisor::dependency_loss_self_check());
    check!(b"service startup failure", supervisor::startup_failure_self_check());
    let Some(service_protocol) = supervisor
        .negotiate(supervisor::VIRTIO_BALLOON, services::Service::VirtioBalloon.protocol())
    else {
        supervisor::report_start_failure(
            supervisor::VIRTIO_BALLOON,
            supervisor::StartStage::Protocol,
        );
        fail!(b"service protocol");
    };
    check!(
        b"service protocol",
        supervisor::protocol_self_check()
            && service_protocol == services::Service::VirtioBalloon.protocol(),
    );
    let Some(session_service_capability) =
        capabilities.grant(capabilities::CapabilityKind::Service)
    else {
        fail!(b"capabilities");
    };
    let Some(recovery_capability) = capabilities.grant(capabilities::CapabilityKind::Recovery)
    else {
        fail!(b"capabilities");
    };
    let Some(session) = session::Context::new(
        session::Id(1),
        session::Principal::LOCAL,
        &[recovery_capability, session_service_capability],
    ) else {
        fail!(b"session");
    };
    let Some(service_capability) = supervisor.grant(
        supervisor::VIRTIO_BALLOON,
        &mut capabilities,
        capabilities::CapabilityKind::Service,
    ) else {
        supervisor::report_start_failure(
            supervisor::VIRTIO_BALLOON,
            supervisor::StartStage::Capability,
        );
        fail!(b"service capability");
    };
    check!(
        b"service capability",
        supervisor::grant_self_check()
            && capabilities.allows(service_capability, capabilities::CapabilityKind::Service),
    );
    check!(b"service diagnostics", supervisor::diagnostics_self_check());
    let mut service_health = supervisor::Health::new();
    check!(
        b"service health",
        supervisor::health_self_check()
            && service_health.watch(
                &supervisor,
                supervisor::VIRTIO_BALLOON,
                100,
                interrupts::ticks(),
            ),
    );
    let Some(mut service_lifecycle) =
        supervisor::Lifecycle::new(&supervisor, supervisor::VIRTIO_BALLOON)
    else {
        fail!(b"service lifecycle");
    };
    check!(b"service lifecycle", supervisor::lifecycle_self_check());
    let mut services = services::Registry::new();
    let Some(virtio_handle) =
        services.register(&capabilities, service_capability, services::Service::VirtioBalloon)
    else {
        supervisor::report_start_failure(
            supervisor::VIRTIO_BALLOON,
            supervisor::StartStage::Register,
        );
        fail!(b"services");
    };
    check!(b"services", services.resolve(services::Service::VirtioBalloon) == Some(virtio_handle),);
    let Some(virtio_gsi) = acpi.and_then(|tables| {
        let (bus, slot, _) = device.location();
        tables.pci_gsi(bus, slot, device.interrupt_pin().checked_sub(1)?)
    }) else {
        fail!(b"acpi pci routing");
    };
    let Some(mut virtio_service) =
        platform::Service::bind(device, virtio_gsi, virtio_handle, &mut memory)
    else {
        supervisor::report_start_failure(supervisor::VIRTIO_BALLOON, supervisor::StartStage::Bind);
        fail!(b"virtio");
    };
    let channel = ipc::Channel::new();
    let responses = ipc::Channel::new();
    let Some(ping_request) = channel.send(
        &capabilities,
        service_capability,
        session::Principal::LOCAL,
        virtio_handle,
        ipc::Message::Ping,
    ) else {
        fail!(b"ipc");
    };
    {
        let mut service_task = platform::Task::new(
            &mut virtio_service,
            &channel,
            &responses,
            &capabilities,
            service_capability,
            virtio_handle.principal(),
            &mut memory,
        );
        let mut service_scheduler = scheduler::Scheduler::new();
        if service_scheduler.spawn(&mut service_task).is_none() {
            fail!(b"scheduler");
        }
        if !service_scheduler.run_next() {
            fail!(b"scheduler");
        }
        check!(
            b"ipc",
            responses.receive().is_some_and(|reply| {
                reply.message == ipc::Message::Pong && reply.request == ping_request
            }) && ipc::self_check(),
        );
        check!(
            b"service task",
            channel
                .send(
                    &capabilities,
                    service_capability,
                    session::Principal::LOCAL,
                    virtio_handle,
                    ipc::Message::Ping
                )
                .is_some()
                && service_scheduler.run_next()
                && responses.receive().is_some_and(|reply| reply.message == ipc::Message::Pong),
        );
        check!(
            b"virtio",
            channel
                .send(
                    &capabilities,
                    service_capability,
                    session::Principal::LOCAL,
                    virtio_handle,
                    ipc::Message::Inflate
                )
                .is_some()
                && service_scheduler.run_next()
                && {
                    interrupts::wait_for_virtio();
                    service_scheduler.wake_event(scheduler::Event::VIRTIO) > 0
                }
                && service_scheduler.run_next()
                && responses.receive().is_some_and(|reply| reply.message == ipc::Message::Complete),
        );
        check!(
            b"driver recovery",
            channel
                .send(
                    &capabilities,
                    service_capability,
                    session::Principal::LOCAL,
                    virtio_handle,
                    ipc::Message::Recover
                )
                .is_some()
                && service_scheduler.run_next()
                && responses.receive().is_some_and(|reply| reply.message == ipc::Message::Complete),
        );
        let Some(cancel_request) = channel.send(
            &capabilities,
            service_capability,
            session::Principal::LOCAL,
            virtio_handle,
            ipc::Message::Cancel,
        ) else {
            fail!(b"ipc cancel");
        };
        check!(
            b"ipc cancel",
            service_scheduler.run_next()
                && responses.receive().is_some_and(|reply| {
                    reply.message == ipc::Message::Failed && reply.request == cancel_request
                }),
        );
    }
    check!(b"service reclamation", virtio_service.resources_reclaimed());
    let mut replacement = None;
    check!(
        b"service replacement",
        supervisor::replacement_self_check()
            && supervisor.replace(supervisor::VIRTIO_BALLOON, || {
                if !virtio_service.release(&mut memory) {
                    return false;
                }
                replacement =
                    platform::Service::bind(device, virtio_gsi, virtio_handle, &mut memory);
                replacement.is_some()
            }),
    );
    let Some(mut virtio_service) = replacement else {
        fail!(b"service replacement");
    };
    let mut service_task = platform::Task::new(
        &mut virtio_service,
        &channel,
        &responses,
        &capabilities,
        service_capability,
        virtio_handle.principal(),
        &mut memory,
    );
    let mut service_scheduler = scheduler::Scheduler::new();
    if service_scheduler.spawn(&mut service_task).is_none() {
        supervisor::report_start_failure(supervisor::VIRTIO_BALLOON, supervisor::StartStage::Task);
        fail!(b"scheduler rebind");
    }
    let mut display = display::Service::new(
        boot_info.framebuffer,
        boot_info.framebuffer_size,
        boot_info.resolution.0,
        boot_info.resolution.1,
        boot_info.stride,
    );
    let mut input = input::Service::new();
    let _ = input.next(interrupts::ticks(), keyboard::poll_scancode);
    let text = text::Service::new();
    let mut probe = terminal::Model::new();
    let normal_ready = display.as_mut().is_some_and(|display| {
        display::Service::self_check()
            && display.present(0, 0, [0; 3])
            && keyboard::self_check()
            && input::Service::self_check()
            && text::Service::self_check()
            && terminal::Model::self_check()
            && probe.apply(input::Event::Key {
                physical: input::PhysicalKey(0x22),
                logical: input::LogicalKey::Text(b'g'),
                state: input::State::Press,
                modifiers: input::Modifiers::none(),
            })
            && probe.render(display, &text)
            && commands::self_check()
    });
    let mut terminal = terminal::Model::new();
    let coordinator = mode::Coordinator::new(normal_ready);
    check!(b"console mode", mode::Coordinator::self_check());
    check!(b"command registry", commands::self_check());
    check!(b"formatters", format::self_check());
    check!(b"session", session::Context::self_check());
    check!(b"input normalization", input::Service::self_check());
    check!(b"terminal editing", terminal::Model::self_check());
    check!(b"terminal navigation", terminal::Model::self_check());
    check!(b"terminal layout", terminal::Model::self_check());
    check!(b"terminal scrollback", terminal::Model::self_check());
    check!(b"terminal history", terminal::Model::self_check());
    check!(b"terminal selection", terminal::Model::self_check());
    check!(b"terminal output", terminal::Model::self_check());
    check!(b"terminal display restart", terminal::Model::self_check());
    check!(b"terminal caret", terminal::Model::self_check());
    check!(b"text font", text::Service::self_check());
    coordinator.announce();
    check!(b"trace", trace::self_check());
    health.finish();
    let mut console_mode = coordinator.mode();
    if console_mode == mode::ConsoleMode::Normal {
        debug::write_line(b"LogOS: normal terminal active");
        let _ = terminal.write_output(b"LogOS terminal");
        if !terminal.render(display.as_mut().unwrap(), &text) {
            debug::write_line(b"LogOS: normal terminal initial redraw failed");
            console_mode = mode::ConsoleMode::Recovery;
        }
        let mut blink_tick = interrupts::ticks();
        while console_mode == mode::ConsoleMode::Normal {
            let tick = interrupts::ticks();
            if service_lifecycle.due(tick) {
                let _ = channel.send(
                    &capabilities,
                    service_capability,
                    session::Principal::LOCAL,
                    virtio_handle,
                    ipc::Message::Recover,
                );
            }
            if platform::completion_pending() {
                let _ = service_scheduler.wake_event(scheduler::Event::VIRTIO);
            }
            if service_scheduler.run_next() {
                let _ = service_health.beat(supervisor::VIRTIO_BALLOON, tick);
            }
            if !service_health.healthy(supervisor::VIRTIO_BALLOON, tick) {
                debug::write_line(b"LogOS: service heartbeat overdue");
                let _ = service_lifecycle.failed(tick);
            }
            if tick.wrapping_sub(blink_tick) >= 50 {
                terminal.blink();
                if !terminal.render_caret(display.as_mut().unwrap(), &text) {
                    console_mode = mode::ConsoleMode::Recovery;
                    break;
                }
                blink_tick = tick;
            }
            if let Some(event) = input.next(tick, keyboard::poll_scancode) {
                if event.is_enter() {
                    let submission = terminal.submit();
                    let _ = terminal.write_output(submission.as_bytes());
                    match commands::pipeline(
                        submission,
                        &session,
                        &capabilities,
                        commands::Invocation::new(tick.wrapping_add(50)),
                        tick,
                    ) {
                        commands::Outcome::Recovery => {
                            debug::write_line(b"LogOS: recovery handoff requested");
                            console_mode = mode::ConsoleMode::Recovery;
                            break;
                        }
                        commands::Outcome::Reboot => {
                            if !acpi::reset() {
                                let _ = terminal.write_output(b"reboot unavailable");
                            }
                        }
                        commands::Outcome::PowerOff => {
                            if !acpi::power_off() {
                                let _ = terminal.write_output(b"poweroff unavailable");
                            }
                        }
                        commands::Outcome::Clear => terminal.clear_output(),
                        commands::Outcome::Layout(layout) => {
                            input.set_layout(layout);
                            let _ = terminal.write_output(match layout {
                                input::Layout::Qwerty => b"layout qwerty",
                                input::Layout::Azerty => b"layout azerty",
                            });
                        }
                        commands::Outcome::Tasks => {
                            let _ = terminal.write_output(b"scheduler active");
                        }
                        commands::Outcome::Services => {
                            let _ = terminal.write_output(b"virtio-balloon running");
                        }
                        commands::Outcome::Drivers => {
                            let _ = terminal.write_output(b"virtio-balloon driver bound");
                        }
                        commands::Outcome::Trace => {
                            let _ = terminal.write_output(trace::message(trace::latest()));
                        }
                        commands::Outcome::Inspect(resource) => {
                            let _ = terminal.write_output(resource.as_bytes());
                        }
                        commands::Outcome::Restart => {
                            let _ = terminal.write_output(if service_lifecycle.restart(tick) {
                                b"restart scheduled"
                            } else {
                                b"restart unavailable"
                            });
                        }
                        commands::Outcome::Cancel => {
                            let _ = channel.send(
                                &capabilities,
                                service_capability,
                                session::Principal::LOCAL,
                                virtio_handle,
                                ipc::Message::Cancel,
                            );
                            let _ = terminal.write_output(b"cancel requested");
                        }
                        commands::Outcome::CommandList => {
                            for line in commands::COMMAND_LIST {
                                let _ = terminal.write_output(line);
                            }
                        }
                        commands::Outcome::Text(value) => {
                            if let Some(value) = format::render(value, format::Style::Human) {
                                let _ = terminal.write_output(value.as_bytes());
                            }
                        }
                        commands::Outcome::Error(commands::Error::Denied) => {
                            let _ = terminal.write_output(b"permission denied");
                        }
                        commands::Outcome::Error(commands::Error::UnknownCommand) => {
                            let _ = terminal.write_output(b"unknown command");
                        }
                        commands::Outcome::Error(commands::Error::Cancelled) => {
                            let _ = terminal.write_output(b"cancelled");
                        }
                        commands::Outcome::Error(commands::Error::TimedOut) => {
                            let _ = terminal.write_output(b"timed out");
                        }
                    }
                    if !terminal.render(display.as_mut().unwrap(), &text) {
                        console_mode = mode::ConsoleMode::Recovery;
                        break;
                    }
                } else if terminal.apply(event)
                    && !terminal.render(display.as_mut().unwrap(), &text)
                {
                    console_mode = mode::ConsoleMode::Recovery;
                    break;
                }
            } else {
                unsafe { core::arch::asm!("hlt") };
            }
        }
    }
    if console_mode == mode::ConsoleMode::Recovery {
        startup.start();
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
        console.run(|| {
            let tick = interrupts::ticks();
            if platform::completion_pending() {
                let _ = service_scheduler.wake_event(scheduler::Event::VIRTIO);
            }
            if service_scheduler.run_next() {
                let _ = service_health.beat(supervisor::VIRTIO_BALLOON, tick);
            }
        })
    }
    loop {
        unsafe { core::arch::asm!("cli", "hlt") };
    }
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
