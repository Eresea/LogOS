#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ConsoleMode {
    Normal,
    Recovery,
}

impl ConsoleMode {
    pub const fn new(normal_ready: bool) -> Self {
        if normal_ready { Self::Normal } else { Self::Recovery }
    }

    pub fn announce(&self) {
        crate::debug::write_line(match self {
            Self::Normal => b"LogOS: console mode normal",
            Self::Recovery => b"LogOS: console mode recovery",
        });
    }

    pub fn self_check() -> bool {
        Self::new(true) == Self::Normal && Self::new(false) == Self::Recovery
    }
}
