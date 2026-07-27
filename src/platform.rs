#[path = "virtio.rs"]
mod driver;

pub use driver::{ServiceTask as Task, VirtioService as Service, completion_pending, interrupt};
