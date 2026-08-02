use crate::arch::{acpi, cpu, interrupts, pci};
use crate::console::{native_display, recovery as console};
use crate::drivers::{
    block as block_driver, device, keyboard, network as network_driver, resources, supervisor,
    virtio,
};
use crate::ipc::{self, approvals, effects};
use crate::mm::{address_space, memory, virtual_memory};
use crate::platform::{
    audit, balloon, block, entropy, health, identity, inference, mode, network, payload, pe,
    root_key, secrets, services, session, storage, time, trace,
};
use crate::sched::{native_task, scheduler};
#[cfg(feature = "test-hooks")]
use crate::test_hooks;
use crate::{boot, debug};
#[cfg(feature = "test-hooks")]
use core::cell::Cell;

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
    check!(b"machine identity", entropy::self_check() && identity::self_check() && machine.valid(),);
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
    let Some(mut shared_owner) = address_space::AddressSpace::new(&mut memory) else {
        fail!(b"shared page owner");
    };
    let Some(mut shared_borrower) = address_space::AddressSpace::new(&mut memory) else {
        fail!(b"shared page borrower");
    };
    check!(
        b"shared page mapping",
        shared_owner
            .map_shared_owned(&mut memory)
            .is_some_and(|page| shared_borrower.map_shared_borrowed(page))
            && shared_borrower.release(&mut memory)
            && shared_owner.release(&mut memory),
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
    let network_pci = network_driver::discover(&devices);
    let mut network_device = network_pci.and_then(|network_pci| {
        let (bus, slot, _) = network_pci.location();
        let gsi = acpi.and_then(|tables| {
            tables.pci_gsi(bus, slot, network_pci.interrupt_pin().checked_sub(1)?)
        });
        gsi.and_then(|gsi| network_driver::Device::bind(network_pci, gsi, &mut memory))
    });
    check!(
        b"block device",
        block_driver::self_check()
            && block::self_check()
            && block::NAME == b"virtio-block"
            && block_device.info().valid()
            && block_device.diagnostics() == (0, 0, false),
    );
    check!(b"network driver", network_driver::self_check());
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
    let Some(session_store_read_capability) = capabilities
        .grant_scoped(capabilities::CapabilityKind::StoreRead, storage::TERMINAL_NAMESPACE.0)
    else {
        fail!(b"capabilities");
    };
    let Some(session_store_write_capability) = capabilities
        .grant_scoped(capabilities::CapabilityKind::StoreWrite, storage::TERMINAL_NAMESPACE.0)
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
        &[
            recovery_capability,
            session_service_capability,
            session_input_capability,
            session_display_capability,
            session_capability,
            session_store_read_capability,
            session_store_write_capability,
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
    #[cfg(feature = "test-hooks")]
    let Some(read_only_session) = session::Context::new(
        session::Id(4),
        session::Principal::LOCAL,
        &[session_capability, session_store_read_capability],
    ) else {
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
    let network_service_capability = capabilities.grant(capabilities::CapabilityKind::Service);
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
    let network_service_handle = network_service_capability
        .and_then(|capability| services.register(&capabilities, capability, network::SERVICE));
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
            &mut memory as *mut _,
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
    let mut native_network = (network_device.is_some() && network_service_handle.is_some())
        .then(|| native_task::Task::load(&mut memory, payloads.network, &privilege))
        .flatten();
    check!(b"storage heap", native_storage.map_heap(&mut memory).is_some());
    let mut shared_pages = logos_core::shared_pages::SharedPages::new();
    let terminal_owner = session.principal().page_owner();
    let storage_owner = storage_service_handle.principal().page_owner();
    let network_owner = network_service_handle.map(|handle| handle.principal().page_owner());
    let shared_history = native_terminal.map_shared_owned(&mut memory).and_then(|page| {
        shared_pages
            .register(terminal_owner, page, 1)
            .filter(|_| native_storage.map_shared_borrowed(page))
    });
    let Some(shared_history) = shared_history else {
        fail!(b"terminal storage page");
    };
    let Some((storage_block_physical, storage_block_virtual)) =
        native_storage.map_block_owned(&mut memory)
    else {
        fail!(b"storage block page");
    };
    #[cfg_attr(not(feature = "test-hooks"), allow(unused_mut))]
    let Some(mut storage_block_page) =
        shared_pages.register(storage_owner, storage_block_physical, 1)
    else {
        fail!(b"storage block page");
    };
    check!(
        b"terminal storage page",
        shared_pages.address(terminal_owner, shared_history).is_some()
    );
    check!(b"shared pages", logos_core::shared_pages::self_check());
    let network_setup = native_network
        .as_mut()
        .and_then(|task| task.map_network_owned(&mut memory))
        .and_then(|((rx_physical, rx_virtual), (tx_physical, tx_virtual))| {
            let owner = network_owner?;
            let rx = shared_pages.register(owner, rx_physical, 2)?;
            let Some(tx) = shared_pages.register(owner, tx_physical, 2) else {
                let _ = shared_pages.release(owner, rx);
                return None;
            };
            Some((owner, rx, rx_physical, rx_virtual, tx, tx_physical, tx_virtual))
        });
    if network_setup.is_none() {
        if let Some(task) = native_network.take() {
            let _ = task.release(&mut memory);
        }
    }
    check!(b"network service pages", network_device.is_none() || network_setup.is_some());
    let mut service_task = virtio::ServiceTask::new(
        &mut virtio_service,
        &channel,
        &responses,
        &capabilities,
        service_capability,
        virtio_handle.principal(),
        &mut memory as *mut _,
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
    check!(b"terminal model", terminal::Model::self_check());
    check!(b"text font", text::Service::self_check());
    console_mode.announce();
    check!(b"trace", trace::self_check());
    let native_input = native_terminal.input_endpoint();
    let native_command = native_terminal.syscall_endpoint();
    let native_display = native_terminal.display_endpoint();
    let native_sessions_endpoint = native_sessions.session_endpoint();
    let native_storage_block = native_storage.block_endpoint();
    let native_store = native_terminal.store_endpoint();
    let native_storage_store = native_storage.store_endpoint();
    let native_network_endpoint = native_network.as_ref().map(native_task::Task::network_endpoint);
    check!(
        b"storage shared page",
        native_store.configure_shared_page(shared_history)
            && native_storage_store.configure_shared_page(shared_history),
    );
    check!(
        b"storage block page",
        native_storage_block.configure(logos_core::native_service::BlockPage {
            handle: storage_block_page,
            address: storage_block_virtual,
        }),
    );
    let network_dma = network_setup.as_ref().map(|(_, _, rx_physical, _, _, tx_physical, _)| {
        NetworkDmaPages { rx_address: *rx_physical, tx_address: *tx_physical }
    });
    let network_pages_ready = native_network_endpoint.zip(network_setup).is_some_and(
        |(endpoint, (_, rx, _, rx_address, tx, _, tx_address))| {
            endpoint.configure(logos_core::native_service::NetworkPages {
                rx_handle: rx,
                rx_address,
                tx_handle: tx,
                tx_address,
            })
        },
    );
    check!(
        b"network page configuration",
        network_device.is_none() || native_network_endpoint.is_none() || network_pages_ready
    );
    let mut block_dispatch = block::Dispatch::new();
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
    #[cfg_attr(not(feature = "test-hooks"), allow(unused_mut))]
    let Some(mut storage_handle) = native_scheduler.spawn(&mut native_storage) else {
        fail!(b"native storage task");
    };
    if !native_scheduler.run_next() {
        fail!(b"native storage ready");
    }
    check!(
        b"storage startup",
        run_storage_startup(
            &mut block_dispatch,
            &mut block::DispatchContext {
                endpoint: native_storage_block,
                pages: &mut shared_pages,
                store_owner: storage_owner,
                store_page: storage_block_page,
                device: &mut block_device,
                memory: &mut memory,
            },
            &mut native_scheduler,
            storage_handle
        ),
    );
    let network_handle = if let Some(network_task) = native_network.as_mut()
        && let Some(handle) = native_scheduler.spawn(network_task)
    {
        if native_scheduler.run_next() && !native_scheduler.failed(handle) {
            Some(handle)
        } else {
            debug::write_line(b"LogOS: network service unavailable");
            None
        }
    } else {
        None
    };
    let mut network_pending = None;
    let mut network_probe = Some(0x8000_0001u32);
    let mut network_probe_due = 0;
    let mut network_reported = false;
    health.finish();
    let mut store_relay_state = StoreRelayState::new();
    if !native_input.deliver(logos_abi::InputEvent::STARTUP) {
        fail!(b"terminal history startup");
    }
    if !native_scheduler.wake(native_handle) {
        fail!(b"terminal history startup");
    }
    if !native_scheduler.run(native_handle) {
        fail!(b"terminal history startup");
    }
    let terminal_history_startup = relay_terminal_store_requests(
        native_store,
        native_storage_store,
        &mut block_dispatch,
        &mut block::DispatchContext {
            endpoint: native_storage_block,
            pages: &mut shared_pages,
            store_owner: storage_owner,
            store_page: storage_block_page,
            device: &mut block_device,
            memory: &mut memory,
        },
        terminal_owner,
        storage_owner,
        shared_history,
        &mut native_scheduler,
        native_handle,
        storage_handle,
        &session,
        &capabilities,
        &mut store_relay_state,
        interrupts::ticks(),
    );
    debug::write_line(if terminal_history_startup {
        b"LogOS: terminal history startup relay passed"
    } else {
        b"LogOS: terminal history startup relay failed"
    });
    if !terminal_history_startup
        || !resume_display(
            native_display,
            &session,
            &capabilities,
            session_display_capability,
            &mut native_scheduler,
            native_handle,
        )
    {
        fail!(b"terminal history startup");
    }
    #[cfg(feature = "test-hooks")]
    let proof = Cell::new(false);
    #[cfg(feature = "test-hooks")]
    let network_device_ptr = &mut network_device as *mut Option<network_driver::Device>;
    #[cfg(feature = "test-hooks")]
    let native_scheduler_ptr = &mut native_scheduler as *mut _;
    #[cfg(feature = "test-hooks")]
    let network_pending_ptr = &mut network_pending as *mut Option<PendingNetworkDevice>;
    #[cfg(feature = "test-hooks")]
    let network_probe_ptr = &mut network_probe as *mut Option<u32>;
    #[cfg(feature = "test-hooks")]
    let network_probe_due_ptr = &mut network_probe_due as *mut u64;
    #[cfg(feature = "test-hooks")]
    let network_reported_ptr = &mut network_reported as *mut bool;
    #[cfg(feature = "test-hooks")]
    test_hooks::serve(
        unsafe {
            logos_core::native_service::Context::storage_status_at(native_storage_store.context())
        }
        .unwrap_or(logos_core::native_service::STORAGE_IO_FAILED),
        |value| {
            let tick = interrupts::ticks();
            if service_scheduler.run_next() {
                let _ = service_health.beat(balloon::NAME, tick);
            }
            if value == "__reset" {
                proof.set(false);
                let Some(lifecycle) = supervisor::Lifecycle::new(&supervisor, balloon::NAME) else {
                    return false;
                };
                service_lifecycle = lifecycle;
                debug::write_line(b"LogOS: reset begin");
                let previous_terminal = native_handle;
                if !native_scheduler.fail(previous_terminal) || !startup.start() {
                    return false;
                }
                let Some(restarted_terminal) =
                    restart_native_service(&mut native_scheduler, previous_terminal)
                else {
                    return false;
                };
                native_handle = restarted_terminal;
                store_relay_state.clear();
                debug::write_line(b"LogOS: reset terminal ready");
                if !native_store.configure_shared_page(shared_history)
                    || !native_scheduler.run(native_handle)
                    || native_scheduler.wake(previous_terminal)
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

                let previous_sessions = sessions_handle;
                if !native_scheduler.fail(previous_sessions) || !startup.start() {
                    return false;
                }
                let Some(restarted_sessions) =
                    restart_native_service(&mut native_scheduler, previous_sessions)
                else {
                    return false;
                };
                sessions_handle = restarted_sessions;
                debug::write_line(b"LogOS: reset sessions ready");
                if !native_scheduler.run(sessions_handle)
                    || native_scheduler.wake(previous_sessions)
                {
                    return false;
                }

                let previous_storage = storage_handle;
                block_dispatch.cancel_on_exit(&mut block::DispatchContext {
                    endpoint: native_storage_block,
                    pages: &mut shared_pages,
                    store_owner: storage_owner,
                    store_page: storage_block_page,
                    device: &mut block_device,
                    memory: &mut memory,
                });
                let Some(page) = shared_pages.release(storage_owner, storage_block_page) else {
                    return false;
                };
                let Some(replacement) = shared_pages.register(storage_owner, page, 1) else {
                    return false;
                };
                storage_block_page = replacement;
                if !native_scheduler.fail(previous_storage) || !startup.start() {
                    return false;
                }
                let Some(restarted_storage) = native_scheduler.restart(previous_storage) else {
                    return false;
                };
                if !native_storage_store.configure_shared_page(shared_history)
                    || !native_storage_block.configure(logos_core::native_service::BlockPage {
                        handle: storage_block_page,
                        address: storage_block_virtual,
                    })
                    || !native_scheduler.run(restarted_storage)
                {
                    return false;
                }
                storage_handle = restarted_storage;
                debug::write_line(b"LogOS: reset storage ready");
                if native_scheduler.wake(previous_storage) {
                    return false;
                }
                if !run_storage_startup(
                    &mut block_dispatch,
                    &mut block::DispatchContext {
                        endpoint: native_storage_block,
                        pages: &mut shared_pages,
                        store_owner: storage_owner,
                        store_page: storage_block_page,
                        device: &mut block_device,
                        memory: &mut memory,
                    },
                    &mut native_scheduler,
                    storage_handle,
                ) {
                    return false;
                }
                if !native_input.deliver(logos_abi::InputEvent::STARTUP)
                    || !native_scheduler.wake(native_handle)
                    || !native_scheduler.run(native_handle)
                {
                    return false;
                }
                if !relay_terminal_store_requests(
                    native_store,
                    native_storage_store,
                    &mut block_dispatch,
                    &mut block::DispatchContext {
                        endpoint: native_storage_block,
                        pages: &mut shared_pages,
                        store_owner: storage_owner,
                        store_page: storage_block_page,
                        device: &mut block_device,
                        memory: &mut memory,
                    },
                    terminal_owner,
                    storage_owner,
                    shared_history,
                    &mut native_scheduler,
                    native_handle,
                    storage_handle,
                    &session,
                    &capabilities,
                    &mut store_relay_state,
                    interrupts::ticks(),
                ) || !resume_display(
                    native_display,
                    &session,
                    &capabilities,
                    session_display_capability,
                    &mut native_scheduler,
                    native_handle,
                ) {
                    return false;
                }
                return true;
            }
            if value == "persistence/block-read-flush" {
                debug::write_line(b"LogOS: storage proof passed");
                proof.set(true);
                return true;
            }
            if value == "persistence/block-timeout-reset" {
                let timeout_id = u32::MAX - 1;
                let timeout_request = logos_abi::BlockRequest {
                    id: timeout_id,
                    operation: logos_abi::BlockOperation::Flush,
                    lba: 0,
                    blocks: 0,
                    page: logos_abi::PageHandle(0),
                    deadline: 0,
                };
                let before = block_device.diagnostics();
                let timeout = unsafe {
                    logos_core::native_service::Context::request_block_at(
                        native_storage_block.context(),
                        timeout_request,
                    )
                } && block_dispatch
                    .poll(
                        &mut block::DispatchContext {
                            endpoint: native_storage_block,
                            pages: &mut shared_pages,
                            store_owner: storage_owner,
                            store_page: storage_block_page,
                            device: &mut block_device,
                            memory: &mut memory,
                        },
                        interrupts::ticks(),
                    )
                    .is_some_and(|reply| {
                        reply.id == timeout_id
                            && reply.status == logos_abi::PersistenceStatus::TimedOut
                            && native_storage_block.reply(reply)
                            && unsafe {
                                logos_core::native_service::Context::block_reply_at(
                                    native_storage_block.context(),
                                    timeout_id,
                                )
                                .is_some_and(|reply| {
                                    reply.status == logos_abi::PersistenceStatus::TimedOut
                                })
                            }
                    })
                    && native_scheduler.wake(storage_handle)
                    && native_scheduler.run(storage_handle);
                let after = block_device.diagnostics();
                if !timeout
                    || after.0 != before.0.saturating_add(1)
                    || after.1 != before.1.saturating_add(1)
                {
                    proof.set(false);
                    return false;
                }

                let read_id = timeout_id - 1;
                let read_request = logos_abi::BlockRequest {
                    id: read_id,
                    operation: logos_abi::BlockOperation::Read,
                    lba: 0,
                    blocks: 1,
                    page: storage_block_page,
                    deadline: interrupts::ticks().saturating_add(100_000),
                };
                let read = unsafe {
                    logos_core::native_service::Context::request_block_at(
                        native_storage_block.context(),
                        read_request,
                    )
                };
                let read = if read {
                    loop {
                        let Some(reply) = block_dispatch.poll(
                            &mut block::DispatchContext {
                                endpoint: native_storage_block,
                                pages: &mut shared_pages,
                                store_owner: storage_owner,
                                store_page: storage_block_page,
                                device: &mut block_device,
                                memory: &mut memory,
                            },
                            interrupts::ticks(),
                        ) else {
                            interrupts::wait_for_virtio();
                            continue;
                        };
                        break reply.id == read_id
                            && reply.status == logos_abi::PersistenceStatus::Complete
                            && native_storage_block.reply(reply)
                            && unsafe {
                                logos_core::native_service::Context::block_reply_at(
                                    native_storage_block.context(),
                                    read_id,
                                )
                                .is_some_and(|reply| {
                                    reply.status == logos_abi::PersistenceStatus::Complete
                                })
                            }
                            && native_scheduler.wake(storage_handle)
                            && native_scheduler.run(storage_handle);
                    }
                } else {
                    false
                };
                proof.set(read);
                return read;
            }
            if value == "persistence/terminal-history" {
                let status = unsafe {
                    logos_core::native_service::Context::storage_status_at(
                        native_storage_store.context(),
                    )
                };
                if status == Some(logos_core::native_service::STORAGE_FORMATTED) {
                    proof.set(true);
                    return true;
                }
                if !matches!(
                    status,
                    Some(logos_core::native_service::STORAGE_RECOVERED)
                        | Some(logos_core::native_service::STORAGE_RECOVERED_INCOMPLETE)
                ) {
                    return false;
                }
                input.set_layout(input::Layout::Azerty);
                let mut send = |event: logos_abi::InputEvent, expected: Option<&[u8]>| {
                    if !native_input.deliver(event) {
                        return false;
                    }
                    if !native_scheduler.wake(native_handle) || !native_scheduler.run(native_handle)
                    {
                        return false;
                    }
                    if !relay_terminal_store_requests(
                        native_store,
                        native_storage_store,
                        &mut block_dispatch,
                        &mut block::DispatchContext {
                            endpoint: native_storage_block,
                            pages: &mut shared_pages,
                            store_owner: storage_owner,
                            store_page: storage_block_page,
                            device: &mut block_device,
                            memory: &mut memory,
                        },
                        terminal_owner,
                        storage_owner,
                        shared_history,
                        &mut native_scheduler,
                        native_handle,
                        storage_handle,
                        &session,
                        &capabilities,
                        &mut store_relay_state,
                        interrupts::ticks(),
                    ) {
                        return false;
                    }
                    if !resume_display(
                        native_display,
                        &session,
                        &capabilities,
                        session_display_capability,
                        &mut native_scheduler,
                        native_handle,
                    ) {
                        return false;
                    }
                    let Some(request) = native_command.request() else {
                        return expected.is_none();
                    };
                    let matched = expected.is_some_and(|expected| {
                        request.syscall == logos_abi::Syscall::SetInputLayout
                            && request.argument[..request.length] == *expected
                    });
                    if !native_command.reply(&[]) {
                        return false;
                    }
                    if !native_scheduler.wake(native_handle)
                        || !native_scheduler.run(native_handle)
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
                    matched
                };
                let navigation = [
                    (logos_abi::InputEvent::UP, None),
                    (logos_abi::InputEvent::UP, None),
                    (logos_abi::InputEvent::DOWN, None),
                ]
                .into_iter()
                .all(|(event, expected)| send(event, expected))
                    && send(
                        logos_abi::InputEvent::ENTER,
                        Some(&[logos_abi::InputLayout::Qwerty.wire()]),
                    )
                    && [logos_abi::InputEvent::UP; 5].into_iter().all(|event| send(event, None))
                    && send(
                        logos_abi::InputEvent::ENTER,
                        Some(&[logos_abi::InputLayout::Azerty.wire()]),
                    );
                proof.set(proof.get() || navigation);
                return navigation;
            }
            let terminal_restart = value == "assert-terminal-service-restart";
            let sessions_restart = value == "assert-sessions-service-restart";
            let storage_restart = value == "assert-storage-service-restart";
            if terminal_restart {
                let previous = native_handle;
                if !native_scheduler.fail(previous) || !startup.start() {
                    return false;
                }
                let Some(restarted) = restart_native_service(&mut native_scheduler, previous)
                else {
                    return false;
                };
                native_handle = restarted;
                store_relay_state.clear();
                if native_scheduler.wake(previous)
                    || !native_store.configure_shared_page(shared_history)
                    || !native_scheduler.run(native_handle)
                    || !resume_display(
                        native_display,
                        &session,
                        &capabilities,
                        session_display_capability,
                        &mut native_scheduler,
                        native_handle,
                    )
                    || !native_input.deliver(logos_abi::InputEvent::STARTUP)
                    || !native_scheduler.wake(native_handle)
                    || !native_scheduler.run(native_handle)
                    || !relay_terminal_store_requests(
                        native_store,
                        native_storage_store,
                        &mut block_dispatch,
                        &mut block::DispatchContext {
                            endpoint: native_storage_block,
                            pages: &mut shared_pages,
                            store_owner: storage_owner,
                            store_page: storage_block_page,
                            device: &mut block_device,
                            memory: &mut memory,
                        },
                        terminal_owner,
                        storage_owner,
                        shared_history,
                        &mut native_scheduler,
                        native_handle,
                        storage_handle,
                        &session,
                        &capabilities,
                        &mut store_relay_state,
                        interrupts::ticks(),
                    )
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
                let Some(restarted) = restart_native_service(&mut native_scheduler, previous)
                else {
                    return false;
                };
                sessions_handle = restarted;
                if !native_scheduler.run(sessions_handle) || native_scheduler.wake(previous) {
                    return false;
                }
            }
            if storage_restart {
                let previous = storage_handle;
                block_dispatch.cancel_on_exit(&mut block::DispatchContext {
                    endpoint: native_storage_block,
                    pages: &mut shared_pages,
                    store_owner: storage_owner,
                    store_page: storage_block_page,
                    device: &mut block_device,
                    memory: &mut memory,
                });
                let Some(page) = shared_pages.release(storage_owner, storage_block_page) else {
                    return false;
                };
                let Some(replacement) = shared_pages.register(storage_owner, page, 1) else {
                    return false;
                };
                storage_block_page = replacement;
                if !native_scheduler.fail(previous) || !startup.start() {
                    return false;
                }
                let Some(restarted) = native_scheduler.restart(previous) else {
                    return false;
                };
                if !native_storage_store.configure_shared_page(shared_history)
                    || !native_storage_block.configure(logos_core::native_service::BlockPage {
                        handle: storage_block_page,
                        address: storage_block_virtual,
                    })
                    || !native_scheduler.run(restarted)
                {
                    return false;
                };
                storage_handle = restarted;
                store_relay_state.clear();
                if native_scheduler.wake(previous) || !native_scheduler.wake(restarted) {
                    return false;
                }
            }
            if value == "persistence/write-interruption" || value == "persistence/recovery" {
                let status = unsafe {
                    logos_core::native_service::Context::storage_status_at(
                        native_storage_store.context(),
                    )
                };
                let passed = matches!(
                    status,
                    Some(logos_core::native_service::STORAGE_RECOVERED)
                        | Some(logos_core::native_service::STORAGE_RECOVERED_INCOMPLETE)
                );
                proof.set(proof.get() || passed);
                return passed;
            }
            if value == "persistence/corruption-detected" {
                let status = unsafe {
                    logos_core::native_service::Context::storage_status_at(
                        native_storage_store.context(),
                    )
                };
                let passed = status == Some(logos_core::native_service::STORAGE_CORRUPT);
                proof.set(proof.get() || passed);
                return passed;
            }
            if value == "persistence/capability-denied" {
                let history_page = shared_history;
                let mut denied =
                    |request: logos_abi::StoreRequest, request_session: &session::Context| {
                        let delivered = unsafe {
                            logos_core::native_service::Context::request_store_at(
                                native_store.context(),
                                request,
                            )
                        } || native_store.deliver(request);
                        if !delivered {
                            return false;
                        }
                        let relayed = relay_store_request(
                            native_store,
                            native_storage_store,
                            &mut block_dispatch,
                            &mut block::DispatchContext {
                                endpoint: native_storage_block,
                                pages: &mut shared_pages,
                                store_owner: storage_owner,
                                store_page: storage_block_page,
                                device: &mut block_device,
                                memory: &mut memory,
                            },
                            terminal_owner,
                            storage_owner,
                            history_page,
                            &mut native_scheduler,
                            storage_handle,
                            request_session,
                            &capabilities,
                            &mut store_relay_state,
                            interrupts::ticks(),
                        )
                        .ok();
                        if !relayed {
                            return false;
                        }
                        let response = native_store.response(request.id).is_some_and(|reply| {
                            reply.id == request.id
                                && reply.status == logos_abi::PersistenceStatus::Denied
                        });
                        if !response {
                            return false;
                        }
                        if !native_scheduler.wake(native_handle)
                            || !native_scheduler.run(native_handle)
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
                        true
                    };
                let open = test_store_request(
                    0x51,
                    logos_abi::StoreOperation::OpenRead,
                    storage::TERMINAL_NAMESPACE,
                    logos_abi::VersionSelector::Current,
                );
                let restricted = denied(open, &restricted_session);
                let cross_namespace = denied(
                    test_store_request(
                        0x52,
                        logos_abi::StoreOperation::OpenRead,
                        storage::TEXT_NAMESPACE,
                        logos_abi::VersionSelector::Current,
                    ),
                    &session,
                );
                let read_only_write = denied(
                    test_store_request(
                        0x53,
                        logos_abi::StoreOperation::BeginReplace,
                        storage::TERMINAL_NAMESPACE,
                        logos_abi::VersionSelector::None,
                    ),
                    &read_only_session,
                );
                let passed = restricted && cross_namespace && read_only_write;
                proof.set(proof.get() || passed);
                return passed;
            }
            if value == "assert-sessions" {
                let passed = native_sessions_endpoint.deliver(logos_abi::SessionRequest::new(
                    logos_abi::Syscall::Tasks,
                    [0; logos_abi::MAX_SESSION_TEXT],
                    0,
                )) && native_scheduler.wake(sessions_handle)
                    && native_scheduler.run(sessions_handle)
                    && native_sessions_endpoint.effect().is_some_and(|effect| {
                        effect.effect == logos_abi::Effect::ReadTasks
                            && native_sessions_endpoint
                                .reply_effect(logos_abi::EffectResult::TasksActive)
                    })
                    && native_scheduler.wake(sessions_handle)
                    && native_scheduler.run(sessions_handle)
                    && native_sessions_endpoint.reply().is_some_and(|reply| {
                        reply.length == b"scheduler active".len()
                            && reply.text[..reply.length] == *b"scheduler active"
                    });
                proof.set(proof.get() || passed);
                return passed;
            }
            if value == "assert-crash-restart" {
                let tick = interrupts::ticks();
                let passed =
                    service_lifecycle.failed(tick) && service_lifecycle.due(tick.saturating_add(2));
                proof.set(proof.get() || passed);
                return passed;
            }
            if value == "assert-restart-backoff" {
                let tick = interrupts::ticks();
                let passed = service_lifecycle.failed(tick)
                    && !service_lifecycle.due(tick.saturating_add(1))
                    && service_lifecycle.due(tick.saturating_add(2))
                    && service_lifecycle.failed(tick.saturating_add(2))
                    && !service_lifecycle.due(tick.saturating_add(5))
                    && service_lifecycle.due(tick.saturating_add(6));
                proof.set(proof.get() || passed);
                return passed;
            }
            let proof_input = value.starts_with("assert-") || value.starts_with("deny-");
            let deny_display = value == "deny-display";
            let (value, request_session, expected, expect_qwerty) = if terminal_restart
                || sessions_restart
            {
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
                let passed = logos_abi::InputEvent::from_byte(b'x').is_some_and(|event| {
                    native_input.deliver(event)
                        && native_scheduler.wake(native_handle)
                        && native_scheduler.run(native_handle)
                        && !resume_display(
                            native_display,
                            &denied_session,
                            &capabilities,
                            session_display_capability,
                            &mut native_scheduler,
                            native_handle,
                        )
                });
                proof.set(proof.get() || passed);
                return passed;
            }
            let passed = value.bytes().chain(core::iter::once(b'\n')).all(|byte| {
                logos_abi::InputEvent::from_byte(byte)
                    .is_some_and(|event| native_input.deliver(event))
                    && native_scheduler.wake(native_handle)
                    && native_scheduler.run(native_handle)
                    && relay_terminal_store_requests(
                        native_store,
                        native_storage_store,
                        &mut block_dispatch,
                        &mut block::DispatchContext {
                            endpoint: native_storage_block,
                            pages: &mut shared_pages,
                            store_owner: storage_owner,
                            store_page: storage_block_page,
                            device: &mut block_device,
                            memory: &mut memory,
                        },
                        terminal_owner,
                        storage_owner,
                        shared_history,
                        &mut native_scheduler,
                        native_handle,
                        storage_handle,
                        &session,
                        &capabilities,
                        &mut store_relay_state,
                        interrupts::ticks(),
                    )
                    && resume_display(
                        native_display,
                        &session,
                        &capabilities,
                        session_display_capability,
                        &mut native_scheduler,
                        native_handle,
                    )
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
                                && native_scheduler.run(native_handle)
                                && resume_display(
                                    native_display,
                                    &session,
                                    &capabilities,
                                    session_display_capability,
                                    &mut native_scheduler,
                                    native_handle,
                                )
                        }))
            });
            if proof_input {
                proof.set(proof.get() || passed);
            }
            passed
        },
        || {
            // SAFETY: the test hook is single-threaded and never outlives these locals.
            let _ = poll_network(
                unsafe { (*network_device_ptr).as_mut() },
                native_network_endpoint,
                network_handle,
                network_dma,
                unsafe { &mut *native_scheduler_ptr },
                unsafe { &mut *network_pending_ptr },
                unsafe { &mut *network_probe_ptr },
                unsafe { &mut *network_probe_due_ptr },
                unsafe { &mut *network_reported_ptr },
                interrupts::ticks(),
            );
        },
        |id| {
            id == "core/boot-normal"
                || (cfg!(feature = "block-probe") && id == "persistence/block-read-flush")
                || (id == "network/transport-dhcp" && network_reported)
                || proof.get()
        },
    );
    // ponytail: one bootstrap retry; use supervisor policy when native services join System lifecycle.
    let mut terminal_restart_available = true;
    let mut sessions_restart_available = true;
    if console_mode == mode::ConsoleMode::Normal {
        while keyboard::poll_scancode().is_some() {}
        input = input::Service::new();
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
            if !dispatch_store_block(
                &mut block_dispatch,
                &mut block::DispatchContext {
                    endpoint: native_storage_block,
                    pages: &mut shared_pages,
                    store_owner: storage_owner,
                    store_page: storage_block_page,
                    device: &mut block_device,
                    memory: &mut memory,
                },
                &mut native_scheduler,
                storage_handle,
                tick,
            ) {
                debug::write_line(b"LogOS: storage block reply failed");
            }
            if !poll_network(
                network_device.as_mut(),
                native_network_endpoint,
                network_handle,
                network_dma,
                &mut native_scheduler,
                &mut network_pending,
                &mut network_probe,
                &mut network_probe_due,
                &mut network_reported,
                tick,
            ) {
                debug::write_line(b"LogOS: network service unavailable");
            }
            if let Some(event) = input.next(tick, keyboard::poll_scancode) {
                if let Some(native_event) = native_input_event(event) {
                    if !native_input.deliver(native_event)
                        || !native_scheduler.wake(native_handle)
                        || !native_scheduler.run(native_handle)
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
                            if !cancel_store_transaction(
                                native_storage_store,
                                &mut native_scheduler,
                                storage_handle,
                            ) {
                                console_mode = mode::ConsoleMode::Recovery;
                                break;
                            }
                            if let Some(restarted) =
                                restart_native_service(&mut native_scheduler, native_handle)
                            {
                                native_handle = restarted;
                                store_relay_state.clear();
                                if native_scheduler.wake(native_handle)
                                    && native_scheduler.run(native_handle)
                                    && resume_display(
                                        native_display,
                                        &session,
                                        &capabilities,
                                        session_display_capability,
                                        &mut native_scheduler,
                                        native_handle,
                                    )
                                {
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
                    if native_store.request().is_some() {
                        if !relay_terminal_store_requests(
                            native_store,
                            native_storage_store,
                            &mut block_dispatch,
                            &mut block::DispatchContext {
                                endpoint: native_storage_block,
                                pages: &mut shared_pages,
                                store_owner: storage_owner,
                                store_page: storage_block_page,
                                device: &mut block_device,
                                memory: &mut memory,
                            },
                            terminal_owner,
                            storage_owner,
                            shared_history,
                            &mut native_scheduler,
                            native_handle,
                            storage_handle,
                            &session,
                            &capabilities,
                            &mut store_relay_state,
                            tick,
                        ) || !resume_display(
                            native_display,
                            &session,
                            &capabilities,
                            session_display_capability,
                            &mut native_scheduler,
                            native_handle,
                        ) {
                            debug::write_line(b"LogOS: Store relay failed");
                            console_mode = mode::ConsoleMode::Recovery;
                            break;
                        }
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
                                    || !native_scheduler.run(native_handle)
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
            let _ = dispatch_store_block(
                &mut block_dispatch,
                &mut block::DispatchContext {
                    endpoint: native_storage_block,
                    pages: &mut shared_pages,
                    store_owner: storage_owner,
                    store_page: storage_block_page,
                    device: &mut block_device,
                    memory: &mut memory,
                },
                &mut native_scheduler,
                storage_handle,
                tick,
            );
            let _ = poll_network(
                network_device.as_mut(),
                native_network_endpoint,
                network_handle,
                network_dma,
                &mut native_scheduler,
                &mut network_pending,
                &mut network_probe,
                &mut network_probe_due,
                &mut network_reported,
                tick,
            );
        })
    }
    loop {
        unsafe { core::arch::asm!("cli", "hlt") };
    }
}

fn dispatch_store_block(
    dispatch: &mut block::Dispatch,
    context: &mut block::DispatchContext<'_>,
    scheduler: &mut scheduler::Scheduler<'_>,
    handle: scheduler::TaskHandle,
    tick: u64,
) -> bool {
    let Some(reply) = dispatch.poll(context, tick) else {
        return true;
    };
    context.endpoint.reply(reply) && scheduler.wake(handle) && scheduler.run(handle)
}

#[allow(clippy::too_many_arguments)]
fn poll_network(
    device: Option<&mut network_driver::Device>,
    endpoint: Option<native_task::NetworkEndpoint>,
    handle: Option<scheduler::TaskHandle>,
    dma: Option<NetworkDmaPages>,
    scheduler: &mut scheduler::Scheduler<'_>,
    pending: &mut Option<PendingNetworkDevice>,
    probe: &mut Option<u32>,
    probe_due: &mut u64,
    reported: &mut bool,
    tick: u64,
) -> bool {
    let (Some(device), Some(endpoint), Some(handle)) = (device, endpoint, handle) else {
        return true;
    };
    if let Some(request) = pending.as_ref().copied() {
        if tick >= request.request.deadline {
            debug::write_line(b"LogOS: network TX timeout");
            let reset = device.reset();
            let info = device.info();
            let reply = logos_abi::NetworkDeviceReply {
                id: request.request.id,
                status: if reset {
                    logos_abi::NetworkStatus::TimedOut
                } else {
                    logos_abi::NetworkStatus::Reset
                },
                generation: info.generation,
                info: network_info(info),
            };
            let delivered = if unsafe {
                logos_core::native_service::Context::network_waiting_at(endpoint.context())
            } {
                endpoint.deliver_device_reply(reply)
            } else {
                endpoint.reply_device(reply)
            };
            *pending = None;
            return delivered && scheduler.wake(handle) && scheduler.run(handle);
        }
        match device.complete_transmit() {
            Ok(Some(())) => {
                let info = device.info();
                let reply = logos_abi::NetworkDeviceReply {
                    id: request.request.id,
                    status: logos_abi::NetworkStatus::Complete,
                    generation: info.generation,
                    info: network_info(info),
                };
                let delivered = if unsafe {
                    logos_core::native_service::Context::network_waiting_at(endpoint.context())
                } {
                    endpoint.deliver_device_reply(reply)
                } else {
                    endpoint.reply_device(reply)
                };
                *pending = None;
                return delivered && scheduler.wake(handle) && scheduler.run(handle);
            }
            Ok(None) => return true,
            Err(_) => {
                debug::write_line(b"LogOS: network TX completion invalid");
                let _ = device.reset();
                let info = device.info();
                let reply = logos_abi::NetworkDeviceReply {
                    id: request.request.id,
                    status: logos_abi::NetworkStatus::Reset,
                    generation: info.generation,
                    info: network_info(info),
                };
                let delivered = if unsafe {
                    logos_core::native_service::Context::network_waiting_at(endpoint.context())
                } {
                    endpoint.deliver_device_reply(reply)
                } else {
                    endpoint.reply_device(reply)
                };
                *pending = None;
                return delivered && scheduler.wake(handle) && scheduler.run(handle);
            }
        }
    }
    if unsafe { logos_core::native_service::Context::network_device_pending_at(endpoint.context()) }
        && endpoint.device_request().is_none()
    {
        debug::write_line(b"LogOS: invalid network device request");
        return false;
    }
    if let Some(request) = endpoint.device_request() {
        debug::write_line(b"LogOS: network device request");
        let info = device.info();
        let response = match request.operation {
            logos_abi::NetworkDeviceOperation::Info => Some(logos_abi::NetworkDeviceReply {
                id: request.id,
                status: logos_abi::NetworkStatus::Complete,
                generation: info.generation,
                info: network_info(info),
            }),
            logos_abi::NetworkDeviceOperation::Reset => {
                let status = if request.generation == info.generation && device.reset() {
                    logos_abi::NetworkStatus::Complete
                } else {
                    logos_abi::NetworkStatus::Reset
                };
                let info = device.info();
                Some(logos_abi::NetworkDeviceReply {
                    id: request.id,
                    status,
                    generation: info.generation,
                    info: network_info(info),
                })
            }
            logos_abi::NetworkDeviceOperation::Transmit => {
                if request.generation != info.generation {
                    Some(logos_abi::NetworkDeviceReply {
                        id: request.id,
                        status: logos_abi::NetworkStatus::Reset,
                        generation: info.generation,
                        info: network_info(info),
                    })
                } else {
                    let Some(dma) = dma else {
                        return false;
                    };
                    let frame = unsafe {
                        core::slice::from_raw_parts(
                            dma.tx_address as *const u8,
                            usize::from(request.length),
                        )
                    };
                    match device.transmit(frame) {
                        Ok(()) => {
                            *pending = Some(PendingNetworkDevice { request });
                            return scheduler.wake(handle) && scheduler.run(handle);
                        }
                        Err(error) => Some(logos_abi::NetworkDeviceReply {
                            id: request.id,
                            status: match error {
                                network_driver::NetworkError::Busy => {
                                    logos_abi::NetworkStatus::Busy
                                }
                                network_driver::NetworkError::Length => {
                                    logos_abi::NetworkStatus::Invalid
                                }
                                network_driver::NetworkError::Device => {
                                    logos_abi::NetworkStatus::Io
                                }
                            },
                            generation: info.generation,
                            info: network_info(info),
                        }),
                    }
                }
            }
        };
        if let Some(reply) = response {
            return endpoint.reply_device(reply) && scheduler.wake(handle) && scheduler.run(handle);
        }
        return true;
    }
    if let Some(id) = *probe {
        if let Some(reply) = endpoint.response(id) {
            if reply.status == logos_abi::NetworkStatus::Complete
                && reply.info.configuration == 1
                && reply.info.ipv4 == u32::from_be_bytes([10, 0, 2, 15])
                && reply.info.subnet_mask == u32::from_be_bytes([255, 255, 255, 0])
                && reply.info.router == u32::from_be_bytes([10, 0, 2, 2])
            {
                debug::write_line(
                    b"LOGOS/1 NETWORK transport-dhcp status=bound ipv4=10.0.2.15 mask=255.255.255.0 router=10.0.2.2",
                );
                *reported = true;
                *probe = None;
            } else {
                *probe = Some(id.wrapping_add(1).max(1));
                *probe_due = tick.saturating_add(64);
            }
            let ok = scheduler.wake(handle) && scheduler.run(handle);
            return ok;
        }
    }
    if !unsafe { logos_core::native_service::Context::network_waiting_at(endpoint.context()) } {
        return true;
    }
    if !*reported {
        if tick >= *probe_due {
            if let Some(id) = *probe {
                let request = logos_abi::NetworkRequest {
                    id,
                    operation: logos_abi::NetworkOperation::Status,
                    endpoint: logos_abi::NetworkEndpoint(0),
                    peer: logos_abi::NetworkScope(0),
                    page: logos_abi::PageHandle(0),
                    length: 0,
                    generation: 0,
                    deadline: u64::MAX / 2,
                };
                if endpoint.deliver(request) {
                    return scheduler.wake(handle) && scheduler.run(handle);
                }
            }
        }
    }
    if unsafe { logos_core::native_service::Context::network_deadline_at(endpoint.context()) }
        .is_some_and(|deadline| tick >= deadline)
    {
        let event = logos_abi::NetworkEvent {
            id: tick.try_into().unwrap_or(1).max(1),
            kind: logos_abi::NetworkEventKind::Timer,
            generation: device.info().generation,
            length: 0,
            now: tick.max(1),
        };
        return endpoint.deliver_event(event) && scheduler.wake(handle) && scheduler.run(handle);
    }
    let Some(dma) = dma else {
        return false;
    };
    let frame = unsafe {
        core::slice::from_raw_parts_mut(dma.rx_address as *mut u8, logos_net::ETHERNET_MAX_FRAME)
    };
    match device.receive(frame) {
        Ok(Some(length)) => {
            let event = logos_abi::NetworkEvent {
                id: tick.try_into().unwrap_or(1).max(1),
                kind: logos_abi::NetworkEventKind::Frame,
                generation: device.info().generation,
                length: length as u16,
                now: tick.max(1),
            };
            endpoint.deliver_event(event) && scheduler.wake(handle) && scheduler.run(handle)
        }
        Ok(None) => true,
        Err(_) => device.reset(),
    }
}

#[derive(Clone, Copy)]
struct PendingNetworkDevice {
    request: logos_abi::NetworkDeviceRequest,
}

#[derive(Clone, Copy)]
struct NetworkDmaPages {
    rx_address: u64,
    tx_address: u64,
}

fn network_info(info: network_driver::Info) -> logos_abi::NetworkInfo {
    logos_abi::NetworkInfo {
        mac: info.mac,
        mtu: info.mtu,
        generation: info.generation,
        link_up: 1,
        ..logos_abi::NetworkInfo::default()
    }
}

fn run_storage_startup(
    dispatch: &mut block::Dispatch,
    context: &mut block::DispatchContext<'_>,
    scheduler: &mut scheduler::Scheduler<'_>,
    handle: scheduler::TaskHandle,
) -> bool {
    interrupts::enable();
    loop {
        if scheduler.failed(handle) {
            return false;
        }
        let waiting = unsafe {
            logos_core::native_service::Context::input_waiting_at(context.endpoint.context())
        };
        if waiting {
            debug::write_line(b"LogOS: startup waiting");
            let marker: &[u8] = match unsafe {
                logos_core::native_service::Context::storage_status_at(context.endpoint.context())
            } {
                Some(logos_core::native_service::STORAGE_FORMATTED) => b"LogOS: storage formatted",
                Some(logos_core::native_service::STORAGE_RECOVERED) => b"LogOS: storage recovered",
                Some(logos_core::native_service::STORAGE_RECOVERED_INCOMPLETE) => {
                    b"LogOS: storage recovered-incomplete"
                }
                Some(logos_core::native_service::STORAGE_CORRUPT) => b"LogOS: storage corrupt",
                Some(logos_core::native_service::STORAGE_IO_FAILED) => b"LogOS: storage io-failed",
                _ => {
                    return false;
                }
            };
            debug::write_line(marker);
            return true;
        }
        let Some(reply) = dispatch.poll(context, interrupts::ticks()) else {
            debug::write_line(if dispatch.accepts_new_request() {
                b"LogOS: storage startup no request"
            } else {
                b"LogOS: storage startup block pending"
            });
            if dispatch.accepts_new_request() {
                interrupts::wait_for_tick();
            } else {
                interrupts::wait_for_virtio();
            }
            continue;
        };
        if !context.endpoint.reply(reply) {
            return false;
        }
        if !scheduler.wake(handle) {
            return false;
        }
        if !scheduler.run(handle) {
            return false;
        }
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
                input::LogicalKey::Up => Some(logos_abi::InputEvent::UP.byte()),
                input::LogicalKey::Down => Some(logos_abi::InputEvent::DOWN.byte()),
                _ => None,
            })
        })
        .and_then(logos_abi::InputEvent::from_byte)
}

