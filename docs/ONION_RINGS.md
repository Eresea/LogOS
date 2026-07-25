# LogOS Onion Architecture Annex

> **Status:** Living architecture reference  
> **Updated:** 2026-07-24  
> **Applies to:** Core, native services, sessions, WASM applications, and graphical environments

## 1. Purpose

This annex defines the LogOS architectural rings in enough detail to guide implementation, review, and subsystem placement.

The onion model exists to preserve four properties as LogOS grows:

1. **A small trusted core.**
2. **Explicit ownership and authority.**
3. **Replaceable outer systems.**
4. **Failure containment without machine-wide restart.**

The rings are architectural dependency and trust boundaries. They are not direct equivalents of x86 privilege rings, protection levels, processes, crates, or deployment packages.

A ring answers:

- how close a subsystem is to privileged mechanisms;
- what authority it may hold;
- whether it is required for recovery;
- what it may depend on;
- how independently it can fail, restart, and update;
- whether it should be native Rust or a sandboxed WASM component.

## 2. The onion at a glance

```text
┌───────────────────────────────────────────────────────────────┐
│ Ring 5 — Experience                                          │
│ Compositor, desktop shell, graphical applications, clients   │
│                                                               │
│  ┌─────────────────────────────────────────────────────────┐  │
│  │ Ring 4 — Runtime                                       │  │
│  │ WASM runtime, packages, apps, workspaces, tool registry│  │
│  │                                                         │  │
│  │  ┌───────────────────────────────────────────────────┐  │  │
│  │  │ Ring 3 — Sessions                                │  │  │
│  │  │ Sessions, commands, shell, terminal, remote      │  │  │
│  │  │                                                   │  │  │
│  │  │  ┌─────────────────────────────────────────────┐  │  │  │
│  │  │  │ Ring 2 — System                           │  │  │  │
│  │  │  │ Supervisor, identity, store, network,     │  │  │  │
│  │  │  │ secrets, audit, update                    │  │  │  │
│  │  │  │                                             │  │  │  │
│  │  │  │  ┌───────────────────────────────────────┐  │  │  │  │
│  │  │  │  │ Ring 1 — Foundation                 │  │  │  │  │
│  │  │  │  │ Drivers, devices, display, input,   │  │  │  │  │
│  │  │  │  │ glyphs, block and network devices   │  │  │  │  │
│  │  │  │  │                                       │  │  │  │  │
│  │  │  │  │  ┌─────────────────────────────────┐  │  │  │  │  │
│  │  │  │  │  │ Ring 0 — Core                 │  │  │  │  │  │
│  │  │  │  │  │ Privileged mechanisms and     │  │  │  │  │  │
│  │  │  │  │  │ global safety invariants      │  │  │  │  │  │
│  │  │  │  │  └─────────────────────────────────┘  │  │  │  │  │
│  │  │  │  └───────────────────────────────────────┘  │  │  │  │
│  │  │  └─────────────────────────────────────────────┘  │  │  │
│  │  └───────────────────────────────────────────────────┘  │  │
│  └─────────────────────────────────────────────────────────┘  │
└───────────────────────────────────────────────────────────────┘
```

The normal dependency direction is inward:

```text
Experience → Runtime → Sessions → System → Foundation → Core
```

Interactions may cross more than one ring, but only through published contracts and explicit capabilities. An outer subsystem must not obtain an inner implementation reference merely because it ultimately uses that implementation.

## 3. The dimensions behind the rings

Ring placement should not be decided from one criterion alone.

| Dimension | Inward tendency | Outward tendency |
|---|---|---|
| Hardware privilege | Requires interrupts, mappings, DMA, CPU control | Uses abstract resources |
| Safety invariant | Enforces global ownership or isolation | Applies local policy |
| Boot criticality | Required before recovery can function | Optional after normal boot |
| Failure impact | Failure compromises machine correctness | Failure affects one service, session, or app |
| Update freedom | Rare and tightly coordinated | Frequent and independently replaceable |
| Trust | Fully trusted native code | Capability-limited sandbox |
| Portability | Architecture-specific | Device- and architecture-independent |
| User policy | Mechanism only | Increasingly user-facing and configurable |
| Persistence | Minimal boot diagnostics | Rich machine, workspace, and application state |
| Interface richness | Small bounded primitives | High-level typed and graphical contracts |

No subsystem should be moved inward only because an inward implementation is initially easier.

## 4. Global ring rules

### 4.1 Dependency direction

- Outer rings may consume inner contracts.
- Inner rings must not depend on outer-ring implementations.
- Core must not know about sessions, applications, workspaces, desktop concepts, or WASM.
- Foundation must not know shell syntax, users, package formats, or application policy.
- System must not depend on a terminal renderer or desktop.
- Sessions must remain usable without WASM or graphical services.
- Runtime must remain usable without the desktop.
- Experience must remain disposable.

### 4.2 Contract rule

Every cross-ring contract must define:

- protocol identifier and version;
- request and response schemas;
- capability requirements;
- ownership transfer rules;
- cancellation and timeout behavior;
- queue and resource bounds;
- restart and reconnection behavior;
- structured error categories;
- diagnostic and audit behavior.

### 4.3 Resource rule

A handle does not automatically grant ownership or authority.

