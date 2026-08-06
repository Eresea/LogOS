use super::Runner;

pub(crate) const fn is_configuration(runner: Runner) -> bool {
    matches!(runner, Runner::NetworkConfiguration)
}
