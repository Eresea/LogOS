# LogOS Architecture Annex

> **Status:** Living architecture reference  
> **Updated:** 2026-07-24

## Testing boundary

Repository testing is not an OS ring or runtime service. Host tests prove portable models; the
`logos-test` harness proves assembled contracts in QEMU. Test builds alone expose `LOGOS/1` over
COM2, semantic fault controls, virtual time, and debug-exit completion. Production builds expose
none of that control surface. Completed milestone proof IDs remain regression contracts until the
corresponding public contract is explicitly deprecated. See [ADR-0002](adr/0002-test-control-boundary.md).

## 1. Purpose

This document defines where responsibilities belong, how components depend on one another, and how LogOS preserves a small kernel while still becoming a complete operating system.

The ring model is architectural, not a direct representation of CPU privilege levels.

## 2. Ring model

## Ring 0 — Core

Core owns privileged mechanisms and global invariants.

### Responsibilities

- CPU and interrupt initialization
- task scheduling and wake-up
- physical-page ownership
- virtual mappings and permissions
- capability tables and enforcement
- IPC transport primitives
- bounded kernel diagnostics
- fault and panic containment
- minimal device discovery required to start drivers
- minimal recovery console
- reset and power-off

### Exclusions

- keyboard layouts
- fonts
- normal terminal behavior
- command parsing
- user identities
- secrets policy
- files and directories
- network protocols
- package management
- WASM execution
- application policy
- desktop composition

Core exposes mechanisms. It does not decide normal system policy.

## Ring 1 — Foundation

Foundation contains trusted native Rust services closest to hardware.

### Typical services

- driver host and device binding
- input
- display
- text and glyph rasterization
- block devices
- network devices
- entropy devices
- hardware clock access

### Console v1 input slice

The Foundation input service owns normal PS/2 scancode decoding and exposes initial text, backspace, enter, and escape events. The kernel recovery console retains a direct PS/2 input path and does not depend on the service. A later terminal consumes the service; it must not consume recovery input.

Normal input now exposes physical and logical keys, press/release state, modifiers, and bounded repeats. QWERTY is the default layout; the session `layout <qwerty|azerty>` command switches to AZERTY until preference storage exists.

### Console v1 display slice

The Foundation display service validates and presents normal framebuffer pixels. The kernel recovery console retains direct framebuffer output and does not depend on the service. A later terminal owns the normal display-service client role; recovery output remains available independently.

### Console v1 text slice

The Foundation text service rasterizes the fixed bitmap glyphs through the display service. The recovery console retains direct bitmap-glyph rendering; font loading, metrics beyond fixed cells, and UTF-8 remain future work.

Normal text embeds the printable-ASCII Iosevka Term Regular 34.7.0 bitmap at fixed 8×20 cell metrics with a fallback glyph. The source font is OFL-1.1 licensed; storage-backed font loading remains deferred to Persistence v1.

### Rules

- Hardware access requires explicit capabilities.
- Drivers own their queues, mappings, interrupts, and DMA resources.
- Device-independent protocols face outward.
- Driver crashes must not require kernel restart where hardware permits recovery.
- Normal services do not depend on a specific driver implementation.

## Ring 2 — System

System services provide shared machine-wide behavior.

### Typical services

- service supervision
- identity and principals
- secrets
- trusted time
- storage
- networking
- firewall and trust store
- audit
- update and rollback
- package metadata

### Rules

- Machine-wide policy lives here, not in drivers.
- System services expose versioned typed protocols.
- Services own persistent state through storage capabilities.
- Privileged effects emit audit events.
- System services remain remotely operable without a graphical environment.

### Platform v1 bootstrap

The bootstrap supervisor validates static manifests before starting their services. A
manifest has a stable service name, explicit dependencies, and declared capabilities;
startup rejects duplicate names, missing dependencies, and cycles. It grants a service
only the capability kinds declared by that service's manifest. The current bootstrap plan starts
`virtio-balloon` after `supervisor`; it is bounded, in-memory, and has no package loader,
or package ABI loader. A service negotiates its declared ABI major and compatible protocol
minor version before registration; restart policy remains bounded bootstrap state.

Failed bootstrap service starts emit the service name and the failed protocol, capability,
registration, binding, or task stage to the debug console before the startup health gate stops.