Each resource interaction distinguishes:

- **identity** — which resource is referenced;
- **ownership** — which component must release it;
- **authority** — which operations are permitted;
- **lifetime** — what ends or invalidates it;
- **accounting** — whose quota is charged.

### 4.4 Failure rule

An outer-ring failure must not force an inner-ring restart.

The expected direction of containment is:

```text
app failure
  ⤷ app or runtime action

session failure
  ⤷ session or renderer action

system service failure
  ⤷ supervisor recovery or degraded mode

driver failure
  ⤷ device quiesce/reset/rebind

core invariant failure
  ⤷ controlled panic, diagnostics, halt/reset
```

### 4.5 Recovery rule

Recovery paths may use deliberately limited inward mechanisms that normal operation must not depend upon.

Examples:

- Core retains minimal framebuffer output even when normal display ownership belongs to Foundation.
- Core retains minimal keyboard input even when layouts and text input belong to Foundation.
- Core exposes a tiny recovery command set even when normal commands belong to Sessions.

These are explicit emergency exceptions, not duplicate normal implementations.

---

# 5. Ring 0 — Core

## 5.1 Mission

Core establishes the smallest privileged substrate on which all other LogOS systems can safely run.

Its role is to enforce invariants and expose mechanisms. It must not become the place where convenient shared functionality accumulates.

## 5.2 Trust and lifecycle

| Property | Ring 0 expectation |
|---|---|
| Implementation | Native Rust with minimal required assembly |
| Privilege | Highest |
| Trust | Fully trusted |
| Boot role | Mandatory |
| Restartability | Normally requires reboot |
| Update cadence | Lowest |
| Hardware specificity | Highest |
| Policy | Minimal |
| Public surface | Small, bounded, versioned |

## 5.3 Core owns

### Execution

- CPU initialization;
- interrupt and exception entry;
- task state;
- ready, blocked, and wake-up mechanics;
- timer/deadline mechanisms;
- idle and halt behavior.

### Fabric

- physical-page ownership;
- virtual address spaces;
- mapping creation and removal;
- mapping permissions;
- ownership-tagged reclamation;
- kernel mapping invariants.

### Authority

- capability representation;
- capability validation;
- kernel-object access checks;
- safe delegation primitives;
- revocation mechanics where Core enforcement is required.

### Channels

- bounded IPC transport;
- request/reply correlation primitives;
- wait queues;
- interrupt-safe production;
- cancellation signaling;
- producer backpressure;
- endpoint lifetime.

### Containment

- exception isolation where possible;
- kernel panic handling;
- task/service teardown primitives;
- resource reclamation hooks;
- driver isolation mechanisms available at the hardware level.

### Bootstrap and recovery

- UEFI handoff;
- ACPI topology required by Core;
- minimal PCI/device information needed to launch driver ownership;
- bounded trace diagnostics;
- emergency text output;
- emergency keyboard path;
- reset and power-off.

## 5.4 Core exposes

Core-facing contracts should remain close to mechanisms:

```rust
trait TaskControl {
    fn spawn(&self, image: NativeImage, grants: CapabilitySet)
        -> Result<TaskHandle, SpawnError>;

    fn cancel(&self, task: TaskHandle, reason: CancelReason)
        -> Result<(), TaskError>;
}

trait MappingControl {
    fn map_owned(
        &self,
        owner: OwnerId,
        pages: PageRange,
        address: VirtualRange,
        permissions: MappingPermissions,
    ) -> Result<MappingHandle, MappingError>;
}

trait ChannelControl {
    fn send(
        &self,
        endpoint: EndpointHandle,
        message: MessageEnvelope,
    ) -> Result<SendReceipt, SendError>;
}
```

These examples are conceptual. They do not require one monolithic kernel API.

## 5.5 Core must not own

- filesystem semantics;
- persistent object schemas;
- network protocols;
- service dependency policy;
- user accounts;
- authentication workflows;
- secret rotation;
- keyboard layouts;
- fonts and shaping;
- terminal history;
- shell syntax;
- command discovery;
- package manifests;
- WASM engines;
- application lifecycle;
- windows, surfaces, or desktop policy;
- AI agent memory or tools.

## 5.6 Acceptable Core exceptions

Some minimal duplicate capabilities are allowed only for recovery:

| Normal owner | Core exception |
|---|---|
| Foundation Display | Fixed emergency framebuffer text output |
| Foundation Input | Basic emergency keyboard events |
| Sessions Commands | Tiny fixed recovery command parser |
| System Supervisor | Primitive start/stop/recovery hooks |
| System Store | Optional raw crash export target, not general persistence |

The emergency path should be auditable, bounded, and intentionally uncomfortable for normal use.

## 5.7 Core placement test

A feature belongs in Core only when at least one is true:

1. it requires CPU or hardware privilege;
2. it enforces a machine-wide ownership or isolation invariant;
3. it must act before services can safely execute;
4. placing it outside Core would make the invariant unenforceable.

Even then, only the enforcing mechanism belongs in Core. Policy remains outward.

## 5.8 Core anti-patterns

