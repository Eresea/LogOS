# ADR-0005: Suspend native services at Core-owned gates

- Status: Accepted
- Date: 2026-07-28

## Context

The terminal service must wait for input and display completion without retaining Core on its call
stack. A gate that always returns to Core can prove entry, but cannot host a long-lived service.

## Decision

Core owns each native service's saved Ring-3 registers and `iretq` frame. A gate either completes a
synchronous operation and resumes the saved frame, or marks the service blocked and returns to
Core. Only Core may resume a blocked service on its Core-owned supervisor stack.

## Consequences

- Gate handlers preserve user registers before calling Core code.
- Input and display operations can block; their completion resumes the same service frame.
- The service never supplies a return frame, kernel stack, CR3, or raw device mapping.
- QEMU must prove a blocked terminal service resumes and recovery remains available after failure.

## Alternatives considered

- Keep Core synchronously inside every terminal call — rejected because input waits would stop Core.
- Let the service retain or construct its own return frame — rejected because it crosses the ring boundary.
