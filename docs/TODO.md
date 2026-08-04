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

## 2. Current milestone: Remote Foundation v1

Network v1 is complete. Remote Foundation v1 is tracked in [REMOTE.md](REMOTE.md); its Gateway,
Sessions, protected-state, and typed remote-gate implementation is present. The registered remote
proofs remain permanent regression contracts and need a QEMU/OVMF run whenever the environment
provides those tools.

1. [x] Define the minimal versioned Input, Display, and Session capability contracts required by
   `logos-terminal`.
2. [x] Replace the bounded Core context gate with capability-scoped typed endpoint contracts.
3. [x] Move normal command/session dispatch out of Core while keeping privileged execution
   capability-gated.
4. [x] Prove Terminal and Sessions failure, restart, capability denial, and recovery handoff in
   the supported QEMU harness when available.
5. Keep the recovery console kernel-owned while Remote Foundation advances.

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
