# ADR-0007: Send typed terminal calls across the native command gate

- Status: Accepted
- Date: 2026-07-29

## Context

The Ring-3 terminal parses normal command text; Core enforces capabilities and privileged effects.

## Decision

Remote operations cross the native gate as a versioned command enum plus a bounded argument.

## Consequences

- The native-service ABI is version 2.
- No general shell transport is added.