#[cfg(feature = "test-hooks")]
fn test_store_request(
    id: u32,
    operation: logos_abi::StoreOperation,
    namespace: logos_abi::NamespaceId,
    version: logos_abi::VersionSelector,
) -> logos_abi::StoreRequest {
    let mut name = [0; logos_abi::MAX_OBJECT_NAME];
    name[0] = b'x';
    let identifies = matches!(
        operation,
        logos_abi::StoreOperation::OpenRead | logos_abi::StoreOperation::BeginReplace
    );
    logos_abi::StoreRequest {
        id,
        operation,
        namespace: if identifies { namespace } else { logos_abi::NamespaceId(0) },
        name: if identifies { name } else { [0; logos_abi::MAX_OBJECT_NAME] },
        name_length: if identifies { 1 } else { 0 },
        version: if identifies { version } else { logos_abi::VersionSelector::None },
        offset: 0,
        length: 0,
        page: logos_abi::PageHandle(0),
        deadline: 0,
    }
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
            || !scheduler.run(handle)
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
            || !scheduler.run(handle)
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

struct StoreRelayState {
    read_namespace: Option<logos_abi::NamespaceId>,
    replace_namespace: Option<logos_abi::NamespaceId>,
}

impl StoreRelayState {
    const fn new() -> Self {
        Self { read_namespace: None, replace_namespace: None }
    }

    fn clear(&mut self) {
        self.read_namespace = None;
        self.replace_namespace = None;
    }
}

fn store_namespace(
    request: logos_abi::StoreRequest,
    state: &StoreRelayState,
) -> Option<logos_abi::NamespaceId> {
    match request.operation {
        logos_abi::StoreOperation::OpenRead | logos_abi::StoreOperation::BeginReplace => {
            Some(request.namespace)
        }
        logos_abi::StoreOperation::ReadChunk => state.read_namespace,
        logos_abi::StoreOperation::WriteChunk
        | logos_abi::StoreOperation::Commit
        | logos_abi::StoreOperation::Abort
        | logos_abi::StoreOperation::Cancel => state.replace_namespace,
    }
}

fn store_capability(operation: logos_abi::StoreOperation) -> capabilities::CapabilityKind {
    match operation {
        logos_abi::StoreOperation::OpenRead | logos_abi::StoreOperation::ReadChunk => {
            capabilities::CapabilityKind::StoreRead
        }
        logos_abi::StoreOperation::BeginReplace
        | logos_abi::StoreOperation::WriteChunk
        | logos_abi::StoreOperation::Commit
        | logos_abi::StoreOperation::Abort
        | logos_abi::StoreOperation::Cancel => capabilities::CapabilityKind::StoreWrite,
    }
}

fn update_store_state(
    state: &mut StoreRelayState,
    request: logos_abi::StoreRequest,
    status: logos_abi::PersistenceStatus,
) {
    if status != logos_abi::PersistenceStatus::Complete {
        return;
    }
    match request.operation {
        logos_abi::StoreOperation::OpenRead => state.read_namespace = Some(request.namespace),
        logos_abi::StoreOperation::BeginReplace => {
            state.replace_namespace = Some(request.namespace)
        }
        logos_abi::StoreOperation::Commit
        | logos_abi::StoreOperation::Abort
        | logos_abi::StoreOperation::Cancel => {
            state.replace_namespace = None;
            if request.operation == logos_abi::StoreOperation::Cancel {
                state.read_namespace = None;
            }
        }
        logos_abi::StoreOperation::ReadChunk | logos_abi::StoreOperation::WriteChunk => {}
    }
}

fn relay_store_request(
    terminal: native_task::StoreEndpoint,
    storage: native_task::StoreEndpoint,
    dispatch: &mut block::Dispatch,
    block_context: &mut block::DispatchContext<'_>,
    terminal_owner: u64,
    storage_owner: u64,
    history_page: logos_abi::PageHandle,
    scheduler: &mut scheduler::Scheduler<'_>,
    storage_handle: scheduler::TaskHandle,
    session: &session::Context,
    capabilities: &capabilities::CapabilityManager,
    state: &mut StoreRelayState,
    tick: u64,
) -> SessionRelay {
    let Some(request) = terminal.request() else {
        return SessionRelay::Handled(true);
    };
    let Some(namespace) = store_namespace(request, state) else {
        let _ = terminal.reply(logos_abi::StoreReply {
            id: request.id,
            status: logos_abi::PersistenceStatus::Denied,
            version: 0,
            length: 0,
        });
        return SessionRelay::Handled(true);
    };
    if !session.allows_scoped(capabilities, store_capability(request.operation), namespace.0) {
        let replied = terminal.reply(logos_abi::StoreReply {
            id: request.id,
            status: logos_abi::PersistenceStatus::Denied,
            version: 0,
            length: 0,
        });
        return SessionRelay::Handled(replied);
    }
    let needs_page = matches!(
        request.operation,
        logos_abi::StoreOperation::ReadChunk | logos_abi::StoreOperation::WriteChunk
    );
    let mut loaned = false;
    if needs_page {
        let Some(page) =
            (unsafe { logos_core::native_service::Context::shared_page_at(terminal.context()) })
        else {
            let _ = terminal.reply(logos_abi::StoreReply {
                id: request.id,
                status: logos_abi::PersistenceStatus::Denied,
                version: 0,
                length: 0,
            });
            return SessionRelay::Handled(true);
        };
        if page != history_page
            || block_context.pages.address(storage_owner, page).is_some()
            || block_context.pages.address(terminal_owner, page).is_none()
            || !block_context.pages.lend(terminal_owner, page, storage_owner)
        {
            let _ = terminal.reply(logos_abi::StoreReply {
                id: request.id,
                status: logos_abi::PersistenceStatus::Denied,
                version: 0,
                length: 0,
            });
            return SessionRelay::Handled(true);
        }
        loaned = true;
    }
    if !storage.deliver(request) {
        if loaned {
            let _ = block_context.pages.return_loan(storage_owner, request.page);
        }
        return SessionRelay::Handled(false);
    }
    if !scheduler.wake(storage_handle) || !scheduler.run(storage_handle) {
        if loaned {
            let _ = block_context.pages.return_loan(storage_owner, request.page);
        }
        return SessionRelay::Handled(false);
    }
    let mut current_tick = tick;
    loop {
        if let Some(reply) = storage.response(request.id) {
            if loaned {
                let _ = block_context.pages.return_loan(storage_owner, request.page);
            }
            update_store_state(state, request, reply.status);
            return SessionRelay::Handled(
                scheduler.wake(storage_handle)
                    && scheduler.run(storage_handle)
                    && terminal.reply(reply),
            );
        }
        if scheduler.run_next() {
            continue;
        }
        if scheduler.failed(storage_handle) {
            if loaned {
                let _ = block_context.pages.return_loan(storage_owner, request.page);
            }
            return SessionRelay::Handled(false);
        }
        if let Some(reply) = dispatch.poll(block_context, current_tick) {
            if !block_context.endpoint.reply(reply)
                || !scheduler.wake(storage_handle)
                || !scheduler.run(storage_handle)
            {
                if loaned {
                    let _ = block_context.pages.return_loan(storage_owner, request.page);
                }
                return SessionRelay::Handled(false);
            }
        } else if dispatch.accepts_new_request() {
            interrupts::wait_for_tick();
        } else {
            interrupts::wait_for_virtio();
        }
        current_tick = interrupts::ticks();
    }
}

fn relay_terminal_store_requests(
    terminal: native_task::StoreEndpoint,
    storage: native_task::StoreEndpoint,
    dispatch: &mut block::Dispatch,
    block_context: &mut block::DispatchContext<'_>,
    terminal_owner: u64,
    storage_owner: u64,
    history_page: logos_abi::PageHandle,
    scheduler: &mut scheduler::Scheduler<'_>,
    terminal_handle: scheduler::TaskHandle,
    storage_handle: scheduler::TaskHandle,
    session: &session::Context,
    capabilities: &capabilities::CapabilityManager,
    state: &mut StoreRelayState,
    tick: u64,
) -> bool {
    while terminal.request().is_some() {
        if !relay_store_request(
            terminal,
            storage,
            dispatch,
            block_context,
            terminal_owner,
            storage_owner,
            history_page,
            scheduler,
            storage_handle,
            session,
            capabilities,
            state,
            tick,
        )
        .ok()
        {
            let Some(request) = terminal.request() else { return false };
            if !terminal.reply(logos_abi::StoreReply {
                id: request.id,
                status: logos_abi::PersistenceStatus::Io,
                version: 0,
                length: 0,
            }) {
                return false;
            }
        }
        if !scheduler.wake(terminal_handle) || !scheduler.run(terminal_handle) {
            return false;
        }
    }
    true
}

fn cancel_store_transaction(
    storage: native_task::StoreEndpoint,
    scheduler: &mut scheduler::Scheduler<'_>,
    storage_handle: scheduler::TaskHandle,
) -> bool {
    let request = logos_abi::StoreRequest {
        id: u32::MAX,
        operation: logos_abi::StoreOperation::Cancel,
        namespace: logos_abi::NamespaceId(0),
        name: [0; logos_abi::MAX_OBJECT_NAME],
        name_length: 0,
        version: logos_abi::VersionSelector::None,
        offset: 0,
        length: 0,
        page: logos_abi::PageHandle(0),
        deadline: 0,
    };
    storage.deliver(request)
        && scheduler.wake(storage_handle)
        && scheduler.run(storage_handle)
        && storage
            .response(request.id)
            .is_some_and(|reply| reply.status == logos_abi::PersistenceStatus::Complete)
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
    scheduler.restart(handle)
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
    if !sessions.deliver(request)
        || !scheduler.wake(sessions_handle)
        || !scheduler.run(sessions_handle)
    {
        return SessionRelay::Handled(false);
    }
    let Some(effect) = sessions.effect() else {
        return SessionRelay::Handled(false);
    };
    let result = effects::execute(effect, context);
    if !sessions.reply_effect(result)
        || !scheduler.wake(sessions_handle)
        || !scheduler.run(sessions_handle)
    {
        return SessionRelay::Handled(false);
    }
    let Some(reply) = sessions.reply() else {
        return SessionRelay::Handled(false);
    };
    if !terminal.reply(&reply.text[..reply.length]) {
        return SessionRelay::Handled(false);
    }
    if !scheduler.wake(sessions_handle) || !scheduler.run(sessions_handle) {
        return SessionRelay::Handled(false);
    }
    let forwarded = true;
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