- Adding a kernel API because several services currently duplicate code.
- Adding a syscall for a user-facing concept.
- Passing unbounded strings or collections through privileged paths.
- Letting Core parse package, filesystem, shell, or network formats.
- Giving one “system service” broad ambient kernel authority.
- Treating all native Rust code as equally trusted.
- Keeping a bootstrap implementation in Core after an outer owner exists.

## 5.9 Core failure and testing

Core failures are exceptional. Tests must focus on invariants:

- stale and generation-mismatched handles;
- double release;
- mapping permission violations;
- cancellation races;
- queue saturation;
- endpoint disappearance;
- interrupt/concurrent producers;
- service teardown with outstanding resources;
- malformed firmware data;
- driver failure during completion;
- deterministic panic diagnostics.

---

# 6. Ring 1 — Foundation

## 6.1 Mission

Foundation converts hardware-specific devices and low-level facilities into stable, device-independent services.

It is the hardware adaptation layer of LogOS.

## 6.2 Trust and lifecycle

| Property | Ring 1 expectation |
|---|---|
| Implementation | Native Rust |
| Privilege | Explicit hardware capabilities |
| Trust | Highly trusted but isolated from Core |
| Boot role | Required according to available hardware/profile |
| Restartability | Expected where hardware permits |
| Update cadence | Low to moderate |
| Hardware specificity | High internally, low externally |
| Policy | Device selection and mechanics only |
| Public surface | Device-independent protocols |

## 6.3 Foundation owns

### Devices

- discovery results exposed by Core;
- driver matching and binding;
- interrupt endpoint ownership;
- DMA mappings and queues;
- device initialization;
- quiesce, reset, recovery, and rebinding;
- device health.

### Input

- physical key codes;
- logical key mapping;
- modifiers;
- key repeat;
- keyboard layouts;
- text composition;
- pointer and other input events;
- input device identity.

### Display

- framebuffer and scanout ownership;
- display modes;
- presentation buffers;
- basic surface submission;
- device reset;
- display health.

Display does not own windows, focus, or desktop policy.

### Glyph

- font discovery and loading;
- glyph metrics;
- rasterization;
- font fallback;
- shaping when introduced;
- text measurement.

Glyph does not own terminal history or document layout policy.

### Block

- asynchronous block operations;
- flush;
- discard;
- timeout;
- cancellation;
- reset;
- block-device identity;
- partition exposure.

Block does not define files, objects, transactions, or namespaces.

### Net Device

- transmit and receive queues;
- packet buffers;
- device offload capabilities;
- link state;
- reset and recovery.

Net Device does not implement TCP, DNS, firewall policy, or remote sessions.

### Hardware sources

- entropy devices;
- RTC access;
- high-resolution timer devices where not owned directly by Core;
- sensors exposed as typed device data.

## 6.4 Foundation exposes

Examples of outward abstractions:

```rust
pub enum InputEvent {
    Key(KeyEvent),
    Pointer(PointerEvent),
    Text(TextInputEvent),
    Device(DeviceInputEvent),
}

pub trait BlockDevice {
    fn read(&self, request: ReadBlocks) -> Pending<BlockBuffer>;
    fn write(&self, request: WriteBlocks) -> Pending<WriteReceipt>;
    fn flush(&self) -> Pending<()>;
}

pub trait DisplayTarget {
    fn capabilities(&self) -> DisplayCapabilities;
    fn present(&self, frame: PresentRequest) -> Pending<PresentReceipt>;
}
```

The exact driver model may change. Outer services should remain insulated from it.

## 6.5 Foundation may depend on

- Core scheduling, mappings, channels, timers, capabilities, and tracing;
- other Foundation services only through explicit contracts;
- limited System configuration after System becomes available.

Early boot must not create hidden dependency cycles. A driver needed by storage cannot require storage to start.

## 6.6 Foundation must not own

- service restart policy;
- user identities;
- device access policy beyond capability enforcement;
- persistent configuration semantics;
- filesystems;
- network protocols above packet transport;
- terminal editing;
- command execution;
- application manifests;
- graphical window policy.

## 6.7 Driver placement

The long-term target is:

```text
Core
  provides mappings, interrupts, scheduling, capabilities, IPC
      ↓
Foundation driver service
  owns device and exposes device-independent protocol
```

Transitional in-kernel drivers are acceptable only when documented with:

- why the driver cannot yet live in Foundation;
- what Core mechanism is missing;
- which interface will face outward;
- what milestone removes the exception.

## 6.8 Foundation anti-patterns

- Exposing raw PCI details to normal applications.
- Returning driver-owned pointers across service boundaries.
- Putting a filesystem inside the block driver.
- Putting keyboard layout tables inside the kernel interrupt handler.
- Making display ownership synonymous with desktop ownership.
- Letting one driver failure stall a global event loop.
- Assuming one device instance or one hardware implementation.

## 6.9 Failure behavior

A Foundation service failure should:

1. stop new operations;
2. fail or cancel outstanding requests with structured errors;
3. reclaim mappings, buffers, queues, and interrupts;
4. quiesce or reset the device;
5. publish health and diagnostic state;
6. allow Supervisor to restart or replace the driver;
7. notify dependent services through lifecycle events.

---

# 7. Ring 2 — System

## 7.1 Mission

System provides stable machine-wide services and policy on top of Foundation mechanisms.

This is the first ring that turns a booted kernel into a coherent operating system.

