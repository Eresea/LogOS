use super::Runner;

pub(crate) const fn is_proof(runner: Runner) -> bool {
    matches!(
        runner,
        Runner::PersistenceWriteInterruption
            | Runner::PersistenceRecovery
            | Runner::PersistenceCorruption
            | Runner::PersistenceFixture
            | Runner::PersistenceTimeoutReset
            | Runner::PersistenceTerminalHistory
            | Runner::PersistenceCapabilityDenied
    )
}
