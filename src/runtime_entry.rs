use crate::{
    arch_fatal, current_ticks,
    health::{CommandError, HealthCommand, HealthError, HealthResponse, HealthService},
    runtime::{CommandError as RuntimeCommandError, Runtime, RuntimeCommand, RuntimeResponse},
    sleep_current_for,
};

#[cfg(feature = "qemu-proof")]
use crate::proof;

pub(crate) fn run() {
    crate::start_services();
    #[cfg(feature = "qemu-proof")]
    proof::verify_service_manager_boundary();
    #[cfg(feature = "qemu-proof")]
    if !crate::arch::dynamic_ipc_proof() {
        arch_fatal(b"LogOS vNext: dynamic IPC proof");
    }
    #[cfg(feature = "qemu-proof")]
    if !crate::arch::allocator_proof() {
        arch_fatal(b"LogOS vNext: allocator proof");
    }
    #[cfg(feature = "qemu-proof")]
    if !crate::arch::event_proof() {
        arch_fatal(b"LogOS vNext: dynamic event proof");
    }
    #[cfg(feature = "qemu-proof")]
    proof::handoff_started();
    #[cfg(feature = "qemu-proof")]
    crate::suppress_service_heartbeat(logos_abi::ServiceId::Terminal);
    #[cfg(feature = "qemu-proof")]
    if !wait_for_manager_restart() {
        arch_fatal(b"LogOS vNext: manager restart proof");
    }

    #[cfg(feature = "package-proof")]
    {
        sleep_current_for(2);
        if crate::arch::activate_service_package(logos_abi::ServiceId::Input).is_err() {
            arch_fatal(b"LogOS vNext: package activation");
        }
        if crate::arch::activate_service_package(logos_abi::ServiceId::Session).is_err() {
            arch_fatal(b"LogOS vNext: second package activation");
        }
        proof::package_activation_complete();
        if crate::arch::activate_service_package(logos_abi::ServiceId::Display).is_ok() {
            arch_fatal(b"LogOS vNext: corrupt package accepted");
        }
        proof::package_corrupt_rejected();
        if !crate::arch::restart_service_graph_for_proof() {
            arch_fatal(b"LogOS vNext: package persistence restart");
        }
        proof::package_persistence_restarted();
    }

    if cfg!(feature = "qemu-proof") {
        let mut runtime = Runtime::new();
        if runtime.submit(RuntimeCommand::Submit).is_err() {
            arch_fatal(b"LogOS vNext: runtime capacity");
        }
        if runtime.submit(RuntimeCommand::Submit) != Err(RuntimeCommandError::Busy) {
            arch_fatal(b"LogOS vNext: runtime mailbox capacity");
        }
        #[cfg(feature = "qemu-proof")]
        proof::runtime_mailbox_busy();
        if !runtime.step() {
            arch_fatal(b"LogOS vNext: runtime step");
        }
        let timed = match runtime.take_response() {
            Some(RuntimeResponse::Submitted(handle)) => handle,
            _ => arch_fatal(b"LogOS vNext: runtime capacity response"),
        };
        let deadline = current_ticks().saturating_add(1);
        if send_runtime(&mut runtime, RuntimeCommand::Wait { handle: timed, deadline })
            != RuntimeResponse::Waiting(timed)
        {
            arch_fatal(b"LogOS vNext: runtime wait");
        }
        sleep_current_for(3);
        if send_runtime(
            &mut runtime,
            RuntimeCommand::Timeout { handle: timed, now: current_ticks() },
        ) != RuntimeResponse::TimedOut(timed)
        {
            arch_fatal(b"LogOS vNext: runtime timeout");
        }
        #[cfg(feature = "qemu-proof")]
        proof::runtime_timed_out();
        if send_runtime(&mut runtime, RuntimeCommand::Reclaim { handle: timed })
            != RuntimeResponse::Reclaimed(timed)
        {
            arch_fatal(b"LogOS vNext: runtime reclaim");
        }

        let completed = match send_runtime(&mut runtime, RuntimeCommand::Submit) {
            RuntimeResponse::Submitted(handle) => handle,
            _ => arch_fatal(b"LogOS vNext: runtime reuse"),
        };
        if send_runtime(
            &mut runtime,
            RuntimeCommand::Wait { handle: completed, deadline: current_ticks().saturating_add(3) },
        ) != RuntimeResponse::Waiting(completed)
        {
            arch_fatal(b"LogOS vNext: runtime wait");
        }
        sleep_current_for(3);
        if send_runtime(&mut runtime, RuntimeCommand::Complete { handle: completed })
            != RuntimeResponse::Completed(completed)
        {
            arch_fatal(b"LogOS vNext: runtime complete");
        }
        #[cfg(feature = "qemu-proof")]
        proof::runtime_completed();
        if send_runtime(&mut runtime, RuntimeCommand::Reclaim { handle: completed })
            != RuntimeResponse::Reclaimed(completed)
        {
            arch_fatal(b"LogOS vNext: runtime reclaim");
        }

        let replacement = match send_runtime(&mut runtime, RuntimeCommand::Submit) {
            RuntimeResponse::Submitted(handle) => handle,
            _ => arch_fatal(b"LogOS vNext: runtime reuse"),
        };
        if replacement.slot() != timed.slot() || replacement.generation() == timed.generation() {
            arch_fatal(b"LogOS vNext: runtime generation");
        }
        if send_runtime(&mut runtime, RuntimeCommand::Cancel { handle: replacement })
            != RuntimeResponse::Cancelled(replacement)
            || send_runtime(&mut runtime, RuntimeCommand::Reclaim { handle: replacement })
                != RuntimeResponse::Reclaimed(replacement)
        {
            arch_fatal(b"LogOS vNext: runtime cancel");
        }
        #[cfg(feature = "qemu-proof")]
        proof::runtime_slot_reused();

        let mut health = HealthService::new();
        let ping = match send_health(&mut health, HealthCommand::Ping { request_id: 1 }) {
            HealthResponse::PingAccepted(handle) => handle,
            _ => arch_fatal(b"LogOS vNext: health start"),
        };
        sleep_current_for(1);
        if send_health(&mut health, HealthCommand::Restart)
            != (HealthResponse::Restarted { count: 1 })
        {
            arch_fatal(b"LogOS vNext: health restart");
        }
        #[cfg(feature = "qemu-proof")]
        proof::health_restarted();
        if send_health(&mut health, HealthCommand::CompletePing { handle: ping })
            != HealthResponse::Rejected(HealthError::StaleOperation)
        {
            arch_fatal(b"LogOS vNext: health stale completion");
        }
        #[cfg(feature = "qemu-proof")]
        proof::health_late_completion_rejected();
        if send_health(&mut health, HealthCommand::Reclaim { handle: ping })
            != HealthResponse::Reclaimed(ping)
        {
            arch_fatal(b"LogOS vNext: health reclaim");
        }
        let retry = match send_health(&mut health, HealthCommand::Ping { request_id: 2 }) {
            HealthResponse::PingAccepted(handle) => handle,
            _ => arch_fatal(b"LogOS vNext: health retry"),
        };
        sleep_current_for(1);
        if send_health(&mut health, HealthCommand::CompletePing { handle: retry })
            != HealthResponse::PingCompleted(retry)
        {
            arch_fatal(b"LogOS vNext: health complete");
        }
        #[cfg(feature = "qemu-proof")]
        proof::health_retry_completed();
        if send_health(&mut health, HealthCommand::Reclaim { handle: retry })
            != HealthResponse::Reclaimed(retry)
        {
            arch_fatal(b"LogOS vNext: health reclaim");
        }
    }

    loop {
        if crate::supervise_services() {
            #[cfg(feature = "qemu-proof")]
            proof::live_service_restarted();
        }
        sleep_current_for(3);
        #[cfg(feature = "qemu-proof")]
        proof::runtime_wait_resumed();
    }
}

