# TODO

> Working list derived from ROADMAP.md, FLOW.md, ARCHITECTURE.md/ONION_RINGS.md, NAMING.md, and study_considerations.md.
> This is a planning aid, not a replacement for ROADMAP.md's authoritative checklists.

## 1. Implemented bootstrap

- [x] `logos-terminal` owns the bounded terminal model, input normalization, framebuffer rendering, and font rasterization.
- [x] `logos-terminal-service` runs as a separately loaded Ring-3 payload through a bounded Core gate.
- [x] Platform bootstrap code provides static manifests, lifecycle policy, capability grants, identities, time, entropy, secrets, audit, and driver binding.
- [x] The kernel-owned recovery console remains a direct, independent fallback.

## 2. Current milestone: Remote Foundation v1

Network v1 is complete. Remote Foundation v1 is tracked in [REMOTE.md](REMOTE.md) and composes the
smallest Network v2, Platform v2, Persistence v2, and Sessions v2 slices.

1. [x] Define the minimal versioned Input, Display, and Session capability contracts required by `logos-terminal`.
2. [x] Replace the bounded Core context gate with those contracts without granting raw framebuffer or PS/2 access.
3. [x] Move normal command/session dispatch out of Core while keeping privileged execution capability-gated.
4. [x] Prove Terminal and Sessions failure, restart, capability denial, and recovery handoff in headless QEMU.
5. Keep the recovery console kernel-owned while Network advances.

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

| Idea                                                                                     | Suggested home                                                                                                            | Priority read                                                                                                                |
| ---------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------- |
| **Signed action receipts** for multi-agent trust                                         | System.audit extension — portable, verifiable version of existing audit records                                           | Medium — only matters once agent-to-agent delegation is a real scenario, not yet in scope                                    |
| **Deterministic causal replay**                                                          | Continuous Core lane (§14) — extends existing Trace/generation-tagged handles                                             | Medium — high debugging value for capability-delegation bugs, but bounded/opt-in like existing trace philosophy              |
| **Semantic diff/plan before privileged apply** (system-wide `terraform plan` equivalent) | Extends Flow's existing `flow simulate` (already in Phase 13) to Update, capability grants, and Supervisor policy changes | Medium — natural generalization of work already planned                                                                      |
| **CHERI-backed capabilities**                                                            | Continuous Core lane, as a stated long-term hardware target rather than a near-term milestone                             | Low near-term / worth a design note now given RISC-V is already a reference target in hardware_ideas.md                      |
| **Heterogeneous compute as typed `ComputeRef`**                                          | Ring 0/Ring 1 boundary question — would need its own placement-checklist pass per ONION_RINGS.md §12                      | Low near-term — no accelerator hardware target committed yet (`system.inference` accelerator binding is explicitly deferred) |
| **Power domains as first-class capability/quota**                                        | Extends existing quiesce/reset driver lifecycle + WASM quota model                                                        | Low near-term — most relevant once Phase 2 hardware target (PineTab2) is active                                              |

Keep the remaining proposals as candidates until their target milestone begins.

## 5. Naming register follow-up

Per NAMING.md §10, any new roadmap item above should get a naming pass before
being added for real — in particular "Sensitive" and "compensating action" both
need one-sentence-scope checks against the Reserved Vocabulary table before they
turn into actual type/command names.

--Review 20260803

This file's structure tells its own story pretty clearly. Here's what stands out, in priority order:

**1. `main()` is a ~4,700-line function, and that's the core problem**

The file has 38 functions total but only ends around line 5200 — `main` alone appears to run from line 30 to roughly line 4000+. Everything else (`replace_terminal`, `replace_storage`, `replace_network`, `restart_native_service`, the hex helpers, `task_a`/`task_b`) is bolted on after it. A function this size in a kernel entry point is a maintainability and reviewability risk regardless of language: you can't reason locally about any 20-line chunk of it because state (memory, capabilities, scheduler, service handles) is all shared through one enormous local scope.

Concretely, I'd split `main` along the phase boundaries that are already visible in the code:

