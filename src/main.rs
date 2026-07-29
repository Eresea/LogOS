#![no_main]
#![no_std]

mod acpi;
mod address_space;
mod approvals;
mod audit;
mod capabilities;
mod commands;
mod console;
mod cpu;
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
mod native_display;
mod native_task;
mod payload;
mod pci;
mod pe;
mod platform;
mod resources;
mod scheduler;
mod secrets;
mod services;
mod session;
mod supervisor;
#[cfg(feature = "test-hooks")]
mod test_hooks;
mod time;
mod trace;
mod virtual_memory;

use logos_terminal::{command, display, input, terminal, text};
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

    let payload = payload::stage();
    let memory_map = unsafe { boot::exit_boot_services(None) };
    kernel_main(boot_info, memory_map, acpi, machine, wall_clock, payload)
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

#[cfg_attr(feature = "test-hooks", allow(unreachable_code, unused_mut, unused_variables))]
fn kernel_main(
    boot_info: BootInfo,
    memory_map: impl MemoryMap,
    acpi: Option<acpi::Tables>,
    machine: identity::Machine,
    wall_clock: time::WallClock,
    payload: Option<payload::Payload>,
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
        b"native payload",
        payload.is_some() && logos_core::native_service::self_check() && pe::self_check(),
    );
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
        b"native display",
        native_display::install(
            boot_info.framebuffer,
            boot_info.framebuffer_size,
            boot_info.resolution.0,
            boot_info.resolution.1,
            boot_info.stride,
        ),
    );
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
    let Some(privilege) = cpu::Privilege::install(&mut memory) else {
        fail!(b"service privilege");
    };
    check!(b"service privilege", privilege.self_check());
    let Some(service_address_space) = address_space::AddressSpace::new(&mut memory) else {
        fail!(b"service address space");
    };
    check!(
        b"service address space",
        service_address_space.stack_top() != 0
            && service_address_space.verifies_isolation()
            && service_address_space.release(&mut memory),
    );
    let Some(payload) = payload else {
        fail!(b"native image map");
    };
    let Some(mut service_address_space) = address_space::AddressSpace::new(&mut memory) else {
        fail!(b"native image map");
    };
    check!(
        b"native image map",
        service_address_space
            .map_image(&mut memory, payload)
            .is_some_and(|entry| entry != 0 && service_address_space.verifies_isolation())
            && service_address_space.release(&mut memory),
    );
    let keyboard_interrupts = interrupts::install(madt);
    interrupts::enable();
    interrupts::wait_for_tick();
    check!(b"interrupts", keyboard_interrupts);
    let Some(mut service_address_space) = address_space::AddressSpace::new(&mut memory) else {
        fail!(b"service transition");
    };
    check!(
        b"service transition",
        service_address_space.map_probe(&mut memory).is_some_and(|entry| {
            privilege.run_entry(&mut service_address_space, entry, 0)
                == Some(cpu::EntryState::Returned)
        }) && service_address_space.release(&mut memory),
    );
    let Some(mut terminal_task) = native_task::Terminal::load(&mut memory, payload, &privilege)
    else {
        fail!(b"native service entry");
    };
    let terminal_input = terminal_task.input_endpoint();
    let terminal_display = terminal_task.display_endpoint();
    let terminal_result = {
        let mut terminal_scheduler = scheduler::Scheduler::new();
        let terminal_handle = terminal_scheduler.spawn(&mut terminal_task);
        terminal_handle.is_some()
            && terminal_scheduler.run_next()
            && resume_probe_display(
                terminal_display,
                &mut terminal_scheduler,
                terminal_handle.unwrap(),
            )
            && native_display::matches(33, 35, [0, 0xff, 0])
            && !terminal_scheduler.run_next()
            && terminal_input.deliver(logos_abi::InputEvent::from_byte(b'k').unwrap())
            && terminal_scheduler.wake(terminal_handle.unwrap())
            && terminal_scheduler.run_next()
            && resume_probe_display(
                terminal_display,
                &mut terminal_scheduler,
                terminal_handle.unwrap(),
            )
            && !terminal_scheduler.run_next()
            && native_display::matches(33, 35, [0, 0xff, 0])
            && terminal_input.deliver(logos_abi::InputEvent::ESCAPE)
            && terminal_scheduler.wake(terminal_handle.unwrap())
            && terminal_scheduler.run_next()
            && !terminal_scheduler.run_next()
    };
    check!(
        b"native service entry",
        terminal_result && terminal_task.complete() && terminal_task.release(&mut memory),
    );
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
    check!(b"supervisor manifest", supervisor::self_check() && supervisor.starts(platform::NAME),);
    check!(b"service profiles", supervisor::profiles_self_check());
    check!(b"service dependency loss", supervisor::dependency_loss_self_check());
    check!(b"service startup failure", supervisor::startup_failure_self_check());
    let Some(service_protocol) = supervisor.negotiate(platform::NAME, platform::SERVICE.protocol())
    else {
        supervisor::report_start_failure(platform::NAME, supervisor::StartStage::Protocol);
        fail!(b"service protocol");
    };
    check!(
        b"service protocol",
        supervisor::protocol_self_check() && service_protocol == platform::SERVICE.protocol(),
    );
    let Some(session_service_capability) =
        capabilities.grant(capabilities::CapabilityKind::Service)
    else {
        fail!(b"capabilities");
    };
    let Some(session_input_capability) = capabilities.grant(capabilities::CapabilityKind::Input)
    else {
        fail!(b"capabilities");
    };
    let Some(session_display_capability) =
        capabilities.grant(capabilities::CapabilityKind::Display)
    else {
        fail!(b"capabilities");
    };
    let Some(session_capability) = capabilities.grant(capabilities::CapabilityKind::Session) else {
        fail!(b"capabilities");
    };
    let Some(recovery_capability) = capabilities.grant(capabilities::CapabilityKind::Recovery)
    else {
        fail!(b"capabilities");
    };
    let Some(session) = session::Context::new(
        session::Id(1),
        session::Principal::LOCAL,
        &[
            recovery_capability,
            session_service_capability,
            session_input_capability,
            session_display_capability,
            session_capability,
        ],
    ) else {
        fail!(b"session");
    };
    #[cfg(feature = "test-hooks")]
    let Some(denied_session) =
        session::Context::new(session::Id(2), session::Principal::LOCAL, &[])
    else {
        fail!(b"session");
    };
    let Some(service_capability) =
        supervisor.grant(platform::NAME, &mut capabilities, capabilities::CapabilityKind::Service)
    else {
        supervisor::report_start_failure(platform::NAME, supervisor::StartStage::Capability);
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
            && service_health.watch(&supervisor, platform::NAME, 100, interrupts::ticks(),),
    );
    let Some(mut service_lifecycle) = supervisor::Lifecycle::new(&supervisor, platform::NAME)
    else {
        fail!(b"service lifecycle");
    };
    check!(b"service lifecycle", supervisor::lifecycle_self_check());
    let mut services = services::Registry::new();
    let Some(virtio_handle) =
        services.register(&capabilities, service_capability, platform::SERVICE)
    else {
        supervisor::report_start_failure(platform::NAME, supervisor::StartStage::Register);
        fail!(b"services");
    };
    check!(b"services", services.resolve(platform::SERVICE) == Some(virtio_handle),);
    let Some(virtio_gsi) = acpi.and_then(|tables| {
        let (bus, slot, _) = device.location();
        tables.pci_gsi(bus, slot, device.interrupt_pin().checked_sub(1)?)
    }) else {
        fail!(b"acpi pci routing");
    };
    let Some(mut virtio_service) =
        platform::Service::bind(device, virtio_gsi, virtio_handle, &mut memory)
    else {
        supervisor::report_start_failure(platform::NAME, supervisor::StartStage::Bind);
        fail!(b"platform service");
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
            && supervisor.replace(platform::NAME, || {
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
    let Some(mut native_terminal) = native_task::Terminal::load(&mut memory, payload, &privilege)
    else {
        fail!(b"native terminal task");
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
        supervisor::report_start_failure(platform::NAME, supervisor::StartStage::Task);
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
            && command::self_check()
    });
    let coordinator = mode::Coordinator::new(normal_ready);
    check!(b"console mode", mode::Coordinator::self_check());
    check!(b"command registry", commands::self_check() && command::self_check());
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
    let native_input = native_terminal.input_endpoint();
    let native_command = native_terminal.syscall_endpoint();
    let native_display = native_terminal.display_endpoint();
    let mut native_scheduler = scheduler::Scheduler::new();
    let Some(native_handle) = native_scheduler.spawn(&mut native_terminal) else {
        fail!(b"native terminal task");
    };
    if !native_scheduler.run_next() {
        fail!(b"native terminal task");
    }
    if !resume_display(
        native_display,
        &session,
        &capabilities,
        session_display_capability,
        &mut native_scheduler,
        native_handle,
    ) {
        fail!(b"native terminal display");
    }
    health.finish();
    #[cfg(feature = "test-hooks")]
    test_hooks::serve(|value| {
        let deny_display = value == "deny-display";
        let (value, request_session, expected, expect_qwerty) = if value == "deny-recovery" {
            ("recovery", &denied_session, Some(b"permission denied" as &[u8]), false)
        } else if value == "deny-layout" {
            ("layout azerty", &denied_session, Some(b"permission denied" as &[u8]), true)
        } else if value == "deny-session" {
            ("tasks", &denied_session, Some(b"permission denied" as &[u8]), false)
        } else if value == "assert-tasks" {
            ("tasks", &session, Some(b"scheduler active" as &[u8]), false)
        } else if value == "assert-restart" {
            ("restart virtio-balloon", &session, Some(b"restart scheduled" as &[u8]), false)
        } else if value == "assert-cancel" {
            ("cancel virtio-balloon", &session, Some(b"cancel requested" as &[u8]), false)
        } else if deny_display {
            ("x", &session, None, false)
        } else {
            (value, &session, None, false)
        };
        if deny_display {
            return logos_abi::InputEvent::from_byte(b'x').is_some_and(|event| {
                native_input.deliver(event)
                    && native_scheduler.wake(native_handle)
                    && native_scheduler.run_next()
                    && !resume_display(
                        native_display,
                        &denied_session,
                        &capabilities,
                        session_display_capability,
                        &mut native_scheduler,
                        native_handle,
                    )
            });
        }
        value.bytes().chain(core::iter::once(b'\n')).all(|byte| {
            logos_abi::InputEvent::from_byte(byte).is_some_and(|event| native_input.deliver(event))
                && native_scheduler.wake(native_handle)
                && native_scheduler.run_next()
                && (if native_display.pending() {
                    resume_display(
                        native_display,
                        &session,
                        &capabilities,
                        session_display_capability,
                        &mut native_scheduler,
                        native_handle,
                    )
                } else {
                    true
                })
                && (native_command.request().is_none()
                    || ({
                        let reply = native_syscall_reply(
                            native_command,
                            NativeCommandContext {
                                session: request_session,
                                capabilities: &capabilities,
                                tick: interrupts::ticks(),
                                input: &mut input,
                                lifecycle: &mut service_lifecycle,
                                service_healthy: service_health
                                    .healthy(platform::NAME, interrupts::ticks()),
                                channel: &channel,
                                responses: &responses,
                                service_scheduler: &mut service_scheduler,
                                service_capability,
                                service: virtio_handle,
                            },
                        );
                        reply.ok()
                            && expected.is_none_or(|expected| {
                                matches!(reply, CommandReply::Handled(true))
                                    && native_command.reply_matches(expected)
                            })
                            && (!expect_qwerty || input.layout() == input::Layout::Qwerty)
                            && native_scheduler.wake(native_handle)
                            && native_scheduler.run_next()
                    }))
        })
    });
    let mut console_mode = coordinator.mode();
    if console_mode == mode::ConsoleMode::Normal {
        debug::write_line(b"LogOS: native terminal active");
        loop {
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
                let _ = service_health.beat(platform::NAME, tick);
            }
            if !service_health.healthy(platform::NAME, tick) {
                debug::write_line(b"LogOS: service heartbeat overdue");
                let _ = service_lifecycle.failed(tick);
            }
            if let Some(event) = input.next(tick, keyboard::poll_scancode) {
                if let Some(native_event) = native_input_event(event) {
                    if !native_input.deliver(native_event)
                        || !native_scheduler.wake(native_handle)
                        || !native_scheduler.run_next()
                        || !resume_display(
                            native_display,
                            &session,
                            &capabilities,
                            session_display_capability,
                            &mut native_scheduler,
                            native_handle,
                        )
                    {
                        debug::write_line(b"LogOS: native terminal display failed");
                        console_mode = mode::ConsoleMode::Recovery;
                        break;
                    }
                    if event.pressed().is_some_and(|(key, _)| key == input::LogicalKey::Escape) {
                        debug::write_line(b"LogOS: recovery handoff requested");
                        console_mode = mode::ConsoleMode::Recovery;
                        break;
                    }
                    if native_command.request().is_some() {
                        match native_syscall_reply(
                            native_command,
                            NativeCommandContext {
                                session: &session,
                                capabilities: &capabilities,
                                tick,
                                input: &mut input,
                                lifecycle: &mut service_lifecycle,
                                service_healthy: service_health.healthy(platform::NAME, tick),
                                channel: &channel,
                                responses: &responses,
                                service_scheduler: &mut service_scheduler,
                                service_capability,
                                service: virtio_handle,
                            },
                        ) {
                            CommandReply::Recovery => {
                                debug::write_line(b"LogOS: recovery handoff requested");
                                console_mode = mode::ConsoleMode::Recovery;
                                break;
                            }
                            CommandReply::Handled(ok) => {
                                let _ = ok
                                    && native_scheduler.wake(native_handle)
                                    && native_scheduler.run_next();
                            }
                        }
                    }
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
                let _ = service_health.beat(platform::NAME, tick);
            }
        })
    }
    loop {
        unsafe { core::arch::asm!("cli", "hlt") };
    }
}

