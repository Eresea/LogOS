//! Flow's client state machines. Each one is a bounded, allocation-free
//! single-flight request driver that talks to its peer only through a
//! [`crate::Transport`]; the image owns capability discovery and the event loop.

pub mod device;
pub mod output;

pub use device::DeviceClient;
pub use output::PendingOutput;
