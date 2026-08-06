use crate::platform::secrets::RemoteState;

pub struct RemoteRuntime {
    state: Option<RemoteState>,
    gateway_started: bool,
}

impl RemoteRuntime {
    pub fn state(&self) -> Option<&RemoteState> {
        self.state.as_ref()
    }

    pub fn state_mut(&mut self) -> &mut Option<RemoteState> {
        &mut self.state
    }

    pub fn replace_state(&mut self, state: RemoteState) {
        self.state = Some(state);
    }

    pub fn new(bootstrap: Option<logos_remote::Bootstrap>) -> Self {
        Self { state: bootstrap.and_then(RemoteState::new), gateway_started: false }
    }

    pub fn start(
        &mut self,
        network_reported: bool,
        gateway: Option<crate::sched::native_task::Handle>,
        scheduler: &mut crate::sched::native_task::Scheduler<'_>,
    ) -> bool {
        if !self.gateway_started && network_reported {
            self.gateway_started =
                gateway.is_some_and(|handle| scheduler.run(handle) && !scheduler.failed(handle));
            return self.gateway_started;
        }
        false
    }

    pub const fn started(&self) -> bool {
        self.gateway_started
    }

    pub fn reset_transport(&mut self) {
        self.gateway_started = false;
        if let Some(state) = self.state.as_mut() {
            state.reset_transport();
        }
    }
}

/// Remote owns the cross-service gate that decides whether the replaceable
/// gateway may start. The gateway receives no policy state; Core only supplies
/// the resulting typed endpoint.
pub fn gateway_allowed(
    network_ready: bool,
    sessions_ready: bool,
    storage_ready: bool,
    state: Option<&RemoteState>,
    test_hooks: bool,
) -> bool {
    network_ready
        && sessions_ready
        && storage_ready
        && (state.is_some_and(RemoteState::available) || test_hooks && state.is_some())
}

pub fn self_check() -> bool {
    !gateway_allowed(false, true, true, None, false)
        && !gateway_allowed(true, false, true, None, false)
        && !gateway_allowed(true, true, true, None, true)
}
