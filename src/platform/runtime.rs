use crate::arch::{acpi, cpu, interrupts, pci};
use crate::console::{native_display, recovery as console};
use crate::drivers::{
    block as block_driver, device, keyboard, network as network_driver, resources, supervisor,
    virtio,
};
use crate::ipc::{self, approvals, effects};
use crate::mm::{address_space, memory, virtual_memory};
#[cfg(feature = "test-hooks")]
use crate::platform::proofs;
use crate::platform::{
    audit, balloon, block, entropy, health, identity, inference, mode, network, payload, pe,
    remote, root_key, secrets, services, session, storage, time, trace,
};
use crate::sched::{native_task, scheduler};
#[cfg(feature = "test-hooks")]
use crate::test_hooks;
use crate::{boot, debug};
use network::{NetworkClientSlot, Resources as NetworkResources};

use logos_core::capabilities;
use logos_terminal::{command, display, input, terminal, text};
use uefi::mem::memory_map::MemoryMap;

#[cfg_attr(feature = "test-hooks", allow(unreachable_code, unused_mut, unused_variables))]
#[allow(clippy::too_many_arguments)]
pub(crate) fn run(
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
    let mut remote_runtime = remote::RemoteRuntime::new(remote_bootstrap);
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
    let Some(mut terminal_task) = native_task::Task::load(
        &mut memory,
        payload,
        &privilege,
        services::Service::Terminal.spec().endpoints,
    ) else {
        fail!(b"native service entry");
    };
    let Some(terminal_input) = terminal_task.input_endpoint() else {
        fail!(b"native input endpoint");
    };
    let Some(terminal_display) = terminal_task.display_endpoint() else {
        fail!(b"native display endpoint");
    };
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
    let network_device = network_pci.and_then(|network_pci| {
        let (bus, slot, _) = network_pci.location();
        let gsi = acpi.and_then(|tables| {
            tables.pci_gsi(bus, slot, network_pci.interrupt_pin().checked_sub(1)?)
        });
        gsi.and_then(|gsi| network_driver::Device::bind(network_pci, gsi, &mut memory))
    });
    let mut network_runtime = network::NetworkRuntime::new(network_device);
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
    #[cfg(feature = "test-hooks")]
    let Some(tcp_test_bind_capability) = capabilities.grant_scoped64(
        capabilities::CapabilityKind::NetworkBind,
        logos_abi::NetworkScope::new(
            logos_abi::NetworkProtocol::Tcp,
            0,
            logos_abi::REMOTE_TCP_PORT,
        )
        .0,
    ) else {
        fail!(b"capabilities");
    };
    #[cfg(feature = "test-hooks")]
    let Some(tcp_test_send_capability) = capabilities.grant_scoped64(
        capabilities::CapabilityKind::NetworkSend,
        logos_abi::NetworkScope::new(
            logos_abi::NetworkProtocol::Tcp,
            0,
            logos_abi::REMOTE_TCP_PORT,
        )
        .0,
    ) else {
        fail!(b"capabilities");
    };
    #[cfg(feature = "test-hooks")]
    let Some(tcp_test_receive_capability) = capabilities.grant_scoped64(
        capabilities::CapabilityKind::NetworkReceive,
        logos_abi::NetworkScope::new(
            logos_abi::NetworkProtocol::Tcp,
            0,
            logos_abi::REMOTE_TCP_PORT,
        )
        .0,
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
    let Some(tcp_test_session) = session::Context::new(
        session::Id(5),
        session::Principal::LOCAL,
        &[tcp_test_bind_capability, tcp_test_send_capability, tcp_test_receive_capability],
    ) else {
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
        services::self_check()
            && services.resolve(balloon::SERVICE) == Some(virtio_handle)
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
    let Some(mut native_terminal) = native_task::Task::load(
        &mut memory,
        payload,
        &privilege,
        services::Service::Terminal.spec().endpoints,
    ) else {
        fail!(b"native terminal task");
    };
    let native_sessions = payloads.sessions.and_then(|payload| {
        native_task::Task::load(
            &mut memory,
            payload,
            &privilege,
            services::Service::Sessions.spec().endpoints,
        )
    });
    let mut native_storage = payloads.storage.and_then(|payload| {
        native_task::Task::load(
            &mut memory,
            payload,
            &privilege,
            services::Service::Storage.spec().endpoints,
        )
    });
    let mut native_network = payloads.network.and_then(|payload| {
        (network_runtime.has_device() && network_service_handle.is_some())
            .then(|| {
                native_task::Task::load(
                    &mut memory,
                    payload,
                    &privilege,
                    services::Service::Network.spec().endpoints,
                )
            })
            .flatten()
    });
    let mut native_gateway = payloads.gateway.and_then(|payload| {
        (gateway_service_handle.is_some() && gateway_network_session.is_some())
            .then(|| {
                native_task::Task::load(
                    &mut memory,
                    payload,
                    &privilege,
                    services::Service::Gateway.spec().endpoints,
                )
            })
            .flatten()
    });
    let terminal_spec = services::Service::Terminal.spec();
    let storage_spec = services::Service::Storage.spec();
    let network_spec = services::Service::Network.spec();
    let gateway_spec = services::Service::Gateway.spec();
    check!(
        b"typed endpoint map",
        terminal_spec.endpoints.contains(services::EndpointSet::INPUT)
            && terminal_spec.endpoints.contains(services::EndpointSet::DISPLAY)
            && terminal_spec.endpoints.contains(services::EndpointSet::STORE_CLIENT)
            && terminal_spec.endpoints.contains(services::EndpointSet::NETWORK_CLIENT)
            && storage_spec.endpoints.contains(services::EndpointSet::STORE_SERVER)
            && storage_spec.endpoints.contains(services::EndpointSet::BLOCK_CLIENT)
            && network_spec.endpoints.contains(services::EndpointSet::NETWORK_DEVICE)
            && network_spec.endpoints.contains(services::EndpointSet::NETWORK_EVENT)
            && gateway_spec.endpoints.contains(services::EndpointSet::NETWORK_CLIENT)
            && gateway_spec.endpoints.contains(services::EndpointSet::REMOTE)
            && gateway_spec.endpoints.contains(services::EndpointSet::STORE_CLIENT),
    );
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
    let mut gateway_page = native_gateway
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
    let mut _storage_block_virtual = storage_block.map(|(_, address)| address).unwrap_or(0);
    #[cfg_attr(not(feature = "test-hooks"), allow(unused_mut))]
    let mut storage_block_page = storage_block
        .and_then(|(physical, _)| shared_pages.register(storage_owner, physical, 1))
        .unwrap_or(logos_abi::PageHandle(0));
    check!(
        b"terminal storage page",
        shared_pages.address(terminal_owner, shared_history).is_some()
    );
    check!(b"shared pages", logos_core::shared_pages::self_check());
    let network_resources = native_network
        .as_mut()
        .and_then(|task| task.map_network_owned(&mut memory))
        .and_then(|((rx_physical, _rx_virtual), (tx_physical, _tx_virtual))| {
            let owner = network_owner?;
            let rx = shared_pages.register(owner, rx_physical, 2)?;
            let Some(tx) = shared_pages.register(owner, tx_physical, 2) else {
                let _ = shared_pages.release(owner, rx);
                return None;
            };
            Some(NetworkResources {
                owner,
                rx,
                rx_virtual: rx_physical,
                tx,
                tx_virtual: tx_physical,
            })
        });
    if network_resources.is_none() {
        if let Some(task) = native_network.take() {
            let _ = task.release(&mut memory);
        }
    }
    check!(
        b"network service pages",
        !network_runtime.has_device() || native_network.is_none() || network_resources.is_some()
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
    check!(b"remote gate", remote::self_check());
    let Some(mut native_input) = native_terminal.input_endpoint() else {
        fail!(b"native input endpoint");
    };
    let mut native_command = native_terminal.syscall_endpoint();
    let Some(mut native_display) = native_terminal.display_endpoint() else {
        fail!(b"native display endpoint");
    };
    let mut native_sessions_endpoint =
        native_sessions.as_ref().map(native_task::Task::session_endpoint);
    #[cfg_attr(not(feature = "test-hooks"), allow(unused_mut))]
    let mut native_storage_block = native_storage
        .as_ref()
        .and_then(native_task::Task::block_client_endpoint)
        .unwrap_or_else(native_task::BlockClientEndpoint::unavailable);
    let mut native_store = native_terminal
        .store_client_endpoint()
        .unwrap_or_else(native_task::StoreClientEndpoint::unavailable);
    #[cfg_attr(not(feature = "test-hooks"), allow(unused_mut))]
    let mut native_storage_store = native_storage
        .as_ref()
        .and_then(native_task::Task::store_server_endpoint)
        .unwrap_or_else(native_task::StoreServerEndpoint::unavailable);
    let Some(mut native_terminal_network) = native_terminal.network_client_endpoint() else {
        fail!(b"native terminal network client endpoint");
    };
    let mut native_gateway_network =
        native_gateway.as_ref().and_then(native_task::Task::network_client_endpoint);
    let mut native_gateway_remote =
        native_gateway.as_ref().and_then(native_task::Task::remote_endpoint);
    let mut native_gateway_store =
        native_gateway.as_ref().and_then(native_task::Task::store_client_endpoint);
    let mut native_network_endpoint =
        native_network.as_ref().map(native_task::Task::network_endpoint);
    check!(
        b"storage shared page",
        native_store.configure_transfer(shared_history)
            && (!native_storage_store.available()
                || native_storage_store.configure_transfer(shared_history)),
    );
    check!(
        b"gateway shared page",
        native_gateway_store
            .zip(gateway_page)
            .is_none_or(|(endpoint, page)| endpoint.configure_transfer(page)),
    );
    check!(
        b"network client transfer pages",
        native_terminal_network.configure_transfer(shared_history)
            && native_gateway_network
                .zip(gateway_page)
                .is_none_or(|(endpoint, page)| endpoint.configure_transfer(page)),
    );
    check!(
        b"storage block page",
        !native_storage_block.available()
            || native_storage_block.configure_transfer(storage_block_page),
    );
    let network_pages_ready = network_resources.is_some();
    check!(
        b"network page configuration",
        !network_runtime.has_device() || native_network_endpoint.is_none() || network_pages_ready
    );
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
    let mut sessions_runtime = session::SessionsRuntime::new(native_command);
    sessions_runtime.bind_sessions(native_sessions_endpoint, sessions_handle);
    #[cfg_attr(not(feature = "test-hooks"), allow(unused_mut))]
    let mut storage_handle = native_storage
        .and_then(|task| native_scheduler.spawn(task))
        .unwrap_or_else(native_task::Handle::unavailable);
    let mut storage_runtime = storage::StorageRuntime::new(
        native_store,
        native_storage_store,
        native_storage_block,
        storage_handle,
    );
    if storage_handle.available() {
        check!(b"native storage ready", native_scheduler.run(storage_handle));
        check!(
            b"storage startup",
            storage_runtime.startup(
                &mut block::DispatchContext {
                    endpoint: native_storage_block,
                    pages: &mut shared_pages,
                    store_owner: storage_owner,
                    store_page: storage_block_page,
                    device: &mut block_device,
                    memory: &mut memory,
                },
                &mut native_scheduler,
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
        let status = storage_runtime.protected_store_read(
            &mut block::DispatchContext {
                endpoint: native_storage_block,
                pages: &mut shared_pages,
                store_owner: storage_owner,
                store_page: storage_block_page,
                device: &mut block_device,
                memory: &mut memory,
            },
            &mut native_scheduler,
            shared_history,
            page_address,
            logos_abi::TRUST_NAMESPACE,
            logos_abi::TRUST_ENROLLMENT_NAME,
            &mut blob,
            interrupts::ticks(),
        );
        if status == logos_abi::PersistenceStatus::Complete {
            remote_runtime
                .replace_state(secrets::RemoteState::load_enrollment(bootstrap, &mut blob));
        } else if status != logos_abi::PersistenceStatus::NotFound {
            remote_runtime.replace_state(secrets::RemoteState::unavailable(bootstrap));
        }
    } else if let Some(bootstrap) = remote_bootstrap {
        remote_runtime.replace_state(secrets::RemoteState::unavailable(bootstrap));
    }
    if remote_runtime.state().is_some_and(secrets::RemoteState::available)
        && let Some(page_address) = shared_pages.address(terminal_owner, shared_history)
    {
        let mut blob = [0; logos_remote::REMOTE_CONTROL_BLOB_BYTES];
        let status = storage_runtime.protected_store_read(
            &mut block::DispatchContext {
                endpoint: native_storage_block,
                pages: &mut shared_pages,
                store_owner: storage_owner,
                store_page: storage_block_page,
                device: &mut block_device,
                memory: &mut memory,
            },
            &mut native_scheduler,
            shared_history,
            page_address,
            logos_abi::TRUST_NAMESPACE,
            logos_abi::TRUST_SESSION_NAME,
            &mut blob,
            interrupts::ticks(),
        );
        if status == logos_abi::PersistenceStatus::Complete {
            let _ = remote_runtime.load_control(&mut blob);
        } else if status != logos_abi::PersistenceStatus::NotFound {
            remote_runtime.disable();
        }
    }
    let mut network_handle = if let Some(network_task) = native_network.take()
        && let Some(handle) = native_scheduler.spawn(network_task)
    {
        let bound = network_resources.is_some_and(|resources| {
            let Some(device_endpoint) = native_scheduler
                .network_device_endpoint(handle, network_runtime.device_generation())
            else {
                return false;
            };
            let Some(event_endpoint) = native_scheduler
                .network_event_endpoint(handle, network_runtime.device_generation())
            else {
                return false;
            };
            let Some(server_endpoint) = native_scheduler.network_server_endpoint(handle) else {
                return false;
            };
            network_runtime.bind(
                handle,
                server_endpoint,
                device_endpoint,
                event_endpoint,
                resources,
            )
        });
        let ran = bound && native_scheduler.run(handle) && !native_scheduler.failed(handle);
        ran.then_some(handle)
    } else {
        None
    };
    if network_handle.is_some() {
        native_services.ready(supervisor::NativeService::Network);
    } else {
        let _ = native_services.missing(supervisor::NativeService::Network);
    }
    let network_bound = network_handle.is_some() && network_runtime.task().is_some();
    check!(
        b"network typed endpoints",
        !network_runtime.has_device() || network_handle.is_none() || network_bound,
    );
    let mut gateway_handle = if remote::gateway_allowed(
        network_handle.is_some(),
        sessions_handle.is_some(),
        storage_handle.available(),
        remote_runtime.state(),
        cfg!(feature = "test-hooks"),
    ) {
        native_gateway.take().and_then(|task| native_scheduler.spawn(task))
    } else {
        None
    };
    if gateway_handle.is_none() {
        let _ = native_services.missing(supervisor::NativeService::Gateway);
    }
    #[cfg(feature = "test-hooks")]
    let mut network_qemu_asserted = false;

    macro_rules! poll_gateway {
        () => {{
            let started_now = remote_runtime.start(
                network_runtime.configured(),
                gateway_handle,
                &mut native_scheduler,
            );
            if started_now {
                debug::write_line(b"LogOS: Gateway started");
                native_services.ready(supervisor::NativeService::Gateway);
            }
            if !remote_runtime.started() {
                true
            } else {
                poll_gateway(
                    &mut network_runtime,
                    native_gateway_network,
                    native_gateway_remote,
                    gateway_handle,
                    gateway_network_session.as_ref(),
                    gateway_owner,
                    native_network_endpoint,
                    network_handle,
                    &mut native_scheduler,
                    &capabilities,
                    gateway_page,
                    shared_history,
                    terminal_owner,
                    &mut remote_runtime,
                    native_sessions_endpoint,
                    sessions_handle,
                    remote_session.as_ref(),
                    &mut storage_runtime,
                    &mut block::DispatchContext {
                        endpoint: native_storage_block,
                        pages: &mut shared_pages,
                        store_owner: storage_owner,
                        store_page: storage_block_page,
                        device: &mut block_device,
                        memory: &mut memory,
                    },
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
            }
        }};
    }
    health.finish();
    if !native_input.deliver(logos_abi::InputEvent::STARTUP) {
        fail!(b"terminal history startup");
    }
    if !native_scheduler.wake(native_handle) {
        fail!(b"terminal history startup");
    }
    if !native_scheduler.run(native_handle) {
        fail!(b"terminal history startup");
    }
    let terminal_history_startup = storage_runtime.relay_terminal_store_requests(
        native_store,
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
        &session,
        &capabilities,
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
    let proof = proofs::State::new();
    #[cfg(feature = "test-hooks")]
    test_hooks::serve(
        if native_storage_store.available() {
            native_storage_store.status().unwrap_or(logos_core::native_service::STORAGE_IO_FAILED)
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
                    proof.reset();
                    input.set_layout(input::Layout::Qwerty);
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
                        &mut network_runtime,
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
                    storage_runtime.rebind_client(native_store);
                    sessions_runtime.bind_terminal(native_command);
                    shared_history = history;
                    storage_runtime.reset_relay();
                    debug::write_line(b"LogOS: reset terminal ready");
                    if !native_store.configure_transfer(shared_history)
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
                        sessions_runtime.bind_sessions(native_sessions_endpoint, sessions_handle);
                        debug::write_line(b"LogOS: reset sessions ready");
                        if !native_scheduler.run(restarted_sessions)
                            || native_scheduler.wake(previous_sessions)
                        {
                            return false;
                        }
                    }

                    let previous_storage = storage_handle;
                    storage_runtime.cancel_block(&mut block::DispatchContext {
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
                    _storage_block_virtual = block_virtual;
                    if !native_scheduler.run(restarted_storage) {
                        return false;
                    }
                    storage_handle = restarted_storage;
                    storage_runtime.rebind(
                        native_storage_store,
                        native_storage_block,
                        storage_handle,
                    );
                    debug::write_line(b"LogOS: reset storage ready");
                    if native_scheduler.wake(previous_storage) {
                        return false;
                    }
                    if !storage_runtime.startup(
                        &mut block::DispatchContext {
                            endpoint: native_storage_block,
                            pages: &mut shared_pages,
                            store_owner: storage_owner,
                            store_page: storage_block_page,
                            device: &mut block_device,
                            memory: &mut memory,
                        },
                        &mut native_scheduler,
                    ) {
                        return false;
                    }
                    if !native_input.deliver(logos_abi::InputEvent::STARTUP)
                        || !native_scheduler.wake(native_handle)
                        || !native_scheduler.run(native_handle)
                    {
                        return false;
                    }
                    if !storage_runtime.relay_terminal_store_requests(
                        native_store,
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
                        &session,
                        &capabilities,
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
                    #[cfg(feature = "test-hooks")]
                    {
                        network_qemu_asserted = false;
                    }
                    return true;
                }
                if value == "persistence/block-read-flush" {
                    debug::write_line(b"LogOS: storage proof passed");
                    proof.record(true);
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
                    let timeout = native_storage_block.deliver(timeout_request)
                        && storage_runtime
                            .block_reply(
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
                                    && native_storage_block.response(timeout_id).is_some_and(
                                        |reply| {
                                            reply.status == logos_abi::PersistenceStatus::TimedOut
                                        },
                                    )
                            });
                    let after = block_device.diagnostics();
                    if !timeout
                        || after.0 != before.0.saturating_add(1)
                        || after.1 != before.1.saturating_add(1)
                    {
                        proof.reset();
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
                    let read = native_storage_block.deliver(read_request);
                    let read = if read {
                        loop {
                            let Some(reply) = storage_runtime.block_reply(
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
                                && native_storage_block.response(read_id).is_some_and(|reply| {
                                    reply.status == logos_abi::PersistenceStatus::Complete
                                });
                        }
                    } else {
                        false
                    };
                    proof.record(read);
                    return read;
                }
                if value == "persistence/terminal-history" {
                    let status = native_storage_store.status();
                    if status == Some(logos_core::native_service::STORAGE_FORMATTED) {
                        proof.record(true);
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
                        if !storage_runtime.relay_terminal_store_requests(
                            native_store,
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
                            &session,
                            &capabilities,
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
                    proof.record(navigation);
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
                        &mut network_runtime,
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
                    storage_runtime.rebind_client(native_store);
                    sessions_runtime.bind_terminal(native_command);
                    shared_history = history;
                    storage_runtime.reset_relay();
                    if restarted.generation() == previous.generation()
                        || native_display.context() == previous_context
                        || native_scheduler.wake(previous)
                        || !native_store.configure_transfer(shared_history)
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
                        || !storage_runtime.relay_terminal_store_requests(
                            native_store,
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
                            &session,
                            &capabilities,
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
                    sessions_runtime.bind_sessions(native_sessions_endpoint, sessions_handle);
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
                        native_storage_store.deliver(
                            test_store_request(
                                id,
                                logos_abi::StoreOperation::Cancel,
                                logos_abi::NamespaceId(0),
                                logos_abi::VersionSelector::None,
                            ),
                            0,
                        ) && native_scheduler.wake(previous)
                            && native_scheduler.run(previous)
                            && native_scheduler.failed(previous)
                    } else {
                        native_scheduler.fail(previous)
                    };
                    if !failed {
                        return false;
                    }
                    storage_runtime.cancel_block(&mut block::DispatchContext {
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
                    _storage_block_virtual = block_virtual;
                    if restarted.generation() == previous.generation()
                        || native_storage_store.context() == previous_context
                        || !native_scheduler.run(restarted)
                    {
                        return false;
                    };
                    storage_handle = restarted;
                    storage_runtime.rebind(
                        native_storage_store,
                        native_storage_block,
                        storage_handle,
                    );
                    storage_runtime.reset_relay();
                    if native_scheduler.wake(previous) || !native_scheduler.wake(restarted) {
                        return false;
                    }
                    proof.record(true);
                    return true;
                }
                if matches!(value, "assert-network-service-panic" | "assert-network-service-fault")
                {
                    let (Some(previous), Some(_previous_endpoint), Some(resources)) =
                        (network_handle, native_network_endpoint, network_runtime.resources())
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
                    if !network_runtime
                        .server_endpoint()
                        .is_some_and(|endpoint| endpoint.deliver(terminal_owner, request))
                    {
                        return false;
                    }
                    if native_scheduler.wake(previous) {
                        if !native_scheduler.run(previous) {
                            return false;
                        }
                    } else if !native_scheduler.run(previous) {
                        return false;
                    }
                    if !native_scheduler.failed(previous) {
                        return false;
                    }
                    let Some((restarted, endpoint, resources)) = replace_network(
                        &mut native_scheduler,
                        &mut network_runtime,
                        previous,
                        &mut memory,
                        &mut shared_pages,
                        resources,
                    ) else {
                        return false;
                    };
                    let Some(device_endpoint) = native_scheduler
                        .network_device_endpoint(restarted, network_runtime.device_generation())
                    else {
                        return false;
                    };
                    let Some(event_endpoint) = native_scheduler
                        .network_event_endpoint(restarted, network_runtime.device_generation())
                    else {
                        return false;
                    };
                    let Some(server_endpoint) = native_scheduler.network_server_endpoint(restarted)
                    else {
                        return false;
                    };
                    if restarted.generation() == previous.generation()
                        || endpoint.context() == _previous_endpoint.context()
                        || !network_runtime.bind(
                            restarted,
                            server_endpoint,
                            device_endpoint,
                            event_endpoint,
                            resources,
                        )
                        || !native_scheduler.run(restarted)
                        || native_scheduler.wake(previous)
                    {
                        return false;
                    }
                    network_handle = Some(restarted);
                    native_network_endpoint = Some(endpoint);
                    #[cfg(feature = "test-hooks")]
                    {
                        network_qemu_asserted = false;
                    }
                }
                if value == "persistence/write-interruption" || value == "persistence/recovery" {
                    let status = native_storage_store.status();
                    let passed = matches!(
                        status,
                        Some(logos_core::native_service::STORAGE_RECOVERED)
                            | Some(logos_core::native_service::STORAGE_RECOVERED_INCOMPLETE)
                    );
                    proof.record(passed);
                    return passed;
                }
                if value == "persistence/corruption-detected" {
                    let status = native_storage_store.status();
                    let passed = status == Some(logos_core::native_service::STORAGE_CORRUPT);
                    proof.record(passed);
                    return passed;
                }
                if value == "persistence/capability-denied" {
                    let history_page = shared_history;
                    let mut denied =
                        |request: logos_abi::StoreRequest, request_session: &session::Context| {
                            let delivered = native_store.deliver(request);
                            if !delivered {
                                return false;
                            }
                            let relayed = storage_runtime
                                .relay_store_request(
                                    native_store,
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
                                    request_session,
                                    &capabilities,
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
                    proof.record(passed);
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
                    proof.record(passed);
                    return passed;
                }
                if value == "assert-crash-restart" {
                    let tick = interrupts::ticks();
                    let passed = service_lifecycle.failed(tick)
                        && service_lifecycle.due(tick.saturating_add(2));
                    proof.record(passed);
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
                    proof.record(passed);
                    return passed;
                }
                let proof_input = proofs::is_assertion_input(value);
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
                    proof.record(passed);
                    return passed;
                }
                let passed = value.bytes().chain(core::iter::once(b'\n')).all(|byte| {
                    logos_abi::InputEvent::from_byte(byte)
                        .is_some_and(|event| native_input.deliver(event))
                        && native_scheduler.wake(native_handle)
                        && native_scheduler.run(native_handle)
                        && storage_runtime.relay_terminal_store_requests(
                            native_store,
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
                            &session,
                            &capabilities,
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
                        && ({
                            let request = native_command.request();
                            let remote_command = request.filter(|request| {
                                matches!(
                                    request.syscall,
                                    logos_abi::Syscall::RemoteKey
                                        | logos_abi::Syscall::Enroll
                                        | logos_abi::Syscall::Unenroll
                                )
                            });
                            if let Some(request) = remote_command {
                                debug::write_line(b"LogOS: test remote local command");
                                debug::write_line(
                                    if remote_runtime
                                        .state()
                                        .is_some_and(secrets::RemoteState::available)
                                    {
                                        b"LogOS: test remote state available"
                                    } else {
                                        b"LogOS: test remote state unavailable"
                                    },
                                );
                                debug::write_line(
                                    if remote_bootstrap.is_some()
                                        && shared_pages
                                            .address(terminal_owner, shared_history)
                                            .is_some()
                                    {
                                        b"LogOS: test remote persistence inputs"
                                    } else {
                                        b"LogOS: test remote persistence inputs missing"
                                    },
                                );
                                let local = remote_runtime.local_command(
                                    request,
                                    remote_bootstrap,
                                    &mut storage_runtime,
                                    &mut block::DispatchContext {
                                        endpoint: native_storage_block,
                                        pages: &mut shared_pages,
                                        store_owner: storage_owner,
                                        store_page: storage_block_page,
                                        device: &mut block_device,
                                        memory: &mut memory,
                                    },
                                    &mut native_scheduler,
                                    shared_history,
                                    terminal_owner,
                                    interrupts::ticks(),
                                );
                                debug::write_line(if local.enrolled {
                                    b"LogOS: test remote enrollment passed"
                                } else {
                                    b"LogOS: test remote enrollment failed"
                                });
                                let gateway_ready = if local.enrolled
                                    && gateway_handle.is_none()
                                    && network_handle.is_some()
                                    && let Some(task) = native_gateway.take()
                                {
                                    gateway_handle = native_scheduler.spawn(task);
                                    if let Some(handle) = gateway_handle {
                                        native_gateway_network =
                                            native_scheduler.network_client_endpoint(handle);
                                        native_gateway_remote =
                                            native_scheduler.remote_endpoint(handle);
                                        native_gateway_store =
                                            native_scheduler.store_client_endpoint(handle);
                                        native_gateway_network.zip(gateway_page).is_none_or(
                                            |(endpoint, page)| endpoint.configure_transfer(page),
                                        ) && native_gateway_store.zip(gateway_page).is_none_or(
                                            |(endpoint, page)| endpoint.configure_transfer(page),
                                        )
                                    } else {
                                        false
                                    }
                                } else {
                                    true
                                };
                                let started = remote_runtime.started()
                                    || remote_runtime.start(
                                        network_runtime.configured(),
                                        gateway_handle,
                                        &mut native_scheduler,
                                    );
                                debug::write_line(if started {
                                    b"LogOS: test remote gateway start passed"
                                } else {
                                    b"LogOS: test remote gateway start failed"
                                });
                                let gateway_ready = gateway_ready && started;
                                if remote_runtime.started() {
                                    debug::write_line(b"LogOS: Gateway started");
                                }
                                gateway_ready
                                    && native_command.reply(&local.reply.text[..local.reply.length])
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
                            } else if request.is_none() {
                                true
                            } else {
                                let reply = sessions_runtime.relay(
                                    &mut native_scheduler,
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
                            }
                        })
                });
                if proof_input {
                    proof.record(passed);
                }
                passed
            }
            test_hooks::Action::Poll => true,
            test_hooks::Action::Query(query) => {
                matches!(query, "network/configured") && network_runtime.configured()
            }
            test_hooks::Action::Advance(ticks) => {
                for step in 0..ticks.min(4096) {
                    if !poll_network(
                        &mut network_runtime,
                        &mut native_scheduler,
                        interrupts::ticks().saturating_add(step),
                        native_terminal_network,
                        native_handle,
                        &session,
                        &capabilities,
                        &shared_pages,
                        terminal_owner,
                    ) {
                        return false;
                    }
                    assert_qemu_network_configuration(&network_runtime, &mut network_qemu_asserted);
                    if !poll_gateway!() {
                        return false;
                    }
                }
                true
            }
            test_hooks::Action::Run(id) => {
                if id == "network/tcp-stream" {
                    return run_network_tcp_stream(
                        native_terminal_network,
                        &mut network_runtime,
                        &mut native_scheduler,
                        &tcp_test_session,
                        &capabilities,
                        &shared_pages,
                        terminal_owner,
                        shared_history,
                    );
                }
                if id == "network/simultaneous-client-busy" {
                    if gateway_handle.is_none() {
                        let Some(task) = native_gateway.take() else {
                            return false;
                        };
                        gateway_handle = native_scheduler.spawn(task);
                        let Some(handle) = gateway_handle else {
                            return false;
                        };
                        native_gateway_network = native_scheduler.network_client_endpoint(handle);
                        native_gateway_remote = native_scheduler.remote_endpoint(handle);
                        native_gateway_store = native_scheduler.store_client_endpoint(handle);
                        if !native_gateway_network
                            .zip(gateway_page)
                            .is_none_or(|(endpoint, page)| endpoint.configure_transfer(page))
                            || !native_gateway_store
                                .zip(gateway_page)
                                .is_none_or(|(endpoint, page)| endpoint.configure_transfer(page))
                            || !remote_runtime.start(
                                network_runtime.configured(),
                                gateway_handle,
                                &mut native_scheduler,
                            )
                        {
                            return false;
                        }
                    }
                    let (
                        Some(gateway_client),
                        Some(gateway_task),
                        Some(gateway_session),
                        Some(gateway_owner),
                    ) = (
                        native_gateway_network,
                        gateway_handle,
                        gateway_network_session.as_ref(),
                        gateway_owner,
                    )
                    else {
                        return false;
                    };
                    let terminal_request = logos_abi::NetworkRequest {
                        id: 0x9000_0200,
                        operation: logos_abi::NetworkOperation::Status,
                        endpoint: logos_abi::NetworkEndpoint(0),
                        peer: logos_abi::NetworkScope(0),
                        page: logos_abi::PageHandle(0),
                        length: 0,
                        generation: 0,
                        deadline: u64::MAX / 2,
                    };
                    if !network_runtime.relay_client(
                        NetworkClientSlot::Gateway,
                        gateway_client,
                        gateway_task,
                        gateway_session,
                        &capabilities,
                        &shared_pages,
                        gateway_owner,
                        interrupts::ticks(),
                    ) {
                        return false;
                    }
                    let Some(busy) = run_network_request(
                        terminal_request,
                        native_terminal_network,
                        &mut network_runtime,
                        &mut native_scheduler,
                        &session,
                        &capabilities,
                        &shared_pages,
                        terminal_owner,
                    ) else {
                        return false;
                    };
                    if busy.status != logos_abi::NetworkStatus::Busy
                        || !network_runtime.invalidate_client(
                            NetworkClientSlot::Gateway,
                            logos_abi::NetworkStatus::Cancelled,
                        )
                    {
                        return false;
                    }
                    if !drain_network_wakes(&mut network_runtime, &mut native_scheduler) {
                        return false;
                    }
                    let gateway_request = logos_abi::NetworkRequest {
                        id: 0x9000_0201,
                        operation: logos_abi::NetworkOperation::Listen,
                        peer: logos_abi::NetworkScope::new(
                            logos_abi::NetworkProtocol::Tcp,
                            0,
                            logos_abi::REMOTE_TCP_PORT,
                        ),
                        ..terminal_request
                    };
                    if !gateway_client.issue(gateway_request)
                        || !network_runtime.relay_client(
                            NetworkClientSlot::Gateway,
                            gateway_client,
                            gateway_task,
                            gateway_session,
                            &capabilities,
                            &shared_pages,
                            gateway_owner,
                            interrupts::ticks(),
                        )
                    {
                        return false;
                    }
                    if !drain_network_wakes(&mut network_runtime, &mut native_scheduler) {
                        return false;
                    }
                    let passed = gateway_client
                        .response(gateway_request.id)
                        .is_some_and(|reply| reply.status == logos_abi::NetworkStatus::Busy)
                        && network_runtime.invalidate_client(
                            NetworkClientSlot::Terminal,
                            logos_abi::NetworkStatus::Cancelled,
                        )
                        && drain_network_wakes(&mut network_runtime, &mut native_scheduler);
                    return passed;
                }
                if id == "network/unauthorized-operation" {
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
                            page: shared_history,
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
                            page: shared_history,
                            length: logos_abi::MAX_NETWORK_PAYLOAD as u16,
                            generation: 1,
                            deadline: u64::MAX / 2,
                        },
                    ];
                    for request in requests {
                        let Some(reply) = run_network_request(
                            request,
                            native_terminal_network,
                            &mut network_runtime,
                            &mut native_scheduler,
                            &denied_session,
                            &capabilities,
                            &shared_pages,
                            terminal_owner,
                        ) else {
                            return false;
                        };
                        if reply.status != logos_abi::NetworkStatus::Denied
                            || reply.counters.denied != 1
                        {
                            return false;
                        }
                    }
                    return true;
                }
                if id == "network/icmp-echo" {
                    if !network_runtime.configured() {
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
                        &mut network_runtime,
                        &mut native_scheduler,
                        &session,
                        &capabilities,
                        &shared_pages,
                        terminal_owner,
                    );
                    let Some(reply) = reply else {
                        return false;
                    };
                    if reply.status != logos_abi::NetworkStatus::Complete {
                        return false;
                    }
                    return reply.source_address == 0x0a00_0202;
                }
                if id == "network/udp-round-trip" {
                    if !network_runtime.configured() {
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
                        &mut network_runtime,
                        &mut native_scheduler,
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
                        &mut network_runtime,
                        &mut native_scheduler,
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
                        &mut network_runtime,
                        &mut native_scheduler,
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
                    if !network_runtime.configured() {
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
                        &mut network_runtime,
                        &mut native_scheduler,
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
                        &mut network_runtime,
                        &mut native_scheduler,
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
                        &mut network_runtime,
                        &mut native_scheduler,
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
                    if !network_runtime.configured() {
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
                        &mut network_runtime,
                        &mut native_scheduler,
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
                        &mut network_runtime,
                        &mut native_scheduler,
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
                    if !network_runtime.has_resources() {
                        return false;
                    }
                    let tick = interrupts::ticks();
                    let request = logos_abi::NetworkDeviceRequest {
                        id: 0x9000_0141,
                        operation: logos_abi::NetworkDeviceOperation::Transmit,
                        length: logos_abi::NETWORK_MIN_FRAME as u16,
                        generation: network_runtime.device_generation() as u16,
                        deadline: tick.saturating_add(1),
                    };
                    let Some(reply) = run_network_device_request(
                        &mut network_runtime,
                        &mut native_scheduler,
                        request,
                        tick,
                    ) else {
                        return false;
                    };
                    return reply.status == logos_abi::NetworkStatus::TimedOut
                        && network_runtime.device_generation() == u32::from(reply.generation);
                }
                if id == "network/reset-reconnect" {
                    let generation = network_runtime.device_generation() as u16;
                    let request = logos_abi::NetworkDeviceRequest {
                        id: 0x9000_0152,
                        operation: logos_abi::NetworkDeviceOperation::Reset,
                        length: 0,
                        generation,
                        deadline: interrupts::ticks().saturating_add(64),
                    };
                    let old_generation = network_runtime.device_generation();
                    let Some(reply) = run_network_device_request(
                        &mut network_runtime,
                        &mut native_scheduler,
                        request,
                        interrupts::ticks(),
                    ) else {
                        return false;
                    };
                    return reply.status == logos_abi::NetworkStatus::Complete
                        && u32::from(reply.generation) != old_generation
                        && network_runtime.device_generation() == u32::from(reply.generation);
                }
                if id == "network/device-bind" {
                    let request = logos_abi::NetworkDeviceRequest {
                        id: 0x9000_0001,
                        operation: logos_abi::NetworkDeviceOperation::Info,
                        length: 0,
                        generation: 0,
                        deadline: interrupts::ticks().saturating_add(64),
                    };
                    let Some(reply) = run_network_device_request(
                        &mut network_runtime,
                        &mut native_scheduler,
                        request,
                        interrupts::ticks(),
                    ) else {
                        return false;
                    };
                    return reply.status == logos_abi::NetworkStatus::Complete
                        && u32::from(reply.info.generation) == network_runtime.device_generation()
                        && reply.info.link_up != 0;
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
                    || (id == "network/transport-dhcp" && network_qemu_asserted)
                    || (id == "network/configuration" && network_qemu_asserted)
                    || (id.starts_with("remote/") && gateway_handle.is_some())
                    || proof.passed()
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
                        sessions_runtime.bind_sessions(native_sessions_endpoint, sessions_handle);
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
                    storage_runtime.cancel_block(&mut block::DispatchContext {
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
                        storage_runtime.rebind(
                            native_storage_store,
                            native_storage_block,
                            storage_handle,
                        );
                        storage_block_page = block_page;
                        let _ = block_virtual;
                        if native_scheduler.run(restarted)
                            && storage_runtime.startup(
                                &mut block::DispatchContext {
                                    endpoint: native_storage_block,
                                    pages: &mut shared_pages,
                                    store_owner: storage_owner,
                                    store_page: storage_block_page,
                                    device: &mut block_device,
                                    memory: &mut memory,
                                },
                                &mut native_scheduler,
                            )
                        {
                            storage_runtime.reset_relay();
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
                    && let (Some(failed_network), Some(resources)) =
                        (network_handle, network_runtime.resources())
                {
                    if let Some((restarted, endpoint, resources)) = replace_network(
                        &mut native_scheduler,
                        &mut network_runtime,
                        failed_network,
                        &mut memory,
                        &mut shared_pages,
                        resources,
                    ) {
                        network_handle = Some(restarted);
                        native_network_endpoint = Some(endpoint);
                        #[cfg(feature = "test-hooks")]
                        {
                            network_qemu_asserted = false;
                        }
                        let bound = native_scheduler
                            .network_device_endpoint(restarted, network_runtime.device_generation())
                            .zip(native_scheduler.network_event_endpoint(
                                restarted,
                                network_runtime.device_generation(),
                            ))
                            .is_some_and(|(device_endpoint, event_endpoint)| {
                                native_scheduler.network_server_endpoint(restarted).is_some_and(
                                    |server_endpoint| {
                                        network_runtime.bind(
                                            restarted,
                                            server_endpoint,
                                            device_endpoint,
                                            event_endpoint,
                                            resources,
                                        )
                                    },
                                )
                            });
                        if bound && native_scheduler.run(restarted) {
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
                if !storage_runtime.poll_block(
                    &mut block::DispatchContext {
                        endpoint: native_storage_block,
                        pages: &mut shared_pages,
                        store_owner: storage_owner,
                        store_page: storage_block_page,
                        device: &mut block_device,
                        memory: &mut memory,
                    },
                    &mut native_scheduler,
                    tick,
                ) {
                    debug::write_line(b"LogOS: storage block reply failed");
                }
                if !poll_network(
                    &mut network_runtime,
                    &mut native_scheduler,
                    tick,
                    native_terminal_network,
                    native_handle,
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
                #[cfg(feature = "test-hooks")]
                assert_qemu_network_configuration(&network_runtime, &mut network_qemu_asserted);
                if !poll_gateway!() {
                    debug::write_line(b"LogOS: Gateway service unavailable");
                    if gateway_handle.is_some() {
                        let _ = native_services.failed(supervisor::NativeService::Gateway, tick);
                    }
                }
                if native_services.due(supervisor::NativeService::Gateway, tick)
                    && let Some(failed) = gateway_handle
                    && let Some((restarted, page)) = replace_gateway(
                        &mut native_scheduler,
                        &mut network_runtime,
                        failed,
                        &mut memory,
                        &mut shared_pages,
                        gateway_owner,
                        gateway_page,
                    )
                {
                    gateway_handle = Some(restarted);
                    gateway_page = Some(page);
                    native_gateway_network = native_scheduler.network_client_endpoint(restarted);
                    native_gateway_remote = native_scheduler.remote_endpoint(restarted);
                    remote_runtime.reset_transport();
                    if native_gateway_network
                        .is_none_or(|endpoint| endpoint.configure_transfer(page))
                        && native_scheduler.run(restarted)
                    {
                        debug::write_line(b"LogOS: Gateway restarted");
                    } else {
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
                                if !storage_runtime.cancel_store_transaction(&mut native_scheduler)
                                {
                                    console_mode = mode::ConsoleMode::Recovery;
                                    break;
                                }
                                if let Some((restarted, endpoints, history)) = replace_terminal(
                                    &mut native_scheduler,
                                    &mut network_runtime,
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
                                    storage_runtime.rebind_client(native_store);
                                    sessions_runtime.bind_terminal(native_command);
                                    shared_history = history;
                                    storage_runtime.reset_relay();
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
                            && (!storage_runtime.relay_terminal_store_requests(
                                native_store,
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
                                &session,
                                &capabilities,
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
                            let local = remote_runtime.local_command(
                                request,
                                remote_bootstrap,
                                &mut storage_runtime,
                                &mut block::DispatchContext {
                                    endpoint: native_storage_block,
                                    pages: &mut shared_pages,
                                    store_owner: storage_owner,
                                    store_page: storage_block_page,
                                    device: &mut block_device,
                                    memory: &mut memory,
                                },
                                &mut native_scheduler,
                                shared_history,
                                terminal_owner,
                                tick,
                            );
                            if local.enrolled
                                && gateway_handle.is_none()
                                && network_handle.is_some()
                                && let Some(task) = native_gateway.take()
                            {
                                gateway_handle = native_scheduler.spawn(task);
                                if let Some(handle) = gateway_handle {
                                    native_gateway_network =
                                        native_scheduler.network_client_endpoint(handle);
                                    native_gateway_remote =
                                        native_scheduler.remote_endpoint(handle);
                                    native_gateway_store =
                                        native_scheduler.store_client_endpoint(handle);
                                    if !native_gateway_network.zip(gateway_page).is_none_or(
                                        |(endpoint, page)| endpoint.configure_transfer(page),
                                    ) || !native_gateway_store.zip(gateway_page).is_none_or(
                                        |(endpoint, page)| endpoint.configure_transfer(page),
                                    ) {
                                        gateway_handle = None;
                                    }
                                }
                                if gateway_handle.is_some() {
                                    native_services.ready(supervisor::NativeService::Gateway);
                                }
                            }
                            if !native_command.reply(&local.reply.text[..local.reply.length])
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
                            let mut relay = sessions_runtime.relay(
                                &mut native_scheduler,
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
                        sessions_runtime.bind_sessions(native_sessions_endpoint, sessions_handle);
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
                    if storage_runtime.cancel_store_transaction(&mut native_scheduler)
                        && let Some((restarted, endpoints, history)) = replace_terminal(
                            &mut native_scheduler,
                            &mut network_runtime,
                            native_handle,
                            storage_handle,
                            &mut memory,
                            &mut shared_pages,
                            terminal_owner,
                            storage_owner,
                            shared_history,
                        )
                    {
                        native_handle = restarted;
                        (
                            native_input,
                            native_command,
                            native_display,
                            native_store,
                            native_terminal_network,
                        ) = endpoints;
                        storage_runtime.rebind_client(native_store);
                        sessions_runtime.bind_terminal(native_command);
                        shared_history = history;
                        storage_runtime.reset_relay();
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
                let _ = storage_runtime.poll_block(
                    &mut block::DispatchContext {
                        endpoint: native_storage_block,
                        pages: &mut shared_pages,
                        store_owner: storage_owner,
                        store_page: storage_block_page,
                        device: &mut block_device,
                        memory: &mut memory,
                    },
                    &mut native_scheduler,
                    tick,
                );
                let _ = poll_network(
                    &mut network_runtime,
                    &mut native_scheduler,
                    tick,
                    native_terminal_network,
                    native_handle,
                    &session,
                    &capabilities,
                    &shared_pages,
                    terminal_owner,
                );
                #[cfg(feature = "test-hooks")]
                assert_qemu_network_configuration(&network_runtime, &mut network_qemu_asserted);
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

#[cfg_attr(not(feature = "test-hooks"), allow(dead_code))]
fn run_network_device_request(
    runtime: &mut network::NetworkRuntime,
    scheduler: &mut native_task::Scheduler<'_>,
    request: logos_abi::NetworkDeviceRequest,
    tick: u64,
) -> Option<logos_abi::NetworkDeviceReply> {
    for step in 0..16 {
        if runtime.device_endpoint().pending() && !runtime.poll(tick.saturating_add(step)) {
            return None;
        }
        if !drain_network_wakes(runtime, scheduler) {
            return None;
        }
        if !runtime.device_endpoint().pending() {
            break;
        }
    }
    if !runtime.device_endpoint().issue(request) {
        return None;
    }
    for step in 0..256 {
        if !runtime.poll_device_proof(tick.saturating_add(step)) {
            return None;
        }
        if let Some(reply) = runtime.device_endpoint().response(request.id) {
            return Some(reply);
        }
    }
    None
}

#[allow(clippy::too_many_arguments)]
fn drain_network_wakes(
    runtime: &mut network::NetworkRuntime,
    scheduler: &mut native_task::Scheduler<'_>,
) -> bool {
    while let Some(handle) = runtime.take_wake() {
        if scheduler.failed(handle) || !scheduler.wake(handle) || !scheduler.run(handle) {
            return false;
        }
    }
    true
}

#[allow(clippy::too_many_arguments)]
fn poll_network(
    runtime: &mut network::NetworkRuntime,
    scheduler: &mut native_task::Scheduler<'_>,
    tick: u64,
    terminal: native_task::NetworkClientEndpoint,
    terminal_handle: native_task::Handle,
    session: &session::Context,
    capabilities: &capabilities::CapabilityManager,
    shared_pages: &logos_core::shared_pages::SharedPages,
    terminal_owner: u64,
) -> bool {
    if runtime.task().is_none() {
        return true;
    }
    if !runtime.poll(tick) || !drain_network_wakes(runtime, scheduler) {
        return false;
    }
    if !runtime.relay_client(
        NetworkClientSlot::Terminal,
        terminal,
        terminal_handle,
        session,
        capabilities,
        shared_pages,
        terminal_owner,
        tick,
    ) {
        return false;
    }
    if !drain_network_wakes(runtime, scheduler) || !runtime.poll(tick) {
        return false;
    }
    if !drain_network_wakes(runtime, scheduler) {
        return false;
    }
    if !runtime.relay_client(
        NetworkClientSlot::Terminal,
        terminal,
        terminal_handle,
        session,
        capabilities,
        shared_pages,
        terminal_owner,
        tick,
    ) {
        return false;
    }
    if !drain_network_wakes(runtime, scheduler) {
        return false;
    }
    true
}

#[cfg(feature = "test-hooks")]
fn assert_qemu_network_configuration(runtime: &network::NetworkRuntime, asserted: &mut bool) {
    if *asserted {
        return;
    }
    let Some(info) = runtime.info() else { return };
    if info.configuration == 1
        && info.ipv4 == u32::from_be_bytes([10, 0, 2, 15])
        && info.subnet_mask == u32::from_be_bytes([255, 255, 255, 0])
        && info.router == u32::from_be_bytes([10, 0, 2, 2])
    {
        debug::write_line(
            b"LOGOS/1 NETWORK transport-dhcp status=bound ipv4=10.0.2.15 mask=255.255.255.0 router=10.0.2.2",
        );
        *asserted = true;
    }
}

#[cfg(feature = "test-hooks")]
#[allow(clippy::too_many_arguments)]
fn run_network_request(
    request: logos_abi::NetworkRequest,
    terminal: native_task::NetworkClientEndpoint,
    runtime: &mut network::NetworkRuntime,
    scheduler: &mut native_task::Scheduler<'_>,
    session: &session::Context,
    capabilities: &capabilities::CapabilityManager,
    shared_pages: &logos_core::shared_pages::SharedPages,
    terminal_owner: u64,
) -> Option<logos_abi::NetworkReply> {
    if !terminal.issue(request) {
        return None;
    }
    runtime.task()?;
    for _ in 0..1_000_000 {
        let tick = interrupts::ticks();
        if !runtime.poll(tick) {
            return None;
        }
        if !drain_network_wakes(runtime, scheduler) {
            return None;
        }
        if !runtime.relay_probe(terminal, session, capabilities, shared_pages, terminal_owner, tick)
        {
            return None;
        }
        if !drain_network_wakes(runtime, scheduler) || !runtime.poll(tick) {
            return None;
        }
        if !runtime.relay_probe(terminal, session, capabilities, shared_pages, terminal_owner, tick)
            || !drain_network_wakes(runtime, scheduler)
        {
            return None;
        }
        if let Some(reply) = terminal.response(request.id) {
            return Some(reply);
        }
    }
    None
}

#[cfg(feature = "test-hooks")]
#[allow(clippy::too_many_arguments)]
fn run_network_tcp_stream(
    client: native_task::NetworkClientEndpoint,
    runtime: &mut network::NetworkRuntime,
    scheduler: &mut native_task::Scheduler<'_>,
    session: &session::Context,
    capabilities: &capabilities::CapabilityManager,
    shared_pages: &logos_core::shared_pages::SharedPages,
    owner: u64,
    page: logos_abi::PageHandle,
) -> bool {
    const ID: &str = "network/tcp-stream";
    const DEADLINE: u64 = u64::MAX / 2;
    const PORT: u16 = logos_abi::REMOTE_TCP_PORT;
    let scope = logos_abi::NetworkScope::new(logos_abi::NetworkProtocol::Tcp, 0, PORT);
    let request = |id, operation, endpoint, page, length, generation| logos_abi::NetworkRequest {
        id,
        operation,
        endpoint,
        peer: logos_abi::NetworkScope::new(logos_abi::NetworkProtocol::Tcp, 0, PORT),
        page,
        length,
        generation,
        deadline: DEADLINE,
    };
    test_hooks::event(ID, "starting");
    if !runtime.has_device() {
        test_hooks::event(ID, "network_device_unavailable");
        return false;
    }
    if runtime.resources().is_none() {
        test_hooks::event(ID, "network_resources_unavailable");
        return false;
    }
    if runtime.task().is_none() {
        test_hooks::event(ID, "network_unavailable");
        return false;
    }

    let listen = request(
        0x9000_0300,
        logos_abi::NetworkOperation::Listen,
        logos_abi::NetworkEndpoint(0),
        logos_abi::PageHandle(0),
        0,
        0,
    );
    let Some(listen_reply) = run_network_request(
        listen,
        client,
        runtime,
        scheduler,
        session,
        capabilities,
        shared_pages,
        owner,
    ) else {
        test_hooks::event(ID, "listener_failed");
        return false;
    };
    if listen_reply.status != logos_abi::NetworkStatus::Complete {
        test_hooks::event(ID, network_status_label(listen_reply.status));
        return false;
    }
    if !listen_reply.endpoint.valid() || listen_reply.generation == 0 {
        test_hooks::event(ID, "listener_shape_invalid");
        return false;
    }
    test_hooks::event(ID, "listener_waiting");

    let accept = request(
        0x9000_0301,
        logos_abi::NetworkOperation::Accept,
        listen_reply.endpoint,
        logos_abi::PageHandle(0),
        0,
        0,
    );
    let Some(accept_reply) = run_network_request(
        accept,
        client,
        runtime,
        scheduler,
        session,
        capabilities,
        shared_pages,
        owner,
    ) else {
        test_hooks::event(ID, "accept_failed");
        return false;
    };
    if accept_reply.status != logos_abi::NetworkStatus::Complete
        || !accept_reply.endpoint.valid()
        || accept_reply.generation != listen_reply.generation
        || accept_reply.source_address == 0
        || accept_reply.source_port == 0
    {
        test_hooks::event(ID, network_status_label(accept_reply.status));
        return false;
    }
    test_hooks::event(ID, "connection_established");

    let address = match shared_pages.address(owner, page) {
        Some(address) => address,
        None => return false,
    };
    let read = request(
        0x9000_0302,
        logos_abi::NetworkOperation::Read,
        accept_reply.endpoint,
        page,
        logos_abi::MAX_TCP_PAYLOAD as u16,
        accept_reply.generation,
    );
    let Some(read_reply) = run_network_request(
        read,
        client,
        runtime,
        scheduler,
        session,
        capabilities,
        shared_pages,
        owner,
    ) else {
        return false;
    };
    let hello =
        unsafe { core::slice::from_raw_parts(address as *const u8, read_reply.length as usize) };
    if read_reply.status != logos_abi::NetworkStatus::Complete || hello != b"hello" {
        return false;
    }
    test_hooks::event(ID, "connection_readable");

    unsafe {
        core::ptr::copy_nonoverlapping(b"world".as_ptr(), address as *mut u8, 5);
    }
    test_hooks::event(ID, "write_pending");
    let write = request(
        0x9000_0303,
        logos_abi::NetworkOperation::Write,
        accept_reply.endpoint,
        page,
        5,
        accept_reply.generation,
    );
    let Some(write_reply) = run_network_request(
        write,
        client,
        runtime,
        scheduler,
        session,
        capabilities,
        shared_pages,
        owner,
    ) else {
        return false;
    };
    if write_reply.status != logos_abi::NetworkStatus::Complete
        || write_reply.endpoint != accept_reply.endpoint
    {
        return false;
    }

    let mut expected = [0; logos_abi::MAX_TCP_PAYLOAD];
    for (index, byte) in expected.iter_mut().take(512).enumerate() {
        *byte = (index as u8).wrapping_mul(37).wrapping_add(11);
    }
    let large_read = request(
        0x9000_0304,
        logos_abi::NetworkOperation::Read,
        accept_reply.endpoint,
        page,
        logos_abi::MAX_TCP_PAYLOAD as u16,
        accept_reply.generation,
    );
    let Some(large_read_reply) = run_network_request(
        large_read,
        client,
        runtime,
        scheduler,
        session,
        capabilities,
        shared_pages,
        owner,
    ) else {
        return false;
    };
    let received = unsafe {
        core::slice::from_raw_parts(address as *const u8, large_read_reply.length as usize)
    };
    if large_read_reply.status != logos_abi::NetworkStatus::Complete || received != &expected[..512]
    {
        return false;
    }
    test_hooks::event(ID, "write_acknowledged");

    unsafe {
        for byte in core::slice::from_raw_parts_mut(address as *mut u8, 512) {
            *byte ^= 0xa5;
        }
    }
    let write_large = request(
        0x9000_0305,
        logos_abi::NetworkOperation::Write,
        accept_reply.endpoint,
        page,
        512,
        accept_reply.generation,
    );
    let Some(write_large_reply) = run_network_request(
        write_large,
        client,
        runtime,
        scheduler,
        session,
        capabilities,
        shared_pages,
        owner,
    ) else {
        test_hooks::event(ID, "large_write_failed");
        return false;
    };
    if write_large_reply.status != logos_abi::NetworkStatus::Complete {
        test_hooks::event(ID, network_status_label(write_large_reply.status));
        return false;
    }

    test_hooks::event(ID, "connection_closed");
    scope.valid()
}

#[cfg(feature = "test-hooks")]
const fn network_status_label(status: logos_abi::NetworkStatus) -> &'static str {
    match status {
        logos_abi::NetworkStatus::Complete => "complete",
        logos_abi::NetworkStatus::Denied => "denied",
        logos_abi::NetworkStatus::Invalid => "invalid",
        logos_abi::NetworkStatus::Busy => "busy",
        logos_abi::NetworkStatus::Full => "full",
        logos_abi::NetworkStatus::Offline => "offline",
        logos_abi::NetworkStatus::NoRoute => "no_route",
        logos_abi::NetworkStatus::AddressInUse => "address_in_use",
        logos_abi::NetworkStatus::MessageTooLarge => "message_too_large",
        logos_abi::NetworkStatus::TimedOut => "timed_out",
        logos_abi::NetworkStatus::Cancelled => "cancelled",
        logos_abi::NetworkStatus::Reset => "reset",
        logos_abi::NetworkStatus::Io => "io",
    }
}

#[allow(clippy::too_many_arguments)]
fn poll_gateway(
    runtime: &mut network::NetworkRuntime,
    client: Option<native_task::NetworkClientEndpoint>,
    remote: Option<native_task::RemoteEndpoint>,
    gateway_handle: Option<native_task::Handle>,
    gateway_session: Option<&session::Context>,
    gateway_owner: Option<u64>,
    network_service: Option<native_task::NetworkEndpoint>,
    network_handle: Option<native_task::Handle>,
    scheduler: &mut native_task::Scheduler<'_>,
    capabilities: &capabilities::CapabilityManager,
    gateway_page: Option<logos_abi::PageHandle>,
    persistence_page: logos_abi::PageHandle,
    persistence_owner: u64,
    remote_runtime: &mut remote::RemoteRuntime,
    sessions: Option<native_task::SessionEndpoint>,
    sessions_handle: Option<native_task::Handle>,
    remote_session: Option<&session::Context>,
    storage_runtime: &mut storage::StorageRuntime,
    block_context: &mut block::DispatchContext<'_>,
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
        Some(_network_service),
        Some(_network_handle),
        Some(gateway_session),
        Some(owner),
    ) = (client, gateway_handle, network_service, network_handle, gateway_session, gateway_owner)
    else {
        return true;
    };
    if !runtime.relay_client(
        NetworkClientSlot::Gateway,
        client,
        handle,
        gateway_session,
        capabilities,
        block_context.pages,
        owner,
        tick,
    ) {
        return false;
    }
    if !drain_network_wakes(runtime, scheduler) {
        return false;
    }
    let (Some(remote), Some(page), Some(remote_session)) = (remote, gateway_page, remote_session)
    else {
        return true;
    };
    let Some(request) = remote.request() else { return true };
    let Some(address) = block_context.pages.address(owner, page).filter(|_| request.page == page)
    else {
        return remote.reply(logos_abi::service::RemotePageReply {
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
    let outcome = remote_runtime
        .handle_request(|remote_state| match request.operation {
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
                            core::ptr::copy_nonoverlapping(
                                output.as_ptr(),
                                address as *mut u8,
                                sealed,
                            )
                        };
                        (logos_abi::service::RemoteGateStatus::Complete, sealed, 0)
                    })
            }
            logos_abi::service::RemoteGateOperation::Invoke => remote_invoke(
                remote_state,
                &source[..length],
                address,
                storage_runtime,
                block_context,
                scheduler,
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
                remote_subscription(remote_state, &source[..length], address, request.operation)
            }
            logos_abi::service::RemoteGateOperation::Reset => {
                remote_state.reset_transport();
                Some((logos_abi::service::RemoteGateStatus::Complete, 0, 0))
            }
        })
        .flatten();
    let (status, length, cursor) =
        outcome.unwrap_or((logos_abi::service::RemoteGateStatus::Denied, 0, 0));
    remote.reply(logos_abi::service::RemotePageReply {
        id: request.id,
        status,
        length: length as u16,
        cursor,
    }) && scheduler.wake(handle)
        && scheduler.run(handle)
}

fn remote_subscription(
    state: &mut secrets::RemoteState,
    input: &[u8],
    address: u64,
    operation: logos_abi::service::RemoteGateOperation,
) -> Option<(logos_abi::service::RemoteGateStatus, usize, u64)> {
    let message = if input.is_empty() {
        None
    } else {
        Some(logos_remote::RemoteMessage::decode(input).ok()?)
    };
    let mut subscription = state.subscription();
    if operation == logos_abi::service::RemoteGateOperation::Subscribe {
        let message = message?;
        if message.kind != logos_remote::RemoteMessageKind::Subscribe {
            return None;
        }
        let source = if message.payload[..usize::from(message.payload_length)]
            == *logos_remote::REMOTE_SUBSCRIBE_TRACE
        {
            1
        } else if message.payload[..usize::from(message.payload_length)]
            == *logos_remote::REMOTE_SUBSCRIBE_LOG
        {
            2
        } else {
            return None;
        };
        subscription = secrets::RemoteSubscription {
            attachment: message.id,
            source,
            cursor: message.cursor,
            credits: 0,
            in_flight: 0,
        };
        state.replace_subscription(subscription);
        return remote_simple_reply(input, address, b"subscribed");
    }
    if let Some(message) = message {
        if message.kind == logos_remote::RemoteMessageKind::Cancel {
            state.replace_subscription(secrets::RemoteSubscription::empty());
            return remote_simple_reply(input, address, b"unfollowed");
        }
        if message.kind != logos_remote::RemoteMessageKind::Credit
            || operation != logos_abi::service::RemoteGateOperation::Credit
        {
            return None;
        }
        let credit = message.cursor;
        if credit == 0
            || credit > logos_remote::REMOTE_EVENT_CREDIT
            || u16::from(subscription.credits) + credit as u16
                > logos_remote::REMOTE_EVENT_CREDIT as u16
        {
            return None;
        }
        subscription.credits = subscription.credits.saturating_add(credit as u8);
    } else if operation != logos_abi::service::RemoteGateOperation::Acknowledge {
        return None;
    } else if subscription.in_flight != 0 {
        subscription.cursor = subscription.in_flight;
        subscription.in_flight = 0;
        subscription.credits = subscription.credits.saturating_sub(1);
    }
    state.replace_subscription(subscription);
    let mut payload = [0; logos_remote::REMOTE_MESSAGE_PAYLOAD];
    let (cursor, length, gap) = if subscription.source == 1 {
        let mut output = [0; 160];
        let (cursor, length, gap) = crate::platform::trace::next(subscription.cursor, &mut output);
        if gap {
            payload[..9].copy_from_slice(b"gap trace");
            (cursor, 9, true)
        } else {
            payload[..length].copy_from_slice(&output[..length]);
            (cursor, length, false)
        }
    } else if subscription.source == 2 {
        let mut output = [0; 160];
        let (cursor, length, gap) = crate::debug::since(subscription.cursor, &mut output);
        if gap {
            payload[..7].copy_from_slice(b"gap log");
            (cursor, 7, true)
        } else {
            payload[..length].copy_from_slice(&output[..length]);
            (cursor, length, false)
        }
    } else {
        return None;
    };
    let mut subscription = state.subscription();
    if subscription.credits == 0 || subscription.in_flight != 0 || length == 0 {
        return Some((logos_abi::service::RemoteGateStatus::Complete, 0, 0));
    }
    subscription.in_flight = cursor;
    state.replace_subscription(subscription);
    let reply = logos_remote::RemoteMessage {
        kind: logos_remote::RemoteMessageKind::Event,
        id: subscription.attachment,
        sequence: 0,
        cursor,
        payload,
        payload_length: length as u16,
    };
    let mut encoded = [0; logos_remote::MAX_FRAME];
    let encoded_length = reply.encode(&mut encoded).ok()?;
    unsafe { core::ptr::copy_nonoverlapping(encoded.as_ptr(), address as *mut u8, encoded_length) };
    let _ = gap;
    Some((logos_abi::service::RemoteGateStatus::Complete, encoded_length, cursor))
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
    storage_runtime: &mut storage::StorageRuntime,
    block_context: &mut block::DispatchContext<'_>,
    scheduler: &mut native_task::Scheduler<'_>,
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
    if !storage_runtime.persist_remote_control(state, block_context, scheduler, page, owner, tick) {
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
    if !storage_runtime.persist_remote_control(state, block_context, scheduler, page, owner, tick) {
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
            || !endpoint
                .page()
                .is_some_and(|page| native_display::handle(page, endpoint.generation()))
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
        if !endpoint.page().is_some_and(|page| native_display::handle(page, endpoint.generation()))
            || !scheduler.wake(handle)
            || !scheduler.run(handle)
        {
            return false;
        }
    }
    true
}

type TerminalEndpoints = (
    native_task::InputEndpoint,
    native_task::SyscallEndpoint,
    native_task::DisplayEndpoint,
    native_task::StoreClientEndpoint,
    native_task::NetworkClientEndpoint,
);

fn terminal_endpoints(
    scheduler: &native_task::Scheduler<'_>,
    handle: native_task::Handle,
) -> Option<TerminalEndpoints> {
    Some((
        scheduler.input_endpoint(handle)?,
        scheduler.syscall_endpoint(handle)?,
        scheduler.display_endpoint(handle)?,
        scheduler.store_client_endpoint(handle)?,
        scheduler.network_client_endpoint(handle)?,
    ))
}

#[allow(clippy::too_many_arguments)]
fn replace_terminal(
    scheduler: &mut native_task::Scheduler<'_>,
    network_runtime: &mut network::NetworkRuntime,
    handle: native_task::Handle,
    storage_handle: native_task::Handle,
    memory: &mut memory::PhysicalMemory,
    pages: &mut logos_core::shared_pages::SharedPages,
    terminal_owner: u64,
    _storage_owner: u64,
    history: logos_abi::PageHandle,
) -> Option<(native_task::Handle, TerminalEndpoints, logos_abi::PageHandle)> {
    if !network_runtime
        .invalidate_client(NetworkClientSlot::Terminal, logos_abi::NetworkStatus::Cancelled)
    {
        return None;
    }
    if !drain_network_wakes(network_runtime, scheduler) {
        return None;
    }
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
    if !endpoints.3.configure_transfer(new_history) {
        return None;
    }
    if !endpoints.4.configure_transfer(new_history) {
        return None;
    }
    if storage_handle.available() {
        if !scheduler.task_mut(storage_handle)?.remap_shared_borrowed(address) {
            debug::write_line(b"LogOS: terminal replacement Store remap failed");
            return None;
        }
        let storage = scheduler.store_server_endpoint(storage_handle)?;
        if !storage.configure_transfer(new_history) {
            debug::write_line(b"LogOS: terminal replacement Store configure failed");
            return None;
        }
    }
    Some((replacement, endpoints, new_history))
}

type StorageReplacement = (
    native_task::Handle,
    native_task::StoreServerEndpoint,
    native_task::BlockClientEndpoint,
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
    let store = scheduler.store_server_endpoint(replacement)?;
    let block = scheduler.block_client_endpoint(replacement)?;
    if !store.configure_transfer(history) || !block.configure_transfer(block_page) {
        return None;
    }
    Some((replacement, store, block, block_page, block_virtual))
}

fn replace_network(
    scheduler: &mut native_task::Scheduler<'_>,
    network_runtime: &mut network::NetworkRuntime,
    handle: native_task::Handle,
    memory: &mut memory::PhysicalMemory,
    pages: &mut logos_core::shared_pages::SharedPages,
    previous: NetworkResources,
) -> Option<(native_task::Handle, native_task::NetworkEndpoint, NetworkResources)> {
    if !network_runtime.invalidate_active(logos_abi::NetworkStatus::Reset) {
        return None;
    }
    if !drain_network_wakes(network_runtime, scheduler) {
        return None;
    }
    if !scheduler.failed(handle) && !scheduler.fail(handle) {
        return None;
    }
    let mut mapped = None;
    let replacement = scheduler.replace(handle, memory, |task, memory| {
        mapped = task.map_network_owned(memory);
        mapped.is_some()
    })?;
    let ((rx_physical, _rx_virtual), (tx_physical, _tx_virtual)) = mapped?;
    pages.release(previous.owner, previous.rx)?;
    pages.release(previous.owner, previous.tx)?;
    let rx = pages.register(previous.owner, rx_physical, 2)?;
    let tx = pages.register(previous.owner, tx_physical, 2)?;
    let resources = NetworkResources {
        owner: previous.owner,
        rx,
        rx_virtual: rx_physical,
        tx,
        tx_virtual: tx_physical,
    };
    let endpoint = scheduler.network_endpoint(replacement)?;
    Some((replacement, endpoint, resources))
}

fn replace_gateway(
    scheduler: &mut native_task::Scheduler<'_>,
    network_runtime: &mut network::NetworkRuntime,
    handle: native_task::Handle,
    memory: &mut memory::PhysicalMemory,
    pages: &mut logos_core::shared_pages::SharedPages,
    owner: Option<u64>,
    previous: Option<logos_abi::PageHandle>,
) -> Option<(native_task::Handle, logos_abi::PageHandle)> {
    if !network_runtime
        .invalidate_client(NetworkClientSlot::Gateway, logos_abi::NetworkStatus::Cancelled)
    {
        return None;
    }
    if !drain_network_wakes(network_runtime, scheduler) {
        return None;
    }
    let owner = owner?;
    let previous = previous?;
    if !scheduler.failed(handle) && !scheduler.fail(handle) {
        return None;
    }
    let mut address = None;
    let replacement = scheduler.replace(handle, memory, |task, memory| {
        address = task.map_shared_owned(memory);
        address.is_some()
    })?;
    pages.release(owner, previous)?;
    let page = pages.register(owner, address?, 1)?;
    Some((replacement, page))
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
