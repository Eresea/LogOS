# ADR-0002: Service restart operation outcomes

- Status: Accepted
- Date: 2026-08-12

## Context

The scheduler uses slot generations to prevent a stale task handle from affecting a later task in
the same fixed slot. That safety epoch is not a service-delivery guarantee. A service may fail and
restart while requests from the old instance are still in flight, so late completions need an
explicit operation outcome rather than being mistaken for successful work or silently retried.

## Decision

- Each service-owned operation occupies one fixed slot and records its request identity and service
  epoch.
- Restart advances the service epoch and transitions every in-flight operation to `Restarted`.
- Completions from the old epoch, wrong request identity, terminal operations, or reclaimed slots are
  rejected.
- The owner must reclaim terminal operation records before their slots can be reused.
- No automatic retry is performed. The owner may retry only when the operation is idempotent and its
  request identity makes replay safe.
- Durable work must keep its resumable state outside the replaceable service instance.
- Scheduler slot generations and service epochs remain separate concepts and are not interchangeable.

## Consequences

Late work is safely rejected without claiming that the job succeeded. A caller can distinguish a
service restart from ordinary cancellation and choose retry or failure according to the operation's
contract. Fixed capacity remains observable: exhaustion returns `Capacity`, and unreclaimed restart
records continue to consume slots.

`service_lifecycle::ServiceLifecycle` is the host-tested contract model. It does not add service
loading, IPC, persistence, retries, or a generic executor; those require a concrete service
milestone and their own proofs.

## Deferred

Service replacement, durable retry journals, idempotency contracts, IPC transport, capability
validation, and QEMU restart proofs remain deferred until the first real service is introduced.
