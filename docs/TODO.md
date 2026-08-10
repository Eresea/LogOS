# TODO

This is the active implementation list. Current contracts and proof evidence live in the linked
subsystem documents; completed work and old milestone ledgers are kept in `reviewed/` and ADRs.

## Current milestone: ABI v5 bounded async migration

- [x] Repair the Network suite proofs: `network/simultaneous-client-busy` and
      `network/tcp-stream` pass with structured contention, exact-byte, and watermark assertions.
- [x] Add bounded/fair per-connection TCP TX scheduling without making Gateway or Remote part of
      Network scheduling; broader RX/concurrent-operation work remains deferred.
- [ ] Prove Gateway and `logosctl` through the real TCP foundation; current Remote auth stops after
      the gate denial/close path.
- [ ] Implement the five skipped Remote proofs with real multi-boot/reconnect/restart orchestration
      and precise persisted-state postconditions.
- [ ] Freeze ABI v5 structure only after Network, Remote, Storage, Gateway, and ownership gates pass.

## Async-first follow-up

- [x] Sessions owns bounded `Deliver -> EffectPending -> ReplyPending` state with a typed
      owner/generation/request identity; scheduler composition consumes only its runnable hint.
- [ ] Convert Storage relay and protected Remote persistence into bounded request/block/completion
      phases without changing durable commit semantics or page-loan cleanup; current wrappers still
      synchronously drive dependent services.
- [x] Define observable sequence/generation conventions in the ABI v5 operation token and completion envelope.
- [ ] Replace bootstrap Network global wait slots only after the multi-connection proof.
- [ ] Move Gateway from the single connection loop to a fixed connection table with per-phase backpressure.

All boundary work uses the [bounded task contract template](task-contract-template.md). Host
acceptance tests cover local state and ownership; QEMU/fault proofs are selected by the modified
isolation seam rather than crate ownership.

## Documentation debt

- [ ] Reconcile the deferred FLOW.md Phase 0 charter when that milestone is scheduled.
- [ ] Graduate `system.inference` and `session.remote` from Candidate to Working only when their
      milestones begin.

## Deferred candidates

Long-term ideas remain in [reviewed roadmap candidates](../reviewed/roadmap-candidates.md); they are
not active implementation work.