## 7.2 Trust and lifecycle

| Property | Ring 2 expectation |
|---|---|
| Implementation | Primarily native Rust |
| Privilege | Narrow machine capabilities |
| Trust | Trusted policy services |
| Boot role | Required according to boot profile |
| Restartability | Required |
| Update cadence | Moderate |
| Hardware specificity | Low |
| Policy | Machine-wide |
| Public surface | Versioned typed service protocols |

## 7.3 System owns

### Supervisor

- service manifests;
- dependency graph;
- boot profiles;
- startup ordering;
- health checks;
- restart and backoff policy;
- quiesce and shutdown;
- degraded-mode decisions;
- version compatibility checks;
- resource reclamation coordination.

Supervisor is not a general command shell and should not contain device-specific code.

### Identity

- machine identity;
- user principals;
- service principals;
- application principals;
- agent principals;
- authentication results;
- principal lifecycle;
- mapping authenticated identities to policy inputs.

Identity does not itself grant arbitrary authority. Capabilities determine authority.

### Vault

- protected secrets;
- service-scoped access;
- encryption key hierarchy;
- rotation;
- revocation;
- secret metadata;
- prevention of secret leakage into traces and diagnostics.

### Time

- wall-clock time;
- synchronization;
- source trust;
- timezone and civil-time support where needed;
- explicit “unknown” or “untrusted” states.

Core Clock remains responsible for monotonic deadlines.

### Store

- persistent objects and streams;
- transactions;
- immutable versions;
- namespaces;
- quotas;
- checksums;
- crash recovery;
- snapshots;
- file compatibility views.

### Network

- Ethernet/IP integration above Net Device;
- TCP and UDP;
- DNS;
- DHCP;
- connection ownership;
- bind/listen/connect capabilities;
- connection limits;
- firewall integration;
- TLS and trust-store integration.

### Audit

- durable records of privileged and security-relevant effects;
- principal, operation, target, result, and time;
- append and retention policy;
- protected access.

Audit is distinct from Trace. Trace diagnoses behavior; Audit records authority-sensitive actions.

### Update

- signed package validation;
- dependency and compatibility resolution;
- staging;
- activation;
- health gates;
- rollback;
- update journal;
- signing identity revocation.

## 7.4 System may depend on

- Core mechanisms;
- Foundation device-independent services;
- other System services through explicit, acyclic startup contracts.

Examples:

- Store depends on Block and may depend on Vault for encryption.
- Network depends on Net Device, Time, Identity, Vault, and trust data.
- Update depends on Store, Identity, Vault, Supervisor, and optionally Network.
- Audit depends on Store and Time but must define behavior before trusted wall time exists.

## 7.5 System must not own

- terminal cell rendering;
- shell syntax;
- interactive history;
- user application behavior;
- WASM component execution;
- compositor policy;
- graphical widgets;
- application-specific data models.

## 7.6 Supervisor authority

Supervisor is powerful but must not become an ambient root process.

Its authority should be decomposed:

- start a declared service;
- grant only manifest-approved capabilities;
- query health;
- request quiesce;
- reclaim resources through Core-owned lifecycle;
- activate a validated version;
- enter a defined recovery profile.

Supervisor should not automatically receive access to every service’s data or secrets.

## 7.7 System anti-patterns

- A global registry that exposes unrestricted service handles.
- Identity conflated with capability.
- Secrets stored in general configuration objects.
- Audit implemented as ordinary logs.
- Store designed only as path-based files.
- Network access granted to every service by default.
- Update replacing live components without health gates.
- Supervisor becoming the implementation of all system policy.

## 7.8 Failure behavior

System service failure produces one of:

- restart with preserved authoritative state;
- restart with reconstructed transient state;
- degraded mode;
- recovery boot;
- controlled machine reset only when no safe continuation exists.

Each service must document which result applies.

---

# 8. Ring 3 — Sessions

## 8.1 Mission

Sessions provides the human, automation, and remote-control layer of LogOS.

It converts typed system operations into persistent interactive contexts without coupling system administration to a particular terminal or client.

## 8.2 Trust and lifecycle

| Property | Ring 3 expectation |
|---|---|
| Implementation | Native Rust initially; extensions may be WASM |
| Privilege | Session-scoped capabilities |
| Trust | Trusted mediation, untrusted input |
| Boot role | Required for normal administration |
| Restartability | Required |
| Update cadence | Moderate to high |
| Hardware specificity | None |
| Policy | Interactive and automation policy |
| Public surface | Commands, sessions, streams, rendering models |

## 8.3 Sessions owns

### Session Manager

- authenticated principal context;
- active capability set;
- environment and preferences;
- session lifetime;
- local and remote attachment;
- resumability;
- session expiry;
- job ownership;
- client presence.

### Commands

- discoverable command descriptors;
- stable command identifiers;
- typed arguments;
- input and output schemas;
- required capabilities;
- side-effect metadata;
- invocation;
- structured errors;
- completion metadata;
- tool exposure.

Commands may adapt System services but do not replace them.

### Shell

- syntax;
- expressions;
- variables;
- structured pipelines;
- conditionals;
- jobs;
- foreground/background behavior;
- cancellation;
- redirection into typed resources;
- reusable scripts.

