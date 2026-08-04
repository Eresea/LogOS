#[derive(Clone, Copy)]
pub(crate) enum Runner {
    Default,
    NetworkConfiguration,
    PersistenceWriteInterruption,
    PersistenceRecovery,
    PersistenceCorruption,
    PersistenceFixture,
    PersistenceTimeoutReset,
    PersistenceTerminalHistory,
    PersistenceCapabilityDenied,
    RemoteAuthDenied,
    RemoteTypedInvoke,
}
