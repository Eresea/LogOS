# ADR-0085: Live input settings control

- Status: Accepted
- Date: 2026-09-13

## Decision

Atrium owns the volatile settings UI and sends a fixed `InputSettings` payload
to the Input service whenever keyboard layout or mouse acceleration changes.
Input applies the payload before decoding the next hardware packet. Keyboard
layout is decoded at the source, while pointer acceleration is applied to
relative packet deltas before absolute cursor integration.

Mouse acceleration has four bounded levels: Off, Low, Medium, and High, with
Medium enabled by default. All levels preserve one-to-one motion for small
deltas; enabled levels increase larger packet deltas with fixed-point gains.
Settings are runtime-only and reset with the existing Atrium/Input service
state.

## Consequences

- Changing AZERTY/QWERTY affects subsequent keyboard events without restarting.
- Mouse acceleration follows normal desktop semantics and does not alter button
  states or cursor bounds.
- The new control path is fixed-size, allocation-free, and wakes Input through
  its existing capability event wait.
- Persistence, timing-based acceleration curves, and pointer-device profiles
  remain deferred.
