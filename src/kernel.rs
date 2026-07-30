use crate::arch::{acpi, cpu, interrupts, pci};
use crate::console::{native_display, recovery as console};
use crate::drivers::{block as block_driver, device, keyboard, resources, supervisor, virtio};
use crate::ipc::{self, approvals, effects};
use crate::mm::{address_space, memory, virtual_memory};
use crate::platform::{
    audit, balloon, block, entropy, health, identity, inference, mode, payload, pe, root_key,
    secrets, services, session, storage, time, trace,
};
use crate::sched::{native_task, scheduler};
#[cfg(feature = "test-hooks")]
use crate::test_hooks;
use crate::{boot, debug};

use logos_core::capabilities;
use logos_terminal::{command, display, input, terminal, text};
use uefi::mem::memory_map::MemoryMap;

#[cfg_attr(feature = "test-hooks", allow(unreachable_code, unused_mut, unused_variables))]
pub(crate) fn main(
    boot_info: boot::Info,
    memory_map: impl MemoryMap,
    acpi: Option<acpi::Tables>,
    machine: identity::Machine,
    mut secret_root: Option<root_key::RootKey>,
    wall_clock: time::WallClock,
    payload: Option<payload::Payloads>,
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
        payload.is_some()
            && logos_core::native_service::self_check()
            && pe::self_check()
            && payload::relocation_self_check(),
    );
    check!(
        b"machine identity",
        entropy::self_check() && identity::self_check() && machine.id() == machine.id(),
    );
    check!(b"secret store", secrets::self_check(),);
    if let Some(key) = secret_root.as_mut() {
        key.wipe();
        check!(b"secret root wiped", key.is_wiped());
    }
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
    let Some(payloads) = payload else {
        fail!(b"native image map");
    };
    let payload = payloads.terminal;
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
    let Some(mut sessions_address_space) = address_space::AddressSpace::new(&mut memory) else {
        fail!(b"sessions image map");
    };
    check!(
        b"sessions image map",
        sessions_address_space
            .map_image(&mut memory, payloads.sessions)
            .is_some_and(|entry| entry != 0 && sessions_address_space.verifies_isolation())
            && sessions_address_space.release(&mut memory),
    );
    let Some(mut storage_address_space) = address_space::AddressSpace::new(&mut memory) else {
        fail!(b"storage image map");
    };
    check!(
        b"storage image map",
        storage_address_space
            .map_image(&mut memory, payloads.storage)
            .is_some_and(|entry| entry != 0 && storage_address_space.verifies_isolation())
            && storage_address_space.release(&mut memory),
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
            privilege.run_entry(&mut service_address_space, entry, 0, &mut cpu::GateState::new())
                == Some(cpu::EntryState::Returned)
        }) && service_address_space.release(&mut memory),
    );
    let Some(mut terminal_task) = native_task::Task::load(&mut memory, payload, &privilege) else {
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
    let Some(device) = balloon::discover(&devices) else {
        fail!(b"platform device");
    };
    let Some(block_pci) = block::discover(&devices) else {
        fail!(b"block device");
    };
    let Some(block_gsi) = acpi.and_then(|tables| {
        let (bus, slot, _) = block_pci.location();
        tables.pci_gsi(bus, slot, block_pci.interrupt_pin().checked_sub(1)?)
    }) else {
        fail!(b"block routing");
    };
    let Some(mut block_device) = block_driver::Device::bind(block_pci, block_gsi, &mut memory)
    else {
        fail!(b"block bind");
    };
    check!(
        b"block device",
        block_driver::self_check()
            && block::NAME == b"virtio-block"
            && block_device.info().valid()
            && block_device.diagnostics() == (0, 0, false),
    );
    #[cfg(feature = "block-probe")]
    check!(b"block probe", block_probe(&mut block_device, &mut memory));
    let Some(supervisor) = supervisor::boot_plan(supervisor::Profile::Normal).ok() else {
        fail!(b"supervisor manifest");
    };
    check!(b"supervisor manifest", supervisor::self_check() && supervisor.starts(balloon::NAME),);
    check!(b"service profiles", supervisor::profiles_self_check());
    check!(b"service dependency loss", supervisor::dependency_loss_self_check());
    check!(b"service startup failure", supervisor::startup_failure_self_check());
    let Some(service_protocol) = supervisor.negotiate(balloon::NAME, balloon::SERVICE.protocol())
    else {
        supervisor::report_start_failure(balloon::NAME, supervisor::StartStage::Protocol);
        fail!(b"service protocol");
    };
    check!(
        b"service protocol",
        supervisor::protocol_self_check() && service_protocol == balloon::SERVICE.protocol(),
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
    #[cfg(feature = "test-hooks")]
    let Some(restricted_session) =
        session::Context::new(session::Id(3), session::Principal::LOCAL, &[session_capability])
    else {
        fail!(b"session");
    };
    let Some(service_capability) =
        supervisor.grant(balloon::NAME, &mut capabilities, capabilities::CapabilityKind::Service)
    else {
        supervisor::report_start_failure(balloon::NAME, supervisor::StartStage::Capability);
        fail!(b"service capability");
    };
    let Some(block_service_capability) =
        supervisor.grant(block::NAME, &mut capabilities, capabilities::CapabilityKind::Service)
    else {
        fail!(b"block capability");
    };
    let Some(storage_service_capability) =
        supervisor.grant(storage::NAME, &mut capabilities, capabilities::CapabilityKind::Service)
    else {
        fail!(b"storage capability");
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
            && service_health.watch(&supervisor, balloon::NAME, 100, interrupts::ticks(),),
    );
    let Some(mut service_lifecycle) = supervisor::Lifecycle::new(&supervisor, balloon::NAME) else {
        fail!(b"service lifecycle");
    };
    check!(b"service lifecycle", supervisor::lifecycle_self_check());
    let mut services = services::Registry::new();
    let Some(virtio_handle) =
        services.register(&capabilities, service_capability, balloon::SERVICE)
    else {
        supervisor::report_start_failure(balloon::NAME, supervisor::StartStage::Register);
        fail!(b"services");
    };
    let Some(block_handle) =
        services.register(&capabilities, block_service_capability, block::SERVICE)
    else {
        fail!(b"block register");
    };
    let Some(storage_service_handle) =
        services.register(&capabilities, storage_service_capability, storage::SERVICE)
    else {
        fail!(b"storage register");
    };
    check!(
        b"services",
        services.resolve(balloon::SERVICE) == Some(virtio_handle)
            && services.resolve(block::SERVICE) == Some(block_handle)
            && services.resolve(storage::SERVICE) == Some(storage_service_handle),
    );
    check!(
        b"storage namespaces",
        storage::TERMINAL_NAMESPACE != storage::TEXT_NAMESPACE
            && storage::AUDIT_NAMESPACE != storage::SECRETS_NAMESPACE,
    );
    let Some(virtio_gsi) = acpi.and_then(|tables| {
        let (bus, slot, _) = device.location();
        tables.pci_gsi(bus, slot, device.interrupt_pin().checked_sub(1)?)
    }) else {
        fail!(b"acpi pci routing");
    };
    let Some(mut virtio_service) =
        virtio::VirtioService::bind(device, virtio_gsi, virtio_handle, &mut memory)
    else {
        supervisor::report_start_failure(balloon::NAME, supervisor::StartStage::Bind);
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
        let mut service_task = virtio::ServiceTask::new(
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
            && supervisor.replace(balloon::NAME, || {
                if !virtio_service.release(&mut memory) {
                    return false;
                }
                replacement =
                    virtio::VirtioService::bind(device, virtio_gsi, virtio_handle, &mut memory);
                replacement.is_some()
            }),
    );
    let Some(mut virtio_service) = replacement else {
        fail!(b"service replacement");
    };
    let Some(mut native_terminal) = native_task::Task::load(&mut memory, payload, &privilege)
    else {
        fail!(b"native terminal task");
    };
    let Some(mut native_sessions) =
        native_task::Task::load(&mut memory, payloads.sessions, &privilege)
    else {
        fail!(b"native sessions task");
    };
    let Some(mut native_storage) =
        native_task::Task::load(&mut memory, payloads.storage, &privilege)
    else {
        fail!(b"native storage task");
    };
    let mut service_task = virtio::ServiceTask::new(
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
        supervisor::report_start_failure(balloon::NAME, supervisor::StartStage::Task);
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
            && command::self_check()
    });
    let mut console_mode = mode::ConsoleMode::new(normal_ready);
    check!(b"console mode", mode::ConsoleMode::self_check());
    check!(b"command registry", command::self_check());
    check!(b"effect executor", effects::self_check());
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
    console_mode.announce();
    check!(b"trace", trace::self_check());
    let native_input = native_terminal.input_endpoint();
    let native_command = native_terminal.syscall_endpoint();
    let native_display = native_terminal.display_endpoint();
    let native_sessions_endpoint = native_sessions.session_endpoint();
    let mut native_scheduler = scheduler::Scheduler::new();
    let Some(mut native_handle) = native_scheduler.spawn(&mut native_terminal) else {
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
    let Some(mut sessions_handle) = native_scheduler.spawn(&mut native_sessions) else {
        fail!(b"native sessions task");
    };
    if !native_scheduler.run_next() {
        fail!(b"native sessions ready");
    }
    let Some(storage_handle) = native_scheduler.spawn(&mut native_storage) else {
        fail!(b"native storage task");
    };
    if !native_scheduler.run_next() {
        fail!(b"native storage ready");
    }
    let _ = storage_handle;
    health.finish();
    #[cfg(feature = "test-hooks")]
    test_hooks::serve(|value| {
        let terminal_restart = value == "assert-terminal-service-restart";
        let sessions_restart = value == "assert-sessions-service-restart";
        let storage_restart = value == "assert-storage-service-restart";
        if terminal_restart {
            let previous = native_handle;
            if !native_scheduler.fail(previous) || !startup.start() {
                return false;
            }
            let Some(restarted) = restart_native_service(&mut native_scheduler, previous) else {
                return false;
            };
            native_handle = restarted;
            if native_scheduler.wake(previous)
                || !resume_display(
                    native_display,
                    &session,
                    &capabilities,
                    session_display_capability,
                    &mut native_scheduler,
                    native_handle,
                )
            {
                return false;
            }
        }
        if sessions_restart {
            let previous = sessions_handle;
            if !native_scheduler.fail(previous) || !startup.start() {
                return false;
            }
            let Some(restarted) = restart_native_service(&mut native_scheduler, previous) else {
                return false;
            };
            sessions_handle = restarted;
            if native_scheduler.wake(previous) {
                return false;
            }
        }
        if storage_restart {
            let previous = storage_handle;
            if !native_scheduler.fail(previous) || !startup.start() {
                return false;
            }
            let Some(restarted) = restart_native_service(&mut native_scheduler, previous) else {
                return false;
            };
            let _ = restarted;
            if native_scheduler.wake(previous) {
                return false;
            }
        }
        if value == "assert-sessions" {
            return native_sessions_endpoint.deliver(logos_abi::SessionRequest::new(
                logos_abi::Syscall::Tasks,
                [0; logos_abi::MAX_SESSION_TEXT],
                0,
            )) && native_scheduler.wake(sessions_handle)
                && native_scheduler.run_next()
                && native_sessions_endpoint.effect().is_some_and(|effect| {
                    effect.effect == logos_abi::Effect::ReadTasks
                        && native_sessions_endpoint
                            .reply_effect(logos_abi::EffectResult::TasksActive)
                })
                && native_scheduler.wake(sessions_handle)
                && native_scheduler.run_next()
                && native_sessions_endpoint.reply().is_some_and(|reply| {
                    reply.length == b"scheduler active".len()
                        && reply.text[..reply.length] == *b"scheduler active"
                });
        }
        if value == "assert-crash-restart" {
            let tick = interrupts::ticks();
            return service_lifecycle.failed(tick) && service_lifecycle.due(tick.saturating_add(2));
        }
        if value == "assert-restart-backoff" {
            let tick = interrupts::ticks();
            return service_lifecycle.failed(tick)
                && !service_lifecycle.due(tick.saturating_add(1))
                && service_lifecycle.due(tick.saturating_add(2))
                && service_lifecycle.failed(tick.saturating_add(2))
                && !service_lifecycle.due(tick.saturating_add(5))
                && service_lifecycle.due(tick.saturating_add(6));
        }
        let deny_display = value == "deny-display";
        let (value, request_session, expected, expect_qwerty) =
            if terminal_restart || sessions_restart {
                ("tasks", &session, Some(b"scheduler active" as &[u8]), false)
            } else if value == "deny-recovery" {
                ("recovery", &restricted_session, Some(b"permission denied" as &[u8]), false)
            } else if value == "deny-layout" {
                ("layout azerty", &restricted_session, Some(b"permission denied" as &[u8]), true)
            } else if value == "deny-session" {
                ("tasks", &denied_session, Some(b"permission denied" as &[u8]), false)
            } else if value == "assert-tasks" {
                ("tasks", &session, Some(b"scheduler active" as &[u8]), false)
            } else if value == "assert-ping" {
                ("ping", &session, Some(b"pong" as &[u8]), false)
            } else if value == "assert-services" {
                ("services", &session, Some(b"platform service running" as &[u8]), false)
            } else if value == "assert-drivers" {
                ("drivers", &session, Some(b"platform driver bound" as &[u8]), false)
            } else if value == "assert-inspect" {
                (
                    "inspect service:/virtio-balloon",
                    &session,
                    Some(b"service:/virtio-balloon" as &[u8]),
                    false,
                )
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
                        let reply = relay_session_request(
                            native_command,
                            native_sessions_endpoint,
                            &mut native_scheduler,
                            sessions_handle,
                            effects::Context {
                                session: request_session,

                                capabilities: &capabilities,
                                tick: interrupts::ticks(),
                                input: &mut input,
                                lifecycle: &mut service_lifecycle,
                                service_healthy: service_health
                                    .healthy(balloon::NAME, interrupts::ticks()),
                                channel: &channel,
                                responses: &responses,
                                service_scheduler: &mut service_scheduler,
                                service_capability,
                                service: virtio_handle,
                            },
                        );
                        reply.ok()
                            && expected.is_none_or(|expected| {
                                matches!(reply, SessionRelay::Handled(true))
                                    && native_command.reply_matches(expected)
                            })
                            && (!expect_qwerty || input.layout() == input::Layout::Qwerty)
                            && native_scheduler.wake(native_handle)
                            && native_scheduler.run_next()
                            && resume_display(
                                native_display,
                                &session,
                                &capabilities,
                                session_display_capability,
                                &mut native_scheduler,
                                native_handle,
                            )
                    }))
        })
    });
    // ponytail: one bootstrap retry; use supervisor policy when native services join System lifecycle.
    let mut terminal_restart_available = true;
    let mut sessions_restart_available = true;
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
            if virtio::completion_pending() {
                let _ = service_scheduler.wake_event(scheduler::Event::VIRTIO);
            }
            if service_scheduler.run_next() {
                let _ = service_health.beat(balloon::NAME, tick);
            }
            if !service_health.healthy(balloon::NAME, tick) {
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
                        if terminal_restart_available {
                            terminal_restart_available = false;
                            if let Some(restarted) =
                                restart_native_service(&mut native_scheduler, native_handle)
                            {
                                native_handle = restarted;
                                if resume_display(
                                    native_display,
                                    &session,
                                    &capabilities,
                                    session_display_capability,
                                    &mut native_scheduler,
                                    native_handle,
                                ) {
                                    debug::write_line(b"LogOS: native terminal restarted");
                                    continue;
                                }
                            }
                        }
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
                        let mut relay = relay_session_request(
                            native_command,
                            native_sessions_endpoint,
                            &mut native_scheduler,
                            sessions_handle,
                            effects::Context {
                                session: &session,
                                capabilities: &capabilities,
                                tick,
                                input: &mut input,
                                lifecycle: &mut service_lifecycle,
                                service_healthy: service_health.healthy(balloon::NAME, tick),
                                channel: &channel,
                                responses: &responses,
                                service_scheduler: &mut service_scheduler,
                                service_capability,
                                service: virtio_handle,
                            },
                        );

                        if matches!(relay, SessionRelay::Handled(false))
                            && sessions_restart_available
                        {
                            sessions_restart_available = false;
                            if let Some(restarted) =
                                restart_native_service(&mut native_scheduler, sessions_handle)
                            {
                                sessions_handle = restarted;
                                debug::write_line(b"LogOS: native Sessions restarted");
                                relay = SessionRelay::Handled(
                                    native_command.reply(b"session restarted; retry command"),
                                );
                            }
                        }
                        match relay {
                            SessionRelay::Recovery => {
                                debug::write_line(b"LogOS: recovery handoff requested");
                                console_mode = mode::ConsoleMode::Recovery;
                                break;
                            }
                            SessionRelay::Handled(false) => {
                                debug::write_line(b"LogOS: Sessions relay failed");
                                console_mode = mode::ConsoleMode::Recovery;
                                break;
                            }
                            SessionRelay::Handled(true) => {
                                if !native_scheduler.wake(native_handle)
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
        let _ = startup.start();
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
            if virtio::completion_pending() {
                let _ = service_scheduler.wake_event(scheduler::Event::VIRTIO);
            }
            if service_scheduler.run_next() {
                let _ = service_health.beat(balloon::NAME, tick);
            }
        })
    }
    loop {
        unsafe { core::arch::asm!("cli", "hlt") };
    }
}

