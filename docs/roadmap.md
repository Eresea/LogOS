# LogOS Roadmap

> **Status:** Living document
> **Updated:** 2026-07-24
> **Current milestone:** Core v1 complete
> **Primary target:** A remotely operable, capability-based Rust OS with replaceable native services and sandboxed WASM applications.

## 1. Vision

LogOS is a small, safe, efficient operating system built in Rust.

The kernel is deliberately minimal. It provides privileged mechanisms, ownership rules, fault containment, and capability enforcement. Functional operating-system behavior is added through replaceable services arranged in outward architectural rings.

LogOS does not aim to reproduce Unix internally. Compatibility may be provided at the edges, but the native system model should be:

- capability-based rather than account-and-mode-bit based;
- service-oriented rather than kernel-feature oriented;
- structured rather than byte-stream oriented;
- remotely operable before it becomes graphically complete;
- addressable by humans, software, and AI agents through the same typed interfaces;
- recoverable, observable, and updateable without treating reboot as the normal failure response.

## 2. Architectural principles

1. **Mechanism stays inward; policy moves outward.**
2. **Outer rings depend on inner contracts, never inner implementations.**
3. **Capabilities are explicit, narrow, transferable only by policy, and revocable where practical.**
4. **Services own their state and resources.**
5. **Failure boundaries are designed before convenience APIs.**
6. **Local and remote operation use the same underlying session and command contracts.**
7. **Structured values are native; formatted text is a presentation format.**
8. **WASM is the default application sandbox, not a kernel dependency.**
9. **Native Rust services are reserved for trusted, hardware-facing, latency-critical, or boot-critical responsibilities.**
10. **Names are part of architecture. Every subsystem name must communicate scope without binding it to one implementation.**
11. **AI is a client of typed capabilities, not an implicit superuser.**
12. **Each milestone ends with automated QEMU proofs, not only manual demonstrations.**

## 3. Architectural rings

The rings describe dependency and trust placement. They are not conventional CPU privilege rings.

| Ring | Name           | Responsibility                                                                                                 |
| ---: | -------------- | -------------------------------------------------------------------------------------------------------------- |
|    0 | **Core**       | Privileged execution, memory ownership, scheduling, interrupts, capabilities, IPC transport, fault containment |
|    1 | **Foundation** | Hardware-facing native services and stable device-independent interfaces                                       |
|    2 | **System**     | Shared machine services: supervision, identity, secrets, time, storage, networking, updates                    |
|    3 | **Sessions**   | Authentication context, commands, structured shell, terminal rendering, local and remote interaction           |
|    4 | **Runtime**    | WASM components, packages, workspaces, application lifecycle and application-facing APIs                       |
|    5 | **Experience** | Compositor, graphical shell, accessibility, desktop applications and rich clients                              |

A component may be moved inward only when measurements or correctness requirements prove that an outer service cannot satisfy its contract.

See [Architecture Annex](ARCHITECTURE.md).

## 4. Current state

## Core v1 — Complete

Core v1 is a dependable, event-driven kernel foundation. It is not a desktop OS, userspace, network stack, filesystem, or WASM runtime.

### Demonstrated

- [x] UEFI boot, debug output, startup health gate, and automated headless QEMU verification.
- [x] Framebuffer recovery console with PS/2 IRQ keyboard input.
- [x] IDT, exception halt path, PIT bootstrap clock, and ACPI-derived IOAPIC VirtIO completion IRQ.
- [x] Cooperative ready/blocked task scheduler, generation-tagged task handles, and event-driven idle.
- [x] Physical-page allocation across conventional-memory ranges, owned-page recycling, and reversible bootstrap virtual mappings.
- [x] Generation-tagged capability grants and revocation, service registry, queued IPC requests/replies, PCI discovery, and legacy VirtIO balloon service.
- [x] ACPI RSDP/XSDT/MADT validation and APIC topology discovery.
- [x] ACPI PCI routing without reliance on firmware-programmed interrupt lines.
- [x] Permissioned mappings and service-lifetime memory reclamation.
- [x] Event wait queues.
- [x] Interrupt-safe IPC producers, bounded backpressure, cancellation, and request/reply correlation.
- [x] Generalized VirtIO queue ownership, completion, error handling, and reset.
- [x] Driver lifecycle: discover, bind, interrupt, quiesce, recover.
- [x] Panic diagnostics, structured health reporting, driver recovery policy, and trace export.
- [x] ACPI power-off and reset.
- [x] QEMU integration checks for console input, IPC replies, task wake-up, and driver recovery.