fn native_input_event(event: input::Event) -> Option<logos_abi::InputEvent> {
    event
        .text()
        .or_else(|| {
            event.pressed().and_then(|(key, _)| match key {
                input::LogicalKey::Escape => Some(0x1b),
                input::LogicalKey::Enter => Some(b'\n'),
                input::LogicalKey::Backspace => Some(0x08),
                _ => None,
            })
        })
        .and_then(logos_abi::InputEvent::from_byte)
}

fn resume_display(
    endpoint: native_task::DisplayEndpoint,
    session: &session::Context,
    capabilities: &capabilities::CapabilityManager,
    capability: capabilities::Capability,
    scheduler: &mut scheduler::Scheduler<'_>,
    handle: scheduler::TaskHandle,
) -> bool {
    while endpoint.pending() {
        if !session.allows(capabilities, capabilities::CapabilityKind::Display)
            || !capabilities.allows(capability, capabilities::CapabilityKind::Display)
            || !native_display::handle(endpoint.context())
            || !scheduler.wake(handle)
            || !scheduler.run_next()
        {
            return false;
        }
    }
    true
}

fn resume_probe_display(
    endpoint: native_task::DisplayEndpoint,
    scheduler: &mut scheduler::Scheduler<'_>,
    handle: scheduler::TaskHandle,
) -> bool {
    while endpoint.pending() {
        if !native_display::handle(endpoint.context())
            || !scheduler.wake(handle)
            || !scheduler.run_next()
        {
            return false;
        }
    }
    true
}