#[cfg(feature = "block-probe")]
fn block_probe(device: &mut block_driver::Device, memory: &mut memory::PhysicalMemory) -> bool {
    let Some(page) = memory.allocate_owned() else { return false };
    let request = logos_abi::BlockRequest {
        id: 0,
        operation: logos_abi::BlockOperation::Read,
        lba: 0,
        blocks: 1,
        page: logos_abi::PageHandle(0),
        deadline: interrupts::ticks().saturating_add(10),
    };
    if device.submit(request, Some(page.address()), memory)
        != logos_abi::PersistenceStatus::Complete
    {
        debug::write_line(b"LogOS: block probe submit rejected");
        return memory.release_page(page);
    }
    debug_block_state(device.probe_state());
    let status = loop {
        if let Some(status) = device.complete(memory) {
            break status;
        }
        if interrupts::ticks() >= request.deadline {
            break device.timeout(memory);
        }
        interrupts::wait_for_tick();
    };
    let released = memory.release_page(page);
    debug::write_line(match status {
        logos_abi::PersistenceStatus::Complete => b"LogOS: block probe read complete",
        logos_abi::PersistenceStatus::TimedOut => b"LogOS: block probe read timed out",
        _ => b"LogOS: block probe read failed",
    });
    status == logos_abi::PersistenceStatus::Complete
        && released
        && block_probe_flush(device, memory, request)
}

