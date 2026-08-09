# TODO

This is the active implementation list. Current contracts and proof evidence live in the linked
subsystem documents; completed work and old milestone ledgers are kept in `reviewed/` and ADRs.

## Current milestone: ABI v4 stabilization and migration closeout

- [ ] Repair the remaining Network suite proof `network/simultaneous-client-busy` after the
      Gateway second-client scheduling boundary is fixed.
- [ ] Add queued per-connection Network work and bounded RX/TX service budgets without making
      Gateway or Remote part of Network scheduling.
- [ ] Keep the independent `network/tcp-stream` proof green, then prove Gateway and `logosctl`
      through the real TCP foundation.
- [ ] Implement the five skipped Remote proofs with real multi-boot/reconnect/restart orchestration
      and precise persisted-state postconditions.
- [ ] Freeze ABI v4 structure after the Network and Remote cycles; require a new ADR for exceptions.

## Async-first follow-up

- [ ] Convert Sessions relay work into bounded `Deliver -> EffectPending -> ReplyPending` state.
- [ ] Convert Storage relay and protected Remote persistence into bounded request/block/completion
      phases without changing durable commit semantics or page-loan cleanup.
- [ ] Define observable sequence/generation conventions where existing APIs need resynchronization.
- [ ] Replace bootstrap Network global wait slots only after the multi-connection proof.

## Documentation debt

- [ ] Reconcile the deferred FLOW.md Phase 0 charter when that milestone is scheduled.
- [ ] Graduate `system.inference` and `session.remote` from Candidate to Working only when their
      milestones begin.

## Deferred candidates

Long-term ideas remain in [reviewed roadmap candidates](../reviewed/roadmap-candidates.md); they are
not active implementation work.
