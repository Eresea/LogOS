//! IPC dispatch result types shared by the Core runtime boundary.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IpcError {
    Capacity,
    Exhausted,
    Memory,
    InvalidIdentity,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IpcOutcome {
    pub status: logos_abi::IpcStatus,
    pub notified: bool,
}
