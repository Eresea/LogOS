# ADR-0089: Atrium appearance preferences over existing channels

Status: Accepted

## Decision

Atrium owns the runtime appearance preferences chosen in Settings >
Appearance (#79): one of four fixed accents, the FPS overlay, and reduced
motion. It applies them to its own scenes and delivers the one preference
other services need, reduced motion, over channels those services already
receive from Atrium. No new capability, channel, or boot wiring is added.

- **Accent.** `logos-ui-graphics` owns the fixed palette `UI_ACCENTS`
  (blue, teal, purple, orange; each an `accent`/`focus` pair).
  `Atrium::home_theme()` takes both colours; `Atrium::app_theme()` takes only
  `focus`, keeping the red close control. Every Atrium scene publishes with
  these themes, so a change repaints on the next frame. The `System` service
  owns its own scene and theme and is unaffected.
- **FPS overlay.** Display keeps the state; Atrium mirrors it (starting on,
  as Display does) and sends the existing `GuiSurfaceOperation::ToggleFps`
  when the switch flips. Ctrl+F12 flips the mirror too, so the two stay in
  step.
- **Reduced motion.** `APPEARANCE_REDUCED_MOTION` is the only flag of
  `APPEARANCE_FLAGS_MASK`. It reaches:
  - Atrium's own trees through `UiComponentTree::set_reduced_motion`;
  - LockScreen as `GuiHookKind::Appearance` on the existing hook channel,
    flags in `deadline`. LockScreen now ignores hook kinds it does not
    handle instead of treating every hook as a section toggle;
  - Terminal as `MessageKind::Appearance` inside `AtriumSurfaceInput` on the
    existing surface-input channel, flags in `modifiers`. Atrium re-sends it
    to each new Terminal surface.

  Reduced motion makes `UiAnimator` settle transitions at their target and
  hold animations at their base style, and turning it on settles motion
  already running. Terminal's block cursor blinks (500 ms phases, restarting
  visible on activity, settling visible after 10 s idle) unless reduced
  motion is on or the view is scrolled back.

Messages carry flags only; unknown bits are rejected by
`GuiHook::appearance_flags` and `InputMessage::appearance_flags`.

## Rationale

A dedicated appearance channel per service would add capabilities and boot
wiring for a single bit. Both receivers already accept Atrium-originated
messages scoped to them, so a new message kind on each keeps the change
additive: older kinds are untouched and receivers validate the new one
exactly. The `UiStyle::Swatch(u8)` palette index keeps `UiStyle` at its
current size, where an arbitrary `u32` colour would grow every retained node.

## Consequences

- Preferences are runtime-only until settings persistence (S4, #81).
- A future preference for other services adds a flag bit, not a channel.
- The accent palette is fixed at four entries; a light theme (S5, #82) is a
  separate theme, not an accent.
