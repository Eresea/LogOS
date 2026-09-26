//! Bounded wall-clock wire shape (ADR-0086).
//!
//! `WallTime` is the fixed, read-only value the kernel hands to services
//! through `WALL_TIME_SYSCALL`. The RTC decode and monotonic-anchor advance
//! logic are pure functions so they are host-testable without the UEFI
//! target; the kernel side only supplies the raw CMOS bytes and the elapsed
//! tick count.

/// A fixed, local-time wall-clock reading. No time zone, no sub-second
/// precision: this is the smallest shape the acceptance criteria need.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct WallTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

impl WallTime {
    /// Pack into the single `u64` the bounded syscall ABI returns in `rax`.
    pub fn pack(self) -> u64 {
        (u64::from(self.year) << 40)
            | (u64::from(self.month) << 32)
            | (u64::from(self.day) << 24)
            | (u64::from(self.hour) << 16)
            | (u64::from(self.minute) << 8)
            | u64::from(self.second)
    }

    /// Inverse of [`WallTime::pack`].
    pub fn unpack(raw: u64) -> Self {
        WallTime {
            year: ((raw >> 40) & 0xffff) as u16,
            month: ((raw >> 32) & 0xff) as u8,
            day: ((raw >> 24) & 0xff) as u8,
            hour: ((raw >> 16) & 0xff) as u8,
            minute: ((raw >> 8) & 0xff) as u8,
            second: (raw & 0xff) as u8,
        }
    }
}

/// Raw CMOS RTC register bytes, read before any BCD/12h decoding.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RtcRegisters {
    pub second: u8,
    pub minute: u8,
    pub hour: u8,
    pub day: u8,
    pub month: u8,
    /// Two-digit year (register 0x09), 00-99.
    pub year: u8,
    /// Century register (0x32) when the platform exposes it; `None` assumes
    /// the 2000-2099 window, which covers every QEMU/UEFI target this
    /// milestone boots on.
    pub century: Option<u8>,
    /// Status register B: bit 2 set means binary (not BCD), bit 1 set means
    /// 24-hour mode. Bit 7 of the raw hour byte is the PM flag in 12-hour
    /// mode.
    pub status_b: u8,
}

const STATUS_B_BINARY: u8 = 1 << 2;
const STATUS_B_24_HOUR: u8 = 1 << 1;
const HOUR_PM_FLAG: u8 = 1 << 7;

fn bcd_to_binary(value: u8) -> u8 {
    (value & 0x0f) + ((value >> 4) * 10)
}

/// Decode raw CMOS bytes into a validated [`WallTime`]. Returns `None` for
/// any field outside its valid range, so callers never anchor on garbage.
pub fn decode_rtc(raw: RtcRegisters) -> Option<WallTime> {
    let binary = raw.status_b & STATUS_B_BINARY != 0;
    let is_24_hour = raw.status_b & STATUS_B_24_HOUR != 0;

    let (second, minute, day, month, year_low) = if binary {
        (raw.second, raw.minute, raw.day, raw.month, raw.year)
    } else {
        (
            bcd_to_binary(raw.second),
            bcd_to_binary(raw.minute),
            bcd_to_binary(raw.day),
            bcd_to_binary(raw.month),
            bcd_to_binary(raw.year),
        )
    };

    let pm = raw.hour & HOUR_PM_FLAG != 0;
    let hour_field = raw.hour & !HOUR_PM_FLAG;
    let hour_value = if binary { hour_field } else { bcd_to_binary(hour_field) };
    let hour = if is_24_hour {
        hour_value
    } else {
        // 12-hour: register holds 1-12; midnight/noon are special-cased.
        match (hour_value, pm) {
            (12, false) => 0,
            (12, true) => 12,
            (h, false) => h,
            (h, true) => h + 12,
        }
    };

    let century = raw.century.map(|c| if binary { c } else { bcd_to_binary(c) });
    let year = match century {
        Some(century) => u16::from(century) * 100 + u16::from(year_low),
        None => 2000 + u16::from(year_low),
    };

    if !(1..=12).contains(&month) {
        return None;
    }
    if day < 1 || day > days_in_month(year, month) {
        return None;
    }
    if hour > 23 || minute > 59 || second > 59 {
        return None;
    }

    Some(WallTime { year, month, day, hour, minute, second })
}