struct NativeCommandContext<'a, 'task> {
    session: &'a session::Context,
    capabilities: &'a capabilities::CapabilityManager,
    tick: u64,
    input: &'a mut input::Service,
    lifecycle: &'a mut supervisor::Lifecycle,
    service_healthy: bool,
    channel: &'a ipc::Channel,
    responses: &'a ipc::Channel,
    service_scheduler: &'a mut scheduler::Scheduler<'task>,
    service_capability: capabilities::Capability,
    service: services::ServiceHandle,
}

/// Result of handling one native syscall.
///
/// This exists so recovery hand-off is a real outcome the caller matches on,
/// rather than the caller re-parsing the raw request bytes itself (as the
/// old code did, comparing `request.text[..request.length]` against the
/// literal string `b"recovery"` *before* the command was even dispatched).
#[derive(Clone, Copy)]
enum CommandReply {
    /// The command was answered; `true` if the IPC reply itself was sent
    /// successfully.
    Handled(bool),
    /// The command resolved to a recovery hand-off. The reply has already
    /// been sent; the caller should switch console modes.
    Recovery,
}

impl CommandReply {
    /// For contexts (like the test-hooks harness) that only care whether the
    /// step succeeded, not whether it happened to be a recovery hand-off.
    #[cfg_attr(not(feature = "test-hooks"), allow(dead_code))]
    fn ok(self) -> bool {
        match self {
            Self::Handled(ok) => ok,
            Self::Recovery => true,
        }
    }
}

