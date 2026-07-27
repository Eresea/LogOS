# Platform

> **Status:** Platform v1 complete  
> **Owner:** Foundation and System

Platform turns Core mechanisms into supervised native services with explicit identities,
capabilities, health, and versioned contracts.

## V1 — Complete

### Service supervision

- [x] Static declarative manifests, bounded dependency ordering, and profile selection.
- [x] Manifest capability grants plus protocol and ABI negotiation.
- [x] Heartbeats, bounded restart/backoff, quiesce, shutdown, and failed-start diagnostics.
- [x] Service replacement and owned-resource reclamation without driver-specific supervisor policy.

### Machine services

- [x] Stable machine identity with explicit volatile fallback; UEFI entropy and RTC wall-clock states.
- [x] Service, process, and local-user principals with capability-scoped operations.
- [x] Monotonic time, scoped secrets, audit sequencing, and expiring/revocable approvals.
- [x] Typed resource discovery and scoped inference invocation grants.

### Driver and device model

- [x] Versioned input, display, block, network, entropy, and memory interface classes.
- [x] Manifest-driven driver binding and capability declarations.
- [x] Explicit DMA queue, interrupt, device-state, reset, rebind, and replacement ownership.

### Terminal integration

- [x] `services` and `drivers` render Platform-owned status.
- [x] `restart <target>` and `cancel <target>` preserve a typed target; Platform validates it.

### Exit evidence

- [x] Services negotiate explicit contracts and start from manifests.
- [x] Requests carry principal and capability context; failures reclaim and restart without reboot.
- [x] QEMU verifies normal boot, startup rejection, dependency loss, runtime recovery, and replacement.

## V2 — Unplanned

Record future Platform scope here before adding it to the roadmap. Candidate work includes a
platform controller that owns lifecycle wiring, host-side tests for policy modules, richer audit
metadata, broker operations for secrets, and proof with multiple drivers.