The shell is not the terminal renderer.

### Slate

- terminal line/cell model;
- caret and editing state;
- history presentation;
- scrollback;
- selection;
- output search;
- resize behavior;
- rendering state.

Slate consumes Input, Glyph, and Display contracts. It does not own those services.

### Gateway

- client authentication handoff;
- protocol negotiation;
- multiplexed sessions;
- reconnect and resume;
- transport flow control;
- structured command and resource events;
- file/object transfer channels;
- log and trace streaming.

Gateway does not implement TCP or TLS itself; it consumes Network contracts.

### Formatters and views

- human-readable text;
- table and tree rendering;
- JSON or other machine views;
- live progress views;
- resource links;
- error presentation.

## 8.4 Terminal versus shell versus session

```text
Session
  owns principal, authority, environment, jobs, and lifetime

Shell
  interprets language and connects typed operations

Commands
  describe and invoke typed operations

Terminal / Slate
  edits and renders textual interaction

Gateway
  transports session interaction remotely
```

A session may exist without Slate. For example:

- an AI agent session;
- a web management client;
- a background script;
- a resumed remote session with no current renderer.

Slate may reconnect to an existing session after a display restart.

## 8.5 Structured pipelines

Native pipelines should pass values rather than presentation text.

```text
services
| where health.state == "failed"
| select resource, health.reason
| sort resource
```

A command result may include:

- scalar values;
- records;
- lists;
- tables;
- streams;
- binary handles;
- resource references;
- progress events;
- structured errors.

Text remains supported through explicit conversion.

## 8.6 Sessions may depend on

- Identity, Audit, Store, Network, Supervisor, and other System services;
- Input, Glyph, and Display from Foundation;
- Core only indirectly through published service contracts, except narrow bootstrap integration.

## 8.7 Sessions must not own

- kernel task scheduling;
- driver queues;
- raw device access;
- persistent storage internals;
- network protocol implementation;
- application sandboxing;
- desktop window policy.

## 8.8 Sensitive operations

Command descriptors should classify effects:

| Class | Example | Default interaction |
|---|---|---|
| Read-only | Inspect health | Immediate |
| Local mutation | Change session setting | Immediate |
| Service mutation | Restart one service | Confirmation or policy |
| Persistent mutation | Delete stored object | Confirmation and audit |
| Authority mutation | Grant capability | Strong confirmation |
| Machine mutation | Apply update, reboot | Strong confirmation and audit |
| Secret access | Reveal or use secret | Restricted presentation |
| External effect | Network publish/send | Explicit capability and audit policy |

The same metadata can drive human prompts, remote clients, and AI approval gates.

## 8.9 Sessions anti-patterns

- Treating formatted terminal output as the system API.
- Coupling commands directly to keyboard events.
- Making remote administration screen scraping.
- Granting remote sessions a global administrator token.
- Storing authoritative system state in shell variables.
- Letting terminal renderer crashes terminate running services.
- Reusing the recovery console parser as the normal shell.

## 8.10 Failure behavior

- Slate failure: session and jobs remain; renderer reconnects.
- Shell failure: active invocation handling follows explicit cancellation or reattachment rules.
- Gateway failure: local sessions remain; remote sessions may resume.
- Client failure: session remains only within configured lifetime and resource limits.
- Command adapter failure: underlying System service remains authoritative.

---

# 9. Ring 4 — Runtime

## 9.1 Mission

Runtime hosts portable, sandboxed applications and extensions through WASM components.

It provides the default execution environment for code that does not need direct hardware privilege or machine-wide native trust.

## 9.2 Trust and lifecycle

| Property | Ring 4 expectation |
|---|---|
| Implementation | Native WASM host; WASM guest components |
| Privilege | Manifest-declared capability grants |
| Trust | Host trusted, guest untrusted by default |
| Boot role | Optional for recovery; required for applications |
| Restartability | Required |
| Update cadence | High |
| Hardware specificity | None |
| Policy | Application and workspace scoped |
| Public surface | Versioned component interfaces |

## 9.3 Runtime owns

### WASM host

- validation;
- instantiation;
- host interface binding;
- memory limits;
- CPU/fuel/time limits;
- cancellation;
- asynchronous host calls;
- component lifecycle;
- crash isolation;
- runtime diagnostics.

### Packages

- application manifests;
- component metadata;
- dependencies;
- versions;
- signatures;
- requested capabilities;
- state migration declarations;
- application assets.

System Update may validate and activate trusted runtime/package infrastructure. Runtime Packages manages application-level package semantics.

### Applications

- application identity;
- instances;
- application-owned resources;
- background tasks;
- lifecycle;
- health;
- application-scoped configuration;
- versioned state.

### Workspaces

- resource grouping;
- application set;
- shared session context;
- workspace-level capabilities;
- import/export;
- snapshot and restore;
- mobility between machines where policy allows.

### Tool Registry

- operations exposed to authorized automation and AI agents;
- schemas;
- effect classification;
- approval requirement;
- stable resource references;
- audit integration.

Tool Registry should derive from command and service descriptors rather than creating a parallel authority model.

## 9.4 WASM host interfaces

Host interfaces should be narrow and capability-oriented.

Examples:

