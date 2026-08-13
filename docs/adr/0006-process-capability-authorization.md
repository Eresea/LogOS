# ADR-0006: Process capability authorization

- Status: Superseded by ADR-0036
- Date: 2026-08-12

## Decision

Process capabilities are checked through a typed capability enum against the generation-safe process
handle. Missing capabilities return an explicit permission error; stale handles return an invalid
handle error.

## Scope

This slice adds authorization only. Capability transfer, syscall transport, endpoint handles, and
hardware mapping are separate bounded changes.