The supervisor requests replacement by declared service name only. The service-owned callback
releases and binds implementation resources, so supervisor policy has no driver-specific logic.

Machine identity is a 128-bit UEFI runtime variable. It is stable when firmware variable storage
is available; the bootstrap reports an explicit volatile fallback otherwise. Entropy-backed
identity creation is deferred until the entropy service is available.

The entropy service reads the UEFI RNG protocol before boot services exit. Its 256-bit output
seeds new machine identities; an unavailable RNG leaves identity explicitly volatile rather than
using timestamp-derived data as a secure random substitute.

The bootstrap secret store is bounded and memory-only. It requires a `Secret` capability and
matches each record to its owning principal; Persistence v1 will provide durable encryption and
operation-specific credential brokering.

Standing approvals are bounded grants scoped to one principal and capability kind. They expire,
are individually revocable, and emit distinct grant and revoke audit events.

`system.inference` owns the bounded model inventory and accelerator bindings. Invocation is
represented by an expiring `Inference` approval granted only to the requesting runtime process.

Resource discovery returns a typed reference resolved only for its declared kind. The registry
records the owning principal rather than exposing implementation pointers to clients.

The bootstrap VirtIO driver owns its DMA queue pages, pending request page, routed interrupt, and
device state. Recovery quiesces then reactivates the device; replacement releases all owned pages
before a fresh bind, without requiring a machine reboot.

Input, display, block, network, and entropy use versioned device-interface classes. Clients bind
to a class contract instead of a PCI or driver implementation.

Driver manifests declare the hardware match, interface class, and requested capabilities. The
binding policy selects a matching manifest; hardware-specific drivers do not decide policy.

Privileged operations append a bounded audit event carrying the initiating principal and effect.
The bootstrap records secret writes; durable export and retention are Persistence v1 concerns.

The monotonic-time service exposes the PIT tick count as an opaque increasing value. It makes no
wall-clock claim and remains valid across timer consumers. Before boot services exit, the
wall-clock service snapshots the UEFI RTC as explicitly `Untrusted`; a failed or invalid read is
explicitly `Unknown`. Neither state authorizes time-sensitive security decisions until a future
trusted-time source calibrates it.

Services reclaim request-owned resources on completion and on release. The bootstrap VirtIO
service returns its submitted page before replying and releases any pending page with its queue
during replacement or shutdown.

Manifests declare normal, recovery, and diagnostics profile membership. Recovery starts the
minimum service set needed for recovery operations; diagnostics starts only the supervisor until
diagnostic-specific services exist. Normal boot selects the normal profile.

The supervisor watches a bounded heartbeat for each boot service. A scheduled service
updates its heartbeat; an overdue heartbeat schedules bounded exponential-backoff
recovery. The manifest owns retry limits and shutdown state; the existing VirtIO driver
quiesces when released. Replacing a stopped service task remains a later supervisor task.

## Ring 3 — Sessions

Sessions provide interactive and automated control of the system.

### Components

- session manager
- authentication context
- command registry
- shell engine
- job lifecycle
- terminal model and renderer
- remote session gateway
- clipboard and interaction services
- output formatters

### Central rule

A terminal is one renderer for a session. It is not the definition of the session.

The same session and command contracts may be used by:

- the local terminal;
- a graphical terminal;
- a web client;
- a desktop client;
- a mobile client;
- an SSH compatibility service;
- an authorized AI agent.

## Ring 4 — Runtime

Runtime hosts sandboxed applications and user-scoped automation.

### Components

- WASM component runtime
- application package service
- application lifecycle
- application identities
- workspaces
- application-owned storage
- typed inter-application interfaces
- generated SDKs
- tool registry for authorized automation

### Rules

- WASM applications receive no ambient machine authority.
- Host imports are capabilities.
- Resource, CPU, memory, network, and storage limits are explicit.
- Application failure does not compromise the runtime or other applications.
- Native applications are exceptional and require a stronger trust policy.

## Ring 5 — Experience

Experience provides replaceable graphical and user-facing environments.

### Components

- compositor
- graphical shell
- launcher
- settings
- notifications
- accessibility services
- graphical applications
- remote visual clients

The machine must remain manageable when this entire ring is absent or failed.

## 3. Dependency rules

