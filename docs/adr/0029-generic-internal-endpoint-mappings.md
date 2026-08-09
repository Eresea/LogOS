# ADR-0029: Generic internal endpoint mappings

- Status: Accepted
- Date: 2026-08-10

## Context

ABI v4 exposes named typed endpoint pages, while Core previously repeated endpoint identity as a
bitset, mapping booleans, and one physical-page field per endpoint kind. That duplication made a new
endpoint a cross-cutting Core change without improving the service-facing contract.

## Decision

`ServiceSpec::endpoints` is a bounded static slice of typed `EndpointDescriptor` values. Core maps
the requested descriptors into bounded `EndpointMapping` records keyed by an internal
`EndpointKind`. Native tasks retain typed endpoint accessors, while the address-space mapper owns the
single ABI-v4 adapter that populates the existing named `ControlPage` fields.

Endpoint descriptors carry only Core-relevant identity, role, and mapping permissions. Protocol
semantics remain in the owning service and typed ABI page. Mapping remains allocation-free beyond
the existing bounded page allocator and rejects duplicate or exhausted endpoint requests.

## Consequences

- Adding an endpoint declaration no longer adds a generic `Task` storage field or `map_context`
  boolean.
- ABI v4 layout and service-facing typed APIs remain unchanged.
- The current fixed virtual slots remain an internal isolation detail until a later ABI decision.
- A later ABI v5 may replace the adapter without requiring another task lifetime rewrite.
