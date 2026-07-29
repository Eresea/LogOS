# ADR-0009: Introduce `foundation.display` v1 incrementally

- Status: Accepted
- Date: 2026-07-29

## Context

The bootstrap terminal presents raw packed colors through Core-owned framebuffer validation.

## Decision

Define a shared, validated RGB display value first. Core continues to validate coordinates and own
framebuffer writes. Display capability routing is deferred until the bootstrap context has an
explicit service capability context.

## Consequences

- Invalid packed colors are rejected at the Core boundary.
- The terminal still receives no framebuffer mapping.
- No generic display RPC framework is introduced.
