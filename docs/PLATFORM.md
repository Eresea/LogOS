# Platform

> **Status:** Bootstrap mechanisms and Ring-3 gate round trips implemented; suspended service execution pending
> **Owner:** Foundation and System

Platform currently provides the bootstrap mechanisms for supervised native services: identities,
capabilities, health, and versioned contracts. They still execute in the UEFI image until the native task loader and service boundary exist.

## Implemented bootstrap

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
- [ ] Load `logos-terminal` as a separately executable Sessions service with only input/display/session capabilities.

### Exit evidence

- [x] Bootstrap services negotiate explicit contracts and start from manifests.
- [x] Requests carry principal and capability context; failures reclaim and restart without reboot.
- [x] QEMU verifies bootstrap normal boot, startup rejection, dependency loss, runtime recovery, and replacement.
- [ ] QEMU proves an independently loaded native service can start, fail, restart, and lose its capabilities without compromising Core.

## V2 — Unplanned

Record future Platform scope here before adding it to the roadmap. Candidate work includes a
platform controller that owns lifecycle wiring, host-side tests for policy modules, richer audit
metadata, broker operations for secrets, and proof with multiple drivers.
