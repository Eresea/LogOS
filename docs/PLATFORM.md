# Platform

> **Status:** Native Ring-3 terminal and Sessions boundaries implemented; failure/restart proofs remain
> **Owner:** Foundation and System

Platform provides the bootstrap mechanisms for supervised native services: identities,
capabilities, health, versioned contracts, isolated address spaces, and Core-owned suspension at service gates. The normal terminal is the first separately loaded native service; general capability-only service contracts remain pending.

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
- [x] `ping` completes a capability-gated IPC round trip to the platform service.
- [x] `restart <target>` and `cancel <target>` preserve a typed target; Platform validates it.
- [x] Load `logos-terminal-service` as a separately executable Ring-3 service with a bounded bootstrap context gate.
- [x] Gate terminal input, presentation, and typed syscalls with explicit Input, Display, and Session capabilities.
- [x] Return typed privileged-effect results to Sessions for reply formatting and forwarding.

### Exit evidence

- [x] Bootstrap services negotiate explicit contracts and start from manifests.
- [x] Requests carry principal and capability context; failures reclaim and restart without reboot.
- [x] QEMU verifies bootstrap normal boot, startup rejection, dependency loss, runtime recovery, and replacement.
- [x] QEMU proves an independently loaded terminal service can start, redraw, and execute bounded commands.
- [ ] QEMU proves service failure, restart, and capability denial without compromising Core.

## V2 — Unplanned

Record future Platform scope here before adding it to the roadmap. Candidate work includes a
platform controller that owns lifecycle wiring, host-side tests for policy modules, richer audit
metadata, broker operations for secrets, and proof with multiple drivers.
