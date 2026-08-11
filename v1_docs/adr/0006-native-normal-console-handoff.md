# ADR-0006: Hand off from recovery to the native normal terminal

- Status: Accepted
- Date: 2026-07-28

## Decision

Core starts its direct recovery console path, validates the native terminal payload, then routes
normal keyboard, rendering, and command traffic only through the loaded Ring-3 terminal service.
Escape or the authorized `recovery` command returns to the direct recovery console.

## Consequences

- The UEFI normal-mode loop is disabled; recovery remains independent of the payload.
- The current native gate is the bounded bootstrap transport. Input, display, and session services
  remain the next Platform v1 split; they must replace this gate without giving the terminal raw
  hardware mappings.
