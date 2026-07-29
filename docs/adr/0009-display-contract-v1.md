# ADR-0009: Introduce `foundation.display` v1 incrementally

- Status: Accepted
- Date: 2026-07-29

## Context

The bootstrap terminal presents raw packed colors through Core-owned framebuffer validation.

## Decision

Define a shared, validated RGB display value first. Core continues to validate coordinates and own
framebuffer writes. The terminal session receives an explicit Display capability; Core authorizes
each deferred presentation request with it, then resumes the terminal only after rendering.

## Consequences

- Invalid packed colors are rejected at the Core boundary.
- The terminal still receives no framebuffer mapping.
- Missing capability, malformed requests, and renderer failure stop the terminal and enter recovery.
- No generic display RPC framework is introduced.
