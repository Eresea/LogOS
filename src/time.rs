#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Monotonic(u64);

impl Monotonic {
    pub const fn ticks(self) -> u64 {
        self.0
    }
}

pub fn now() -> Monotonic {
    Monotonic(crate::interrupts::ticks())
}

pub fn self_check() -> bool {
    Monotonic(1).ticks() < Monotonic(2).ticks()
}
