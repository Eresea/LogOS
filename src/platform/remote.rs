use crate::platform::secrets::RemoteState;

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
