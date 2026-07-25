#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ConsoleMode {
    Normal,
    Recovery,
}

pub struct Coordinator {
    mode: ConsoleMode,
}

impl Coordinator {
    pub const fn new(normal_ready: bool) -> Self {
        Self { mode: if normal_ready { ConsoleMode::Normal } else { ConsoleMode::Recovery } }
    }

    pub const fn mode(&self) -> ConsoleMode {
        self.mode
    }

    pub fn announce(&self) {
        crate::debug::write_line(match self.mode {
            ConsoleMode::Normal => b"LogOS: console mode normal",
            ConsoleMode::Recovery => b"LogOS: console mode recovery",
        });
    }

    pub fn self_check() -> bool {
        Self::new(true).mode == ConsoleMode::Normal
            && Self::new(false).mode == ConsoleMode::Recovery
    }
}
