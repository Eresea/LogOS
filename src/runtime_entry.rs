use crate::{
    arch_fatal, current_ticks,
    health::{CommandError, HealthCommand, HealthError, HealthResponse, HealthService},
    runtime, sleep_current_for,
};

#[cfg(feature = "qemu-proof")]
use crate::proof;

pub(crate) fn run() {
    #[cfg(feature = "qemu-proof")]
    proof::handoff_started();

    let mut runtime = runtime::Runtime::new();
    let timed = runtime.submit().unwrap_or_else(|_| arch_fatal(b"LogOS vNext: runtime capacity"));
    let deadline = current_ticks().saturating_add(1);
    if !runtime.wait(timed, deadline) {
        arch_fatal(b"LogOS vNext: runtime wait");
    }
    sleep_current_for(3);
    if !runtime.timeout(timed, current_ticks()) {
        arch_fatal(b"LogOS vNext: runtime timeout");
    }
    #[cfg(feature = "qemu-proof")]
    proof::runtime_timed_out();
    if !runtime.reclaim(timed) {
        arch_fatal(b"LogOS vNext: runtime reclaim");
    }

    let completed = runtime.submit().unwrap_or_else(|_| arch_fatal(b"LogOS vNext: runtime reuse"));
    if !runtime.wait(completed, current_ticks().saturating_add(3)) {
        arch_fatal(b"LogOS vNext: runtime wait");
    }
    sleep_current_for(3);
    if !runtime.complete(completed) {
        arch_fatal(b"LogOS vNext: runtime complete");
    }
    #[cfg(feature = "qemu-proof")]
    proof::runtime_completed();
    if !runtime.reclaim(completed) {
        arch_fatal(b"LogOS vNext: runtime reclaim");
    }

    let replacement =
        runtime.submit().unwrap_or_else(|_| arch_fatal(b"LogOS vNext: runtime reuse"));
    if replacement.slot() != timed.slot() || replacement.generation() == timed.generation() {
        arch_fatal(b"LogOS vNext: runtime generation");
    }
    if !runtime.cancel(replacement) || !runtime.reclaim(replacement) {
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
    if send_health(&mut health, HealthCommand::Restart) != (HealthResponse::Restarted { count: 1 })
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

    loop {
        sleep_current_for(3);
        #[cfg(feature = "qemu-proof")]
        proof::runtime_wait_resumed();
    }
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
