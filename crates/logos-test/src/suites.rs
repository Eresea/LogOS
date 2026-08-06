pub(crate) mod catalog;
pub(crate) mod network;
pub(crate) mod persistence;
pub(crate) mod remote;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Fixture {
    Shared,
    Fresh,
    Persistence,
    MissingSessions,
    MissingTerminal,
    IncompatibleSessions,
    MissingStore,
    MissingNetwork,
}

#[derive(Clone, Copy)]
pub(crate) struct Scenario {
    pub(crate) id: &'static str,
    pub(crate) suite: &'static str,
    pub(crate) timeout: u64,
    pub(crate) implemented: bool,
    pub(crate) setup: &'static [&'static str],
    pub(crate) fixture: Fixture,
    pub(crate) runner: Runner,
}

pub(crate) use catalog::SCENARIOS;

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