1. Dependencies point inward through public contracts.
2. No outer component calls an inner implementation directly.
3. Cross-ring communication uses versioned interfaces and capabilities.
4. A service may depend on another service in the same ring only through the supervisor.
5. Cycles in startup dependencies are prohibited.
6. Runtime discovery may be dynamic, but required boot dependencies must remain explicit.
7. Resource ownership is never inferred from reachability alone.

## 4. Boot sequence

A target boot flow is:

1. Core initializes CPU, memory, interrupts, scheduler, capabilities, IPC, diagnostics, and recovery console.
2. Core starts the minimum supervisor entry point.
3. Supervisor validates the boot profile and service manifests.
4. Foundation drivers discover and bind devices.
5. System identity, time, entropy, secrets, and storage start.
6. Normal display, input, and text services start.
7. Session services start and present the local terminal.
8. Networking and the remote gateway start.
9. Update, WASM runtime, workspaces, and applications start according to policy.
10. Graphical experience starts only when configured.

Recovery boot stops at an earlier stage and starts only explicitly permitted services.

## 5. Failure domains

| Failure | Expected containment |
|---|---|
| WASM application | Application stopped; owned resources reclaimed; optional restart |
| Terminal renderer | Session remains; renderer reconnects |
| Shell engine | Jobs are cancelled or reattached according to contract |
| Remote client | Session may remain resumable within policy |
| Compositor | Applications and remote administration continue |
| System service | Supervisor reports, reclaims, restarts, or enters degraded mode |
| Driver | Device quiesced/reset; driver rebound where possible |
| Supervisor | Core enters recovery path rather than continuing silently |
| Core invariant failure | Panic diagnostics and controlled halt/reset |

## 6. Native Rust versus WASM

Use a native Rust service when the component:

- directly controls hardware;
- must participate in early boot or recovery;
- requires privileged mappings, interrupts, or DMA;
- enforces machine-wide security policy;
- has latency constraints that the runtime cannot yet satisfy;
- must remain available when the WASM runtime is unavailable.

Use WASM when the component:

- is an application or extension;
- consumes stable host interfaces;
- should be portable, replaceable, quota-bound, or hot-updated;
- does not require direct hardware ownership;
- benefits from language-independent component interfaces.

A WASM application's native host contract is LogOS WIT component interfaces. WASI may be supplied only as a compatibility layer and must not define the native capability model.

A native implementation should not become permanent merely because it was easier during bootstrap. Record why it remains native.

## 7. Terminal and session architecture

See [Console](CONSOLE.md) for the versioned terminal subsystem scope and checklist.

## Recovery console

Kernel-owned, fixed-function, minimal.

It is activated only when the normal console fails its health gate or an authorized live handoff requests recovery. Healthy normal boots keep recovery framebuffer output dormant.

```text
recovery input
    -> tiny command parser
    -> fixed kernel diagnostics/recovery operations
    -> bitmap framebuffer output
```

## Normal system terminal

```text
input driver
    -> input service
    -> session manager
    -> shell / command registry
    -> typed values and events
    -> terminal model
    -> text service
    -> display service
```

Console v1 currently has a bounded terminal model with scrollback, history, selection, search, and rendering. Its command/session wiring remains in the UEFI bootstrap; it has no independent service boundary or recovery role.

The terminal model stores bounded UTF-8, edits only on character boundaries, and owns cursor position independently of rendering.

The terminal owns caret visibility; the normal terminal loop drives its blink timer and redraws it through the text/display services.

Terminal editing provides insert, delete, character-safe navigation, and Ctrl+left/right word navigation without giving the renderer ownership of the input buffer.

Terminal layout reads display dimensions on each redraw, wrapping cells to the current usable width without retaining display-mode state.

Terminal scrollback is fixed-capacity and owned by the terminal model; persistence remains a separate storage contract.

Command history traverses that bounded terminal state through up/down input events; it has no persistence dependency.

Terminal selection is UTF-8-boundary validated and exposes borrowed selected bytes, ready for a future clipboard service without coupling one into the terminal.

Terminal search validates UTF-8 queries and searches the visible buffer before bounded scrollback, returning byte ranges on character boundaries.

Terminal output is a separate bounded line model; rendering consumes that model plus the editor state, and command results never write framebuffer pixels directly.

Terminal redraw retains no display-service state: rendering the same model on a replacement display service reproduces the output and editor.

