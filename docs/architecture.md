# LogOS Architecture

> **Status:** Active contract
> **Updated:** 2026-08-10

This is the compact source of truth for current ownership, dependency direction, and native-service
transport. Read [the documentation map](README.md) before opening optional annexes.

## Current state

LogOS is a `no_std` Rust UEFI kernel with a small privileged Core and replaceable services. Console,
Platform, Persistence, and Network bootstrap slices are implemented. Scalable Network work and the
remaining Remote proofs are still open; ABI v5 is not frozen until those ownership and proof gates pass.

## Rings

| Ring | Owns | Must not own |
| --- | --- | --- |
| 0 Core | CPU/interrupts, scheduling, physical pages, mappings, IPC, capabilities, DMA, recovery | policy, durable objects, user-facing protocol state |
| 1 Foundation | device drivers and typed device-facing interfaces | system policy or client session state |
| 2 System | supervision, identity, Store, Network, audit, update, trust policy | raw hardware mechanisms or shell behavior |
| 3 Sessions | sessions, terminal, commands, remote client attachment | privileged effects, raw devices, durable storage internals |
| 4 Runtime | WASM execution, packages, application host interfaces | kernel authority or unrestricted device access |
| 5 Experience | compositor, desktop shell, graphical clients | recovery-critical authority |

Rings describe dependency, trust, and failure boundaries; they are not CPU privilege levels or
mandatory process boundaries. Outer code depends inward through typed, capability-scoped contracts.

## Active boundary rules

- Core owns hardware, memory, scheduling, IPC, capability checks, page loans, mappings, and recovery.
- Services own higher-level policy and durable state and may be independently loaded, suspended,
  restarted, and replaced.
- Every long-lived operation is bounded state owned by the subsystem that advances it. Notifications
  are hints; readiness and completion state are authoritative. Scheduler composition stays above services.
- Generation, owner, request ID, deadline, cancellation, and replacement checks prevent stale work from
  affecting a new service instance.
- Fixed capacities are public behavior: return a bounded error such as `Busy` or `Full`; do not grow
  resources implicitly or add an allocator/runtime dependency.
- Production builds expose no test control surface. Host tests prove portable state and codecs; QEMU
  proves boot, devices, isolation, recovery, and public contracts.

## ABI v5 native services

Each service receives one mapped `logos_abi::service::ControlPage` plus the typed endpoint pages named
by its canonical `platform::services::ServiceSpec::endpoints`. Core maps, validates, loans, revokes,
and reclaims pages; services never receive physical addresses.

Active typed pages include Input, Display, Session client/server, Effect, Store client/server, Storage
Block, Network device/event/client/server/stream, and Remote pages. Page state is scalar-validated and
generation-checked. ABI-v4 payloads and descriptors are rejected at startup; there is no compatibility
adapter or dynamic endpoint registry in the active path. Long-lived work carries a bounded
`OperationToken` and `CompletionEnvelope`; only the owning scheduler may advance a token, and a
terminal phase releases any page loan exactly once.

Service replacement advances the generation before releasing the old address space. Stale handles,
pages, replies, and completions are rejected and owned resources are reclaimed exactly once.

## Boot and failure

UEFI transfers control to Core, which establishes memory, interrupts, capabilities, drivers, and the
recovery console before composing optional services. Foundation devices are bound before dependent
System services; Sessions and the normal terminal are loaded only after their contracts negotiate.
Missing or failed optional services leave local recovery usable. Terminal failure falls back to the
kernel recovery console. Service faults are contained and restart from clean payload state.

Detailed ordering and recovery constraints live in [Boot sequence](boot-sequence.md); authority and
isolation constraints live in [Security](security.md).

## Current subsystem boundaries

- [Platform](PLATFORM.md) supervises manifests, capabilities, health, and restart.
- [Persistence](PERSISTENCE.md) owns bounded named objects and atomic recovery on the raw block device.
- [Network](NETWORK.md) owns protocol state and bounded TCP/UDP endpoints; Core owns NIC/DMA.
- [Remote](REMOTE.md) owns trust/enrollment and the structured attachment above Network and Sessions.
- [Console](CONSOLE.md) and [Sessions](SESSIONS.md) own local interaction, not privileged mechanisms.

## Placement test

Before adding a component, record its invariant, owned resources, required capabilities, failure radius,
restart behavior, recovery requirement, outward contract, and host/QEMU proof. If those answers are
unclear, the boundary is not ready. Cross-ring or irreversible decisions require an [ADR](adr/README.md).

## References

- [Roadmap](roadmap.md) — current sequence and exit targets.
- [Milestone policy](MILESTONE-POLICY.md) — completion and documentation rules.
- [Onion rings](ONION_RINGS.md) — optional placement rationale.
- [ADR-0020](adr/0020-typed-native-endpoint-pages.md), [ADR-0028](adr/0028-async-first-subsystem-state.md),
  [ADR-0032](adr/0032-bounded-task-contracts-and-proof-tiers.md) — active transport and work-state decisions.
