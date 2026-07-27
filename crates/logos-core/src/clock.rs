pub trait MonotonicClock {
    fn now(&self) -> u64;
}

#[derive(Default)]
pub struct VirtualClock(u64);

impl VirtualClock {
    pub const fn new() -> Self {
        Self(0)
    }
    pub fn advance(&mut self, ticks: u64) {
        self.0 = self.0.saturating_add(ticks);
    }
}

impl MonotonicClock for VirtualClock {
    fn now(&self) -> u64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn virtual_time_is_monotonic() {
        let mut clock = VirtualClock::new();
        clock.advance(30);
        assert_eq!(clock.now(), 30);
        clock.advance(u64::MAX);
        assert_eq!(clock.now(), u64::MAX);
    }
}
