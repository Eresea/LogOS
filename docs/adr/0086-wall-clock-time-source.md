# ADR-0086: Wall-clock time source

- Status: Accepted
- Date: 2026-09-26

## Decision

Core reads the CMOS RTC once at boot, decodes it into a fixed `WallTime {
year, month, day, hour, minute, second }`, and anchors it to the scheduler's
`TIMER_TICKS` value at that moment. The read is update-in-progress safe (it
waits for CMOS status register A's UIP bit to clear, then requires two
back-to-back snapshots to agree) and handles BCD or binary encoding and
12-hour or 24-hour mode via status register B, including the PM bit.

Services reach the reading through one new bounded syscall,
`WALL_TIME_SYSCALL` (21), which returns the current wall time packed into a
single `u64` in `rax`: the same no-user-pointer shape as the existing
`CURRENT_TICKS_SYSCALL`. The kernel computes the returned value by advancing
the boot anchor by the ticks elapsed since boot, converted to seconds at the
scheduler's fixed 100 ticks/second rate. The RTC decode and the tick-to-date
advance are pure functions in the `logos-abi` crate (`decode_rtc`,
`advance_wall_time`) so they are host-tested without the UEFI target.

Local time only. No time zone conversion, no NTP, no syscall to set the
clock, and no periodic re-read: drift between the RTC and the anchor grows
over an uptime the size of this milestone's proof runs, and correcting it is
deferred to whichever later work needs long-uptime accuracy.

## Consequences

- `WallTime` is a fixed six-field struct; the ABI gains no allocation,
  pointers, or buffers.
- A service calls the syscall directly; no directory or manager round trip.
- An invalid RTC reading (out-of-range field) falls back to a fixed epoch
  date rather than failing boot, since the clock is not safety-critical.
- Wall time is monotonic with the scheduler tick, not with real elapsed time;
  it does not correct for APIC timer calibration drift.
