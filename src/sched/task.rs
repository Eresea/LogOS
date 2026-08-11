pub enum TaskState {
    Ready,
    Blocked(Event),
    Complete,
    Failed,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Event(u8);

impl Event {
    pub const VIRTIO: Self = Self(1);
    pub const INPUT: Self = Self(2);
    pub const COMMAND: Self = Self(4);
    pub const DISPLAY: Self = Self(8);
    pub(crate) const FAILURE: Self = Self(16);
    pub(crate) const SELF_CHECK: Self = Self(3);

    pub(crate) const fn bits(self) -> u8 {
        self.0
    }
}

pub trait Runnable {
    fn run(&mut self) -> TaskState;

    fn restart(&mut self) -> bool {
        false
    }
}
