# ADR-0036: ABI v5 endpoint table activation

- Status: Accepted
- Date: 2026-08-11

## Context

ABI v5 already defined bounded operation identity, but native services still received ABI-v4-style
per-page pointers in `ControlPage`. The opt-in endpoint table existed without being mapped or used by
the production address-space and service-runtime paths.

## Decision

- `ControlPage` carries one aligned `EndpointTable` address instead of named endpoint page pointers.
- `EndpointTable` is fixed at 16 slots; each slot is a known kind, versioned, page-aligned, and bound
  to the ControlPage generation.
- Core maps and initializes the table with the typed pages granted by `ServiceSpec::endpoints`, and
  rebuilds it on every service generation change.
- Native services resolve every endpoint through the table. Missing, malformed, or stale tables fail
  closed; there is no ABI-v4 pointer fallback or compatibility adapter.

## Consequences

Endpoint discovery is bounded and uniform without introducing a dynamic registry or allocator. The
table is an ABI-v5 wire contract, so older payloads must be rejected and any future layout change
requires an explicit compatibility milestone. Typed endpoint page state and Core-owned physical
mappings remain unchanged.
