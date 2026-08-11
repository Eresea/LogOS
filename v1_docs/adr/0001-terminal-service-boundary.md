# ADR-0001: Run the normal terminal outside Core

- Status: Accepted
- Date: 2026-07-27

## Context

`logos-terminal` is a separate `no_std` crate but is statically linked into `logos-uefi`. The UEFI normal-mode loop owns its framebuffer and PS/2 access, command/session dispatch, and lifecycle. This is a bootstrap arrangement, not a Sessions service boundary.

## Decision

Keep only the recovery console in Core. Build and load `logos-terminal` as a native Sessions service from the QEMU boot payload, giving it versioned input, display, and session capabilities instead of raw framebuffer or PS/2 access.

## Consequences

- Platform v1 needs a native task loader and service lifecycle contract before the terminal can leave `logos-uefi`.
- QEMU must prove terminal start, redraw, restart, capability denial, and recovery handoff.
- New normal-terminal features do not add direct hardware access to Core.
- The recovery console stays small, direct, and usable when the terminal service fails.

## Alternatives considered

- Keep the terminal linked into `logos-uefi` — rejected because crate separation alone does not create a Core boundary.
- Move the recovery console out with the terminal — rejected because recovery must work when services cannot start.