### Preserved role

The current framebuffer interface becomes the **recovery console**.

It remains:

- independent of the normal terminal stack;
- deliberately small and auditable;
- usable when services, storage, fonts, or the WASM runtime are unavailable;
- limited to health, diagnostics, service recovery, trace export, reset, and power-off.

It should not grow into the normal user environment.

---

# 5. Roadmap overview

| Order | Milestone           | Primary proof                                                               |
| ----: | ------------------- | --------------------------------------------------------------------------- |
|     1 | **Console v1**      | A usable local structured terminal exists outside the kernel                |
|     2 | **Platform v1**     | Native services start, fail, recover, and negotiate contracts independently |
|     3 | **Persistence v1**  | Configuration and service-owned data survive crashes and resets             |
|     4 | **Network v1**      | LogOS securely reaches and is reachable over a network                      |
|     5 | **Remote v1**       | The machine is fully operable without physical access                       |
|     6 | **Update v1**       | System evolution is signed, atomic, health-gated, and reversible            |
|     7 | **Applications v1** | Sandboxed WASM applications can be installed, run, communicate, and persist |
|     8 | **Experience v1**   | Replaceable graphical environments run entirely on system contracts         |

Core hardening continues alongside these milestones and does not automatically block outward progress.

## Repository migration

The current kernel remains independently bootable while its single crate is split in stages:

1. create a Cargo workspace and extract the no-std `logos-core` mechanisms;
2. extract hardware-facing platform and driver crates;
3. extract `logos-terminal` for the normal terminal stack while retaining the kernel recovery path;
4. retain `logos-uefi` as the UEFI boot binary throughout.

Every extraction must retain the current QEMU proof. Add `logos-abi` only when an independently built native service needs a stable contract; do not create an empty ABI crate in advance.

---

# 6. Console v1 — Usable local operation

## Goal

Replace the recovery console as the normal interface while preserving it as an independent fallback.

The normal terminal is not a kernel shell. It is a renderer and editor attached to a structured session.

## Required components

### Foundation services

- [x] Console v1 slice 1: introduce the Foundation input-service boundary while retaining direct kernel recovery input.
- [x] Console v1 slice 2: introduce the Foundation display-service boundary while retaining direct kernel recovery output.
- [x] Console v1 slice 3: add a minimal terminal model that consumes input events and renders through the display service.
- [x] Console v1 slice 4: route normal terminal glyph rasterization through the Foundation text service.
- [x] Console v1 slice 5: select normal or recovery console mode without rendering recovery output on healthy normal boots.
- [x] Console v1 slice 6: run the normal terminal as the sole input consumer in normal mode.
- [x] Console v1 slice 7: add an explicit recovery capability and live `recovery` command handoff.
- [x] Console v1 slice 8: normalize Foundation input with QWERTY/AZERTY layouts, modifiers, state, and bounded repeat.
- [x] Console v1 slice 9: add bounded UTF-8 terminal editing with character-safe cursor movement and deletion.
- [x] Console v1 slice 10: add an embedded normal-console font with metrics and fallback glyphs.
- [x] Move normal keyboard handling behind an input service.
- [x] Preserve a kernel-owned emergency output and input path for recovery.
- [x] Normalize physical keys, logical keys, text input, modifiers, and repeat.
- [x] Support selectable keyboard layouts, beginning with QWERTY and AZERTY.
- [x] Introduce UTF-8 text handling.
- [x] Introduce font-backed monospace rendering.
- [x] Add a minimal text service for font loading, glyph metrics, and rasterization.

### Terminal renderer

- [x] Cursor and blinking caret.
- [x] Insert/delete, left/right, home/end, word navigation.
- [x] Line wrapping and resize-aware layout.
- [x] Scrollback with bounded memory.
- [x] Command history.
- [x] Selection and clipboard-ready abstractions.
- [x] Search within visible output and scrollback.
- [x] Clear separation between output model and rendered cells.
- [x] Resilient redraw after display-service restart.