```text
logos:store/read
logos:store/write
logos:network/connect
logos:time/monotonic
logos:time/civil
logos:session/command
logos:ui/surface
logos:input/events
logos:app/lifecycle
logos:audit/emit
```

An imported function is not enough by itself. The instance also requires a matching scoped capability.

## 9.5 Runtime may depend on

- Sessions for user context, commands, and interaction;
- System for storage, network, identity, time, audit, and update;
- Foundation display/input only through higher-level application-facing interfaces;
- Core indirectly.

## 9.6 Runtime must not own

- machine-wide user authentication;
- raw devices;
- kernel capabilities outside host mediation;
- firewall implementation;
- trusted secret root keys;
- system rollback policy;
- compositor ownership;
- recovery boot.

## 9.7 Native applications

Native third-party applications are not part of Applications v1.

A native application exception must document:

- why WASM cannot satisfy the requirement;
- what additional trust is required;
- how capabilities remain bounded;
- how memory and fault isolation are enforced;
- how update and rollback work;
- whether the component should instead be a Foundation or System service.

## 9.8 Runtime anti-patterns

- One broad “system” host import.
- Passing raw kernel handles into WASM.
- Granting network or Store access by default.
- Conflating package signature with permission.
- Allowing an application to choose its own quota.
- Treating the WASM runtime as part of kernel recovery.
- Creating a second command/tool system solely for agents.

## 9.9 Failure behavior

A guest failure should:

1. trap or terminate the instance;
2. cancel outstanding host operations;
3. reclaim instance-owned resources;
4. preserve or roll back state according to transaction rules;
5. emit health diagnostics;
6. apply restart policy;
7. leave other applications and the host intact.

A runtime host failure should not damage Store authority or System services. Supervisor restarts it and applications recover according to manifests.

---

# 10. Ring 5 — Experience

## 10.1 Mission

Experience provides replaceable graphical and rich user interfaces.

It is the least trusted and most replaceable architectural ring. The machine must remain fully manageable without it.

## 10.2 Trust and lifecycle

| Property | Ring 5 expectation |
|---|---|
| Implementation | Native compositor initially; WASM or mixed applications |
| Privilege | Surface/input capabilities only |
| Trust | Minimal |
| Boot role | Optional |
| Restartability | Required |
| Update cadence | Highest |
| Hardware specificity | None through Foundation contracts |
| Policy | User-facing presentation and interaction |
| Public surface | Surfaces, semantics, actions, presentation |

## 10.3 Experience owns

### Compositor

- surface composition;
- frame scheduling;
- damage;
- focus;
- input routing;
- stacking and layer policy;
- capture permissions;
- display presentation;
- remote surface streaming hooks.

### Desktop Shell

- login and unlock presentation;
- launcher;
- workspace switching;
- notifications;
- settings presentation;
- status indicators;
- user-facing recovery entry;
- graphical session selection.

### Graphical applications

- editors;
- browsers;
- IDEs;
- file/object explorers;
- dashboards;
- graphical terminal;
- system management clients.

Applications consume Runtime and Sessions contracts. They do not directly manage System internals.

### Accessibility

- semantic trees;
- roles and actions;
- focus semantics;
- text alternatives;
- assistive client access;
- input adaptation.

Accessibility semantics should be part of UI contracts, not added only at the final renderer.

## 10.4 Experience may depend on

- Runtime applications and workspaces;
- Sessions and commands;
- System services;
- Foundation display/input/glyph contracts through compositor-approved paths.

## 10.5 Experience must not own

- authoritative identity;
- machine capabilities;
- service lifecycle truth;
- system storage transactions;
- network stack;
- application sandbox;
- driver state;
- update commit policy.

A settings UI edits System configuration through typed commands. It does not become the configuration authority.

## 10.6 Compositor placement

The compositor should be native initially because it:

- owns display presentation;
- routes input;
- enforces surface isolation;
- must remain responsive under untrusted application load;
- may require low-level buffer coordination.

It remains Ring 5 because it is not required for machine safety or remote administration and must be restartable.

## 10.7 Experience anti-patterns

- Making graphical login the only way to authenticate remotely.
- Keeping authoritative state only inside UI processes.
- Letting applications read global input.
- Letting applications capture arbitrary surfaces.
- Coupling application lifecycle to compositor lifetime.
- Requiring the desktop to perform updates or service recovery.
- Exposing raw framebuffer ownership to ordinary applications.

## 10.8 Failure behavior

When Experience fails:

- remote administration remains available;
- sessions and applications continue where possible;
- compositor-owned presentation resources are reclaimed;
- applications reconnect to a new compositor;
- the recovery console remains available;
- no persistent state is silently lost.

---

# 11. Cross-ring flows

## 11.1 Local keyboard to structured command

```text
PS/2 or USB device
  ↓
Ring 1 Input
  physical key → logical key → text event
  ↓
Ring 3 Session
  routes event to active Slate attachment
  ↓
Ring 3 Slate
  edits command line and caret
  ↓
Ring 3 Shell
  parses typed syntax
  ↓
Ring 3 Commands
  validates schema and capability
  ↓
Ring 2 System service
  performs operation
  ↓
typed result
  ↓
formatter → Slate → Glyph → Display
```

Core participates only through interrupts, scheduling, mappings, IPC, and capabilities.