- boot/console/health self-check phase (lines ~40–270)
- memory + virtual memory + address space bring-up (~110–230)
- native service image mapping (terminal/sessions/storage) — this is copy-pasted three times (lines ~169–206), which is a strong signal it wants to be a helper: `fn map_native_image(memory, name: &[u8], image: Option<Image>) -> Result<...>`
- capability granting (~356–410) — this is a long flat sequence of `capabilities.grant(...)` / `grant_scoped(...)` calls with identical `else { fail!(...) }` shapes; a small table-driven helper or macro would cut this in half and make it obvious at a glance which capabilities exist
- device discovery/binding (PCI, block, network)
- service/session/endpoint wiring
- scheduler handoff

Each phase should return a `Result`/`Option` struct that the next phase consumes, rather than all of it living as flat `let` bindings in one scope. That also makes `main` itself read like a table of contents.

**2. `check!`/`fail!` macros hide a state machine that should be explicit**

The `health.check(...)` / `health.fail(...)` pattern combined with the `let Some(x) = ... else { fail!(...) }` idiom is used ~60+ times. It works, but it means boot failure handling is scattered as a side effect inside expressions rather than being a visible control-flow structure. Since you're already using `Option`/`?`-like early-return patterns, consider whether `health.fail` could just be folded into a `BootError` enum and propagated with `?`, with the health-check recording done in one place (e.g., a `From<BootError> for HealthFailure` at the top level). That would let you delete the macros entirely and get normal Rust error propagation instead of a bespoke checkpoint system threaded through 4000 lines.

**3. Hardcoded network config buried in kernel.rs**

`4000`, `4001`, `0x0a00_0202` (a literal IPv4 address encoded as a magic constant) appear over 20 times, both in the capability-granting block near the top and scattered through what looks like protocol test/exercise code around lines 2200–2750. These belong in named constants (`const GATEWAY_PORT: u16 = 4000;`, `const PEER_ADDR: u32 = 0x0a00_0202; // 10.0.2.2`) at minimum, and arguably in a config/test-fixture module rather than inline in the kernel entry point — especially since `0x0a00_0202` reads like a QEMU user-mode-networking gateway address, which suggests this is dev/test wiring that's currently indistinguishable from production boot logic.

**4. Self-test / harness code is interleaved with real boot logic**

`task_a`/`task_b` (trivial demo tasks), the terminal exercise sequence at lines 240–266 (feeding a `'k'` keypress and an escape byte to prove the terminal task responds), and the network protocol exercises around 2200–2750 all look like integration tests, but they run unconditionally inside `main` on every boot rather than being confined to `#[cfg(test)]` or the existing `test-hooks` feature. You already have a `test-hooks` cfg flag used elsewhere in the file (lines 14, 17, 155–159) — it'd be worth auditing whether _all_ of the self-exercising code (not just the terminal-payload-missing branch) should be gated behind it, both for boot-time cost and for keeping production kernel logic separate from validation logic.

**5. 43 `unsafe` blocks with no visible safety comments in what I sampled**

For `unsafe { core::ptr::copy_nonoverlapping(...) }` (line ~4997) and similar raw-pointer ops, a `// SAFETY: ...` comment above each block explaining the invariant being relied on (alignment, non-overlap, valid-for-reads/writes length) is standard practice in Rust kernels/embedded code and is worth enforcing project-wide — `#![deny(unsafe_op_in_unsafe_fn)]` plus a `cargo clippy -- -W clippy::undocumented_unsafe_blocks` pass would catch the gaps mechanically rather than needing a manual sweep of 43 sites.

**6. Repeated `AddressSpace::new(&mut memory)` + `release` pattern**

This shows up at least 6 times (lines 141, 169, 179, 193, 211, 221, 224) with the same shape: allocate a scratch address space, do one operation, check invariants, release. That's a good candidate for a `with_scratch_address_space(&mut memory, |space| { ... })` closure-based helper — it would also make it structurally impossible to forget the `release` call on an early-return path, which right now relies on every call site remembering to chain `.release()` into the `check!`/`else` correctly.

---

Given this is one file in a ~30-file split, the highest-leverage single change is probably #1 (breaking up `main`) since it's what makes everything else (macro hygiene, config extraction, test/prod separation) easier to do safely afterward — right now any refactor of a 4000-line function is inherently riskier than refactoring smaller units.

If you want, I can go through the middle section (lines ~900–4900) in more detail — I've only sampled the beginning, one mid-section, and the tail so far — particularly the device/service wiring and the `replace_*` recovery functions near the end, which have some interesting error-handling asymmetries (e.g. `replace_storage` silently swallows failures differently than `replace_terminal`).
