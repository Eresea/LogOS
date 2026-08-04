# ADR-0020: Use typed native-service endpoint pages and canonical service specifications

- Status: Accepted
- Date: 2026-08-04

## Context

Native services had one untyped ABI-v3 context page. The same page carried control state,
display operations, session requests, persistence requests, block requests, network traffic, and
remote-gate traffic. Boot manifests, payload staging, and service lookup also repeated semantic
metadata in different modules. The large kernel runtime consequently knew too much about every
replaceable service and the QEMU harness dispatched several proofs by root-level scenario IDs.

## Decision

- The native transport ABI is v4. Every service receives one dedicated mapped `ControlPage`; typed
  `InputPage`, `DisplayPage`, `SessionPage`, `StoreEndpointPage`, `BlockEndpointPage`, `NetworkPage`,
  and `RemotePage` contracts use explicit scalar states, generations, and bounded validation.
- ABI v4 is an atomic migration. ABI-v3 compatibility aliases, adapters, and parallel registries
  are not provided.
- `src/platform/services.rs` is the canonical typed `ServiceSpec` source. Supervisor planning,
  service lookup, and payload header validation consume that source; consumers do not duplicate
  service names, protocols, capabilities, restart policy, or endpoint-page sets. The control page is
  implicit; `ServiceSpec::endpoints` lists only the additional typed pages granted to each service.
- `kernel.rs` remains the privileged boot-facing entry boundary. Its bootstrap composition,
  health gating, privileged setup, and top-level loop are implemented in `src/platform/runtime.rs`;
  Core still owns mappings, capabilities, scheduling, device effects, and endpoint reclamation.
- QEMU proof registration selects a suite runner at registration time. A normal proof adds its
  suite-local catalog entry and implementation without adding root-level scenario-ID branches.

## Consequences

- A v4 payload must be rebuilt and staged with the v4 header. Existing v3 payloads are rejected.
- Endpoint pages remain statically bounded and service-specific; no generic RPC framework,
  trait-object service registry, or dynamic endpoint allocation is introduced.
- Completed proof IDs and their semantic output remain unchanged and are regression contracts.
- A future ABI change requires a new ADR and an atomic migration or a documented compatibility
  milestone.
