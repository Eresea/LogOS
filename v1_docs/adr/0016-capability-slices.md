# ADR-0016: Compose capabilities from independently versioned module contracts

- Status: Accepted
- Date: 2026-08-02

## Context

The linear module roadmap hides cross-module prerequisites and encourages unrelated implementations to advance together. Remote administration, for example, needs slices of Network, Platform, Persistence, and Sessions before the Remote product layer can remain thin.

Parallel work should conflict only when a published boundary changes, not whenever either implementation changes.

## Decision

- The system roadmap tracks user-visible capabilities composed from named module slices.
- Each module owns its implementation, state, lifecycle, and versioned contracts.
- A module version identifies a published contract and behavioral guarantees. Compatible implementation changes keep the version.
- Consumers depend on contract types and negotiate supported versions at discovery; they do not depend on provider implementation crates or private state.
- Incompatible contracts receive a new version. Compatibility adapters live at the provider edge, and coexistence is required only by an explicit migration or rollback proof.
- Each slice is independently testable. A capability adds the smallest composition proof across its pinned slices.
- Module versions and branches do not advance in lockstep.

## Consequences

- Network v2, Platform v2, Persistence v2, and Sessions v2 slices may land separately before the Remote Foundation proof.
- Internal rewrites can be swapped without changing consumers when the contract proofs remain green.
- Contract changes still require coordinated compatibility work and an integration proof.
- This does not require a universal RPC layer, dynamic loading everywhere, one contract per function, or permanent support for every historical version.

## Alternatives considered

- Keep a linear module roadmap — rejected because it obscures prerequisites and couples planning without reducing runtime coupling.
- Version every implementation revision — rejected because it turns private code churn into needless integration work.
- Require all versions to coexist — rejected because most migrations do not need the cost.