#[cfg(feature = "qemu-proof")]
fn wait_for_manager_restart() -> bool {
    let deadline = current_ticks().saturating_add(
        crate::supervisor::STARTUP_GRACE_TICKS
            + crate::supervisor::HEARTBEAT_INTERVAL
                * u64::from(crate::supervisor::MISSED_HEARTBEATS)
            + 1,
    );
    while current_ticks() < deadline {
        if crate::arch::manager_restart_ready(logos_abi::ServiceId::Terminal) {
            return true;
        }
        if crate::supervise_services() {
            proof::live_service_restarted();
        }
        sleep_current_for(3);
    }
    false
}

fn send_health(health: &mut HealthService, command: HealthCommand) -> HealthResponse {
    if health.submit(command) == Err(CommandError::Busy) {
        arch_fatal(b"LogOS vNext: health mailbox");
    }
    if !health.step() {
        arch_fatal(b"LogOS vNext: health step");
    }
    health.take_response().unwrap_or_else(|| arch_fatal(b"LogOS vNext: health response"))
}

fn send_runtime(runtime: &mut Runtime, command: RuntimeCommand) -> RuntimeResponse {
    if runtime.submit(command) == Err(RuntimeCommandError::Busy) {
        arch_fatal(b"LogOS vNext: runtime mailbox");
    }
    if !runtime.step() {
        arch_fatal(b"LogOS vNext: runtime step");
    }
    runtime.take_response().unwrap_or_else(|| arch_fatal(b"LogOS vNext: runtime response"))
}
