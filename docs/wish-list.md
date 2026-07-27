# Wish List

This document contains long-term ideas for LogOS and Flow.

Unlike the roadmap, these features are **not commitments**. They are architectural directions and experiments that may prove worthwhile as the project matures.

A feature should only move from this document into the roadmap once it has been demonstrated to provide clear value without compromising the project's simplicity, performance, or reliability.

---

# Guiding Principles

Every feature should ideally satisfy several of these goals:

- AI-native rather than AI-bolted-on.
- Strongly typed and deterministic where possible.
- Capability-aware.
- Observable and debuggable.
- Event-driven rather than polling.
- Minimal runtime overhead.
- Useful independently of AI.

---

# High Priority Ideas

These ideas fit naturally within the current architecture and appear to provide significant long-term value.

---

## Reactive Event Pipelines

Instead of Unix-style polling loops:

```bash
while true; do
    ...
    sleep 1
done
```

Flow could expose native event streams:

```flow
system.services
    |> watch(service => service.status == ServiceStatus::Failed)
    |> notify.send(channel: "ops");
```

Benefits:

- no polling
- lower CPU usage
- cleaner automation
- naturally event-driven

This aligns closely with the LogOS service architecture.

---

## Scoped Capabilities

Capabilities should be able to be narrowed lexically.

```flow
with capability.narrow(storage, ReadOnly) {

    let logs = storage.read(...);

}
```

When execution leaves the scope:

- temporary permissions disappear
- timeouts automatically expire
- accidental privilege retention becomes much harder

This naturally complements Rust's ownership model.

---

## AI Intent Provenance

Every AI action could belong to a higher-level user intent.

```
User Request
      ↓
AI Planning
      ↓
Capability Grants
      ↓
Operations
      ↓
Audit Log
```

Instead of auditing isolated operations, LogOS could understand *why* they occurred.

This enables:

- explainability
- auditing
- trust
- future rollback mechanisms

---

## Crash Recovery & Persistent Diagnostics

Kernel failures should preserve useful debugging information.

Potential information:

- recent trace buffer
- service states
- IPC queues
- task states
- active capabilities
- panic reason

After reboot:

```
Recovery Console

↓

Inspect previous crash

↓

Continue boot
```

This improves reliability without requiring heavyweight debugging infrastructure.

---

# Medium Priority Ideas

Interesting concepts that appear compatible with the architecture but require more exploration.

---

## System-wide Transactions

Certain groups of operations may eventually execute atomically.

```flow
transaction {

    storage.mount(...)

    service.restart(...)

}
```

Possible benefits:

- cleaner rollback
- consistent observable state
- better AI execution
- safer administration

Many practical questions remain:

- hardware interactions
- irreversible operations
- distributed services
- rollback guarantees

This should be explored carefully rather than rushed.

---

## Zero-Downtime Service Handoff

Long-term goal:

Replace running services without rebooting.

Example:

```
Storage v1

↓

Storage v2

↓

Transfer state

↓

Shutdown v1
```

This requires careful state migration and should only be attempted once service architecture has matured.

---

# Low Priority Ideas

Interesting experiments that should not influence the core architecture today.

---

## Typed Reactive State (Signals)

Inspired partly by Angular Signals.

Instead of exposing only event streams, LogOS could expose typed reactive state.

Example:

```flow
let storage = system.service("storage").state;

let degraded = computed(() =>
    storage.status == ServiceStatus::Degraded
);

effect(() => {

    if degraded() {
        notify.send("Storage degraded");
    }

});
```

Signals are **not** intended to replace events.

Instead:

```
Events

↓

Signals

↓

Computed State

↓

Effects
```

Potential advantages:

- simpler UI development
- cleaner long-running automation
- easier state composition

Concerns:

- hidden execution
- dependency complexity
- unnecessary abstraction
- possible runtime overhead

Signals should remain optional rather than becoming a fundamental programming model.

---

## Deterministic Trace Replay

LogOS already benefits from structured tracing.

A future debugging mode could optionally record enough execution context to replay failures.

Possible uses:

- driver debugging
- kernel testing
- race condition analysis

However:

- recording sufficient context is expensive
- deterministic replay is technically difficult
- storage requirements may become significant

Normal tracing should remain lightweight.

Replay should only exist as an explicit debugging mode if it proves worthwhile.

---

## Hardware-backed WASM Enclaves

Future CPUs provide technologies such as:

- AMD SEV-SNP
- Intel TDX

These could eventually protect sensitive WASM workloads with hardware-backed encrypted memory.

This is primarily an enterprise/security feature and should remain a long-term consideration rather than an early project goal.

---

# Rejected Ideas

These concepts were explored but currently do not appear robust enough.

---

## Universal Flow Simulation / Preflight Execution

Originally proposed as a way to simulate a Flow script before execution and display a complete machine state diff.

After further analysis, this does not hold up as a general language feature.

Reasons include:

- runtime conditions may differ
- concurrent state changes
- external services
- hardware interactions
- network responses
- user input

While individual services may expose planning APIs in the future, Flow itself cannot generally predict the exact outcome of arbitrary programs.

Reading the script and relying on the language's strong typing provides a more truthful understanding than presenting a potentially incomplete "simulation."

As a result, universal script simulation is intentionally **not** part of the long-term architecture.

---

# Philosophy

LogOS should avoid accumulating features simply because they are technically interesting.

Every addition should make the system:

- easier to understand
- easier to debug
- safer
- faster
- more deterministic
- more composable

Novelty alone is never sufficient reason to include a feature.