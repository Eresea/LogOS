//! Flow's client state machines. Each one is a bounded, allocation-free
//! single-flight request driver that talks to its peer only through a
//! [`crate::Transport`]; the image owns capability discovery and the event loop.

pub mod completion;
pub mod device;
pub mod fetch;
pub mod network;
pub mod output;
pub mod package;
pub mod storage;
pub mod user;

pub use completion::{
    CompletionService, completion_message, completion_request, copy_candidate, flow_is_foreground,
    trim_flow_input,
};
pub use device::DeviceClient;
pub use fetch::FetchClient;
pub use network::{NetworkClient, network_command, network_result_text, network_state_text};
pub use output::PendingOutput;
pub use package::PackageClient;
pub use storage::{StorageClient, status_text, storage_ipc_error};
pub use user::{UserClient, user_status_text};
