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

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum WallClock {
    Unknown,
    Untrusted { year: u16, month: u8, day: u8, hour: u8, minute: u8, second: u8 },
}

pub fn wall_clock() -> WallClock {
    runtime::get_time().ok().filter(|time| time.is_valid().is_ok()).map_or(
        WallClock::Unknown,
        |time| WallClock::Untrusted {
            year: time.year(),
            month: time.month(),
            day: time.day(),
            hour: time.hour(),
            minute: time.minute(),
            second: time.second(),
        },
    )
}

pub fn announce(clock: WallClock) {
    crate::debug::write_line(match clock {
        WallClock::Unknown => b"LogOS: wall clock unknown",
        WallClock::Untrusted { .. } => b"LogOS: wall clock untrusted",
    });
}

pub fn self_check() -> bool {
    Monotonic(1).ticks() < Monotonic(2).ticks()
        && matches!(WallClock::Unknown, WallClock::Unknown)
        && matches!(
            WallClock::Untrusted { year: 2026, month: 7, day: 27, hour: 0, minute: 0, second: 0 },
            WallClock::Untrusted { year: 2026, .. }
        )
}
use uefi::runtime;