The normal terminal model lives in the no-std `logos-terminal` crate, but is currently linked into the UEFI binary and run by its normal-mode loop. This is a bootstrap arrangement, not a Core boundary. Platform v1 first stages and validates its versioned boot payload, then loads it as a Sessions service with capability-only input and display contracts; the UEFI binary then retains only recovery-console code.

Each native service has its own Core-owned address space. Only service code, stack, IPC buffers,
and granted endpoints are mapped there; raw devices and kernel memory are never service mappings.

### Native service address-space bootstrap

Core creates a fresh PML4 for each service, retains the current supervisor-only kernel mappings,
and assigns a free lower-half slot to that service. The first mapping set is one read/execute code
page and one read/write stack page, both marked user-accessible; all inherited PML4 entries must
remain supervisor-only. Core owns every page-table and service frame and releases all of them if
setup fails or the service exits. Ring-3 entry, PE-section mapping, IPC buffer mapping, and fault
return are the next loader steps; until then the terminal remains linked into `logos-uefi`.

The loader accepts only a bounded PE32+ image with a valid DOS/PE header, image bounds, executable
entry RVA, and non-empty in-bounds sections. It copies each validated section page into a
Core-owned frame, maps it user-accessible with its write/execute permissions, and marks data and
stack pages non-executable. It will not reparse untrusted offsets while creating page tables.

Before any service entry, Core installs a kernel GDT and bootstrap TSS. The TSS supplies a
Core-owned ring-0 stack for faults and gates entered from Ring 3; service selectors have DPL 3 and
cannot reuse the kernel selectors. The bootstrap is single-CPU until multicore support is in scope.
Each service space maps that stack supervisor-only before entry and restores the bootstrap TSS stack
after return.

The first transition proof switches to the service CR3, enters a user-mapped probe with `iretq`,
and returns only through a DPL-3 gate on the TSS stack. Core restores its original CR3 and stack
before resuming. A native terminal entry may use that path only after it has an explicit IPC ABI.

Native service headers carry the ABI version, service name, and native entry function. The loader
derives that entry's RVA after firmware relocation and accepts it only when it lies in an executable
PE section. Core maps a versioned service-context page and passes its user virtual address to the
entry. `Ready` resumes Ring 3 after Core acknowledges it; `ReadInput` captures a Core-owned service
frame and Core supplies one typed input byte before resuming it; `Complete` returns control to Core.
Display and session operations remain unmapped until their capability-scoped IPC contracts are added.

A long-lived native service is suspended only at a Core gate. Core saves its registers and `iretq`
frame on Core-owned memory, then later restores that frame on the service's supervisor stack. A
service cannot select its return frame, kernel stack, CR3, or device mappings.

See [ADR-0001](adr/0001-terminal-service-boundary.md), [ADR-0003](adr/0003-native-service-payload-contract.md), [ADR-0004](adr/0004-native-service-address-spaces.md), and [ADR-0005](adr/0005-native-service-suspension.md).

In normal mode, the terminal is the sole PS/2 input consumer. Recovery input is activated only after the mode coordinator selects recovery.

The command registry authorizes a live recovery handoff with an explicit recovery capability. The handoff stops normal input processing before recovery activates its direct input/output path.

The local session has a stable local principal and session identifier. Its capability context is explicit and revocable; commands authorize against that context rather than a global grant.

Remote operation replaces only the input/output transport:

```text
remote client
    <-> remote gateway
    <-> session manager
    <-> shell / command registry
```

## Component responsibilities

| Component | Responsibility |
|---|---|
| Input service | Physical keys, logical keys, modifiers, repeat, layouts, text composition |
| Text service | Fonts, glyph metrics, shaping, fallback, rasterization |
| Terminal model | Lines, cells, cursor, selection, scrollback, redraw state |
| Terminal renderer | Converts terminal model into display operations |
| Shell | Syntax, variables, pipelines, jobs, cancellation |
| Command registry | Discovery, descriptors, schemas, invocation |
| Session manager | Identity, capabilities, environment, lifetime, reconnect |
| Remote gateway | Authentication, transport, multiplexing, resume |
| Formatters | Tables, trees, text, JSON, live views |

No single component owns all of these responsibilities.

## 8. Structured command model

Command input and output use schemas.

Each command descriptor declares named argument kinds and requiredness. The initial recovery command has an explicit empty argument schema; parsing remains a shell responsibility.