/// Answer one syscall from the native terminal task.
///
/// All command parsing lives in Ring 3; this function only handles typed syscalls.
/// - executes `Local::Layout`, because the kernel is what owns the physical
///   keyboard scancode decoder (`input::Service`); the terminal payload has
///   no way to act on a layout change itself.
/// - relays every other `Local` resolution's reply text back down, since the
///   terminal payload -- not the kernel -- interprets those replies (e.g.
///   `clear` is acknowledged here but actually performed by the payload).
/// - dispatches `Call` (tasks/services/drivers/trace/inspect/restart/cancel/
///   recovery/reboot/poweroff/ping) through `commands::dispatch`, the one
///   place capability checks happen.
fn native_syscall_reply(
    endpoint: native_task::SyscallEndpoint,
    context: NativeCommandContext<'_, '_>,
) -> CommandReply {
    let NativeCommandContext {
        session,
        capabilities,
        tick,
        input,
        lifecycle,
        service_healthy,
        channel,
        responses,
        service_scheduler,
        service_capability,
        service,
    } = context;
    let Some(request) = endpoint.submission() else {
        return CommandReply::Handled(true);
    };
    if !session.allows(capabilities, capabilities::CapabilityKind::Session) {
        return CommandReply::Handled(endpoint.reply(b"permission denied"));
    }
    let argument =
        logos_terminal::terminal::Submission::from_bytes(&request.argument[..request.length]);
    if request.length != 0 && argument.is_none() {
        return CommandReply::Handled(endpoint.reply(b"unknown command"));
    }
    match request.syscall {
        logos_abi::Syscall::SetInputLayout => {
            let layout = argument
                .filter(|argument| argument.as_bytes().len() == 1)
                .and_then(|argument| logos_abi::InputLayout::from_wire(argument.as_bytes()[0]));
            if !session.allows(capabilities, capabilities::CapabilityKind::Input) {
                CommandReply::Handled(endpoint.reply(b"permission denied"))
            } else if let Some(layout) = layout {
                input.set_layout(match layout {
                    logos_abi::InputLayout::Qwerty => input::Layout::Qwerty,
                    logos_abi::InputLayout::Azerty => input::Layout::Azerty,
                });
                CommandReply::Handled(endpoint.reply(match layout {
                    logos_abi::InputLayout::Qwerty => b"layout qwerty",
                    logos_abi::InputLayout::Azerty => b"layout azerty",
                }))
            } else {
                CommandReply::Handled(endpoint.reply(b"unknown command"))
            }
        }
        command => match commands::dispatch(
            command,
            argument,
            session,
            capabilities,
            commands::Invocation::new(tick.wrapping_add(50)),
            tick,
        ) {
            commands::Outcome::Tasks => CommandReply::Handled(endpoint.reply(b"scheduler active")),
            commands::Outcome::Services => {
                CommandReply::Handled(endpoint.reply(platform::service_status(service_healthy)))
            }
            commands::Outcome::Drivers => {
                CommandReply::Handled(endpoint.reply(platform::driver_status()))
            }
            commands::Outcome::Trace => {
                CommandReply::Handled(endpoint.reply(trace::message(trace::latest())))
            }
            commands::Outcome::Inspect(resource) => {
                CommandReply::Handled(endpoint.reply(resource.as_bytes()))
            }
            commands::Outcome::Restart(target) => CommandReply::Handled(endpoint.reply(
                if platform::matches(target.as_bytes()) && lifecycle.restart(tick) {
                    b"restart scheduled"
                } else {
                    b"unknown or unavailable service"
                },
            )),
            commands::Outcome::Cancel(target) => CommandReply::Handled(
                endpoint.reply(
                    if platform::matches(target.as_bytes())
                        && channel
                            .send(
                                capabilities,
                                service_capability,
                                session::Principal::LOCAL,
                                service,
                                ipc::Message::Cancel,
                            )
                            .is_some()
                    {
                        b"cancel requested"
                    } else {
                        b"unknown or unavailable service"
                    },
                ),
            ),
            commands::Outcome::Recovery => {
                let _ = endpoint.reply(b"recovery requested");
                CommandReply::Recovery
            }
            commands::Outcome::Reboot => CommandReply::Handled(if acpi::reset() {
                true
            } else {
                endpoint.reply(b"reboot unavailable")
            }),
            commands::Outcome::PowerOff => CommandReply::Handled(if acpi::power_off() {
                true
            } else {
                endpoint.reply(b"poweroff unavailable")
            }),
            commands::Outcome::Ping => CommandReply::Handled(endpoint.reply(
                if ping_platform(
                    channel,
                    responses,
                    service_scheduler,
                    capabilities,
                    service_capability,
                    service,
                ) {
                    b"pong"
                } else {
                    b"ping unavailable"
                },
            )),
            commands::Outcome::Error(commands::Error::Denied) => {
                CommandReply::Handled(endpoint.reply(b"permission denied"))
            }
            commands::Outcome::Error(commands::Error::UnknownCommand) => {
                CommandReply::Handled(endpoint.reply(b"unknown command"))
            }
            commands::Outcome::Error(commands::Error::Cancelled) => {
                CommandReply::Handled(endpoint.reply(b"cancelled"))
            }
            commands::Outcome::Error(commands::Error::TimedOut) => {
                CommandReply::Handled(endpoint.reply(b"timed out"))
            }
        },
    }
}

fn ping_platform(
    channel: &ipc::Channel,
    responses: &ipc::Channel,
    scheduler: &mut scheduler::Scheduler<'_>,
    capabilities: &capabilities::CapabilityManager,
    capability: capabilities::Capability,
    service: services::ServiceHandle,
) -> bool {
    let Some(request) = channel.send(
        capabilities,
        capability,
        session::Principal::LOCAL,
        service,
        ipc::Message::Ping,
    ) else {
        return false;
    };
    if !scheduler.run_next() {
        return false;
    }
    (0..4).any(|_| {
        responses
            .receive()
            .is_some_and(|reply| reply.request == request && reply.message == ipc::Message::Pong)
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