fn is_leap_year(year: u16) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn days_in_month(year: u16, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap_year(year) {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

/// Advance a base wall-clock reading by `elapsed_seconds`, carrying across
/// minute/hour/day/month/year boundaries. Bounded: the loop below runs at
/// most once per elapsed day, which is fine for the seconds-to-hours gap
/// between boot and a service query.
///
/// ponytail: naive day-by-day carry, not a closed-form calendar calculation;
/// upgrade to one if callers ever need to advance by years' worth of ticks.
pub fn advance_wall_time(base: WallTime, elapsed_seconds: u64) -> WallTime {
    let mut total = u64::from(base.hour) * 3600
        + u64::from(base.minute) * 60
        + u64::from(base.second)
        + elapsed_seconds;
    let mut day = base.day;
    let mut month = base.month;
    let mut year = base.year;

    let elapsed_days = total / 86_400;
    total %= 86_400;
    let mut remaining_days = elapsed_days;
    while remaining_days > 0 {
        remaining_days -= 1;
        day += 1;
        if day > days_in_month(year, month) {
            day = 1;
            month += 1;
            if month > 12 {
                month = 1;
                year = year.saturating_add(1);
            }
        }
    }

    let hour = (total / 3600) as u8;
    let minute = ((total % 3600) / 60) as u8;
    let second = (total % 60) as u8;

    WallTime { year, month, day, hour, minute, second }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn regs(
        status_b: u8,
        second: u8,
        minute: u8,
        hour: u8,
        day: u8,
        month: u8,
        year: u8,
    ) -> RtcRegisters {
        RtcRegisters { second, minute, hour, day, month, year, century: None, status_b }
    }

    #[test]
    fn decodes_bcd_24_hour() {
        let raw = regs(STATUS_B_24_HOUR, 0x05, 0x30, 0x14, 0x26, 0x09, 0x26);
        let wall = decode_rtc(raw).expect("valid BCD reading");
        assert_eq!(
            wall,
            WallTime { year: 2026, month: 9, day: 26, hour: 14, minute: 30, second: 5 }
        );
    }

    #[test]
    fn decodes_binary() {
        let raw = regs(STATUS_B_BINARY | STATUS_B_24_HOUR, 5, 30, 14, 26, 9, 26);
        let wall = decode_rtc(raw).expect("valid binary reading");
        assert_eq!(
            wall,
            WallTime { year: 2026, month: 9, day: 26, hour: 14, minute: 30, second: 5 }
        );
    }

    #[test]
    fn decodes_12_hour_pm() {
        // 2:30:05 PM in BCD 12-hour registers: hour byte is 0x02 with the PM bit set.
        let raw = regs(0, 0x05, 0x30, 0x02 | HOUR_PM_FLAG, 0x26, 0x09, 0x26);
        let wall = decode_rtc(raw).expect("valid 12h PM reading");
        assert_eq!(wall.hour, 14);
    }

    #[test]
    fn decodes_12_hour_am_midnight() {
        // Midnight is register value 12 in 12-hour mode, PM bit clear.
        let raw = regs(0, 0x00, 0x00, 0x12, 0x01, 0x01, 0x26);
        let wall = decode_rtc(raw).expect("valid 12h midnight reading");
        assert_eq!(wall.hour, 0);
    }

    #[test]
    fn decodes_12_hour_noon() {
        let raw = regs(0, 0x00, 0x00, 0x12 | HOUR_PM_FLAG, 0x01, 0x01, 0x26);
        let wall = decode_rtc(raw).expect("valid 12h noon reading");
        assert_eq!(wall.hour, 12);
    }

    #[test]
    fn rejects_invalid_day_for_month() {
        // April 31st does not exist.
        let raw = regs(STATUS_B_24_HOUR, 0, 0, 0, 0x31, 0x04, 0x26);
        assert!(decode_rtc(raw).is_none());
    }

    #[test]
    fn rejects_invalid_month() {
        let raw = regs(STATUS_B_24_HOUR, 0, 0, 0, 0x01, 0x13, 0x26);
        assert!(decode_rtc(raw).is_none());
    }

    #[test]
    fn rejects_invalid_hour_minute_second() {
        assert!(decode_rtc(regs(STATUS_B_BINARY | STATUS_B_24_HOUR, 0, 0, 24, 1, 1, 26)).is_none());
        assert!(decode_rtc(regs(STATUS_B_BINARY | STATUS_B_24_HOUR, 0, 60, 0, 1, 1, 26)).is_none());
        assert!(decode_rtc(regs(STATUS_B_BINARY | STATUS_B_24_HOUR, 60, 0, 0, 1, 1, 26)).is_none());
    }

    #[test]
    fn honors_century_register() {
        let raw = RtcRegisters {
            second: 0,
            minute: 0,
            hour: 0,
            day: 0x01,
            month: 0x01,
            year: 0x26,
            century: Some(0x20),
            status_b: STATUS_B_24_HOUR,
        };
        assert_eq!(decode_rtc(raw).unwrap().year, 2026);
    }

    #[test]
    fn pack_unpack_round_trips() {
        let wall = WallTime { year: 2026, month: 9, day: 26, hour: 14, minute: 30, second: 5 };
        assert_eq!(WallTime::unpack(wall.pack()), wall);
    }

    #[test]
    fn advances_within_same_day() {
        let base = WallTime { year: 2026, month: 9, day: 26, hour: 23, minute: 59, second: 50 };
        let advanced = advance_wall_time(base, 5);
        assert_eq!(advanced.second, 55);
        assert_eq!(advanced.day, 26);
    }

    #[test]
    fn advances_across_midnight_and_month_end() {
        let base = WallTime { year: 2026, month: 9, day: 30, hour: 23, minute: 59, second: 50 };
        let advanced = advance_wall_time(base, 20);
        assert_eq!(
            advanced,
            WallTime { year: 2026, month: 10, day: 1, hour: 0, minute: 0, second: 10 }
        );
    }

    #[test]
    fn advances_across_leap_day() {
        let base = WallTime { year: 2024, month: 2, day: 28, hour: 23, minute: 59, second: 59 };
        let advanced = advance_wall_time(base, 1);
        assert_eq!(
            advanced,
            WallTime { year: 2024, month: 2, day: 29, hour: 0, minute: 0, second: 0 }
        );
    }

    #[test]
    fn advances_across_year_end_non_leap() {
        let base = WallTime { year: 2025, month: 2, day: 28, hour: 23, minute: 59, second: 59 };
        let advanced = advance_wall_time(base, 1);
        assert_eq!(
            advanced,
            WallTime { year: 2025, month: 3, day: 1, hour: 0, minute: 0, second: 0 }
        );
    }
}