Command invocation returns a typed outcome: a command action or a structured error code. The terminal chooses presentation text and does not parse error strings.

Each command invocation carries cancellation and a monotonic deadline. Commands reject cancelled or expired work before dispatch; later asynchronous commands retain the same context.

Terminal output is fixed-capacity. Once full, producers receive backpressure instead of silently evicting prior output.

Terminal history can be exported and restored as bounded submissions; Persistence v1 owns durable storage of that contract.

Sessions hold a bounded UTF-8 variable map. Variables are session-local; structured pipeline execution remains separate from text expansion.

A pipeline passes values:

```text
services
| where status == "failed"
| select name, reason
| sort name
```

It does not require parsing a text table.

Text-only commands may still participate through adapters:

```text
text-command
| parse lines
| map ...
```

Structured references are first-class values:

```text
service:/network
device:/pci/0000:00:03.0
system:/health
store:/workspaces/main
session:/current
app:/editor
model:/local/default
```

These URI-like forms are user-facing references, not promises that every resource behaves like a file.

## 9. Capability model

Capabilities identify authority over an operation or resource.

Examples:

- invoke a specific command;
- inspect health;
- restart one service;
- read one storage namespace;
- write one object;
- connect to a network endpoint;
- listen on a port;
- access a secret by identifier;
- open an input stream;
- present a surface;
- install a signed package;
- approve a sensitive agent action.

Properties:

- no ambient global authority for normal applications;
- capabilities may be narrowed when delegated;
- capabilities carry scope and, where useful, expiry;
- standing approvals are scoped, expiring, revocable policy grants, distinct from durable capabilities and individual action approvals;
- ownership and authority are separate;
- revocation behavior is explicit;
- denial is a structured result, not an incidental error.

## 10. Identity and workspaces

Identity answers who is acting.

Capabilities answer what that principal may do.

A workspace groups:

- resources;
- applications;
- persistent state;
- shared services;
- sessions;
- capability grants;
- policy.

A workspace is not a Unix home directory and is not inherently tied to one machine.

Agent memory is a Store namespace with workspace visibility, retention, and redaction policy; it is not a separate storage primitive.

## 11. Storage model

The native storage contract is based on objects, streams, versions, and transactions.

A file view may provide:

- paths;
- directories;
- files;
- metadata;
- streams;
- atomic rename and replace.

The compatibility view must not force every native service to use path-based APIs internally.

## 12. Networking model

Applications consume asynchronous connection and datagram interfaces through the network service.

They do not own network drivers.

Policy is separated from mechanism:

- driver: packets and device state;
- network service: protocols and connections;
- firewall: allow/deny policy;
- identity/trust: peer authentication;
- session gateway: remote LogOS protocol.

## 12.1 Inference model

`system.inference` owns model inventory, local inference scheduling, and accelerator binding. Runtime applications invoke it only through scoped capabilities. Accelerator-device support is deferred until hardware needs justify it.

## 13. Update model

An update passes through:

1. acquisition;
2. signature and policy validation;
3. compatibility resolution;
4. staging;
5. activation;
6. health gate;
7. commit or rollback.

Each phase is journaled and recoverable.

Kernel, trusted services, drivers, runtimes, and applications may have different signing and rollout policies.

## 14. AI addressability

AI agents use the same typed command and service registry as other clients.

A tool descriptor should expose:

- stable operation identifier;
- input and output schemas;
- required capabilities;
- side-effect classification;
- cancellation support;
- expected resource limits;
- audit behavior;
- whether human approval is required.

AI integration must remain optional. Disabling the agent layer must not disable normal operation.

Agent-initiated actions produce a bounded decision record correlated with the action audit entry. It records policy outcome, a user-intent reference, and structured tool inputs and outputs; it never records private model reasoning. Untrusted external content requires explicit re-approval before it can trigger a privileged agent effect; global taint tracking is deferred pending a narrower information-flow design.

## 15. Placement checklist

Before adding a component, answer:

1. What invariant does it own?
2. What resources does it own?
3. What capabilities does it require and expose?
4. What is its failure boundary?
5. Can it restart independently?
6. Does it require hardware privilege?
7. Must it exist during recovery?
8. Could it be WASM?
9. What contract faces outward?
10. What automated proof demonstrates correct placement?

When the answers are unclear, the subsystem boundary is not ready.
