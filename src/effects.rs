use crate::{acpi, ipc, platform, scheduler, services, session, supervisor, trace};
use logos_abi::{Effect, EffectRequest, EffectResult, InputLayout};
use logos_core::capabilities::{Capability, CapabilityKind, CapabilityManager};
use logos_terminal::input;

pub struct Context<'a, 'task> {
    pub session: &'a session::Context,
    pub capabilities: &'a CapabilityManager,
    pub tick: u64,
    pub input: &'a mut input::Service,
    pub lifecycle: &'a mut supervisor::Lifecycle,
    pub service_healthy: bool,
    pub channel: &'a ipc::Channel,
    pub responses: &'a ipc::Channel,
    pub service_scheduler: &'a mut scheduler::Scheduler<'task>,
    pub service_capability: Capability,
    pub service: services::ServiceHandle,
}

pub fn execute(request: EffectRequest, context: Context<'_, '_>) -> EffectResult {
    if !request.valid() {
        return EffectResult::Unknown;
    }
    if required_capability(request.effect)
        .is_some_and(|kind| !context.session.allows(context.capabilities, kind))
    {
        return EffectResult::Denied;
    }
    let argument = &request.argument[..request.length];
    match request.effect {
        Effect::EnterRecovery => EffectResult::Recovery,
        Effect::ResetMachine => effect(acpi::reset()),
        Effect::PowerOffMachine => effect(acpi::power_off()),
        Effect::PingService => effect_result(
            ping_platform(
                context.channel,
                context.responses,
                context.service_scheduler,
                context.capabilities,
                context.service_capability,
                context.session.principal(),
                context.service,
            ),
            EffectResult::Pong,
        ),
        Effect::ReadTasks => EffectResult::TasksActive,
        Effect::ReadServices => {
            if context.service_healthy {
                EffectResult::ServiceRunning
            } else {
                EffectResult::ServiceOverdue
            }
        }
        Effect::ReadDrivers => EffectResult::DriverBound,
        Effect::ReadTrace => trace_result(trace::latest()),
        Effect::InspectResource => EffectResult::Inspected,
        Effect::RestartService => effect_result(
            platform::matches(argument) && context.lifecycle.restart(context.tick),
            EffectResult::RestartScheduled,
        ),
        Effect::CancelService => effect_result(
            platform::matches(argument)
                && context
                    .channel
                    .send(
                        context.capabilities,
                        context.service_capability,
                        context.session.principal(),
                        context.service,
                        ipc::Message::Cancel,
                    )
                    .is_some(),
            EffectResult::CancelRequested,
        ),
        Effect::SetInputLayout => {
            match argument.first().copied().and_then(InputLayout::from_wire) {
                Some(InputLayout::Qwerty) if argument.len() == 1 => {
                    context.input.set_layout(input::Layout::Qwerty);
                    EffectResult::LayoutQwerty
                }
                Some(InputLayout::Azerty) if argument.len() == 1 => {
                    context.input.set_layout(input::Layout::Azerty);
                    EffectResult::LayoutAzerty
                }
                _ => EffectResult::Unknown,
            }
        }
    }
}

fn required_capability(effect: Effect) -> Option<CapabilityKind> {
    match effect {
        Effect::EnterRecovery | Effect::ResetMachine | Effect::PowerOffMachine => {
            Some(CapabilityKind::Recovery)
        }
        Effect::PingService | Effect::RestartService | Effect::CancelService => {
            Some(CapabilityKind::Service)
        }
        Effect::SetInputLayout => Some(CapabilityKind::Input),
        _ => None,
    }
}

fn effect(completed: bool) -> EffectResult {
    effect_result(completed, EffectResult::Completed)
}

fn effect_result(completed: bool, result: EffectResult) -> EffectResult {
    if completed { result } else { EffectResult::Unavailable }
}

fn trace_result(event: trace::Event) -> EffectResult {
    match event {
        trace::Event::Empty => EffectResult::TraceNone,
        trace::Event::Boot => EffectResult::TraceBoot,
        trace::Event::TaskBlocked => EffectResult::TraceTaskBlocked,
        trace::Event::TaskWoken => EffectResult::TraceTaskWoken,
        trace::Event::VirtioSubmit => EffectResult::TraceVirtioSubmit,
        trace::Event::VirtioComplete => EffectResult::TraceVirtioComplete,
        trace::Event::DriverBound => EffectResult::TraceDriverBound,
        trace::Event::DriverQuiesced => EffectResult::TraceDriverQuiesced,
        trace::Event::DriverRecovered => EffectResult::TraceDriverRecovered,
        trace::Event::DriverFailed => EffectResult::TraceDriverFailed,
        trace::Event::Fault => EffectResult::TraceFault,
        trace::Event::SelfCheck => EffectResult::TraceSelfCheck,
    }
}

fn ping_platform(
    channel: &ipc::Channel,
    responses: &ipc::Channel,
    scheduler: &mut scheduler::Scheduler<'_>,
    capabilities: &CapabilityManager,
    capability: Capability,
    principal: session::Principal,
    service: services::ServiceHandle,
) -> bool {
    let Some(request) =
        channel.send(capabilities, capability, principal, service, ipc::Message::Ping)
    else {
        return false;
    };
    scheduler.run_next()
        && (0..4).any(|_| {
            responses.receive().is_some_and(|reply| {
                reply.request == request && reply.message == ipc::Message::Pong
            })
        })
}

pub fn self_check() -> bool {
    required_capability(Effect::EnterRecovery) == Some(CapabilityKind::Recovery)
        && required_capability(Effect::PingService) == Some(CapabilityKind::Service)
        && required_capability(Effect::SetInputLayout) == Some(CapabilityKind::Input)
        && required_capability(Effect::ReadTasks).is_none()
        && trace_result(trace::Event::Fault) == EffectResult::TraceFault
        && effect_result(false, EffectResult::Pong) == EffectResult::Unavailable
}
