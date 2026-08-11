#[cfg(feature = "test-hooks")]
use crate::debug;
use crate::platform::{network, session};
use crate::sched::native_task;

use logos_core::capabilities;
use network::NetworkClientSlot;

#[cfg_attr(not(feature = "test-hooks"), allow(dead_code))]
pub(super) fn run_network_device_request(
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

pub(super) fn drain_network_wakes(
    runtime: &mut network::NetworkRuntime,
    scheduler: &mut native_task::Scheduler<'_>,
) -> bool {
    while let Some(handle) = runtime.take_wake() {
        if scheduler.failed(handle) {
            return false;
        }
        if !scheduler.wake(handle) || !scheduler.run(handle) {
            return false;
        }
    }
    true
}

#[allow(clippy::too_many_arguments)]
pub(super) fn poll_network(
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
pub(super) fn assert_qemu_network_configuration(
    runtime: &network::NetworkRuntime,
    asserted: &mut bool,
) {
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