## 11.2 Remote service restart

```text
remote client
  ↓ encrypted connection
Ring 2 Network
  ↓
Ring 3 Gateway
  authenticates attachment and resolves session
  ↓
Ring 3 Commands
  checks `service.restart` descriptor and capability
  ↓
Ring 2 Supervisor
  quiesces, reclaims, restarts, health-checks
  ↓
Ring 2 Audit
  records principal, service, result
  ↓
structured result to client
```

The remote client does not receive a general Supervisor handle.

## 11.3 WASM application reads persistent state

```text
Ring 4 application
  calls `logos:store/read`
  ↓
Ring 4 WASM host
  validates instance capability and quota
  ↓
Ring 2 Store
  resolves namespace and object authority
  ↓
Ring 1 Block
  performs required I/O
  ↓
Core
  schedules, maps, wakes, and transports
```

The application does not know which disk, partition, filesystem view, or block driver is used.

## 11.4 Graphical application presentation

```text
Ring 4 application
  produces UI/surface updates
  ↓
Ring 5 Compositor
  enforces surface and input policy
  ↓
Ring 1 Display
  presents frame
```

Input returns through:

```text
Ring 1 Input
  ↓
Ring 5 Compositor focus routing
  ↓
Ring 4 application
```

## 11.5 Driver recovery

```text
Ring 1 driver detects failure
  ↓
fails outstanding requests and publishes health
  ↓
Ring 2 Supervisor applies driver recovery policy
  ↓
driver quiesce → Core resource reclamation → device reset
  ↓
driver restart/rebind
  ↓
dependent services reconnect or degrade
```

No user-facing application is allowed to reset the device directly.

---

# 12. Placement decision procedure

Use this procedure before creating a new subsystem.

## Step 1 — State the invariant

Complete:

> This subsystem exists to guarantee that...

If the sentence describes presentation or convenience rather than an invariant, the component likely belongs outward.

## Step 2 — Identify owned resources

List:

- memory;
- handles;
- devices;
- persistent objects;
- connections;
- sessions;
- surfaces;
- secrets;
- jobs;
- identities.

If ownership is unclear, the subsystem boundary is not ready.

## Step 3 — Determine minimum privilege

Ask what it actually needs:

- CPU privilege?
- interrupt endpoint?
- mapping authority?
- device access?
- machine policy capability?
- session-scoped capability?
- application-scoped capability?

Grant the minimum and place the component no further inward than required.

## Step 4 — Determine recovery requirement

Ask:

- Must this function exist before Supervisor starts?
- Must it exist when Store is corrupt?
- Must it exist when Network, Runtime, or Experience is unavailable?
- Is a tiny recovery substitute sufficient?

Often only a small substitute belongs inward.

## Step 5 — Determine failure radius

What should restart when it fails?

| Desired restart radius | Likely placement |
|---|---|
| Whole machine | Core, only for true invariants |
| One driver | Foundation |
| One machine service | System |
| One session or renderer | Sessions |
| One application/runtime instance | Runtime |
| One UI environment | Experience |

## Step 6 — Determine implementation class

- Native privileged mechanism → Core.
- Native hardware adapter → Foundation.
- Native machine policy service → System.
- Native interaction mediation → Sessions.
- Sandboxed portable component → Runtime.
- Presentation and visual policy → Experience.

## Step 7 — Define outward contract

The contract should not leak:

- current driver type;
- raw addresses;
- internal task IDs;
- package implementation;
- framebuffer layout;
- storage medium;
- terminal escape parsing;
- WASM engine type.

## Step 8 — Test the name

Check the Naming Register:

- does the name describe responsibility rather than implementation?
- would it remain valid after replacing the backend?
- does it collide with another LogOS concept?
- can its exclusions be stated?

---

# 13. Placement matrix

| Concern | Ring | Reason |
|---|---:|---|
| Physical page ownership | 0 | Global safety invariant |
| Virtual mapping permission | 0 | Hardware-enforced authority |
| Cooperative/preemptive scheduling mechanism | 0 | CPU execution invariant |
| IPC transport and wake-up | 0 | Cross-service safety mechanism |
| Recovery bitmap glyphs | 0 | Emergency-only boot dependency |
| Keyboard interrupt and device driver | 1 | Hardware adaptation |
| Keyboard layout and text composition | 1 | Device-independent input service |
| Font rasterization | 1 | Shared rendering foundation |
| Block requests | 1 | Device mechanism |
| Persistent transactions | 2 | Machine-wide storage policy |
| TCP/DNS/TLS integration | 2 | Machine network service |
| User/service identities | 2 | Machine-wide principal authority |
| Secret rotation | 2 | Machine security policy |
| Service restart policy | 2 | Machine supervision |
| Shell parsing | 3 | Interaction policy |
| Typed command registry | 3 | Human/automation operation surface |
| Terminal scrollback and caret | 3 | Session presentation model |
| Remote session resume | 3 | Session transport |
| WASM engine host | 4 | Application sandbox platform |
| Application package manifest | 4 | Application lifecycle |
| Workspace | 4 | User/application authority grouping |
| AI tool adapter | 4 | Sandboxed automation consumer |
| Compositor | 5 | Replaceable visual policy |
| Desktop shell | 5 | User-facing presentation |
| Graphical terminal | 5 | UI client of Ring 3 |
| SSH compatibility server | 3 | Compatibility transport into Sessions |
| POSIX file compatibility | 2 or 4 | System view if shared; app adapter if isolated |
| Browser | 4/5 | Sandboxed app plus graphical surface |