### Session and shell

- [x] Session identity and capability context.
- [x] Command registry with discoverable descriptors.
- [x] Typed argument schemas.
- [x] Typed results and structured errors.
- [x] Cancellation and timeout propagation.
- [x] Bounded output and backpressure.
- [x] Basic variables and structured pipelines.
- [x] Human-readable, table, tree, and JSON renderers.
- [x] Persistable history contract, even before storage exists.

## Native command model

A command is a typed service operation, not merely an executable writing bytes.

```rust
pub struct CommandDescriptor {
    pub id: CommandId,
    pub name: String,
    pub summary: String,
    pub arguments: Vec<ArgumentDescriptor>,
    pub required_capabilities: Vec<CapabilityKind>,
    pub input_schema: Option<SchemaId>,
    pub output_schema: SchemaId,
}

pub enum Value {
    Null,
    Bool(bool),
    Integer(i64),
    Decimal(f64),
    Text(String),
    Bytes(BinaryHandle),
    Record(RecordValue),
    List(Vec<Value>),
    Table(TableValue),
    Stream(StreamHandle),
    Reference(ResourceRef),
}
```

Formatted text remains supported, but it is not the only interoperable form.

## Initial commands

- [ ] `health`
- [ ] `tasks`
- [ ] `services`
- [ ] `drivers`
- [ ] `trace`
- [ ] `inspect <resource>`
- [ ] `restart <service>`
- [ ] `cancel <request-or-job>`
- [ ] `clear`
- [ ] `reboot`
- [ ] `poweroff`
- [x] `help`
- [x] `commands`

## Exit criteria

- A user can comfortably inspect and operate the current machine without the recovery console.
- Keyboard layout, text entry, font rendering, terminal rendering, shell parsing, and command execution are separate contracts.
- Commands are discoverable and self-describing.
- Structured pipelines operate without parsing presentation text.
- The terminal can crash and restart without affecting the kernel or unrelated services.
- The recovery console remains usable when the normal terminal stack is unavailable.
- QEMU tests cover editing, history, structured commands, cancellation, display restart, and input-service restart.

---

# 7. Platform v1 — Replaceable native services

## Goal

Turn kernel mechanisms into a supervised service platform with explicit identities, dependencies, health, and versioned protocols.

## Service supervision

- [ ] Declarative service manifests.
- [ ] Dependency graph and startup ordering.
- [ ] Capability declarations and grants.
- [ ] Protocol and ABI version negotiation.
- [ ] Health checks and heartbeats.
- [ ] Restart, retry, backoff, quiesce, and shutdown policies.
- [ ] Failed-start diagnostics.
- [ ] Service replacement without kernel-specific knowledge.
- [ ] Service-owned resource reclamation.
- [ ] Boot profiles: normal, recovery, diagnostics.

## Machine services

- [ ] Stable machine identity.
- [ ] Service and process principals.
- [ ] Local user principals.
- [ ] Monotonic time service.
- [ ] RTC-backed wall clock with an explicit unknown/untrusted state.
- [ ] Entropy and secure random service.
- [ ] Secret store with service-scoped access.
- [ ] Audit events for privileged operations.
- [ ] Scoped, expiring, revocable standing-approval grants with separate audit records.
- [ ] `system.inference` service owning model inventory, scheduling, and accelerator binding; Runtime consumers receive scoped invocation grants.
- [ ] Resource discovery through typed references.

## Driver and device model

- [ ] Device-independent interfaces for input, display, block, network, and entropy.
- [ ] Driver binding policy outside the kernel where practical.
- [ ] Driver capability manifests.
- [ ] Device reset and rebinding.
- [ ] Driver replacement without machine reboot.
- [ ] Explicit ownership of DMA memory, queues, interrupts, and device state.

## Exit criteria

- Services start from manifests rather than hard-coded bootstrap assumptions.
- A failed service is diagnosed, reclaimed, and restarted without rebooting the machine.
- Every service operation has a principal and capability context.
- Kernel and service protocols have explicit compatibility rules.
- Normal operation requires no module-specific code in the kernel.
- QEMU tests inject startup failure, runtime failure, dependency loss, and recovery.

