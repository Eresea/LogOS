use super::Runner;

pub(crate) const fn client_secret(runner: Runner) -> [u8; 32] {
    if matches!(runner, Runner::RemoteAuthDenied) { [7; 32] } else { [8; 32] }
}

pub(crate) const fn typed_input(runner: Runner) -> &'static str {
    if matches!(runner, Runner::RemoteTypedInvoke) {
        "ping\ntasks\nservices\nquit\n"
    } else {
        "ping\nquit\n"
    }
}

pub(crate) const fn auth_denied(runner: Runner) -> bool {
    matches!(runner, Runner::RemoteAuthDenied)
}
