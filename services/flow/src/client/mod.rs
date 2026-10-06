//! Flow's client state machines. Each one is a bounded, allocation-free
//! single-flight request driver that talks to its peer only through a
//! [`crate::Transport`]; the image owns capability discovery and the event loop.

pub mod device;
pub mod output;
pub mod storage;
pub mod user;

pub use device::DeviceClient;
pub use output::PendingOutput;
pub use storage::{StorageClient, status_text, storage_ipc_error};
pub use user::{UserClient, user_status_text};