---

# 8. Persistence v1 — Durable system state

## Goal

Provide crash-safe, capability-scoped persistence without making a POSIX filesystem the native system model.

## Block service

- [ ] VirtIO block driver.
- [ ] Asynchronous reads, writes, flushes, discard, timeout, and cancellation.
- [ ] Device identity and partition discovery.
- [ ] Reset and recovery after failed requests.
- [ ] Integrity and completion diagnostics.

## Native storage model

The native model should support:

- named objects;
- byte streams;
- immutable versions;
- transactional metadata;
- atomic replacement;
- service-owned namespaces;
- quotas;
- checksums;
- snapshots or revision history;
- explicit sharing through capabilities.

A conventional directory/file view may be built on top for compatibility and familiarity.

## Required

- [ ] Crash-safe metadata commit or journal.
- [ ] Checksummed metadata and content where appropriate.
- [ ] Service-owned storage namespaces.
- [ ] Atomic object and stream replacement.
- [ ] Quotas and accounting.
- [ ] Consistency checker and recovery mode.
- [ ] Persistent service manifests and configuration.
- [ ] Persistent terminal history.
- [ ] Persistent identity, trust, and secret metadata.
- [ ] Agent-memory Store namespaces with workspace visibility, retention, and redaction policy.
- [ ] File compatibility API.
- [ ] Memory-backed and temporary namespaces.

## Exit criteria

- Data survives reset and simulated unclean shutdown.
- A service cannot access another service’s storage without an explicit capability.
- Corruption is detected and reported.
- Configuration rollback is possible.
- The machine can rebuild nonessential indexes from authoritative data.
- QEMU tests repeatedly interrupt writes at controlled points and verify recovery.

---

# 9. Network v1 — Secure connectivity

## Goal

Provide asynchronous, capability-controlled networking as a service.

## Required

### Link and transport

- [ ] VirtIO network driver.
- [ ] Ethernet.
- [ ] ARP.
- [ ] IPv4.
- [ ] ICMP diagnostics.
- [ ] DHCP.
- [ ] UDP.
- [ ] TCP.
- [ ] DNS.
- [ ] Connection cancellation, timeout, backpressure, and limits.
- [ ] Network lifecycle tracing.

### Policy and security

- [ ] Capability-controlled connect, bind, and listen.
- [ ] Firewall policy service.
- [ ] TLS client and server.
- [ ] Trust-store service.
- [ ] Certificate validation.
- [ ] Secure device enrollment.
- [ ] Explicit trust-on-first-use or pinned-key workflow where certificates are not appropriate.
- [ ] Network access audit events.

### Later compatibility

- [ ] IPv6 without redesigning application-facing APIs.
- [ ] Unix-style socket compatibility where useful.
- [ ] SSH server as an optional compatibility service.

## Exit criteria

- LogOS obtains network configuration automatically.
- It resolves DNS and completes a validated TLS connection.
- A service cannot listen or connect without the corresponding capability.
- Network-driver failure is contained and recoverable.
- Network APIs do not expose IPv4-specific assumptions to applications.
- QEMU tests cover packet loss, timeout, reset, reconnect, and unauthorized operations.

---

# 10. Remote v1 — Primary administration environment

## Goal

Make remote operation the first complete LogOS user experience.

SSH may be supported, but the native architecture is a richer structured protocol.

## Native remote protocol

- [ ] Authenticated machine enrollment.
- [ ] Public-key or device-key authentication.
- [ ] Multiplexed sessions.
- [ ] Structured command invocation and results.
- [ ] Resumable sessions after connection loss.
- [ ] Explicit session capability grants.
- [ ] File and object transfer.
- [ ] Log, trace, and health streaming.
- [ ] Service inspection and control.
- [ ] Update operations.
- [ ] Reboot and power-off.
- [ ] Protocol version negotiation.
- [ ] Compression and bounded flow control.
- [ ] Complete audit trail.

## Client surfaces

- [ ] Minimal native terminal client.
- [ ] Web client.
- [ ] Desktop client.
- [ ] Mobile client.
- [ ] SSH compatibility gateway.

All clients use the same command, session, resource, and authorization contracts.

