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
use network::{
    DmaPages as NetworkDmaPages, PendingClient as PendingNetworkClient,
    PendingDevice as PendingNetworkDevice, Resources as NetworkResources,
};

use logos_core::capabilities;
use logos_terminal::{command, display, input, terminal, text};
use uefi::mem::memory_map::MemoryMap;

#[cfg_attr(feature = "test-hooks", allow(unreachable_code, unused_mut, unused_variables))]
#[allow(clippy::too_many_arguments)]
pub(crate) fn main(
    boot_info: boot::Info,
    memory_map: impl MemoryMap,
    acpi: Option<acpi::Tables>,
    machine: identity::Machine,
    mut secret_root: Option<root_key::RootKey>,
    remote_bootstrap: Option<logos_remote::Bootstrap>,
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
    let mut remote_state = remote_bootstrap.and_then(secrets::RemoteState::new);
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
    let Some(payload) = payloads.terminal else {
        debug::write_line(b"LogOS: terminal payload unavailable; entering recovery");
        #[cfg(feature = "test-hooks")]
        test_hooks::serve(logos_core::native_service::STORAGE_UNAVAILABLE, |action| {
            matches!(action, test_hooks::Action::Run("platform/missing-terminal"))
        });
        #[cfg(not(feature = "test-hooks"))]
        {
            let mut shell = console::Shell::offline(startup);
            let _ = shell.start();
            let _ = shell.run(|_| false);
            loop {
                unsafe { core::arch::asm!("hlt") };
            }
        }
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
    if let Some(sessions) = payloads.sessions {
        let Some(mut space) = address_space::AddressSpace::new(&mut memory) else {
            fail!(b"sessions image map");
        };
        check!(
            b"sessions image map",
            space
                .map_image(&mut memory, sessions)
                .is_some_and(|entry| entry != 0 && space.verifies_isolation())
                && space.release(&mut memory),
        );
    } else {
        debug::write_line(b"LogOS: Sessions payload unavailable");
    }
    if let Some(storage) = payloads.storage {
        let Some(mut space) = address_space::AddressSpace::new(&mut memory) else {
            fail!(b"storage image map");
        };
        check!(
            b"storage image map",
            space
                .map_image(&mut memory, storage)
                .is_some_and(|entry| entry != 0 && space.verifies_isolation())
                && space.release(&mut memory),
        );
    } else {
        debug::write_line(b"LogOS: Store payload unavailable");
    }
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
    let Some(mut native_services) = supervisor::NativeController::new(&supervisor) else {
        fail!(b"native service controller");
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
    let Some(network_bind_capability) = capabilities.grant_scoped64(
        capabilities::CapabilityKind::NetworkBind,
        logos_abi::NetworkScope::new(logos_abi::NetworkProtocol::Udp, 0, 4000).0,
    ) else {
        fail!(b"capabilities");
    };
    let Some(network_send_capability) = capabilities.grant_scoped64(
        capabilities::CapabilityKind::NetworkSend,
        logos_abi::NetworkScope::new(logos_abi::NetworkProtocol::Udp, 0x0a00_0202, 4001).0,
    ) else {
        fail!(b"capabilities");
    };
    let Some(network_icmp_capability) = capabilities.grant_scoped64(
        capabilities::CapabilityKind::NetworkSend,
        logos_abi::NetworkScope::new(logos_abi::NetworkProtocol::Icmp, 0x0a00_0202, 0).0,
    ) else {
        fail!(b"capabilities");
    };
    let Some(network_receive_capability) = capabilities.grant_scoped64(
        capabilities::CapabilityKind::NetworkReceive,
        logos_abi::NetworkScope::new(logos_abi::NetworkProtocol::Udp, 0, 4000).0,
    ) else {
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
            network_bind_capability,
            network_send_capability,
            network_icmp_capability,
            network_receive_capability,
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
    let network_service_capability = supervisor.grant(
        supervisor::NETWORK,
        &mut capabilities,
        capabilities::CapabilityKind::Service,
    );
    let gateway_service_capability = supervisor.grant(
        supervisor::GATEWAY,
        &mut capabilities,
        capabilities::CapabilityKind::Service,
    );
    let tcp_scope = logos_abi::NetworkScope::new(
        logos_abi::NetworkProtocol::Tcp,
        0,
        logos_abi::REMOTE_TCP_PORT,
    )
    .0;
    let gateway_bind_capability = supervisor.grant_scoped64(
        supervisor::GATEWAY,
        &mut capabilities,
        capabilities::CapabilityKind::NetworkBind,
        tcp_scope,
    );
    let gateway_send_capability = supervisor.grant_scoped64(
        supervisor::GATEWAY,
        &mut capabilities,
        capabilities::CapabilityKind::NetworkSend,
        tcp_scope,
    );
    let gateway_receive_capability = supervisor.grant_scoped64(
        supervisor::GATEWAY,
        &mut capabilities,
        capabilities::CapabilityKind::NetworkReceive,
        tcp_scope,
    );
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
    check!(
        b"service lifecycle",
        supervisor::lifecycle_self_check() && supervisor::native_lifecycle_self_check()
    );
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
    let gateway_service_handle = gateway_service_capability.and_then(|capability| {
        services.register(&capabilities, capability, services::Service::Gateway)
    });
    let gateway_network_session = gateway_service_handle.and_then(|handle| {
        session::Context::new(
            session::Id(2),
            handle.principal(),
            &[gateway_bind_capability?, gateway_send_capability?, gateway_receive_capability?],
        )
    });
    let remote_session = session::Context::new(
        session::Id(3),
        session::Principal::process(1),
        &[session_capability, session_service_capability, recovery_capability],
    );
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
    let native_sessions = payloads
        .sessions
        .and_then(|payload| native_task::Task::load(&mut memory, payload, &privilege));
    let mut native_storage = payloads
        .storage
        .and_then(|payload| native_task::Task::load(&mut memory, payload, &privilege));
    let mut native_network = payloads.network.and_then(|payload| {
        (network_device.is_some() && network_service_handle.is_some())
            .then(|| native_task::Task::load(&mut memory, payload, &privilege))
            .flatten()
    });
    let mut native_gateway = payloads.gateway.and_then(|payload| {
        (gateway_service_handle.is_some() && gateway_network_session.is_some())
            .then(|| native_task::Task::load(&mut memory, payload, &privilege))
            .flatten()
    });
    if let Some(storage) = native_storage.as_mut() {
        check!(b"storage heap", storage.map_heap(&mut memory).is_some());
    }
    let mut shared_pages = logos_core::shared_pages::SharedPages::new();
    let terminal_owner = session.principal().page_owner();
    let storage_owner = storage_service_handle.principal().page_owner();
    let network_owner = network_service_handle.map(|handle| handle.principal().page_owner());
    let gateway_owner = gateway_service_handle.map(|handle| handle.principal().page_owner());
    let shared_history = native_terminal.map_shared_owned(&mut memory).and_then(|page| {
        shared_pages.register(terminal_owner, page, 1).filter(|_| {
            native_storage.as_mut().is_none_or(|storage| storage.map_shared_borrowed(page))
        })
    });
    let Some(mut shared_history) = shared_history else {
        fail!(b"terminal storage page");
    };
    let gateway_page = native_gateway
        .as_mut()
        .and_then(|gateway| gateway.map_shared_owned(&mut memory))
        .and_then(|page| shared_pages.register(gateway_owner?, page, 1));
    if native_gateway.is_some() && gateway_page.is_none() {
        if let Some(task) = native_gateway.take() {
            let _ = task.release(&mut memory);
        }
    }
    #[cfg_attr(not(feature = "test-hooks"), allow(unused_mut))]
    let storage_block =
        native_storage.as_mut().and_then(|storage| storage.map_block_owned(&mut memory));
    #[cfg_attr(not(feature = "test-hooks"), allow(unused_mut))]
    let mut storage_block_virtual = storage_block.map(|(_, address)| address).unwrap_or(0);
    #[cfg_attr(not(feature = "test-hooks"), allow(unused_mut))]
    let mut storage_block_page = storage_block
        .and_then(|(physical, _)| shared_pages.register(storage_owner, physical, 1))
        .unwrap_or(logos_abi::PageHandle(0));
    check!(
        b"terminal storage page",
        shared_pages.address(terminal_owner, shared_history).is_some()
    );
    check!(b"shared pages", logos_core::shared_pages::self_check());
    let mut network_setup = native_network
        .as_mut()
        .and_then(|task| task.map_network_owned(&mut memory))
        .and_then(|((rx_physical, rx_virtual), (tx_physical, tx_virtual))| {
            let owner = network_owner?;
            let rx = shared_pages.register(owner, rx_physical, 2)?;
            let Some(tx) = shared_pages.register(owner, tx_physical, 2) else {
                let _ = shared_pages.release(owner, rx);
                return None;
            };
            Some(NetworkResources {
                owner,
                rx,
                rx_physical,
                rx_virtual,
                tx,
                tx_physical,
                tx_virtual,
            })
        });
    if network_setup.is_none() {
        if let Some(task) = native_network.take() {
            let _ = task.release(&mut memory);
        }
    }
    check!(
        b"network service pages",
        network_device.is_none() || native_network.is_none() || network_setup.is_some()
    );
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
    let mut native_input = native_terminal.input_endpoint();
    let mut native_command = native_terminal.syscall_endpoint();
    let mut native_display = native_terminal.display_endpoint();
    let mut native_sessions_endpoint =
        native_sessions.as_ref().map(native_task::Task::session_endpoint);
    #[cfg_attr(not(feature = "test-hooks"), allow(unused_mut))]
    let mut native_storage_block = native_storage
        .as_ref()
        .map(native_task::Task::block_endpoint)
        .unwrap_or_else(native_task::BlockEndpoint::unavailable);
    let mut native_store = native_terminal.store_endpoint();
    #[cfg_attr(not(feature = "test-hooks"), allow(unused_mut))]
    let mut native_storage_store = native_storage
        .as_ref()
        .map(native_task::Task::store_endpoint)
        .unwrap_or_else(native_task::StoreEndpoint::unavailable);
    let mut native_terminal_network = native_terminal.network_endpoint();
    let native_gateway_network = native_gateway.as_ref().map(native_task::Task::network_endpoint);
    let native_gateway_remote = native_gateway.as_ref().map(native_task::Task::remote_endpoint);
    let native_gateway_store = native_gateway.as_ref().map(native_task::Task::store_endpoint);
    let mut native_network_endpoint =
        native_network.as_ref().map(native_task::Task::network_endpoint);
    check!(
        b"storage shared page",
        native_store.configure_shared_page(shared_history)
            && (!native_storage_store.available()
                || native_storage_store.configure_shared_page(shared_history)),
    );
    check!(
        b"gateway shared page",
        native_gateway_store
            .zip(gateway_page)
            .is_none_or(|(endpoint, page)| endpoint.configure_shared_page(page)),
    );
    check!(
        b"storage block page",
        !native_storage_block.available()
            || native_storage_block.configure(logos_core::native_service::BlockPage {
                handle: storage_block_page,
                address: storage_block_virtual,
            }),
    );
    let mut network_dma = network_setup.map(|resources| NetworkDmaPages {
        rx_address: resources.rx_physical,
        tx_address: resources.tx_physical,
    });
    let network_pages_ready =
        native_network_endpoint.zip(network_setup).is_some_and(|(endpoint, resources)| {
            endpoint.configure(logos_core::native_service::NetworkPages {
                rx_handle: resources.rx,
                rx_address: resources.rx_virtual,
                tx_handle: resources.tx,
                tx_address: resources.tx_virtual,
            })
        });
    check!(
        b"network page configuration",
        network_device.is_none() || native_network_endpoint.is_none() || network_pages_ready
    );
    let mut block_dispatch = block::Dispatch::new();
    let mut network_client_pending: Option<PendingNetworkClient> = None;
    let mut native_scheduler = native_task::Scheduler::new();
    let Some(mut native_handle) = native_scheduler.spawn(native_terminal) else {
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
    let mut sessions_handle = native_sessions.and_then(|task| native_scheduler.spawn(task));
    if sessions_handle
        .is_some_and(|handle| !native_scheduler.run(handle) || native_scheduler.failed(handle))
    {
        debug::write_line(b"LogOS: Sessions service unavailable");
        sessions_handle = None;
        native_sessions_endpoint = None;
    }
    if sessions_handle.is_some() {
        native_services.ready(supervisor::NativeService::Sessions);
    } else {
        let _ = native_services.missing(supervisor::NativeService::Sessions);
    }
    #[cfg_attr(not(feature = "test-hooks"), allow(unused_mut))]
    let mut storage_handle = native_storage
        .and_then(|task| native_scheduler.spawn(task))
        .unwrap_or_else(native_task::Handle::unavailable);
    if storage_handle.available() {
        check!(b"native storage ready", native_scheduler.run(storage_handle));
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
                storage_handle,
            ),
        );
        native_services.ready(supervisor::NativeService::Store);
    } else {
        debug::write_line(b"LogOS: Store service unavailable");
        let _ = native_services.missing(supervisor::NativeService::Store);
    }
    if let (Some(bootstrap), Some(page_address)) =
        (remote_bootstrap, shared_pages.address(terminal_owner, shared_history))
    {
        let mut blob = [0; logos_remote::ENROLLMENT_BLOB_BYTES];
        let status = protected_store_read(
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
            &mut native_scheduler,
            storage_handle,
            shared_history,
            page_address,
            logos_abi::TRUST_NAMESPACE,
            logos_abi::TRUST_ENROLLMENT_NAME,
            &mut blob,
            interrupts::ticks(),
        );
        if status == logos_abi::PersistenceStatus::Complete {
            remote_state = Some(secrets::RemoteState::load_enrollment(bootstrap, &mut blob));
        } else if status != logos_abi::PersistenceStatus::NotFound {
            remote_state = Some(secrets::RemoteState::unavailable(bootstrap));
        }
    } else if let Some(bootstrap) = remote_bootstrap {
        remote_state = Some(secrets::RemoteState::unavailable(bootstrap));
    }
    if let (Some(state), Some(page_address)) = (
        remote_state.as_mut().filter(|state| state.available()),
        shared_pages.address(terminal_owner, shared_history),
    ) {
        let mut blob = [0; logos_remote::REMOTE_CONTROL_BLOB_BYTES];
        let status = protected_store_read(
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
            &mut native_scheduler,
            storage_handle,
            shared_history,
            page_address,
            logos_abi::TRUST_NAMESPACE,
            logos_abi::TRUST_SESSION_NAME,
            &mut blob,
            interrupts::ticks(),
        );
        if status == logos_abi::PersistenceStatus::Complete {
            let _ = state.load_control(&mut blob);
        } else if status != logos_abi::PersistenceStatus::NotFound {
            state.disable();
        }
    }
    let mut network_handle = if let Some(network_task) = native_network.take()
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
    if network_handle.is_some() {
        native_services.ready(supervisor::NativeService::Network);
    } else {
        let _ = native_services.missing(supervisor::NativeService::Network);
    }
    let mut gateway_handle = if network_handle.is_some()
        && sessions_handle.is_some()
        && storage_handle.available()
        && remote_state.as_ref().is_some_and(secrets::RemoteState::available)
    {
        native_gateway.take().and_then(|task| native_scheduler.spawn(task))
    } else {
        None
    };
    if gateway_handle
        .is_some_and(|handle| !native_scheduler.run(handle) || native_scheduler.failed(handle))
    {
        gateway_handle = None;
    }
    if gateway_handle.is_some() {
        native_services.ready(supervisor::NativeService::Gateway);
    } else {
        let _ = native_services.missing(supervisor::NativeService::Gateway);
    }
    let mut gateway_network_pending: Option<PendingNetworkClient> = None;
    macro_rules! poll_gateway {
        () => {
            poll_gateway(
                native_gateway_network,
                native_gateway_remote,
                gateway_handle,
                &mut gateway_network_pending,
                gateway_network_session.as_ref(),
                gateway_owner,
                native_network_endpoint,
                network_handle,
                &mut native_scheduler,
                &capabilities,
                gateway_page,
                shared_history,
                terminal_owner,
                &mut remote_state,
                native_sessions_endpoint,
                sessions_handle,
                remote_session.as_ref(),
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
                storage_handle,
                interrupts::ticks(),
                &mut input,
                &mut service_lifecycle,
                service_health.healthy(balloon::NAME, interrupts::ticks()),
                &channel,
                &responses,
                &mut service_scheduler,
                service_capability,
                virtio_handle,
            )
        };
    }
    let _ = poll_gateway!();
    let mut network_pending = None;
    let mut network_probe = Some(0x8000_0001u32);
    let mut network_probe_due = 0;
    let mut network_reported = false;
    health.finish();
    let mut store_relay_state = storage::RelayState::new();
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
    native_services.ready(supervisor::NativeService::Terminal);
    #[cfg(feature = "test-hooks")]
    let proof = Cell::new(false);
    #[cfg(feature = "test-hooks")]
    test_hooks::serve(
        if native_storage_store.available() {
            unsafe {
                logos_core::native_service::Context::storage_status_at(
                    native_storage_store.context(),
                )
            }
            .unwrap_or(logos_core::native_service::STORAGE_IO_FAILED)
        } else {
            logos_core::native_service::STORAGE_UNAVAILABLE
        },
        |action| match action {
            test_hooks::Action::Input(value) => {
                let tick = interrupts::ticks();
                if service_scheduler.run_next() {
                    let _ = service_health.beat(balloon::NAME, tick);
                }
                if value == "__reset" {
                    proof.set(false);
                    let Some(lifecycle) = supervisor::Lifecycle::new(&supervisor, balloon::NAME)
                    else {
                        return false;
                    };
                    service_lifecycle = lifecycle;
                    debug::write_line(b"LogOS: reset begin");
                    let previous_terminal = native_handle;
                    if !native_scheduler.fail(previous_terminal) || !startup.start() {
                        return false;
                    }
                    let Some((restarted_terminal, endpoints, history)) = replace_terminal(
                        &mut native_scheduler,
                        previous_terminal,
                        storage_handle,
                        &mut memory,
                        &mut shared_pages,
                        terminal_owner,
                        storage_owner,
                        shared_history,
                    ) else {
                        return false;
                    };
                    native_handle = restarted_terminal;
                    (
                        native_input,
                        native_command,
                        native_display,
                        native_store,
                        native_terminal_network,
                    ) = endpoints;
                    shared_history = history;
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

                    if let Some(previous_sessions) = sessions_handle {
                        if !native_scheduler.fail(previous_sessions) || !startup.start() {
                            return false;
                        }
                        let Some(restarted_sessions) = restart_native_service(
                            &mut native_scheduler,
                            previous_sessions,
                            &mut memory,
                        ) else {
                            return false;
                        };
                        sessions_handle = Some(restarted_sessions);
                        let Some(endpoint) = native_scheduler.session_endpoint(restarted_sessions)
                        else {
                            return false;
                        };
                        native_sessions_endpoint = Some(endpoint);
                        debug::write_line(b"LogOS: reset sessions ready");
                        if !native_scheduler.run(restarted_sessions)
                            || native_scheduler.wake(previous_sessions)
                        {
                            return false;
                        }
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
                    let Some(history_address) =
                        shared_pages.address(terminal_owner, shared_history)
                    else {
                        return false;
                    };
                    if !startup.start() {
                        return false;
                    }
                    let Some((restarted_storage, store, block, block_page, block_virtual)) =
                        replace_storage(
                            &mut native_scheduler,
                            previous_storage,
                            &mut memory,
                            &mut shared_pages,
                            storage_owner,
                            storage_block_page,
                            history_address,
                            shared_history,
                        )
                    else {
                        return false;
                    };
                    native_storage_store = store;
                    native_storage_block = block;
                    storage_block_page = block_page;
                    storage_block_virtual = block_virtual;
                    if !native_scheduler.run(restarted_storage) {
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
                        if !native_scheduler.wake(native_handle)
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
                        && [logos_abi::InputEvent::UP; 5]
                            .into_iter()
                            .all(|event| send(event, None))
                        && send(
                            logos_abi::InputEvent::ENTER,
                            Some(&[logos_abi::InputLayout::Azerty.wire()]),
                        );
                    proof.set(proof.get() || navigation);
                    return navigation;
                }
                let terminal_restart = matches!(
                    value,
                    "assert-terminal-service-restart"
                        | "assert-terminal-service-panic"
                        | "assert-terminal-service-fault"
                );
                let sessions_restart = matches!(
                    value,
                    "assert-sessions-service-restart"
                        | "assert-sessions-service-panic"
                        | "assert-sessions-service-fault"
                );
                let storage_restart = matches!(
                    value,
                    "assert-storage-service-restart"
                        | "assert-storage-service-panic"
                        | "assert-storage-service-fault"
                );
                if terminal_restart {
                    let previous = native_handle;
                    let previous_context = native_display.context();
                    let failed = if value == "assert-terminal-service-panic" {
                        native_input.deliver_raw(0xfa)
                            && native_scheduler.wake(previous)
                            && native_scheduler.run(previous)
                            && native_scheduler.failed(previous)
                    } else if value == "assert-terminal-service-fault" {
                        native_input.deliver_raw(0xfb)
                            && native_scheduler.wake(previous)
                            && native_scheduler.run(previous)
                            && native_scheduler.failed(previous)
                    } else {
                        native_scheduler.fail(previous)
                    };
                    if !failed || !startup.start() {
                        return false;
                    }
                    let Some((restarted, endpoints, history)) = replace_terminal(
                        &mut native_scheduler,
                        previous,
                        storage_handle,
                        &mut memory,
                        &mut shared_pages,
                        terminal_owner,
                        storage_owner,
                        shared_history,
                    ) else {
                        return false;
                    };
                    native_handle = restarted;
                    (
                        native_input,
                        native_command,
                        native_display,
                        native_store,
                        native_terminal_network,
                    ) = endpoints;
                    shared_history = history;
                    store_relay_state.clear();
                    if restarted.generation() == previous.generation()
                        || native_display.context() == previous_context
                        || native_scheduler.wake(previous)
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
                    let (Some(previous), Some(previous_endpoint)) =
                        (sessions_handle, native_sessions_endpoint)
                    else {
                        return false;
                    };
                    let previous_context = previous_endpoint.context();
                    let failed = if matches!(
                        value,
                        "assert-sessions-service-panic" | "assert-sessions-service-fault"
                    ) {
                        let bytes = if value == "assert-sessions-service-panic" {
                            b"__panic" as &[u8]
                        } else {
                            b"__fault"
                        };
                        let mut argument = [0; logos_abi::MAX_SESSION_TEXT];
                        argument[..bytes.len()].copy_from_slice(bytes);
                        previous_endpoint.deliver(logos_abi::SessionRequest::new(
                            logos_abi::Syscall::Inspect,
                            argument,
                            bytes.len(),
                        )) && native_scheduler.wake(previous)
                            && native_scheduler.run(previous)
                            && native_scheduler.failed(previous)
                    } else {
                        native_scheduler.fail(previous)
                    };
                    if !failed || !startup.start() {
                        return false;
                    }
                    let Some(restarted) =
                        restart_native_service(&mut native_scheduler, previous, &mut memory)
                    else {
                        return false;
                    };
                    sessions_handle = Some(restarted);
                    let Some(endpoint) = native_scheduler.session_endpoint(restarted) else {
                        return false;
                    };
                    native_sessions_endpoint = Some(endpoint);
                    if restarted.generation() == previous.generation()
                        || endpoint.context() == previous_context
                        || !native_scheduler.run(restarted)
                        || native_scheduler.wake(previous)
                    {
                        return false;
                    }
                }
                if storage_restart {
                    let previous = storage_handle;
                    let previous_context = native_storage_store.context();
                    let failed = if matches!(
                        value,
                        "assert-storage-service-panic" | "assert-storage-service-fault"
                    ) {
                        let id = if value == "assert-storage-service-panic" {
                            u32::MAX - 1
                        } else {
                            u32::MAX - 2
                        };
                        native_storage_store.deliver(test_store_request(
                            id,
                            logos_abi::StoreOperation::Cancel,
                            logos_abi::NamespaceId(0),
                            logos_abi::VersionSelector::None,
                        )) && native_scheduler.wake(previous)
                            && native_scheduler.run(previous)
                            && native_scheduler.failed(previous)
                    } else {
                        native_scheduler.fail(previous)
                    };
                    if !failed {
                        return false;
                    }
                    block_dispatch.cancel_on_exit(&mut block::DispatchContext {
                        endpoint: native_storage_block,
                        pages: &mut shared_pages,
                        store_owner: storage_owner,
                        store_page: storage_block_page,
                        device: &mut block_device,
                        memory: &mut memory,
                    });
                    let Some(history_address) =
                        shared_pages.address(terminal_owner, shared_history)
                    else {
                        return false;
                    };
                    if !startup.start() {
                        return false;
                    }
                    let Some((restarted, store, block, block_page, block_virtual)) =
                        replace_storage(
                            &mut native_scheduler,
                            previous,
                            &mut memory,
                            &mut shared_pages,
                            storage_owner,
                            storage_block_page,
                            history_address,
                            shared_history,
                        )
                    else {
                        return false;
                    };
                    native_storage_store = store;
                    native_storage_block = block;
                    storage_block_page = block_page;
                    storage_block_virtual = block_virtual;
                    if restarted.generation() == previous.generation()
                        || native_storage_store.context() == previous_context
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
                if matches!(value, "assert-network-service-panic" | "assert-network-service-fault")
                {
                    let (Some(previous), Some(previous_endpoint), Some(resources)) =
                        (network_handle, native_network_endpoint, network_setup)
                    else {
                        return false;
                    };
                    let id = if value == "assert-network-service-panic" {
                        u32::MAX - 1
                    } else {
                        u32::MAX - 2
                    };
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
                    if !previous_endpoint.deliver(request)
                        || !native_scheduler.wake(previous)
                        || !native_scheduler.run(previous)
                        || !native_scheduler.failed(previous)
                    {
                        return false;
                    }
                    let Some((restarted, endpoint, resources, dma)) = replace_network(
                        &mut native_scheduler,
                        previous,
                        &mut memory,
                        &mut shared_pages,
                        resources,
                    ) else {
                        return false;
                    };
                    if restarted.generation() == previous.generation()
                        || endpoint.context() == previous_endpoint.context()
                        || !native_scheduler.run(restarted)
                        || native_scheduler.wake(previous)
                    {
                        return false;
                    }
                    network_handle = Some(restarted);
                    native_network_endpoint = Some(endpoint);
                    network_setup = Some(resources);
                    network_dma = Some(dma);
                    network_pending = None;
                    network_client_pending = None;
                    network_probe = Some(0x8000_0001);
                    network_probe_due = interrupts::ticks();
                    network_reported = false;
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
                    let (Some(endpoint), Some(handle)) =
                        (native_sessions_endpoint, sessions_handle)
                    else {
                        return false;
                    };
                    let passed = endpoint.deliver(logos_abi::SessionRequest::new(
                        logos_abi::Syscall::Tasks,
                        [0; logos_abi::MAX_SESSION_TEXT],
                        0,
                    )) && native_scheduler.wake(handle)
                        && native_scheduler.run(handle)
                        && endpoint.effect().is_some_and(|effect| {
                            effect.effect == logos_abi::Effect::ReadTasks
                                && endpoint.reply_effect(logos_abi::EffectResult::TasksActive)
                        })
                        && native_scheduler.wake(handle)
                        && native_scheduler.run(handle)
                        && endpoint.reply().is_some_and(|reply| {
                            reply.length == b"scheduler active".len()
                                && reply.text[..reply.length] == *b"scheduler active"
                        });
                    proof.set(proof.get() || passed);
                    return passed;
                }
                if value == "assert-crash-restart" {
                    let tick = interrupts::ticks();
                    let passed = service_lifecycle.failed(tick)
                        && service_lifecycle.due(tick.saturating_add(2));
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
                    (
                        "layout azerty",
                        &restricted_session,
                        Some(b"permission denied" as &[u8]),
                        true,
                    )
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
                                let reply = session::relay_native(
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
                                        matches!(reply, session::Relay::Handled(true))
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
            }
            test_hooks::Action::Poll => {
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
                    interrupts::ticks(),
                    native_terminal_network,
                    &mut network_client_pending,
                    &session,
                    &capabilities,
                    &shared_pages,
                    terminal_owner,
                );
                poll_gateway!()
            }
            test_hooks::Action::Advance(ticks) => {
                for step in 0..ticks.min(4096) {
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
                        interrupts::ticks().saturating_add(step),
                        native_terminal_network,
                        &mut network_client_pending,
                        &session,
                        &capabilities,
                        &shared_pages,
                        terminal_owner,
                    ) {
                        return false;
                    }
                    if !poll_gateway!() {
                        return false;
                    }
                }
                true
            }
            test_hooks::Action::Run(id) => {
                if id == "network/unauthorized-operation" {
                    let (Some(network_endpoint), Some(network_task)) =
                        (native_network_endpoint, network_handle)
                    else {
                        return false;
                    };
                    let endpoint = logos_abi::NetworkEndpoint(0x0001_0001);
                    let requests = [
                        logos_abi::NetworkRequest {
                            id: 0x9000_0020,
                            operation: logos_abi::NetworkOperation::Bind,
                            endpoint: logos_abi::NetworkEndpoint(0),
                            peer: logos_abi::NetworkScope::new(
                                logos_abi::NetworkProtocol::Udp,
                                0,
                                4000,
                            ),
                            page: logos_abi::PageHandle(0),
                            length: 0,
                            generation: 0,
                            deadline: u64::MAX / 2,
                        },
                        logos_abi::NetworkRequest {
                            id: 0x9000_0021,
                            operation: logos_abi::NetworkOperation::SendTo,
                            endpoint,
                            peer: logos_abi::NetworkScope::new(
                                logos_abi::NetworkProtocol::Udp,
                                0x0a00_0202,
                                4001,
                            ),
                            page: logos_abi::PageHandle(1),
                            length: 1,
                            generation: 1,
                            deadline: u64::MAX / 2,
                        },
                        logos_abi::NetworkRequest {
                            id: 0x9000_0022,
                            operation: logos_abi::NetworkOperation::ReceiveFrom,
                            endpoint,
                            peer: logos_abi::NetworkScope::new(
                                logos_abi::NetworkProtocol::Udp,
                                0,
                                4000,
                            ),
                            page: logos_abi::PageHandle(1),
                            length: logos_abi::MAX_NETWORK_PAYLOAD as u16,
                            generation: 1,
                            deadline: u64::MAX / 2,
                        },
                    ];
                    for request in requests {
                        if !unsafe {
                            logos_core::native_service::Context::request_network_at(
                                native_terminal_network.context(),
                                request,
                            )
                        } {
                            return false;
                        }
                        if !relay_network_client(
                            native_terminal_network,
                            network_endpoint,
                            network_task,
                            &mut native_scheduler,
                            &mut network_client_pending,
                            &denied_session,
                            &capabilities,
                            &shared_pages,
                            terminal_owner,
                        ) {
                            return false;
                        }
                        if native_terminal_network.response(request.id).is_none_or(|reply| {
                            reply.status != logos_abi::NetworkStatus::Denied
                                || reply.counters.denied != 1
                        }) {
                            return false;
                        }
                    }
                    return true;
                }
                if id == "network/icmp-echo" {
                    if !network_reported {
                        return false;
                    }
                    let request = logos_abi::NetworkRequest {
                        id: 0x9000_0100,
                        operation: logos_abi::NetworkOperation::Echo,
                        endpoint: logos_abi::NetworkEndpoint(0),
                        peer: logos_abi::NetworkScope::new(
                            logos_abi::NetworkProtocol::Icmp,
                            0x0a00_0202,
                            0,
                        ),
                        page: logos_abi::PageHandle(0),
                        length: 0,
                        generation: 0,
                        deadline: u64::MAX / 2,
                    };
                    let reply = run_network_request(
                        request,
                        native_terminal_network,
                        native_network_endpoint,
                        network_handle,
                        network_device.as_mut(),
                        network_dma,
                        &mut native_scheduler,
                        &mut network_pending,
                        &mut network_probe,
                        &mut network_probe_due,
                        &mut network_reported,
                        &mut network_client_pending,
                        &session,
                        &capabilities,
                        &shared_pages,
                        terminal_owner,
                    );
                    return reply.is_some_and(|reply| {
                        reply.status == logos_abi::NetworkStatus::Complete
                            && reply.source_address == 0x0a00_0202
                    });
                }
                if id == "network/udp-round-trip" {
                    if !network_reported {
                        return false;
                    }
                    let bind = logos_abi::NetworkRequest {
                        id: 0x9000_0110,
                        operation: logos_abi::NetworkOperation::Bind,
                        endpoint: logos_abi::NetworkEndpoint(0),
                        peer: logos_abi::NetworkScope::new(
                            logos_abi::NetworkProtocol::Udp,
                            0,
                            4000,
                        ),
                        page: logos_abi::PageHandle(0),
                        length: 0,
                        generation: 0,
                        deadline: u64::MAX / 2,
                    };
                    let Some(bind_reply) = run_network_request(
                        bind,
                        native_terminal_network,
                        native_network_endpoint,
                        network_handle,
                        network_device.as_mut(),
                        network_dma,
                        &mut native_scheduler,
                        &mut network_pending,
                        &mut network_probe,
                        &mut network_probe_due,
                        &mut network_reported,
                        &mut network_client_pending,
                        &session,
                        &capabilities,
                        &shared_pages,
                        terminal_owner,
                    ) else {
                        return false;
                    };
                    let payload = b"logos-network-v1";
                    let Some(page_address) = shared_pages.address(terminal_owner, shared_history)
                    else {
                        return false;
                    };
                    unsafe {
                        core::ptr::copy_nonoverlapping(
                            payload.as_ptr(),
                            page_address as *mut u8,
                            payload.len(),
                        );
                    }
                    let send = logos_abi::NetworkRequest {
                        id: 0x9000_0111,
                        operation: logos_abi::NetworkOperation::SendTo,
                        endpoint: bind_reply.endpoint,
                        peer: logos_abi::NetworkScope::new(
                            logos_abi::NetworkProtocol::Udp,
                            0x0a00_0202,
                            4001,
                        ),
                        page: shared_history,
                        length: payload.len() as u16,
                        generation: bind_reply.generation,
                        deadline: u64::MAX / 2,
                    };
                    let Some(send_reply) = run_network_request(
                        send,
                        native_terminal_network,
                        native_network_endpoint,
                        network_handle,
                        network_device.as_mut(),
                        network_dma,
                        &mut native_scheduler,
                        &mut network_pending,
                        &mut network_probe,
                        &mut network_probe_due,
                        &mut network_reported,
                        &mut network_client_pending,
                        &session,
                        &capabilities,
                        &shared_pages,
                        terminal_owner,
                    ) else {
                        return false;
                    };
                    let receive = logos_abi::NetworkRequest {
                        id: 0x9000_0112,
                        operation: logos_abi::NetworkOperation::ReceiveFrom,
                        endpoint: bind_reply.endpoint,
                        peer: logos_abi::NetworkScope::new(
                            logos_abi::NetworkProtocol::Udp,
                            0,
                            4000,
                        ),
                        page: shared_history,
                        length: logos_abi::MAX_NETWORK_PAYLOAD as u16,
                        generation: bind_reply.generation,
                        deadline: u64::MAX / 2,
                    };
                    let Some(receive_reply) = run_network_request(
                        receive,
                        native_terminal_network,
                        native_network_endpoint,
                        network_handle,
                        network_device.as_mut(),
                        network_dma,
                        &mut native_scheduler,
                        &mut network_pending,
                        &mut network_probe,
                        &mut network_probe_due,
                        &mut network_reported,
                        &mut network_client_pending,
                        &session,
                        &capabilities,
                        &shared_pages,
                        terminal_owner,
                    ) else {
                        return false;
                    };
                    let received = unsafe {
                        core::slice::from_raw_parts(
                            page_address as *const u8,
                            receive_reply.length as usize,
                        )
                    };
                    return bind_reply.status == logos_abi::NetworkStatus::Complete
                        && send_reply.status == logos_abi::NetworkStatus::Complete
                        && receive_reply.status == logos_abi::NetworkStatus::Complete
                        && receive_reply.source_address == 0x0a00_0202
                        && receive_reply.source_port == 4001
                        && send_reply.counters.tx_frames > 0
                        && receive_reply.counters.rx_frames > 0
                        && received == payload;
                }
                if id == "network/backpressure-cancel" {
                    if !network_reported {
                        return false;
                    }
                    let bind = logos_abi::NetworkRequest {
                        id: 0x9000_0120,
                        operation: logos_abi::NetworkOperation::Bind,
                        endpoint: logos_abi::NetworkEndpoint(0),
                        peer: logos_abi::NetworkScope::new(
                            logos_abi::NetworkProtocol::Udp,
                            0,
                            4000,
                        ),
                        page: logos_abi::PageHandle(0),
                        length: 0,
                        generation: 0,
                        deadline: u64::MAX / 2,
                    };
                    let Some(bind_reply) = run_network_request(
                        bind,
                        native_terminal_network,
                        native_network_endpoint,
                        network_handle,
                        network_device.as_mut(),
                        network_dma,
                        &mut native_scheduler,
                        &mut network_pending,
                        &mut network_probe,
                        &mut network_probe_due,
                        &mut network_reported,
                        &mut network_client_pending,
                        &session,
                        &capabilities,
                        &shared_pages,
                        terminal_owner,
                    ) else {
                        return false;
                    };
                    let cancel = logos_abi::NetworkRequest {
                        id: 0x9000_0121,
                        operation: logos_abi::NetworkOperation::Cancel,
                        endpoint: bind_reply.endpoint,
                        peer: logos_abi::NetworkScope(0),
                        page: logos_abi::PageHandle(0),
                        length: 0,
                        generation: 0,
                        deadline: u64::MAX / 2,
                    };
                    let Some(cancel_reply) = run_network_request(
                        cancel,
                        native_terminal_network,
                        native_network_endpoint,
                        network_handle,
                        network_device.as_mut(),
                        network_dma,
                        &mut native_scheduler,
                        &mut network_pending,
                        &mut network_probe,
                        &mut network_probe_due,
                        &mut network_reported,
                        &mut network_client_pending,
                        &session,
                        &capabilities,
                        &shared_pages,
                        terminal_owner,
                    ) else {
                        return false;
                    };
                    let close = logos_abi::NetworkRequest {
                        id: 0x9000_0122,
                        operation: logos_abi::NetworkOperation::Close,
                        endpoint: bind_reply.endpoint,
                        peer: logos_abi::NetworkScope(0),
                        page: logos_abi::PageHandle(0),
                        length: 0,
                        generation: 0,
                        deadline: u64::MAX / 2,
                    };
                    let Some(close_reply) = run_network_request(
                        close,
                        native_terminal_network,
                        native_network_endpoint,
                        network_handle,
                        network_device.as_mut(),
                        network_dma,
                        &mut native_scheduler,
                        &mut network_pending,
                        &mut network_probe,
                        &mut network_probe_due,
                        &mut network_reported,
                        &mut network_client_pending,
                        &session,
                        &capabilities,
                        &shared_pages,
                        terminal_owner,
                    ) else {
                        return false;
                    };
                    return bind_reply.status == logos_abi::NetworkStatus::Complete
                        && cancel_reply.status == logos_abi::NetworkStatus::Cancelled
                        && cancel_reply.counters.cancellations > 0
                        && close_reply.status == logos_abi::NetworkStatus::Complete;
                }
                if id == "network/packet-loss" {
                    if !network_reported {
                        return false;
                    }
                    let first = logos_abi::NetworkRequest {
                        id: 0x9000_0130,
                        operation: logos_abi::NetworkOperation::Echo,
                        endpoint: logos_abi::NetworkEndpoint(0),
                        peer: logos_abi::NetworkScope::new(
                            logos_abi::NetworkProtocol::Icmp,
                            0x0a00_0202,
                            0,
                        ),
                        page: logos_abi::PageHandle(0),
                        length: 0,
                        generation: 0,
                        deadline: u64::MAX / 2,
                    };
                    let second = logos_abi::NetworkRequest {
                        id: 0x9000_0131,
                        deadline: u64::MAX / 2,
                        ..first
                    };
                    let Some(first_reply) = run_network_request(
                        first,
                        native_terminal_network,
                        native_network_endpoint,
                        network_handle,
                        network_device.as_mut(),
                        network_dma,
                        &mut native_scheduler,
                        &mut network_pending,
                        &mut network_probe,
                        &mut network_probe_due,
                        &mut network_reported,
                        &mut network_client_pending,
                        &session,
                        &capabilities,
                        &shared_pages,
                        terminal_owner,
                    ) else {
                        return false;
                    };
                    let Some(second_reply) = run_network_request(
                        second,
                        native_terminal_network,
                        native_network_endpoint,
                        network_handle,
                        network_device.as_mut(),
                        network_dma,
                        &mut native_scheduler,
                        &mut network_pending,
                        &mut network_probe,
                        &mut network_probe_due,
                        &mut network_reported,
                        &mut network_client_pending,
                        &session,
                        &capabilities,
                        &shared_pages,
                        terminal_owner,
                    ) else {
                        return false;
                    };
                    return first_reply.status == logos_abi::NetworkStatus::Complete
                        && second_reply.status == logos_abi::NetworkStatus::Complete
                        && first_reply.counters.malformed > 0
                        && second_reply.counters.rx_frames >= first_reply.counters.rx_frames;
                }
                if id == "network/timeout" {
                    if !network_reported {
                        return false;
                    }
                    let bind = logos_abi::NetworkRequest {
                        id: 0x9000_0140,
                        operation: logos_abi::NetworkOperation::Bind,
                        endpoint: logos_abi::NetworkEndpoint(0),
                        peer: logos_abi::NetworkScope::new(
                            logos_abi::NetworkProtocol::Udp,
                            0,
                            4000,
                        ),
                        page: logos_abi::PageHandle(0),
                        length: 0,
                        generation: 0,
                        deadline: u64::MAX / 2,
                    };
                    let Some(bind_reply) = run_network_request(
                        bind,
                        native_terminal_network,
                        native_network_endpoint,
                        network_handle,
                        network_device.as_mut(),
                        network_dma,
                        &mut native_scheduler,
                        &mut network_pending,
                        &mut network_probe,
                        &mut network_probe_due,
                        &mut network_reported,
                        &mut network_client_pending,
                        &session,
                        &capabilities,
                        &shared_pages,
                        terminal_owner,
                    ) else {
                        return false;
                    };
                    let receive = logos_abi::NetworkRequest {
                        id: 0x9000_0141,
                        operation: logos_abi::NetworkOperation::ReceiveFrom,
                        endpoint: bind_reply.endpoint,
                        peer: logos_abi::NetworkScope::new(
                            logos_abi::NetworkProtocol::Udp,
                            0,
                            4000,
                        ),
                        page: shared_history,
                        length: logos_abi::MAX_NETWORK_PAYLOAD as u16,
                        generation: bind_reply.generation,
                        deadline: interrupts::ticks().saturating_add(64),
                    };
                    let Some(reply) = run_network_request(
                        receive,
                        native_terminal_network,
                        native_network_endpoint,
                        network_handle,
                        network_device.as_mut(),
                        network_dma,
                        &mut native_scheduler,
                        &mut network_pending,
                        &mut network_probe,
                        &mut network_probe_due,
                        &mut network_reported,
                        &mut network_client_pending,
                        &session,
                        &capabilities,
                        &shared_pages,
                        terminal_owner,
                    ) else {
                        return false;
                    };
                    return bind_reply.status == logos_abi::NetworkStatus::Complete
                        && reply.status == logos_abi::NetworkStatus::TimedOut
                        && reply.counters.timeouts == 1;
                }
                if id == "network/reset-reconnect" {
                    if !network_reported {
                        return false;
                    }
                    let request = logos_abi::NetworkRequest {
                        id: 0x9000_0152,
                        operation: logos_abi::NetworkOperation::Echo,
                        endpoint: logos_abi::NetworkEndpoint(0),
                        peer: logos_abi::NetworkScope::new(
                            logos_abi::NetworkProtocol::Icmp,
                            0x0a00_0202,
                            0,
                        ),
                        page: logos_abi::PageHandle(0),
                        length: 0,
                        generation: 0,
                        deadline: u64::MAX / 2,
                    };
                    let Some(reply) = run_network_request(
                        request,
                        native_terminal_network,
                        native_network_endpoint,
                        network_handle,
                        network_device.as_mut(),
                        network_dma,
                        &mut native_scheduler,
                        &mut network_pending,
                        &mut network_probe,
                        &mut network_probe_due,
                        &mut network_reported,
                        &mut network_client_pending,
                        &session,
                        &capabilities,
                        &shared_pages,
                        terminal_owner,
                    ) else {
                        return false;
                    };
                    return reply.status == logos_abi::NetworkStatus::Complete;
                }
                if id == "network/device-bind" {
                    let request = logos_abi::NetworkRequest {
                        id: 0x9000_0001,
                        operation: logos_abi::NetworkOperation::Bind,
                        endpoint: logos_abi::NetworkEndpoint(0),
                        peer: logos_abi::NetworkScope::new(
                            logos_abi::NetworkProtocol::Udp,
                            0,
                            4000,
                        ),
                        page: logos_abi::PageHandle(0),
                        length: 0,
                        generation: 0,
                        deadline: u64::MAX / 2,
                    };
                    if !network_reported || !native_terminal_network.deliver(request) {
                        return false;
                    }
                    for step in 0..256 {
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
                            interrupts::ticks().saturating_add(step),
                            native_terminal_network,
                            &mut network_client_pending,
                            &session,
                            &capabilities,
                            &shared_pages,
                            terminal_owner,
                        );
                        if let Some(reply) = native_terminal_network.response(request.id) {
                            return reply.status == logos_abi::NetworkStatus::Complete
                                && reply.endpoint.valid();
                        }
                    }
                    return false;
                }
                id == "core/boot-normal"
                    || (matches!(
                        id,
                        "platform/missing-sessions" | "platform/incompatible-sessions"
                    ) && native_services.state(supervisor::NativeService::Sessions)
                        == supervisor::NativeState::Missing
                        && sessions_handle.is_none())
                    || (id == "platform/missing-store"
                        && native_services.state(supervisor::NativeService::Store)
                            == supervisor::NativeState::Missing
                        && !storage_handle.available())
                    || (id == "platform/missing-network"
                        && native_services.state(supervisor::NativeService::Network)
                            == supervisor::NativeState::Missing
                        && network_handle.is_none())
                    || (cfg!(feature = "block-probe") && id == "persistence/block-read-flush")
                    || (id == "network/transport-dhcp" && network_reported)
                    || (id == "network/configuration" && network_reported)
                    || proof.get()
            }
        },
    );
    'console: loop {
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
                if native_services.due(supervisor::NativeService::Sessions, tick)
                    && let Some(failed_sessions) = sessions_handle
                {
                    if let Some(restarted) =
                        restart_native_service(&mut native_scheduler, failed_sessions, &mut memory)
                    {
                        sessions_handle = Some(restarted);
                        native_sessions_endpoint = native_scheduler.session_endpoint(restarted);
                        if native_scheduler.run(restarted) && native_sessions_endpoint.is_some() {
                            native_services.ready(supervisor::NativeService::Sessions);
                            debug::write_line(b"LogOS: native Sessions restarted");
                        } else {
                            let _ =
                                native_services.failed(supervisor::NativeService::Sessions, tick);
                        }
                    } else {
                        let _ = native_services.failed(supervisor::NativeService::Sessions, tick);
                    }
                }
                if storage_handle.available() && native_scheduler.failed(storage_handle) {
                    let _ = native_services.failed(supervisor::NativeService::Store, tick);
                }
                if native_services.due(supervisor::NativeService::Store, tick)
                    && storage_handle.available()
                {
                    block_dispatch.cancel_on_exit(&mut block::DispatchContext {
                        endpoint: native_storage_block,
                        pages: &mut shared_pages,
                        store_owner: storage_owner,
                        store_page: storage_block_page,
                        device: &mut block_device,
                        memory: &mut memory,
                    });
                    if let Some(history_address) =
                        shared_pages.address(terminal_owner, shared_history)
                        && let Some((restarted, store, block, block_page, block_virtual)) =
                            replace_storage(
                                &mut native_scheduler,
                                storage_handle,
                                &mut memory,
                                &mut shared_pages,
                                storage_owner,
                                storage_block_page,
                                history_address,
                                shared_history,
                            )
                    {
                        storage_handle = restarted;
                        native_storage_store = store;
                        native_storage_block = block;
                        storage_block_page = block_page;
                        let _ = block_virtual;
                        if native_scheduler.run(restarted)
                            && run_storage_startup(
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
                            )
                        {
                            store_relay_state.clear();
                            native_services.ready(supervisor::NativeService::Store);
                            debug::write_line(b"LogOS: Store service restarted");
                        } else {
                            let _ = native_services.failed(supervisor::NativeService::Store, tick);
                        }
                    } else {
                        let _ = native_services.failed(supervisor::NativeService::Store, tick);
                    }
                }
                if native_services.due(supervisor::NativeService::Network, tick)
                    && let (Some(failed_network), Some(resources)) = (network_handle, network_setup)
                {
                    if let Some(pending) = network_client_pending.take() {
                        let _ = native_terminal_network.reply(logos_abi::NetworkReply {
                            id: pending.request.id,
                            status: logos_abi::NetworkStatus::Reset,
                            endpoint: logos_abi::NetworkEndpoint(0),
                            generation: pending.request.generation,
                            source_address: 0,
                            source_port: 0,
                            length: 0,
                            info: logos_abi::NetworkInfo::default(),
                            counters: logos_abi::NetworkCounters::default(),
                        });
                    }
                    network_pending = None;
                    if let Some((restarted, endpoint, resources, dma)) = replace_network(
                        &mut native_scheduler,
                        failed_network,
                        &mut memory,
                        &mut shared_pages,
                        resources,
                    ) {
                        network_handle = Some(restarted);
                        native_network_endpoint = Some(endpoint);
                        network_setup = Some(resources);
                        network_dma = Some(dma);
                        network_probe = Some(0x8000_0001);
                        network_probe_due = tick;
                        network_reported = false;
                        if let Some(device) = network_device.as_mut() {
                            let _ = device.reset();
                        }
                        if native_scheduler.run(restarted) {
                            native_services.ready(supervisor::NativeService::Network);
                            debug::write_line(b"LogOS: Network service restarted");
                        } else {
                            let _ =
                                native_services.failed(supervisor::NativeService::Network, tick);
                        }
                    } else {
                        let _ = native_services.failed(supervisor::NativeService::Network, tick);
                    }
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
                    native_terminal_network,
                    &mut network_client_pending,
                    &session,
                    &capabilities,
                    &shared_pages,
                    terminal_owner,
                ) {
                    debug::write_line(b"LogOS: network service unavailable");
                    if network_handle.is_some() {
                        let _ = native_services.failed(supervisor::NativeService::Network, tick);
                    }
                }
                if !poll_gateway!() {
                    debug::write_line(b"LogOS: Gateway service unavailable");
                    if gateway_handle.is_some() {
                        let _ = native_services.failed(supervisor::NativeService::Gateway, tick);
                    }
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
                            if native_services.failed(supervisor::NativeService::Terminal, tick)
                                == supervisor::FailureAction::Retry
                                && native_services.due(supervisor::NativeService::Terminal, tick)
                            {
                                if !cancel_store_transaction(
                                    native_storage_store,
                                    &mut native_scheduler,
                                    storage_handle,
                                ) {
                                    console_mode = mode::ConsoleMode::Recovery;
                                    break;
                                }
                                if let Some((restarted, endpoints, history)) = replace_terminal(
                                    &mut native_scheduler,
                                    native_handle,
                                    storage_handle,
                                    &mut memory,
                                    &mut shared_pages,
                                    terminal_owner,
                                    storage_owner,
                                    shared_history,
                                ) {
                                    native_handle = restarted;
                                    (
                                        native_input,
                                        native_command,
                                        native_display,
                                        native_store,
                                        native_terminal_network,
                                    ) = endpoints;
                                    shared_history = history;
                                    store_relay_state.clear();
                                    if native_scheduler.run(native_handle)
                                        && resume_display(
                                            native_display,
                                            &session,
                                            &capabilities,
                                            session_display_capability,
                                            &mut native_scheduler,
                                            native_handle,
                                        )
                                    {
                                        native_services.ready(supervisor::NativeService::Terminal);
                                        debug::write_line(b"LogOS: native terminal restarted");
                                        continue;
                                    }
                                }
                            }
                            debug::write_line(b"LogOS: native terminal display failed");
                            console_mode = mode::ConsoleMode::Recovery;
                            break;
                        }
                        if event.pressed().is_some_and(|(key, _)| key == input::LogicalKey::Escape)
                        {
                            debug::write_line(b"LogOS: recovery handoff requested");
                            console_mode = mode::ConsoleMode::Recovery;
                            break;
                        }
                        if native_store.request().is_some()
                            && (!relay_terminal_store_requests(
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
                            ))
                        {
                            debug::write_line(b"LogOS: Store relay failed");
                            console_mode = mode::ConsoleMode::Recovery;
                            break;
                        }
                        if native_command.request().is_some_and(|request| {
                            matches!(
                                request.syscall,
                                logos_abi::Syscall::RemoteKey
                                    | logos_abi::Syscall::Enroll
                                    | logos_abi::Syscall::Unenroll
                            )
                        }) {
                            let request = native_command.request().unwrap();
                            let mut key_text = [0; 96];
                            let mut reply = b"remote unavailable" as &[u8];
                            if let Some(state) = remote_state.as_mut() {
                                match request.syscall {
                                    logos_abi::Syscall::RemoteKey => {
                                        if state.available() {
                                            let mut machine_key = [0; 64];
                                            hex_key(&state.machine_public(), &mut machine_key);
                                            key_text[..64].copy_from_slice(&machine_key);
                                            reply = &key_text[..64];
                                        }
                                    }
                                    logos_abi::Syscall::Enroll => {
                                        let mut client_key = [0; 32];
                                        if request.length == 64
                                            && hex_decode_key(
                                                &request.argument[..request.length],
                                                &mut client_key,
                                            )
                                            && state.enroll(client_key).is_some_and(|_| {
                                                persist_remote_enrollment(
                                                    state,
                                                    remote_bootstrap,
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
                                                    &mut native_scheduler,
                                                    storage_handle,
                                                    shared_history,
                                                    terminal_owner,
                                                    tick,
                                                )
                                            })
                                        {
                                            let mut machine_key = [0; 64];
                                            hex_key(&state.machine_public(), &mut machine_key);
                                            key_text[..64].copy_from_slice(&machine_key);
                                            key_text[64] = b':';
                                            let generation = state.enrollment().generation;
                                            let mut digits = [0; 20];
                                            let length = decimal_u64(generation, &mut digits);
                                            key_text[65..65 + length]
                                                .copy_from_slice(&digits[..length]);
                                            reply = &key_text[..65 + length];
                                        } else {
                                            reply = b"invalid enrollment key";
                                        }
                                    }
                                    logos_abi::Syscall::Unenroll => {
                                        if state.unenroll().is_some_and(|_| {
                                            persist_remote_enrollment(
                                                state,
                                                remote_bootstrap,
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
                                                &mut native_scheduler,
                                                storage_handle,
                                                shared_history,
                                                terminal_owner,
                                                tick,
                                            )
                                        }) {
                                            reply = b"remote unenrolled";
                                        }
                                    }
                                    _ => {}
                                }
                            };
                            if !native_command.reply(reply)
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
                                debug::write_line(b"LogOS: remote key reply failed");
                                console_mode = mode::ConsoleMode::Recovery;
                                break;
                            }
                        } else if native_command.request().is_some() {
                            let mut relay = session::relay_native(
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

                            if matches!(relay, session::Relay::Handled(false)) {
                                if sessions_handle.is_some() {
                                    let _ = native_services
                                        .failed(supervisor::NativeService::Sessions, tick);
                                }
                                relay = session::Relay::Handled(
                                    native_command.reply(b"session unavailable; retry command"),
                                );
                            }
                            match relay {
                                session::Relay::Recovery => {
                                    debug::write_line(b"LogOS: recovery handoff requested");
                                    console_mode = mode::ConsoleMode::Recovery;
                                    break;
                                }
                                session::Relay::Handled(false) => {
                                    debug::write_line(b"LogOS: Sessions relay failed");
                                }
                                session::Relay::Handled(true) => {
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
            startup = console.run(|action| {
                let tick = interrupts::ticks();
                if matches!(action, console::Action::RestartSessions) {
                    native_services.manual_restart(supervisor::NativeService::Sessions);
                    if let Some(failed) = sessions_handle
                        && let Some(restarted) =
                            restart_native_service(&mut native_scheduler, failed, &mut memory)
                    {
                        sessions_handle = Some(restarted);
                        native_sessions_endpoint = native_scheduler.session_endpoint(restarted);
                        if native_scheduler.run(restarted) && native_sessions_endpoint.is_some() {
                            native_services.ready(supervisor::NativeService::Sessions);
                            debug::write_line(b"LogOS: Sessions manually restarted");
                        } else {
                            let _ =
                                native_services.failed(supervisor::NativeService::Sessions, tick);
                        }
                    }
                }
                if matches!(action, console::Action::RestartTerminal) {
                    native_services.manual_restart(supervisor::NativeService::Terminal);
                    if cancel_store_transaction(
                        native_storage_store,
                        &mut native_scheduler,
                        storage_handle,
                    ) && let Some((restarted, endpoints, history)) = replace_terminal(
                        &mut native_scheduler,
                        native_handle,
                        storage_handle,
                        &mut memory,
                        &mut shared_pages,
                        terminal_owner,
                        storage_owner,
                        shared_history,
                    ) {
                        native_handle = restarted;
                        (
                            native_input,
                            native_command,
                            native_display,
                            native_store,
                            native_terminal_network,
                        ) = endpoints;
                        shared_history = history;
                        store_relay_state.clear();
                        if native_scheduler.run(native_handle)
                            && resume_display(
                                native_display,
                                &session,
                                &capabilities,
                                session_display_capability,
                                &mut native_scheduler,
                                native_handle,
                            )
                        {
                            native_services.ready(supervisor::NativeService::Terminal);
                            debug::write_line(b"LogOS: Terminal manually restarted");
                            return true;
                        }
                    }
                    let _ = native_services.failed(supervisor::NativeService::Terminal, tick);
                }
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
                    native_terminal_network,
                    &mut network_client_pending,
                    &session,
                    &capabilities,
                    &shared_pages,
                    terminal_owner,
                );
                let _ = poll_gateway!();
                false
            });
            console_mode = mode::ConsoleMode::Normal;
            continue 'console;
        }
        break;
    }
    loop {
        unsafe { core::arch::asm!("cli", "hlt") };
    }
}

fn dispatch_store_block(
    dispatch: &mut block::Dispatch,
    context: &mut block::DispatchContext<'_>,
    scheduler: &mut native_task::Scheduler<'_>,
    handle: native_task::Handle,
    tick: u64,
) -> bool {
    if !context.endpoint.available() || !handle.available() {
        return true;
    }
    let Some(reply) = dispatch.poll(context, tick) else {
        return true;
    };
    context.endpoint.reply(reply) && scheduler.wake(handle) && scheduler.run(handle)
}

const NETWORK_PAYLOAD_OFFSET: u64 = 2048;

#[allow(clippy::too_many_arguments)]
fn relay_network_client(
    terminal: native_task::NetworkEndpoint,
    service: native_task::NetworkEndpoint,
    handle: native_task::Handle,
    scheduler: &mut native_task::Scheduler<'_>,
    pending: &mut Option<PendingNetworkClient>,
    session: &session::Context,
    capabilities: &capabilities::CapabilityManager,
    shared_pages: &logos_core::shared_pages::SharedPages,
    terminal_owner: u64,
) -> bool {
    if let Some(current) = *pending {
        if let Some(reply) = service.response(current.request.id) {
            if !reply.valid_for(current.request) {
                return false;
            }
            *pending = None;
            if matches!(
                current.request.operation,
                logos_abi::NetworkOperation::ReceiveFrom | logos_abi::NetworkOperation::Read
            ) && reply.status == logos_abi::NetworkStatus::Complete
                && let (Some(source), Some(network_pages)) =
                    (shared_pages.address(terminal_owner, current.request.page), unsafe {
                        logos_core::native_service::Context::network_pages_at(service.context())
                    })
            {
                let source = source as *const u8;
                let target = (network_pages.tx_address + NETWORK_PAYLOAD_OFFSET) as *const u8;
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        target,
                        source as *mut u8,
                        reply.length as usize,
                    );
                }
            }
            return terminal.reply(reply) && scheduler.wake(handle) && scheduler.run(handle);
        }
        return true;
    }
    let Some(request) = terminal.request() else {
        return true;
    };
    if !request.valid_shape() {
        return terminal.reply(logos_abi::NetworkReply {
            id: request.id,
            status: crate::platform::network::status_for(request, false),
            endpoint: logos_abi::NetworkEndpoint(0),
            generation: 0,
            source_address: 0,
            source_port: 0,
            length: 0,
            info: logos_abi::NetworkInfo::default(),
            counters: logos_abi::NetworkCounters::default(),
        });
    }
    if let Some((kind, scope)) = crate::platform::network::capability(request)
        && !session.allows_scoped64(capabilities, kind, scope)
    {
        return terminal.reply(logos_abi::NetworkReply {
            id: request.id,
            status: crate::platform::network::status_for(request, false),
            endpoint: logos_abi::NetworkEndpoint(0),
            generation: request.generation,
            source_address: 0,
            source_port: 0,
            length: 0,
            info: logos_abi::NetworkInfo::default(),
            counters: logos_abi::NetworkCounters {
                denied: 1,
                ..logos_abi::NetworkCounters::default()
            },
        });
    }
    if matches!(
        request.operation,
        logos_abi::NetworkOperation::SendTo
            | logos_abi::NetworkOperation::Write
            | logos_abi::NetworkOperation::ReceiveFrom
            | logos_abi::NetworkOperation::Read
    ) {
        let Some(client) = shared_pages.address(terminal_owner, request.page) else {
            return terminal.reply(logos_abi::NetworkReply {
                id: request.id,
                status: logos_abi::NetworkStatus::Invalid,
                endpoint: logos_abi::NetworkEndpoint(0),
                generation: request.generation,
                source_address: 0,
                source_port: 0,
                length: 0,
                info: logos_abi::NetworkInfo::default(),
                counters: logos_abi::NetworkCounters::default(),
            }) && scheduler.wake(handle)
                && scheduler.run(handle);
        };
        if matches!(
            request.operation,
            logos_abi::NetworkOperation::SendTo | logos_abi::NetworkOperation::Write
        ) {
            let Some(network_pages) = (unsafe {
                logos_core::native_service::Context::network_pages_at(service.context())
            }) else {
                return true;
            };
            unsafe {
                core::ptr::copy_nonoverlapping(
                    client as *const u8,
                    (network_pages.tx_address + NETWORK_PAYLOAD_OFFSET) as *mut u8,
                    request.length as usize,
                );
            }
        }
    }
    if !service.deliver_for_owner(request, terminal_owner) {
        return true;
    }
    *pending = Some(PendingNetworkClient { request });
    scheduler.wake(handle) && scheduler.run(handle)
}

#[allow(clippy::too_many_arguments)]
fn poll_network(
    device: Option<&mut network_driver::Device>,
    endpoint: Option<native_task::NetworkEndpoint>,
    handle: Option<native_task::Handle>,
    dma: Option<NetworkDmaPages>,
    scheduler: &mut native_task::Scheduler<'_>,
    pending: &mut Option<PendingNetworkDevice>,
    probe: &mut Option<u32>,
    probe_due: &mut u64,
    reported: &mut bool,
    tick: u64,
    terminal: native_task::NetworkEndpoint,
    client_pending: &mut Option<PendingNetworkClient>,
    session: &session::Context,
    capabilities: &capabilities::CapabilityManager,
    shared_pages: &logos_core::shared_pages::SharedPages,
    terminal_owner: u64,
) -> bool {
    let (Some(device), Some(endpoint), Some(handle)) = (device, endpoint, handle) else {
        return true;
    };
    if !relay_network_client(
        terminal,
        endpoint,
        handle,
        scheduler,
        client_pending,
        session,
        capabilities,
        shared_pages,
        terminal_owner,
    ) {
        return false;
    }
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
    if !*reported && tick >= *probe_due {
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
        core::slice::from_raw_parts_mut(dma.rx_address as *mut u8, logos_abi::NETWORK_MAX_FRAME)
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

#[cfg(feature = "test-hooks")]
#[allow(clippy::too_many_arguments)]
fn run_network_request(
    request: logos_abi::NetworkRequest,
    terminal: native_task::NetworkEndpoint,
    service: Option<native_task::NetworkEndpoint>,
    handle: Option<native_task::Handle>,
    mut device: Option<&mut network_driver::Device>,
    dma: Option<NetworkDmaPages>,
    scheduler: &mut native_task::Scheduler<'_>,
    device_pending: &mut Option<PendingNetworkDevice>,
    probe: &mut Option<u32>,
    probe_due: &mut u64,
    reported: &mut bool,
    client_pending: &mut Option<PendingNetworkClient>,
    session: &session::Context,
    capabilities: &capabilities::CapabilityManager,
    shared_pages: &logos_core::shared_pages::SharedPages,
    terminal_owner: u64,
) -> Option<logos_abi::NetworkReply> {
    if !unsafe {
        logos_core::native_service::Context::request_network_at(terminal.context(), request)
    } {
        return None;
    }
    let endpoint = service?;
    let service_handle = handle?;
    for _ in 0..100_000 {
        if !poll_network(
            device.as_deref_mut(),
            Some(endpoint),
            Some(service_handle),
            dma,
            scheduler,
            device_pending,
            probe,
            probe_due,
            reported,
            interrupts::ticks(),
            terminal,
            client_pending,
            session,
            capabilities,
            shared_pages,
            terminal_owner,
        ) {
            return None;
        }
        if let Some(reply) = terminal.response(request.id) {
            return Some(reply);
        }
    }
    None
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

#[allow(clippy::too_many_arguments)]
fn poll_gateway(
    client: Option<native_task::NetworkEndpoint>,
    remote: Option<native_task::RemoteEndpoint>,
    gateway_handle: Option<native_task::Handle>,
    gateway_pending: &mut Option<PendingNetworkClient>,
    gateway_session: Option<&session::Context>,
    gateway_owner: Option<u64>,
    network_service: Option<native_task::NetworkEndpoint>,
    network_handle: Option<native_task::Handle>,
    scheduler: &mut native_task::Scheduler<'_>,
    capabilities: &capabilities::CapabilityManager,
    gateway_page: Option<logos_abi::PageHandle>,
    persistence_page: logos_abi::PageHandle,
    persistence_owner: u64,
    remote_state: &mut Option<secrets::RemoteState>,
    sessions: Option<native_task::SessionEndpoint>,
    sessions_handle: Option<native_task::Handle>,
    remote_session: Option<&session::Context>,
    storage: native_task::StoreEndpoint,
    block_dispatch: &mut block::Dispatch,
    block_context: &mut block::DispatchContext<'_>,
    storage_handle: native_task::Handle,
    tick: u64,
    input: &mut input::Service,
    lifecycle: &mut supervisor::Lifecycle,
    service_healthy: bool,
    channel: &ipc::Channel,
    responses: &ipc::Channel,
    service_scheduler: &mut scheduler::Scheduler<'_>,
    service_capability: capabilities::Capability,
    service: services::ServiceHandle,
) -> bool {
    let (
        Some(client),
        Some(handle),
        Some(network_service),
        Some(network_handle),
        Some(gateway_session),
        Some(owner),
    ) = (client, gateway_handle, network_service, network_handle, gateway_session, gateway_owner)
    else {
        return true;
    };
    if !relay_network_client(
        client,
        network_service,
        network_handle,
        scheduler,
        gateway_pending,
        gateway_session,
        capabilities,
        block_context.pages,
        owner,
    ) {
        return false;
    }
    let (Some(remote), Some(page), Some(remote_state), Some(remote_session)) =
        (remote, gateway_page, remote_state.as_mut(), remote_session)
    else {
        return true;
    };
    let Some(request) = remote.request() else { return true };
    let Some(address) = block_context.pages.address(owner, page).filter(|_| request.page == page)
    else {
        return remote.reply(logos_abi::service::RemoteGateReply {
            id: request.id,
            status: logos_abi::service::RemoteGateStatus::Denied,
            length: 0,
            cursor: 0,
        }) && scheduler.wake(handle)
            && scheduler.run(handle);
    };
    let length = usize::from(request.length);
    let bytes = unsafe { core::slice::from_raw_parts(address as *const u8, length) };
    let mut source = [0; logos_remote::MAX_FRAME];
    if length > source.len() {
        return false;
    }
    source[..length].copy_from_slice(bytes);
    let outcome = match request.operation {
        logos_abi::service::RemoteGateOperation::Handshake => {
            let mut output = [0; logos_remote::MAX_FRAME];
            remote_state.handshake(&source[..length], &mut output).map(|length| {
                unsafe {
                    core::ptr::copy_nonoverlapping(output.as_ptr(), address as *mut u8, length)
                };
                (logos_abi::service::RemoteGateStatus::Complete, length, 0)
            })
        }
        logos_abi::service::RemoteGateOperation::Open => {
            let plaintext_length = length.checked_sub(16);
            plaintext_length
                .filter(|length| {
                    remote_state
                        .open(&source[..request.length as usize], output_page(address, *length))
                })
                .map(|length| (logos_abi::service::RemoteGateStatus::Complete, length, 0))
        }
        logos_abi::service::RemoteGateOperation::Seal => {
            let mut output = [0; logos_remote::MAX_FRAME];
            let sealed = length.checked_add(16).filter(|sealed| *sealed <= output.len());
            sealed
                .filter(|sealed| remote_state.seal(&source[..length], &mut output[..*sealed]))
                .map(|sealed| {
                    unsafe {
                        core::ptr::copy_nonoverlapping(output.as_ptr(), address as *mut u8, sealed)
                    };
                    (logos_abi::service::RemoteGateStatus::Complete, sealed, 0)
                })
        }
        logos_abi::service::RemoteGateOperation::Invoke => remote_invoke(
            remote_state,
            &source[..length],
            address,
            storage,
            block_dispatch,
            block_context,
            scheduler,
            storage_handle,
            persistence_page,
            persistence_owner,
            tick,
            sessions,
            sessions_handle,
            remote_session,
            capabilities,
            input,
            lifecycle,
            service_healthy,
            channel,
            responses,
            service_scheduler,
            service_capability,
            service,
        ),
        logos_abi::service::RemoteGateOperation::Subscribe
        | logos_abi::service::RemoteGateOperation::Credit
        | logos_abi::service::RemoteGateOperation::Acknowledge => {
            remote_simple_reply(&source[..length], address, b"subscribed")
        }
        logos_abi::service::RemoteGateOperation::Reset => {
            remote_state.reset_transport();
            Some((logos_abi::service::RemoteGateStatus::Complete, 0, 0))
        }
    };
    let (status, length, cursor) =
        outcome.unwrap_or((logos_abi::service::RemoteGateStatus::Denied, 0, 0));
    remote.reply(logos_abi::service::RemoteGateReply {
        id: request.id,
        status,
        length: length as u16,
        cursor,
    }) && scheduler.wake(handle)
        && scheduler.run(handle)
}

fn output_page(address: u64, length: usize) -> &'static mut [u8] {
    unsafe { core::slice::from_raw_parts_mut(address as *mut u8, length) }
}

fn remote_simple_reply(
    input: &[u8],
    address: u64,
    payload: &[u8],
) -> Option<(logos_abi::service::RemoteGateStatus, usize, u64)> {
    let request = logos_remote::RemoteMessage::decode(input).ok()?;
    let mut body = [0; logos_remote::REMOTE_MESSAGE_PAYLOAD];
    body[..payload.len()].copy_from_slice(payload);
    let reply = logos_remote::RemoteMessage {
        kind: logos_remote::RemoteMessageKind::Reply,
        id: request.id,
        sequence: request.sequence.max(1),
        cursor: 0,
        payload: body,
        payload_length: payload.len() as u16,
    };
    let mut encoded = [0; logos_remote::MAX_FRAME];
    let length = reply.encode(&mut encoded).ok()?;
    unsafe { core::ptr::copy_nonoverlapping(encoded.as_ptr(), address as *mut u8, length) };
    Some((logos_abi::service::RemoteGateStatus::Complete, length, 0))
}

#[allow(clippy::too_many_arguments)]
fn remote_invoke(
    state: &mut secrets::RemoteState,
    bytes: &[u8],
    address: u64,
    storage: native_task::StoreEndpoint,
    dispatch: &mut block::Dispatch,
    block_context: &mut block::DispatchContext<'_>,
    scheduler: &mut native_task::Scheduler<'_>,
    storage_handle: native_task::Handle,
    page: logos_abi::PageHandle,
    owner: u64,
    tick: u64,
    sessions: Option<native_task::SessionEndpoint>,
    sessions_handle: Option<native_task::Handle>,
    remote_session: &session::Context,
    capabilities: &capabilities::CapabilityManager,
    input: &mut input::Service,
    lifecycle: &mut supervisor::Lifecycle,
    service_healthy: bool,
    channel: &ipc::Channel,
    responses: &ipc::Channel,
    service_scheduler: &mut scheduler::Scheduler<'_>,
    service_capability: capabilities::Capability,
    service: services::ServiceHandle,
) -> Option<(logos_abi::service::RemoteGateStatus, usize, u64)> {
    let message = logos_remote::RemoteMessage::decode(bytes).ok()?;
    if message.kind != logos_remote::RemoteMessageKind::Invoke {
        return None;
    }
    let invocation = logos_remote::RemoteInvocation::decode(
        &message.payload[..usize::from(message.payload_length)],
    )
    .ok()?;
    let digest = message.digest().ok()?;
    let enrollment = state.enrollment();
    let mut session_id = [0; logos_remote::SESSION_ID_LEN];
    session_id[..8].copy_from_slice(&enrollment.generation.to_be_bytes());
    session_id[8..].copy_from_slice(&message.id.to_be_bytes());
    let current = state.control().session;
    if current.session == session_id && current.sequence == message.sequence {
        if current.digest != digest {
            return remote_error(message, address, b"mismatch");
        }
        if current.pending {
            return remote_error(message, address, b"indeterminate");
        }
        let length = usize::from(current.reply_length);
        unsafe {
            core::ptr::copy_nonoverlapping(current.reply.as_ptr(), address as *mut u8, length)
        };
        return Some((logos_abi::service::RemoteGateStatus::Complete, length, 0));
    }
    if current.session == session_id && message.sequence <= current.sequence {
        return remote_error(message, address, b"stale");
    }
    let mut control = state.control();
    control.session = logos_remote::SessionRecord {
        enrollment_generation: enrollment.generation,
        session: session_id,
        sequence: message.sequence,
        pending: true,
        digest,
        reply: [0; logos_remote::MAX_FRAME],
        reply_length: 0,
    };
    control.append(logos_remote::RemoteAuditEvent {
        sequence: 0,
        enrollment_generation: enrollment.generation,
        session: session_id,
        request_sequence: message.sequence,
        command: invocation.command as u8,
        phase: logos_remote::RemoteAuditPhase::Started,
        outcome: 0,
        tick,
        digest,
    });
    state.replace_control(control);
    if !persist_remote_control(
        state,
        storage,
        dispatch,
        block_context,
        scheduler,
        storage_handle,
        page,
        owner,
        tick,
    ) {
        return None;
    }
    let syscall = match invocation.command {
        logos_remote::RemoteCommand::Health => logos_abi::Syscall::Health,
        logos_remote::RemoteCommand::Ping => logos_abi::Syscall::Ping,
        logos_remote::RemoteCommand::Tasks => logos_abi::Syscall::Tasks,
        logos_remote::RemoteCommand::Services => logos_abi::Syscall::Services,
        logos_remote::RemoteCommand::Drivers => logos_abi::Syscall::Drivers,
        logos_remote::RemoteCommand::Trace => logos_abi::Syscall::Trace,
        logos_remote::RemoteCommand::Inspect => logos_abi::Syscall::Inspect,
        logos_remote::RemoteCommand::Restart => logos_abi::Syscall::Restart,
        logos_remote::RemoteCommand::Cancel => logos_abi::Syscall::Cancel,
        logos_remote::RemoteCommand::Reboot => logos_abi::Syscall::Reboot,
        logos_remote::RemoteCommand::PowerOff => logos_abi::Syscall::PowerOff,
    };
    let mut argument = [0; logos_abi::MAX_SESSION_TEXT];
    let argument_length = usize::from(invocation.argument_length);
    argument[..argument_length].copy_from_slice(&invocation.argument[..argument_length]);
    let reply = session::invoke_native(
        logos_abi::SessionRequest::new(syscall, argument, argument_length),
        sessions,
        scheduler,
        sessions_handle,
        effects::Context {
            session: remote_session,
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
        },
    )?;
    let mut payload = [0; logos_remote::REMOTE_MESSAGE_PAYLOAD];
    payload[..reply.length].copy_from_slice(&reply.text[..reply.length]);
    let response = logos_remote::RemoteMessage {
        kind: logos_remote::RemoteMessageKind::Reply,
        id: message.id,
        sequence: message.sequence,
        cursor: 0,
        payload,
        payload_length: reply.length as u16,
    };
    let mut encoded = [0; logos_remote::MAX_FRAME];
    let length = response.encode(&mut encoded).ok()?;
    let mut control = state.control();
    control.session.pending = false;
    control.session.reply[..length].copy_from_slice(&encoded[..length]);
    control.session.reply_length = length as u16;
    control.append(logos_remote::RemoteAuditEvent {
        sequence: 0,
        enrollment_generation: enrollment.generation,
        session: session_id,
        request_sequence: message.sequence,
        command: invocation.command as u8,
        phase: logos_remote::RemoteAuditPhase::Completed,
        outcome: 1,
        tick,
        digest,
    });
    state.replace_control(control);
    if !persist_remote_control(
        state,
        storage,
        dispatch,
        block_context,
        scheduler,
        storage_handle,
        page,
        owner,
        tick,
    ) {
        return None;
    }
    unsafe { core::ptr::copy_nonoverlapping(encoded.as_ptr(), address as *mut u8, length) };
    Some((logos_abi::service::RemoteGateStatus::Complete, length, 0))
}

fn remote_error(
    request: logos_remote::RemoteMessage,
    address: u64,
    payload: &[u8],
) -> Option<(logos_abi::service::RemoteGateStatus, usize, u64)> {
    let mut bytes = [0; logos_remote::REMOTE_MESSAGE_PAYLOAD];
    bytes[..payload.len()].copy_from_slice(payload);
    let reply = logos_remote::RemoteMessage {
        kind: logos_remote::RemoteMessageKind::Error,
        id: request.id,
        sequence: request.sequence,
        cursor: 0,
        payload: bytes,
        payload_length: payload.len() as u16,
    };
    let mut encoded = [0; logos_remote::MAX_FRAME];
    let length = reply.encode(&mut encoded).ok()?;
    unsafe { core::ptr::copy_nonoverlapping(encoded.as_ptr(), address as *mut u8, length) };
    Some((logos_abi::service::RemoteGateStatus::Complete, length, 0))
}

#[allow(clippy::too_many_arguments)]
fn persist_remote_control(
    state: &mut secrets::RemoteState,
    storage: native_task::StoreEndpoint,
    dispatch: &mut block::Dispatch,
    context: &mut block::DispatchContext<'_>,
    scheduler: &mut native_task::Scheduler<'_>,
    storage_handle: native_task::Handle,
    page: logos_abi::PageHandle,
    owner: u64,
    tick: u64,
) -> bool {
    let Some(address) = context.pages.address(owner, page) else { return false };
    let mut blob = [0; logos_remote::REMOTE_CONTROL_BLOB_BYTES];
    if !state.seal_control_random(&mut blob) {
        return false;
    }
    let status = protected_store_replace(
        storage,
        dispatch,
        context,
        scheduler,
        storage_handle,
        page,
        address,
        logos_abi::TRUST_NAMESPACE,
        logos_abi::TRUST_SESSION_NAME,
        &blob,
        tick,
    );
    if status == logos_abi::PersistenceStatus::Complete {
        true
    } else {
        state.disable();
        false
    }
}

fn run_storage_startup(
    dispatch: &mut block::Dispatch,
    context: &mut block::DispatchContext<'_>,
    scheduler: &mut native_task::Scheduler<'_>,
    handle: native_task::Handle,
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
    scheduler: &mut native_task::Scheduler<'_>,
    handle: native_task::Handle,
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

fn store_namespace(
    request: logos_abi::StoreRequest,
    state: &storage::RelayState,
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
    state: &mut storage::RelayState,
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

#[allow(clippy::too_many_arguments)]
fn relay_store_request(
    terminal: native_task::StoreEndpoint,
    storage: native_task::StoreEndpoint,
    dispatch: &mut block::Dispatch,
    block_context: &mut block::DispatchContext<'_>,
    terminal_owner: u64,
    storage_owner: u64,
    history_page: logos_abi::PageHandle,
    scheduler: &mut native_task::Scheduler<'_>,
    storage_handle: native_task::Handle,
    session: &session::Context,
    capabilities: &capabilities::CapabilityManager,
    state: &mut storage::RelayState,
    tick: u64,
) -> session::Relay {
    let Some(request) = terminal.request() else {
        return session::Relay::Handled(true);
    };
    if !storage.available() || !storage_handle.available() {
        return session::Relay::Handled(terminal.reply(logos_abi::StoreReply {
            id: request.id,
            status: logos_abi::PersistenceStatus::Unavailable,
            version: 0,
            length: 0,
        }));
    }
    let Some(namespace) = store_namespace(request, state) else {
        let _ = terminal.reply(logos_abi::StoreReply {
            id: request.id,
            status: logos_abi::PersistenceStatus::Denied,
            version: 0,
            length: 0,
        });
        return session::Relay::Handled(true);
    };
    if !session.allows_scoped(capabilities, store_capability(request.operation), namespace.0) {
        let replied = terminal.reply(logos_abi::StoreReply {
            id: request.id,
            status: logos_abi::PersistenceStatus::Denied,
            version: 0,
            length: 0,
        });
        return session::Relay::Handled(replied);
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
            return session::Relay::Handled(true);
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
            return session::Relay::Handled(true);
        }
        loaned = true;
    }
    if !storage.deliver(request) {
        if loaned {
            let _ = block_context.pages.return_loan(storage_owner, request.page);
        }
        return session::Relay::Handled(false);
    }
    if !scheduler.wake(storage_handle) || !scheduler.run(storage_handle) {
        if loaned {
            let _ = block_context.pages.return_loan(storage_owner, request.page);
        }
        return session::Relay::Handled(false);
    }
    let mut current_tick = tick;
    loop {
        if let Some(reply) = storage.response(request.id) {
            if loaned {
                let _ = block_context.pages.return_loan(storage_owner, request.page);
            }
            update_store_state(state, request, reply.status);
            return session::Relay::Handled(
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
            return session::Relay::Handled(false);
        }
        if let Some(reply) = dispatch.poll(block_context, current_tick) {
            if !block_context.endpoint.reply(reply)
                || !scheduler.wake(storage_handle)
                || !scheduler.run(storage_handle)
            {
                if loaned {
                    let _ = block_context.pages.return_loan(storage_owner, request.page);
                }
                return session::Relay::Handled(false);
            }
        } else if dispatch.accepts_new_request() {
            interrupts::wait_for_tick();
        } else {
            interrupts::wait_for_virtio();
        }
        current_tick = interrupts::ticks();
    }
}

#[allow(clippy::too_many_arguments)]
fn protected_store_request(
    storage: native_task::StoreEndpoint,
    dispatch: &mut block::Dispatch,
    block_context: &mut block::DispatchContext<'_>,
    scheduler: &mut native_task::Scheduler<'_>,
    storage_handle: native_task::Handle,
    request: logos_abi::StoreRequest,
    tick: u64,
) -> Option<logos_abi::StoreReply> {
    if !storage.available() || !storage.deliver(request) || !scheduler.wake(storage_handle) {
        return None;
    }
    let mut current_tick = tick.max(1);
    if !scheduler.run(storage_handle) {
        return None;
    }
    loop {
        if let Some(reply) = storage.response(request.id) {
            return Some(reply);
        }
        if scheduler.run_next() {
            continue;
        }
        if scheduler.failed(storage_handle) {
            return None;
        }
        if let Some(reply) = dispatch.poll(block_context, current_tick) {
            if !block_context.endpoint.reply(reply)
                || !scheduler.wake(storage_handle)
                || !scheduler.run(storage_handle)
            {
                return None;
            }
        } else if dispatch.accepts_new_request() {
            interrupts::wait_for_tick();
        } else {
            interrupts::wait_for_virtio();
        }
        current_tick = interrupts::ticks();
    }
}

#[allow(clippy::too_many_arguments)]
fn protected_store_replace(
    storage: native_task::StoreEndpoint,
    dispatch: &mut block::Dispatch,
    block_context: &mut block::DispatchContext<'_>,
    scheduler: &mut native_task::Scheduler<'_>,
    storage_handle: native_task::Handle,
    page: logos_abi::PageHandle,
    page_address: u64,
    namespace: logos_abi::NamespaceId,
    name: &[u8],
    bytes: &[u8],
    tick: u64,
) -> logos_abi::PersistenceStatus {
    if name.is_empty()
        || name.len() > logos_abi::MAX_OBJECT_NAME
        || bytes.is_empty()
        || bytes.len() > logos_abi::PAGE_SIZE
        || core::str::from_utf8(name).is_err()
    {
        return logos_abi::PersistenceStatus::Invalid;
    }
    let mut identity = [0; logos_abi::MAX_OBJECT_NAME];
    identity[..name.len()].copy_from_slice(name);
    let begin = logos_abi::StoreRequest {
        id: u32::MAX - 3,
        operation: logos_abi::StoreOperation::BeginReplace,
        namespace,
        name: identity,
        name_length: name.len() as u8,
        version: logos_abi::VersionSelector::None,
        offset: 0,
        length: bytes.len() as u32,
        page: logos_abi::PageHandle(0),
        deadline: tick.max(1).saturating_add(100),
    };
    if protected_store_request(
        storage,
        dispatch,
        block_context,
        scheduler,
        storage_handle,
        begin,
        tick,
    )
    .is_none_or(|reply| reply.status != logos_abi::PersistenceStatus::Complete)
    {
        return logos_abi::PersistenceStatus::Unavailable;
    }
    unsafe {
        core::ptr::copy_nonoverlapping(bytes.as_ptr(), page_address as *mut u8, bytes.len());
    }
    let write = logos_abi::StoreRequest {
        id: u32::MAX - 2,
        operation: logos_abi::StoreOperation::WriteChunk,
        namespace: logos_abi::NamespaceId(0),
        name: [0; logos_abi::MAX_OBJECT_NAME],
        name_length: 0,
        version: logos_abi::VersionSelector::None,
        offset: 0,
        length: bytes.len() as u32,
        page,
        deadline: tick.max(1).saturating_add(100),
    };
    if protected_store_request(
        storage,
        dispatch,
        block_context,
        scheduler,
        storage_handle,
        write,
        tick,
    )
    .is_none_or(|reply| reply.status != logos_abi::PersistenceStatus::Complete)
    {
        let _ = cancel_store_transaction(storage, scheduler, storage_handle);
        return logos_abi::PersistenceStatus::Unavailable;
    }
    let commit = logos_abi::StoreRequest {
        id: u32::MAX - 1,
        operation: logos_abi::StoreOperation::Commit,
        namespace: logos_abi::NamespaceId(0),
        name: [0; logos_abi::MAX_OBJECT_NAME],
        name_length: 0,
        version: logos_abi::VersionSelector::None,
        offset: 0,
        length: 0,
        page: logos_abi::PageHandle(0),
        deadline: tick.max(1).saturating_add(100),
    };
    protected_store_request(
        storage,
        dispatch,
        block_context,
        scheduler,
        storage_handle,
        commit,
        tick,
    )
    .map_or(logos_abi::PersistenceStatus::Unavailable, |reply| reply.status)
}

#[allow(clippy::too_many_arguments)]
fn protected_store_read(
    storage: native_task::StoreEndpoint,
    dispatch: &mut block::Dispatch,
    block_context: &mut block::DispatchContext<'_>,
    scheduler: &mut native_task::Scheduler<'_>,
    storage_handle: native_task::Handle,
    page: logos_abi::PageHandle,
    page_address: u64,
    namespace: logos_abi::NamespaceId,
    name: &[u8],
    output: &mut [u8],
    tick: u64,
) -> logos_abi::PersistenceStatus {
    if name.is_empty()
        || name.len() > logos_abi::MAX_OBJECT_NAME
        || output.is_empty()
        || output.len() > logos_abi::PAGE_SIZE
        || core::str::from_utf8(name).is_err()
    {
        return logos_abi::PersistenceStatus::Invalid;
    }
    let mut identity = [0; logos_abi::MAX_OBJECT_NAME];
    identity[..name.len()].copy_from_slice(name);
    let open = logos_abi::StoreRequest {
        id: u32::MAX - 6,
        operation: logos_abi::StoreOperation::OpenRead,
        namespace,
        name: identity,
        name_length: name.len() as u8,
        version: logos_abi::VersionSelector::Current,
        offset: 0,
        length: 0,
        page: logos_abi::PageHandle(0),
        deadline: tick.max(1).saturating_add(100),
    };
    let Some(reply) = protected_store_request(
        storage,
        dispatch,
        block_context,
        scheduler,
        storage_handle,
        open,
        tick,
    ) else {
        return logos_abi::PersistenceStatus::Unavailable;
    };
    if reply.status != logos_abi::PersistenceStatus::Complete {
        return reply.status;
    }
    let length = reply.length as usize;
    if length == 0 || length > output.len() {
        return logos_abi::PersistenceStatus::Invalid;
    }
    let read = logos_abi::StoreRequest {
        id: u32::MAX - 5,
        operation: logos_abi::StoreOperation::ReadChunk,
        namespace: logos_abi::NamespaceId(0),
        name: [0; logos_abi::MAX_OBJECT_NAME],
        name_length: 0,
        version: logos_abi::VersionSelector::None,
        offset: 0,
        length: length as u32,
        page,
        deadline: tick.max(1).saturating_add(100),
    };
    let Some(reply) = protected_store_request(
        storage,
        dispatch,
        block_context,
        scheduler,
        storage_handle,
        read,
        tick,
    ) else {
        return logos_abi::PersistenceStatus::Unavailable;
    };
    if reply.status == logos_abi::PersistenceStatus::Complete {
        unsafe {
            core::ptr::copy_nonoverlapping(page_address as *const u8, output.as_mut_ptr(), length);
        }
    }
    reply.status
}

#[allow(clippy::too_many_arguments)]
fn persist_remote_enrollment(
    state: &mut secrets::RemoteState,
    bootstrap: Option<logos_remote::Bootstrap>,
    storage: native_task::StoreEndpoint,
    dispatch: &mut block::Dispatch,
    block_context: &mut block::DispatchContext<'_>,
    scheduler: &mut native_task::Scheduler<'_>,
    storage_handle: native_task::Handle,
    page: logos_abi::PageHandle,
    terminal_owner: u64,
    tick: u64,
) -> bool {
    let Some(bootstrap) = bootstrap else { return false };
    let Some(page_address) = block_context.pages.address(terminal_owner, page) else {
        return false;
    };
    let mut blob = [0; logos_remote::ENROLLMENT_BLOB_BYTES];
    if !state.seal_enrollment_random(&mut blob) {
        return false;
    }
    let status = protected_store_replace(
        storage,
        dispatch,
        block_context,
        scheduler,
        storage_handle,
        page,
        page_address,
        logos_abi::TRUST_NAMESPACE,
        logos_abi::TRUST_ENROLLMENT_NAME,
        &blob,
        tick,
    );
    if status == logos_abi::PersistenceStatus::Complete {
        true
    } else {
        *state = secrets::RemoteState::unavailable(bootstrap);
        false
    }
}

#[allow(clippy::too_many_arguments)]
fn relay_terminal_store_requests(
    terminal: native_task::StoreEndpoint,
    storage: native_task::StoreEndpoint,
    dispatch: &mut block::Dispatch,
    block_context: &mut block::DispatchContext<'_>,
    terminal_owner: u64,
    storage_owner: u64,
    history_page: logos_abi::PageHandle,
    scheduler: &mut native_task::Scheduler<'_>,
    terminal_handle: native_task::Handle,
    storage_handle: native_task::Handle,
    session: &session::Context,
    capabilities: &capabilities::CapabilityManager,
    state: &mut storage::RelayState,
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
                status: logos_abi::PersistenceStatus::Unavailable,
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
    scheduler: &mut native_task::Scheduler<'_>,
    storage_handle: native_task::Handle,
) -> bool {
    if !storage.available() || !storage_handle.available() {
        return true;
    }
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

type TerminalEndpoints = (
    native_task::InputEndpoint,
    native_task::SyscallEndpoint,
    native_task::DisplayEndpoint,
    native_task::StoreEndpoint,
    native_task::NetworkEndpoint,
);

fn terminal_endpoints(
    scheduler: &native_task::Scheduler<'_>,
    handle: native_task::Handle,
) -> Option<TerminalEndpoints> {
    Some((
        scheduler.input_endpoint(handle)?,
        scheduler.syscall_endpoint(handle)?,
        scheduler.display_endpoint(handle)?,
        scheduler.store_endpoint(handle)?,
        scheduler.network_endpoint(handle)?,
    ))
}

#[allow(clippy::too_many_arguments)]
fn replace_terminal(
    scheduler: &mut native_task::Scheduler<'_>,
    handle: native_task::Handle,
    storage_handle: native_task::Handle,
    memory: &mut memory::PhysicalMemory,
    pages: &mut logos_core::shared_pages::SharedPages,
    terminal_owner: u64,
    storage_owner: u64,
    history: logos_abi::PageHandle,
) -> Option<(native_task::Handle, TerminalEndpoints, logos_abi::PageHandle)> {
    if !scheduler.failed(handle) && !scheduler.fail(handle) {
        debug::write_line(b"LogOS: terminal replacement quarantine failed");
        return None;
    }
    let Some(previous) = pages.address(terminal_owner, history) else {
        debug::write_line(b"LogOS: terminal replacement history missing");
        return None;
    };
    let mut replacement_page = None;
    let Some(replacement) = scheduler.replace(handle, memory, |task, memory| {
        let Some(address) = task.map_shared_owned(memory) else { return false };
        unsafe {
            core::ptr::copy_nonoverlapping(
                previous as *const u8,
                address as *mut u8,
                logos_abi::PAGE_SIZE,
            );
        }
        replacement_page = Some(address);
        true
    }) else {
        debug::write_line(b"LogOS: terminal replacement load failed");
        return None;
    };
    let address = replacement_page?;
    if pages.release(terminal_owner, history).is_none() {
        debug::write_line(b"LogOS: terminal replacement history release failed");
        return None;
    }
    let Some(new_history) = pages.register(terminal_owner, address, 1) else {
        debug::write_line(b"LogOS: terminal replacement history register failed");
        return None;
    };
    let Some(endpoints) = terminal_endpoints(scheduler, replacement) else {
        debug::write_line(b"LogOS: terminal replacement endpoint failed");
        return None;
    };
    if !endpoints.3.configure_shared_page(new_history) {
        return None;
    }
    if storage_handle.available() {
        if !scheduler.task_mut(storage_handle)?.remap_shared_borrowed(address) {
            debug::write_line(b"LogOS: terminal replacement Store remap failed");
            return None;
        }
        let storage = scheduler.store_endpoint(storage_handle)?;
        if !storage.remap_shared_page(new_history)
            || pages.address(storage_owner, new_history).is_some()
        {
            debug::write_line(b"LogOS: terminal replacement Store configure failed");
            return None;
        }
    }
    Some((replacement, endpoints, new_history))
}

type StorageReplacement = (
    native_task::Handle,
    native_task::StoreEndpoint,
    native_task::BlockEndpoint,
    logos_abi::PageHandle,
    u64,
);

#[allow(clippy::too_many_arguments)]
fn replace_storage(
    scheduler: &mut native_task::Scheduler<'_>,
    handle: native_task::Handle,
    memory: &mut memory::PhysicalMemory,
    pages: &mut logos_core::shared_pages::SharedPages,
    storage_owner: u64,
    block_page: logos_abi::PageHandle,
    history_address: u64,
    history: logos_abi::PageHandle,
) -> Option<StorageReplacement> {
    if !scheduler.failed(handle) && !scheduler.fail(handle) {
        return None;
    }
    let mut mapped_block = None;
    let replacement = scheduler.replace(handle, memory, |task, memory| {
        if task.map_heap(memory).is_none() || !task.map_shared_borrowed(history_address) {
            return false;
        }
        mapped_block = task.map_block_owned(memory);
        mapped_block.is_some()
    })?;
    let (block_physical, block_virtual) = mapped_block?;
    pages.release(storage_owner, block_page)?;
    let block_page = pages.register(storage_owner, block_physical, 1)?;
    let store = scheduler.store_endpoint(replacement)?;
    let block = scheduler.block_endpoint(replacement)?;
    if !store.configure_shared_page(history)
        || !block
            .configure(logos_abi::service::BlockPage { handle: block_page, address: block_virtual })
    {
        return None;
    }
    Some((replacement, store, block, block_page, block_virtual))
}

fn replace_network(
    scheduler: &mut native_task::Scheduler<'_>,
    handle: native_task::Handle,
    memory: &mut memory::PhysicalMemory,
    pages: &mut logos_core::shared_pages::SharedPages,
    previous: NetworkResources,
) -> Option<(native_task::Handle, native_task::NetworkEndpoint, NetworkResources, NetworkDmaPages)>
{
    if !scheduler.failed(handle) && !scheduler.fail(handle) {
        return None;
    }
    let mut mapped = None;
    let replacement = scheduler.replace(handle, memory, |task, memory| {
        mapped = task.map_network_owned(memory);
        mapped.is_some()
    })?;
    let ((rx_physical, rx_virtual), (tx_physical, tx_virtual)) = mapped?;
    pages.release(previous.owner, previous.rx)?;
    pages.release(previous.owner, previous.tx)?;
    let rx = pages.register(previous.owner, rx_physical, 2)?;
    let tx = pages.register(previous.owner, tx_physical, 2)?;
    let resources = NetworkResources {
        owner: previous.owner,
        rx,
        rx_physical,
        rx_virtual,
        tx,
        tx_physical,
        tx_virtual,
    };
    let endpoint = scheduler.network_endpoint(replacement)?;
    if !endpoint.configure(logos_abi::service::NetworkPages {
        rx_handle: rx,
        rx_address: rx_virtual,
        tx_handle: tx,
        tx_address: tx_virtual,
    }) {
        return None;
    }
    Some((
        replacement,
        endpoint,
        resources,
        NetworkDmaPages { rx_address: rx_physical, tx_address: tx_physical },
    ))
}

fn restart_native_service(
    scheduler: &mut native_task::Scheduler<'_>,
    handle: native_task::Handle,
    memory: &mut memory::PhysicalMemory,
) -> Option<native_task::Handle> {
    if !scheduler.failed(handle) && !scheduler.fail(handle) {
        return None;
    }
    scheduler.replace(handle, memory, |_, _| true)
}

fn hex_key(key: &[u8; 32], output: &mut [u8; 64]) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for (index, byte) in key.iter().copied().enumerate() {
        output[index * 2] = HEX[(byte >> 4) as usize];
        output[index * 2 + 1] = HEX[(byte & 0x0f) as usize];
    }
}

fn hex_decode_key(input: &[u8], output: &mut [u8; 32]) -> bool {
    if input.len() != 64 {
        return false;
    }
    for (index, chunk) in input.chunks_exact(2).enumerate() {
        let Some(high) = hex_value(chunk[0]) else { return false };
        let Some(low) = hex_value(chunk[1]) else { return false };
        output[index] = (high << 4) | low;
    }
    !output.iter().all(|byte| *byte == 0)
}

fn decimal_u64(mut value: u64, output: &mut [u8; 20]) -> usize {
    let mut position = output.len();
    loop {
        position -= 1;
        output[position] = b'0' + (value % 10) as u8;
        value /= 10;
        if value == 0 {
            break;
        }
    }
    let length = output.len() - position;
    output.copy_within(position.., 0);
    length
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
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
