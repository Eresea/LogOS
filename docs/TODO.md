# TODO

> Working list derived from ROADMAP.md, FLOW.md, ARCHITECTURE.md, NAMING.md, and
> study_considerations.md. This is a planning aid, not a replacement for ROADMAP.md's
> authoritative checklists.

## 1. Implemented bootstrap

- [x] `logos-terminal` owns the bounded terminal model, input normalization, framebuffer rendering,
      and font rasterization.
- [x] `logos-terminal-service` runs as a separately loaded Ring-3 payload through a bounded Core
      gate.
- [x] Platform bootstrap code provides static typed service specifications, manifests, lifecycle
      policy, capability grants, identities, time, entropy, secrets, audit, and driver binding.
- [x] The kernel-owned recovery console remains a direct, independent fallback.
- [x] ABI v4 typed endpoint pages and the canonical native-service specification are recorded in
      ADR-0020 and consumed by payload staging, supervisor planning, and service lookup.
- [x] The QEMU harness uses suite-selected runners rather than root-level scenario-ID branches.

## 2. Current milestone: ABI v4 stabilization and migration closeout

Network v1 typed transport is implemented, but its full QEMU closure and the Remote Foundation
proofs remain open. ABI v4 restructuring is not frozen. The registered proof IDs remain permanent
regression contracts; the fixed-seed QEMU run records completed-layer passes and current boundary
failures.

1. [x] Define the minimal versioned Input, Display, and Session capability contracts required by
   `logos-terminal`.
2. [x] Replace the bounded Core context gate with capability-scoped typed endpoint contracts.
3. [x] Move normal command/session dispatch out of Core while keeping privileged execution
   capability-gated.
4. [x] Prove Terminal and Sessions failure, restart, capability denial, and recovery handoff in
   the supported QEMU harness when available.
5. Keep the recovery console kernel-owned while Remote Foundation advances.

### Phase 3 Network typed-page and client transport gap

- [x] Replace the legacy Network-client page boundary with scalar-validated, generation-safe typed
      Network pages and restore the canonical service mapping.
- [x] Complete one global Terminal/Gateway client transaction with transactional rollback,
      configured transfer-page authority, exact completion matching, timeout/cancel/reset handling,
      and replacement invalidation.
- [ ] Close the remaining QEMU Network-client scenarios after the service/device scheduling
      boundary is repaired.

### Phase 5 cleanup

- [x] Move Network readiness ownership into `NetworkRuntime`'s direct server `Status` transaction.
- [x] Make every production Network reply wake and run its blocked caller for every status.
- [x] Give white-box QEMU probes an explicit test-only completion target.
- [x] Move proof state semantics out of the platform composition root.
- [x] Split QEMU catalogs and suite runner policy by suite.
- [x] Use the canonical service endpoint set directly for page mapping.
- [x] Split the Remote ABI protocol into `service/remote.rs`.
- [x] Privatize Network and Remote runtime state behind accessors.
- [x] Enforce inward package-ring imports in `scripts/arch-deps.py`.
- [x] Reconcile milestone records with the fixed-seed QEMU evidence and deferred migration IDs.

### Next bounded cycles

- [x] Stabilization cycle: validate and repair the completed typed layers against the main suite;
      fixed-fixture QEMU run records 43 passed, 12 migration-deferred, and 28 skipped.
- [ ] **Network-client cycle:** re-run the QEMU Network/Remote proofs after the completed readiness
      ownership and production caller-wake repair; close the remaining scheduling boundary.
- [ ] Remote cycle: complete one bounded Remote transport migration after Network-client completion.
- [ ] Stop restructuring ABI v4 after those cycles; require a new ADR for any exception.
- [ ] Resume capability development: complete Remote verification or begin Safe System
      Artifacts / Persistence v2.

## 3. Documentation debt

- [ ] Reconcile the deferred FLOW.md Phase 0 charter when that milestone is scheduled.
- [ ] Graduate `system.inference` and `session.remote` from Candidate to Working only when their
      milestones begin.

## 4. Proposed roadmap additions

Keep the following as candidates until their target milestone begins:

| Idea | Suggested home | Priority |
| --- | --- | --- |
| Signed action receipts for multi-agent trust | System.audit extension | Medium |
| Deterministic causal replay | Continuous Core trace lane | Medium |
| Semantic diff/plan before privileged apply | Flow simulation and Update | Medium |
| CHERI-backed capabilities | Long-term Core hardware target | Low near-term |
| Heterogeneous compute as typed `ComputeRef` | Ring 0/Ring 1 placement pass | Low near-term |
| Power domains as first-class capability/quota | Driver lifecycle and WASM quotas | Low near-term |

## 5. Naming register follow-up

Per NAMING.md §10, any new roadmap item gets a naming pass before it becomes a public type or
command. In particular, “Sensitive” and “compensating action” need one-sentence scope checks
against the Reserved Vocabulary table.
