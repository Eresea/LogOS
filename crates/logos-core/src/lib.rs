#![no_std]

pub mod capabilities;
pub mod clock;
pub mod fault;
pub mod native_service {
    pub use logos_abi::service::{
        ABI, ACKNOWLEDGED, BlockClientPage, COMPLETE, ControlPage, DisplayPage, EffectPage,
        EndpointState, InputPage, NetworkDevicePage, NetworkDevicePageState, NetworkDmaResources,
        NetworkEventPage, NetworkEventPageState, PANIC, ProtocolVersion, READ_INPUT, READY,
        REMOTE_GATE, RemoteGateOperation, RemoteGateReply, RemoteGateRequest, RemoteGateStatus,
        RemotePage, SESSION_EFFECT, SESSION_REPLY, STORAGE_CORRUPT, STORAGE_FORMATTED,
        STORAGE_IO_FAILED, STORAGE_RECOVERED, STORAGE_RECOVERED_INCOMPLETE, STORAGE_UNAVAILABLE,
        SessionClientPage, SessionServerPage, StoreClientPage, StoreServerPage, self_check,
    };
}
pub mod shared_pages;
pub mod test_protocol;
