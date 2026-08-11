use crate::drivers::{block as block_driver, supervisor};
use crate::ipc::{self, effects};
use crate::mm::memory;
use crate::platform::{block, remote, secrets, services, session, storage};
use crate::sched::{native_task, scheduler};

use logos_core::capabilities;
use logos_terminal::input;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RemoteLoadPhase {
    Disabled,
    EnrollmentPending,
    Enrollment,
    Control,
    Ready,
    Failed,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn drive_sessions_relay<'task>(
    runtime: &mut session::SessionsRuntime,
    scheduler: &mut native_task::Scheduler<'_>,
    request_session: &session::Context,
    capabilities: &capabilities::CapabilityManager,
    tick: u64,
    input: &mut input::Service,
    lifecycle: &mut supervisor::Lifecycle,
    service_healthy: bool,
    channel: &ipc::Channel,
    responses: &ipc::Channel,
    service_scheduler: &mut scheduler::Scheduler<'task>,
    service_capability: capabilities::Capability,
    service: services::ServiceHandle,
) -> session::Relay {
    for _ in 0..4 {
        let relay = runtime.relay(effects::Context {
            session: request_session,
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
        });
        let session::Relay::Runnable(handle) = relay else { return relay };
        if !scheduler.wake(handle) || !scheduler.run(handle) {
            return session::Relay::Handled(false);
        }
    }
    session::Relay::Handled(false)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn poll_remote_load(
    phase: &mut RemoteLoadPhase,
    storage_runtime: &mut storage::StorageRuntime,
    remote_runtime: &mut remote::RemoteRuntime,
    remote_bootstrap: Option<logos_remote::Bootstrap>,
    enrollment_blob: &mut [u8; logos_remote::ENROLLMENT_BLOB_BYTES],
    control_blob: &mut [u8; logos_remote::REMOTE_CONTROL_BLOB_BYTES],
    native_storage_block: native_task::BlockClientEndpoint,
    terminal_handle: native_task::Handle,
    terminal_owner: u64,
    storage_owner: u64,
    shared_history: logos_abi::PageHandle,
    storage_block_page: logos_abi::PageHandle,
    shared_pages: &mut logos_core::shared_pages::SharedPages,
    block_device: &mut block_driver::Device,
    memory: &mut memory::PhysicalMemory,
    scheduler: &mut native_task::Scheduler<'_>,
    tick: u64,
) -> bool {
    if *phase == RemoteLoadPhase::EnrollmentPending {
        if storage_runtime.operation().is_some()
            || !scheduler.waiting_for_operation(terminal_handle, logos_abi::service::READ_INPUT)
        {
            return true;
        }
        let Some(page_address) = shared_pages.address(terminal_owner, shared_history) else {
            *phase = RemoteLoadPhase::Failed;
            return true;
        };
        if storage_runtime.begin_protected_read(
            shared_history,
            page_address,
            logos_abi::TRUST_NAMESPACE,
            logos_abi::TRUST_ENROLLMENT_NAME,
            scheduler,
            tick,
        ) {
            *phase = RemoteLoadPhase::Enrollment;
        } else {
            *phase = RemoteLoadPhase::Failed;
        }
        return true;
    }
    if !matches!(*phase, RemoteLoadPhase::Enrollment | RemoteLoadPhase::Control) {
        return true;
    }
    let poll = storage_runtime.poll_protected_read(
        &mut block::DispatchContext {
            endpoint: native_storage_block,
            pages: shared_pages,
            store_owner: storage_owner,
            store_page: storage_block_page,
            device: block_device,
            memory,
        },
        scheduler,
        if *phase == RemoteLoadPhase::Enrollment {
            &mut enrollment_blob[..]
        } else {
            &mut control_blob[..]
        },
        tick,
    );
    let storage::ProtectedReadPoll::Ready(status, _) = poll else {
        if matches!(poll, storage::ProtectedReadPoll::Failed) {
            if let Some(bootstrap) = remote_bootstrap {
                remote_runtime.replace_state(secrets::RemoteState::unavailable(bootstrap));
            }
            *phase = RemoteLoadPhase::Failed;
        }
        return true;
    };
    match *phase {
        RemoteLoadPhase::Enrollment => {
            if status == logos_abi::PersistenceStatus::Complete {
                if let Some(bootstrap) = remote_bootstrap {
                    remote_runtime.replace_state(secrets::RemoteState::load_enrollment(
                        bootstrap,
                        enrollment_blob,
                    ));
                }
                if remote_runtime.state().is_some_and(secrets::RemoteState::available) {
                    let started = shared_pages.address(terminal_owner, shared_history).is_some_and(
                        |page_address| {
                            storage_runtime.begin_protected_read(
                                shared_history,
                                page_address,
                                logos_abi::TRUST_NAMESPACE,
                                logos_abi::TRUST_SESSION_NAME,
                                scheduler,
                                tick,
                            )
                        },
                    );
                    *phase =
                        if started { RemoteLoadPhase::Control } else { RemoteLoadPhase::Ready };
                } else {
                    *phase = RemoteLoadPhase::Failed;
                }
            } else if status == logos_abi::PersistenceStatus::NotFound {
                *phase = RemoteLoadPhase::Ready;
            } else {
                if let Some(bootstrap) = remote_bootstrap {
                    remote_runtime.replace_state(secrets::RemoteState::unavailable(bootstrap));
                }
                *phase = RemoteLoadPhase::Failed;
            }
        }
        RemoteLoadPhase::Control => {
            if status == logos_abi::PersistenceStatus::Complete {
                if remote_runtime.load_control(control_blob) {
                    *phase = RemoteLoadPhase::Ready;
                } else {
                    remote_runtime.disable();
                    *phase = RemoteLoadPhase::Failed;
                }
            } else if status == logos_abi::PersistenceStatus::NotFound {
                *phase = RemoteLoadPhase::Ready;
            } else {
                remote_runtime.disable();
                *phase = RemoteLoadPhase::Failed;
            }
        }
        _ => {}
    }
    true
}