## Exit criteria

From another machine, an authenticated user can:

- inspect health, tasks, services, devices, and resources;
- execute structured commands;
- reconnect to interrupted sessions;
- transfer files and objects;
- read and follow logs and traces;
- start, stop, restart, install, and remove services;
- reboot or power off the machine.

A local display or keyboard is not required for normal administration.

---

# 11. Update v1 — Safe system evolution

## Goal

Allow kernel, service, driver, and application versions to evolve independently without making failed updates fatal.

## Required

- [ ] Signed packages and manifests.
- [ ] Dependency and compatibility resolution.
- [ ] Staged installation.
- [ ] Atomic activation.
- [ ] Health-gated rollout.
- [ ] Automatic rollback.
- [ ] Recovery boot profile.
- [ ] Persistent update journal.
- [ ] Remote inspection, apply, cancel, and rollback.
- [ ] Rollback-safe configuration migration.
- [ ] Reproducible package metadata.
- [ ] Revocation of compromised signing identities.
- [ ] Separate policy for kernel, trusted native services, drivers, and WASM applications.

## Exit criteria

- Power loss during staging or activation leaves a bootable system.
- Failed health checks trigger deterministic rollback.
- Kernel and services negotiate compatibility after independent updates.
- An administrator can see exactly what changed and why rollback occurred.
- QEMU tests interrupt every update phase and verify a valid recovery path.

---

# 12. Applications v1 — Sandboxed WASM platform

## Goal

Run portable applications as capability-isolated WASM components without making the WASM runtime part of kernel correctness.

## Runtime

- [ ] WASM component validation and loading.
- [ ] Native LogOS WIT component host interfaces; WASI is compatibility only.
- [ ] Versioned host interfaces.
- [ ] Capability-scoped imports.
- [ ] Memory, CPU, storage, and network quotas.
- [ ] Cooperative cancellation.
- [ ] Asynchronous host operations.
- [ ] Application health and lifecycle.
- [ ] Crash isolation and restart policy.
- [ ] Deterministic or restricted execution mode where useful.
- [ ] Runtime upgrade and rollback.

## Package model

- [ ] Signed application packages.
- [ ] Manifest-declared interfaces and capabilities.
- [ ] Application identity.
- [ ] Application-owned storage namespace.
- [ ] Explicit network permissions.
- [ ] Declared inter-application interfaces.
- [ ] Versioned state migration.
- [ ] Atomic update and rollback.
- [ ] Rust SDK first, generated bindings for other languages later.

## Workspaces

A workspace is the user-facing unit that groups resources and authority without reproducing Unix’s global ambient environment.

- [ ] Workspace identity and metadata.
- [ ] Explicit resource attachments.
- [ ] Workspace-scoped capability grants.
- [ ] Workspace applications and services.
- [ ] Shared session context.
- [ ] Workspace export, import, snapshot, and recovery.
- [ ] Clear separation between machine authority and workspace authority.

## AI-addressable operation

- [ ] Typed tool registry generated from command and service descriptors.
- [ ] Schema-visible inputs, outputs, capabilities, and side effects.
- [ ] Human approval policies for sensitive operations.
- [ ] Resource references usable by humans and agents.
- [ ] Auditable agent sessions.
- [ ] Bounded agent decision records correlated with action audit entries, retaining policy outcome, user-intent reference, and structured tool inputs/outputs but never private model reasoning.
- [ ] Explicit re-approval when untrusted external content would trigger a privileged agent effect; global taint tracking is deferred pending a narrower information-flow design.
- [ ] No implicit agent privilege.

## Exit criteria

- A WASM application can be remotely installed, started, stopped, upgraded, rolled back, and removed.
- It uses storage, network, time, input, display, and IPC only through granted interfaces.
- A failed application cannot corrupt the runtime, kernel, or unrelated applications.
- Applications communicate through declared typed interfaces.
- A workspace can be moved or restored without granting machine-wide access.
- The same operations are available to human clients and authorized AI agents.

---

# 13. Experience v1 — Replaceable graphical environment

## Goal

Build a local graphical experience entirely on existing system, session, and runtime contracts.

## Foundation

