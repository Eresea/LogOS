# Platform

> **Status:** Platform v1 complete
> **Owner:** Foundation and System

Platform provides the bootstrap mechanisms for supervised native services: identities,
capabilities, health, versioned contracts, isolated address spaces, Core-owned suspension, and bounded restart at service gates. The Terminal and Sessions payloads are the first separately loaded native services; broader service extraction remains future work.

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
- [x] QEMU proves Terminal and Sessions failure, restart, capability denial, and direct recovery availability without compromising Core.

## V2 — Trust and control plane

- Durable machine and device identity, enrollment, and trust policy.
- Secret broker operations instead of direct secret access.
- Durable structured audit records.
- Host-testable authorization and lifecycle policy.
- A platform controller that owns service lifecycle wiring.
- Replacement proof with multiple interchangeable drivers.

Remote Foundation may consume the identity, enrollment, secret-broker, and audit slices before V2 is complete.

## V3 — Dynamic system composition

- Dynamic manifests and dependency changes.
- Zero-downtime service handoff and versioned state migration.
- Driver hotplug and replacement policy.
- Policy-controlled service placement and resource allocation.
