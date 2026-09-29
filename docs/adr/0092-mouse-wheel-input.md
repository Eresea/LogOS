# ADR-0092: Mouse-wheel input through the PS/2 driver and pointer ABI

Status: Accepted

## Decision

The Terminal scrollback (T2, #75) gains mouse-wheel scrolling (T2b, #90). The change crosses rings,
so the pieces are fixed here:

- **Kernel driver (`arch::configure_pointer`).** After enabling the auxiliary port the driver runs
  the IntelliMouse handshake: set-sample-rate 200, 100, 80 (each byte ACKed), then Get-ID. ID 3 means
  the device streams 4-byte packets; any other answer, including ID 0 or a missing ACK, leaves plain
  3-byte packets and the wheel disabled. Negotiation is best effort and never fails pointer setup.
  Stream mode (`0xF4`) is enabled last, as before. Wheel only: ID 4 (five buttons) is not requested
  and its extra button bits are ignored if a device sends them.
- **Packet size hand-off.** The kernel publishes the result in a new `wheel` byte on the pointer
  `KeyboardByteRing` (zero-initialised, so the default is "3-byte"). Input re-reads the flag before each byte, and `PointerDecoder::set_wheel`
  applies it only on a packet boundary, so a late publish can never split a packet.
- **Decoder.** `PointerDecoder` reads the fourth byte's low nibble as a signed 4-bit Z delta. The
  device reports wheel-up as negative; the decoder negates it, so positive means up. A packet whose
  only change is Z produces a `Move`-state event; buttons and motion keep their existing meaning.
- **ABI (`ABI_VERSION` 12).** `PointerEvent` gains `wheel: i8`. It travels in `InputMessage.text[0]`
  of `MessageKind::Pointer` (previously always zero); no field, size or message kind was added.
  `InputMessage::pointer` keeps its signature (wheel 0); `pointer_wheel` sets it. All existing
  producers (host tests, lock-screen and Atrium synthetic events) therefore stay valid unchanged.
- **Routing and use.** Atrium sends a nonzero wheel to the focused surface instead of hit-testing
  the pointer, and never coalesces wheel events with plain motion. Terminal maps one notch to
  three `scroll_view` lines (positive scrolls back), which already clamps at both ends.
  Horizontal wheel and extra buttons are out of scope.

## Consequences

- Guest devices that answer ID 0 (or hardware without a wheel) behave exactly as before.
- The negotiation is exercised only under QEMU (the QMP proof sends `wheel-up`/`wheel-down`); real
  hardware is untested.