#[cfg(feature = "block-probe")]
fn block_probe_flush(
    device: &mut block_driver::Device,
    memory: &mut memory::PhysicalMemory,
    request: logos_abi::BlockRequest,
) -> bool {
    let request = logos_abi::BlockRequest {
        operation: logos_abi::BlockOperation::Flush,
        deadline: interrupts::ticks().saturating_add(10),
        ..request
    };
    if device.submit(request, None, memory) != logos_abi::PersistenceStatus::Complete {
        return false;
    }
    loop {
        if let Some(status) = device.complete(memory) {
            return status == logos_abi::PersistenceStatus::Complete;
        }
        if interrupts::ticks() >= request.deadline {
            let _ = device.timeout(memory);
            return false;
        }
        interrupts::wait_for_tick();
    }
}

#[cfg(feature = "block-probe")]
fn debug_block_state((status, queue, pfn, available, used): (u8, u16, u32, u16, u16)) {
    debug::write(b"LogOS: block state ");
    for value in [u32::from(status), u32::from(queue), pfn, u32::from(available), u32::from(used)] {
        for shift in [12, 8, 4, 0] {
            let digit = ((value >> shift) & 15) as u8;
            debug::write(&[if digit < 10 { b'0' + digit } else { b'a' + digit - 10 }]);
        }
        debug::write(b" ");
    }
    debug::write_line(b"");
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

#[derive(Clone, Copy)]
enum SessionRelay {
    Handled(bool),
    Recovery,
}

impl SessionRelay {
    #[cfg_attr(not(feature = "test-hooks"), allow(dead_code))]
    fn ok(self) -> bool {
        match self {
            Self::Handled(ok) => ok,
            Self::Recovery => true,
        }
    }
}

fn restart_native_service(
    scheduler: &mut scheduler::Scheduler<'_>,
    handle: scheduler::TaskHandle,
) -> Option<scheduler::TaskHandle> {
    if !scheduler.failed(handle) && !scheduler.fail(handle) {
        return None;
    }
    let restarted = scheduler.restart(handle)?;
    (scheduler.run_next() && !scheduler.failed(restarted)).then_some(restarted)
}

fn relay_session_request(
    terminal: native_task::SyscallEndpoint,
    sessions: native_task::SessionEndpoint,
    scheduler: &mut scheduler::Scheduler<'_>,
    sessions_handle: scheduler::TaskHandle,
    context: effects::Context<'_, '_>,
) -> SessionRelay {
    let Some(request) = terminal.request() else {
        return SessionRelay::Handled(true);
    };
    if !context.session.allows(context.capabilities, capabilities::CapabilityKind::Session) {
        return SessionRelay::Handled(terminal.reply(b"permission denied"));
    }
    if !sessions.deliver(request) || !scheduler.wake(sessions_handle) || !scheduler.run_next() {
        return SessionRelay::Handled(false);
    }
    let Some(effect) = sessions.effect() else {
        return SessionRelay::Handled(false);
    };
    let result = effects::execute(effect, context);
    if !sessions.reply_effect(result) || !scheduler.wake(sessions_handle) || !scheduler.run_next() {
        return SessionRelay::Handled(false);
    }
    let forwarded = sessions.reply().is_some_and(|reply| {
        terminal.reply(&reply.text[..reply.length])
            && scheduler.wake(sessions_handle)
            && scheduler.run_next()
    });
    if result == logos_abi::EffectResult::Recovery && forwarded {
        SessionRelay::Recovery
    } else {
        SessionRelay::Handled(forwarded)
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
