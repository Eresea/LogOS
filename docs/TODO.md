# TODO

> Working list derived from ROADMAP.md, FLOW.md, ARCHITECTURE.md/ONION_RINGS.md, NAMING.md, and study_considerations.md.
> This is a planning aid, not a replacement for ROADMAP.md's authoritative checklists.

## 1. Console v1

Console v1 is complete. Its checklist and future scope live in [CONSOLE.md](CONSOLE.md).

## 2. Next milestone: Platform v1 kickoff

Nothing here is started. Suggested entry order, since later items depend on earlier
ones:

1. Declarative service manifests + dependency graph (everything else in this
   milestone assumes manifests exist)
2. Health checks/heartbeats + restart/backoff/quiesce policy
3. Machine identity, service/process principals, local user principals (unblocks
   `system.identity` moving from Working → exercised-by-real-services)
4. Secret store + entropy/random service (unblocks Vault actually being used)
5. Driver binding policy + capability manifests (turns the `drivers` command above
   into something backed by real policy instead of a Core v1 stub)
6. Run `logos-terminal` as a separately loaded, capability-only service; keep
   framebuffer/PS/2 access and the recovery console kernel-owned.
7. Everything else in the milestone (time, audit, `system.inference`) can follow
   as needed rather than strictly in listed order

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
- [ ] ROADMAP.md references `reviewed/2026-07-26.md` as an annex; that file isn't
      among the current project files, so it couldn't be factored into this plan.
      Worth confirming it's checked into the project.

## 4. Proposed roadmap additions (from study_considerations.md)

None of these are represented in ROADMAP.md yet. Suggested placement per item:

| Idea | Suggested home | Priority read |
|---|---|---|
| **Sensitive\<T\> information-flow typing** (data can't reach a remote-model capability without explicit declassification) | New subsection under Flow's type system (§9) + a Runtime/System capability rule in ARCHITECTURE.md §14 | High — flagged in the source doc as the most distinctive idea; also directly strengthens the existing "no implicit agent privilege" principle already in ROADMAP.md §12 |
| **Compensating actions / universal undo** (commands declare a reversal, generated from existing schema) | Extends Console v1's command model (`CommandDescriptor`) and Store's version/snapshot support — could be a Platform v1 or Applications v1 addition | High — same rationale, cheap given existing effect-classification table in ARCHITECTURE.md §8.8 |
| **Signed action receipts** for multi-agent trust | System.audit extension — portable, verifiable version of existing audit records | Medium — only matters once agent-to-agent delegation is a real scenario, not yet in scope |
| **Deterministic causal replay** | Continuous Core lane (§14) — extends existing Trace/generation-tagged handles | Medium — high debugging value for capability-delegation bugs, but bounded/opt-in like existing trace philosophy |
| **Semantic diff/plan before privileged apply** (system-wide `terraform plan` equivalent) | Extends Flow's existing `flow simulate` (already in Phase 13) to Update, capability grants, and Supervisor policy changes | Medium — natural generalization of work already planned |
| **CHERI-backed capabilities** | Continuous Core lane, as a stated long-term hardware target rather than a near-term milestone | Low near-term / worth a design note now given RISC-V is already a reference target in hardware_ideas.md |
| **Heterogeneous compute as typed `ComputeRef`** | Ring 0/Ring 1 boundary question — would need its own placement-checklist pass per ONION_RINGS.md §12 | Low near-term — no accelerator hardware target committed yet (`system.inference` accelerator binding is explicitly deferred) |
| **Power domains as first-class capability/quota** | Extends existing quiesce/reset driver lifecycle + WASM quota model | Low near-term — most relevant once Phase 2 hardware target (PineTab2) is active |

Recommendation: add a short "Candidate extensions" appendix to ROADMAP.md (or a new
`EXTENSIONS.md` annex alongside ARCHITECTURE.md/NAMING.md) so these aren't only
living in a study notes file. At minimum, #4 (Sensitive\<T\>) and #6 (compensating
actions) seem worth turning into real roadmap line items now, since both piggyback
on infrastructure ROADMAP.md already commits to building.

## 5. Naming register follow-up

Per NAMING.md §10, any new roadmap item above should get a naming pass before
being added for real — in particular "Sensitive" and "compensating action" both
need one-sentence-scope checks against the Reserved Vocabulary table before they
turn into actual type/command names.
