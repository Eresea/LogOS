#![no_std]

pub mod capabilities;
pub mod clock;
pub mod event;
pub mod fault;
pub mod manifest;
pub use manifest::{MAX_MANIFEST_ENTRIES, ManifestEntry, ManifestError, ServiceManifest};
pub mod native_service {
    pub use logos_abi::service::{
        ABI, ACKNOWLEDGED, BlockClientPage, COMPLETE, ControlPage, DisplayPage, EffectPage,
        EndpointState, InputPage, NetworkClientPage, NetworkDevicePage, NetworkDevicePageState,
        NetworkDmaResources, NetworkEventPage, NetworkEventPageState, NetworkPageState,
        NetworkServerPage, NetworkServerRequest, PANIC, ProtocolVersion, READ_INPUT, READY,
        REMOTE_GATE, RemoteGateOperation, RemoteGateStatus, RemotePage, RemotePageReply,
        RemotePageRequest, RemotePageState, SESSION_EFFECT, SESSION_REPLY, STORAGE_CORRUPT,
        STORAGE_FORMATTED, STORAGE_IO_FAILED, STORAGE_RECOVERED, STORAGE_RECOVERED_INCOMPLETE,
        STORAGE_UNAVAILABLE, SessionClientPage, SessionServerPage, StoreClientPage,
        StoreServerPage, StreamPage, self_check,
    };
}
pub mod poll_runtime;
pub mod resource;
pub mod shared_pages;
pub mod test_protocol;