---

# 14. Transitional architecture from the current system

LogOS currently has Core v1 with a basic framebuffer console and keyboard path.

The next transition should avoid a large rewrite.

## Phase A — Preserve and label recovery paths

- Mark the current console explicitly as `core.recovery`.
- Restrict its commands to diagnostics and recovery.
- Keep fixed pixel glyphs.
- Keep a minimal QWERTY input path.
- Document every dependency it has on Core internals.

## Phase B — Create Foundation abstractions

- Introduce `foundation.display`.
- Introduce `foundation.input`.
- Introduce `foundation.text`.
- Route a new normal terminal through these services.
- Keep recovery output independent.

## Phase C — Create Sessions

- Introduce `session.manager`.
- Introduce `session.commands`.
- Introduce `session.shell`.
- Introduce `session.terminal`.
- Move normal commands out of Core.
- Implement structured values and cancellation.

## Phase D — Introduce System authority

- Move service lifecycle policy into Supervisor.
- Attach Identity and capability context to sessions.
- Add Store-backed history when Persistence v1 exists.
- Add Audit for privileged commands.

## Phase E — Add remote attachment

- Add Network.
- Add Gateway.
- Attach local and remote renderers to the same session model.
- Keep SSH as an adapter, not the native model.

## Phase F — Add WASM and graphical clients

- Expose Commands and System services through WASM host interfaces.
- Run applications under workspace capabilities.
- Build the graphical terminal as a client of Sessions.
- Add Compositor and Desktop Shell without changing the command model.

---

# 15. Architectural review checklist

A pull request introducing or moving a subsystem should answer:

## Boundary

- [ ] The ring is named.
- [ ] The owned invariant is stated.
- [ ] Owned resources are listed.
- [ ] Exclusions are listed.
- [ ] Dependencies point inward through contracts.
- [ ] No startup dependency cycle is introduced.

## Authority

- [ ] Required capabilities are explicit.
- [ ] No ambient authority is assumed.
- [ ] Delegation and revocation behavior are defined.
- [ ] Principal identity and resource ownership are not conflated.

## Lifecycle

- [ ] Startup and health behavior are defined.
- [ ] Cancellation is defined.
- [ ] Timeout behavior is defined.
- [ ] Teardown reclaims all owned resources.
- [ ] Client disappearance is handled.
- [ ] Restart and reconnection are defined.

## Failure

- [ ] Failure radius matches the ring.
- [ ] Inner rings remain operational after failure.
- [ ] Structured diagnostics are emitted.
- [ ] Degraded mode or recovery behavior is explicit.

## Contract

- [ ] Protocol identifier and version exist.
- [ ] Schemas are bounded.
- [ ] Backpressure is defined.
- [ ] Internal implementation details do not leak.
- [ ] Compatibility expectations are documented.

## Naming

- [ ] Canonical namespace follows the Naming Register.
- [ ] Short name fits its scope.
- [ ] The name remains valid after likely implementation changes.
- [ ] Rejected alternatives are recorded when ambiguity exists.

## Proof

- [ ] Unit or model tests cover invariants.
- [ ] Integration tests cover unauthorized access.
- [ ] QEMU tests cover success, failure, cancellation, and restart.
- [ ] The feature has explicit exit criteria.

---

# 16. Open architectural decisions

These decisions should be resolved through focused design notes rather than implicitly during implementation.

1. **Native service isolation:** one address space per native service, shared address spaces with language isolation, or a hybrid.
2. **Driver hosting:** one driver per task, grouped driver hosts, or class-specific hosts.
3. **Protocol representation:** Rust-native schemas initially, WIT/component interfaces, a custom IDL, or a layered approach.
4. **Capability transfer:** copy, move, lend, derive, and revocation semantics.
5. **Session persistence:** which jobs survive client and session detachment.
6. **Structured shell syntax:** custom language versus adapting an existing structured-shell model.
7. **Store implementation:** native object store first, conventional filesystem first, or a shared transactional substrate.
8. **WASM engine:** interpreter, JIT, AOT, or multiple execution modes.
9. **Compositor/application boundary:** WASM UI command streams, shared pixel buffers, retained UI trees, or hybrid surfaces.
10. **Real-hardware scope:** when USB, NVMe, GPU, IOMMU, and multicore support become roadmap blockers.

Each decision should record:

- context;
- alternatives;
- chosen direction;
- consequences;
- migration path;
- conditions that would justify revisiting it.

---

# 17. Summary

The LogOS onion is not merely a diagram of layers. It is a rule for limiting trust and preserving replaceability:

- **Core** enforces privileged invariants.
- **Foundation** adapts hardware.
- **System** provides machine-wide policy and durable services.
- **Sessions** exposes structured operation to humans and automation.
- **Runtime** hosts portable capability-isolated applications.
- **Experience** presents replaceable graphical environments.

The architectural target is reached when removing any outer ring leaves every inner ring valid and operable within its documented scope.
