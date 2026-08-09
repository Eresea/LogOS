use core::cell::Cell;

pub struct State(Cell<bool>);

impl State {
    pub const fn new() -> Self {
        Self(Cell::new(false))
    }

    pub fn reset(&self) {
        self.0.set(false);
    }

    pub fn record(&self, passed: bool) {
        self.0.set(self.0.get() || passed);
    }

    pub fn passed(&self) -> bool {
        self.0.get()
    }
}

pub fn is_assertion_input(value: &str) -> bool {
    value.starts_with("assert-") || value.starts_with("deny-")
}
