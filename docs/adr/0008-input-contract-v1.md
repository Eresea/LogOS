# ADR-0008: Introduce `foundation.input` v1 before other terminal contracts

- Status: Accepted
- Date: 2026-07-29

## Context

The terminal bootstrap gate currently carries raw input bytes and terminal-specific layout syscalls.
Platform v1 requires capability-only Input, Display, and Session contracts.

## Decision

Introduce `foundation.input` v1 first: a versioned typed event stream and typed layout request.
Core owns PS/2 decoding and recovery input, validates the terminal's Input capability, and exposes
neither hardware mappings nor scancodes. Display and Session remain on the bounded bootstrap gate.

## Consequences

- The input wire format is shared from `logos-abi`.
- A later Input service can replace Core's bootstrap adapter without changing terminal semantics.
- This does not create a general service RPC framework.
