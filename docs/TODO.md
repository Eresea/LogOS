# TODO

> Working list derived from ROADMAP.md, FLOW.md, ARCHITECTURE.md/ONION_RINGS.md, NAMING.md, and study_considerations.md.
> This is a planning aid, not a replacement for ROADMAP.md's authoritative checklists.

## 1. Implemented bootstrap

- [x] `logos-terminal` owns the bounded terminal model, input normalization, framebuffer rendering, and font rasterization.
- [x] `logos-terminal-service` runs as a separately loaded Ring-3 payload through a bounded Core gate.
- [x] Platform bootstrap code provides static manifests, lifecycle policy, capability grants, identities, time, entropy, secrets, audit, and driver binding.
- [x] The kernel-owned recovery console remains a direct, independent fallback.

## 2. Current milestone: Platform v1 service boundary

The execution boundary exists. The remaining work is replacing its bootstrap transport with service contracts:

1. Define the minimal versioned Input, Display, and Session capability contracts required by `logos-terminal`.
2. Replace the bounded Core context gate with those contracts without granting raw framebuffer or PS/2 access.
3. Move normal command/session dispatch out of Core while keeping privileged execution capability-gated.
4. Prove terminal-service failure, restart, capability denial, and recovery handoff in headless QEMU.
5. Only then move further bootstrap services out of `logos-uefi`; keep the recovery console kernel-owned.

## 3. Documentation debt (cheap, should happen soon)

- [ ] FLOW.md §33 Phase 0 checkboxes are all unchecked despite §35 already recording
      several "Approved" decisions (pipeline operator, enum syntax, no-null policy,
      named arguments, etc.). Reconcile: mark Phase 0 items complete where §35
      already settled them, or move settled items out of "Open Decisions" if they're
      actually Phase 0 charter items.
- [ ] NAMING.md has two **Candidate** entries (`system.inference`, `session.remote`)
      sitting in an otherwise mostly-**Working**/**Reserved** table. Worth deciding
      whether they graduate to Working now or stay pinned until Platform v1 /
      Remote v1 actually start — right now it's ambiguous which.
- [x] `docs/reviewed/2026-07-26.md` is present; its accepted decisions are reflected
      in the roadmap and architecture documents.

## 4. Proposed roadmap additions (from study_considerations.md)

Information-flow labels and compensating actions are now roadmap items. The remaining ideas are candidates; suggested placement follows:

| Idea | Suggested home | Priority read |
|---|---|---|
| **Signed action receipts** for multi-agent trust | System.audit extension — portable, verifiable version of existing audit records | Medium — only matters once agent-to-agent delegation is a real scenario, not yet in scope |
| **Deterministic causal replay** | Continuous Core lane (§14) — extends existing Trace/generation-tagged handles | Medium — high debugging value for capability-delegation bugs, but bounded/opt-in like existing trace philosophy |
| **Semantic diff/plan before privileged apply** (system-wide `terraform plan` equivalent) | Extends Flow's existing `flow simulate` (already in Phase 13) to Update, capability grants, and Supervisor policy changes | Medium — natural generalization of work already planned |
| **CHERI-backed capabilities** | Continuous Core lane, as a stated long-term hardware target rather than a near-term milestone | Low near-term / worth a design note now given RISC-V is already a reference target in hardware_ideas.md |
| **Heterogeneous compute as typed `ComputeRef`** | Ring 0/Ring 1 boundary question — would need its own placement-checklist pass per ONION_RINGS.md §12 | Low near-term — no accelerator hardware target committed yet (`system.inference` accelerator binding is explicitly deferred) |
| **Power domains as first-class capability/quota** | Extends existing quiesce/reset driver lifecycle + WASM quota model | Low near-term — most relevant once Phase 2 hardware target (PineTab2) is active |

Keep the remaining proposals as candidates until their target milestone begins.

## 5. Naming register follow-up

Per NAMING.md §10, any new roadmap item above should get a naming pass before
being added for real — in particular "Sensitive" and "compensating action" both
need one-sentence-scope checks against the Reserved Vocabulary table before they
turn into actual type/command names.