- [ ] Modern keyboard and pointer devices.
- [ ] USB HID.
- [ ] Display modes and scanout ownership.
- [ ] Shared surfaces and buffer lifecycle.
- [ ] Damage tracking.
- [ ] Presentation timing.
- [ ] GPU strategy that does not block an initial software compositor.

## Compositor

- [ ] Surface protocol.
- [ ] Window and layer policy.
- [ ] Input routing and focus.
- [ ] Clipboard and drag-and-drop.
- [ ] Accessibility tree and semantic actions.
- [ ] Permissioned screen capture.
- [ ] Remote surface streaming.
- [ ] Compositor restart without kernel restart.

## Graphical shell

- [ ] Login and unlock.
- [ ] Application launcher.
- [ ] Notifications.
- [ ] Settings.
- [ ] Workspace switching.
- [ ] System health and remote-machine management.
- [ ] Graphical terminal using the existing session protocol.

## Exit criteria

- Multiple isolated applications render through the compositor.
- Applications cannot inspect arbitrary input or surfaces.
- The compositor and graphical shell can restart independently.
- The system remains fully operable remotely when the graphical stack fails.
- The terminal, web, desktop, mobile, and agent clients share the same underlying system contracts.

---

# 14. Continuous Core lane

These items improve the kernel but should be promoted to milestone blockers only when required by measured needs or outward contracts.

- [ ] Replace PIT with APIC timer or HPET while retaining a fallback.
- [ ] MSI/MSI-X support.
- [ ] Multicore bootstrap and per-CPU state.
- [ ] SMP-safe scheduler, IPC, tracing, and allocation.
- [ ] Architecture separation for future non-x86 targets.
- [ ] Stronger model and property tests for handles, queues, mappings, and cancellation.
- [ ] Persistent crash export through a reserved region or virtual diagnostic device.
- [ ] Preemption only when cooperative scheduling demonstrably fails latency or fairness requirements.
- [ ] IOMMU support when real hardware or stronger DMA isolation becomes a target.

## Core change rule

A feature belongs in Core only when at least one of these is true:

1. it must execute with hardware privilege;
2. it establishes or enforces a global ownership invariant;
3. it is required to contain faults before services can run;
4. moving it outward would make correctness unverifiable or materially less safe.

Performance alone is not sufficient without measurement.

---

# 15. Cross-cutting requirements

Every milestone must define and test:

- ownership and reclamation;
- capability requirements;
- cancellation behavior;
- bounded queues and memory;
- timeout behavior;
- protocol versioning;
- structured errors;
- health and diagnostics;
- restart and recovery;
- audit events for privileged effects;
- QEMU integration coverage;
- explicit non-goals.

## Definition of done

A feature is not complete until:

- its boundary and owner are documented;
- its public contract is versioned;
- failure and cancellation are handled;
- resources are reclaimed after success, failure, timeout, and client disappearance;
- unauthorized access is tested;
- diagnostic output is actionable;
- automated integration tests prove its exit criteria.

---

# 16. Explicit non-goals for the next cycle

- POSIX compatibility as the native system architecture.
- A desktop before remote administration is dependable.
- A browser or IDE before application, update, and recovery contracts exist.
- Running arbitrary native third-party binaries.
- Moving WASM execution into the kernel.
- Treating AI agents as trusted kernel actors.
- Full real-hardware support before QEMU contracts stabilize.
- Preemptive scheduling without a measured requirement.
- A monolithic shell that owns rendering, parsing, execution, sessions, and transport.
- Replacing precise subsystem boundaries with thematic names alone.

---

# 17. Document maintenance

This roadmap is updated as implementation changes the understanding of the system.

For every material change:

1. update the relevant milestone and exit criteria;
2. update the architecture annex if a responsibility moves between rings;
3. update the naming register if a subsystem is introduced, renamed, split, or merged;
4. record a brief decision note for irreversible or cross-cutting choices;
5. keep completed criteria in place as historical evidence;
6. avoid rewriting past scope to make an incomplete milestone appear complete.

## Annexes

- [Architecture and boundary model](ARCHITECTURE.md)
- [Subsystem naming register](NAMING.md)

- [Reviewed architecture proposals — 2026-07-26](reviewed/2026-07-26.md)
