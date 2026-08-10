# LogOS Architecture Annex

> **Status:** Living architecture reference  
> **Updated:** 2026-08-08

## Testing boundary

Repository testing is not an OS ring or runtime service. Host tests prove portable models; the
`logos-test` harness proves assembled contracts in QEMU. Test builds alone expose `LOGOS/1` over
COM2, semantic fault controls, virtual time, deterministic `RESET`, and debug-exit completion.
Production builds expose none of that control surface. Readiness and postconditions use structured
COM2 queries; debugcon is diagnostic only. Pure protocol/state-machine logic runs on
the host; QEMU proves target boot, interrupts, memory, devices, isolation, and recovery. Completed
milestone proof IDs remain regression contracts until the corresponding public contract is
explicitly deprecated. See [ADR-0002](adr/0002-test-control-boundary.md).

## ABI v4 and native-service ownership

Native service transport uses a dedicated mapped `logos_abi::service::ControlPage` header (ABI v4). The
header carries lifecycle, generation, and bounded notification state; typed endpoint pages carry
protocol payloads. Active pages include `InputPage`, `DisplayPage`, `SessionClientPage`,
`SessionServerPage`, `EffectPage`, independent `StoreClientPage`/`StoreServerPage` pairs, the
Storage-owned `BlockClientPage`, distinct `NetworkDevicePage` and `NetworkEventPage`, and `RemotePage`. Every page uses scalar wire
states, generation checks, and bounded validation. Core owns endpoint mappings, capability checks,
page loans, and reclamation; a service receives only the endpoint set declared by its canonical
`platform::services::ServiceSpec`.
The control page is implicit; `ServiceSpec::endpoints` is the single map for additional Input,
Display, Session, Store, Block, Network device/event/stream, and Remote pages. `ControlPage` carries only
lifecycle and notification state for Network and Remote; device payloads, event payloads, deadlines,
DMA handles, and remote request/reply scalars live in typed endpoint pages.

Input transitions `Ready -> Waiting -> Reply -> Ready`; Display transitions `Ready -> Request ->
Complete -> Ready`. Core and services validate scalar state and generation on every transition. Native
task replacement advances the generation before releasing the old address space, so stale handles and
physically reused pages cannot affect the replacement. The concrete mapping/reclamation pattern is
recorded in [ADR-0021](adr/0021-typed-input-display-pages.md).

The canonical specification is consumed by supervisor planning, service lookup, and payload header
validation. `src/kernel.rs` is the privileged boot-facing entry boundary, with bootstrap composition,
health gating, privileged setup, and the run loop implemented in `platform::runtime`; subsystem
coordination remains below `src/platform`, while hardware, memory, scheduling, IPC, and capability
enforcement remain Core responsibilities. The migration is atomic: ABI-v3 payloads are
rejected and no compatibility adapter or dynamic endpoint registry exists. See [ADR-0020](adr/0020-typed-native-endpoint-pages.md).

### Network scalable stream slice

The typed Network endpoint remains the single capability, ownership, generation, and replacement
boundary. `SubmitWrite` and `PollStream` are additional operations; `StreamPage` is auxiliary state
transport, not a second Network ABI. Network service state uses separate bounded listener and
connection tables, with one listener and eight connections initially. Each connection owns byte
stream RX/TX storage and cumulative accepted/acknowledged watermarks; TCP sequence numbers are
assigned only when bytes are transmitted. Readiness is coalesced per connection and completion
records are bounded with sequence/loss detection. `PollStream` is authoritative for current
readiness and byte watermarks; `StreamPage` is only a notification cache. On overflow, clients poll
each owned endpoint and clear the flag. NetworkRuntime routes notifications to client pages; the
scheduler owns task execution. See [ADR-0027](adr/0027-network-scalable-stream-slice.md).

## 1. Purpose

This document defines where responsibilities belong, how components depend on one another, and how LogOS preserves a small kernel while still becoming a complete operating system.

Kernel source follows the same ownership boundaries: `arch`, `mm`, `sched`, `ipc`, `drivers`,
`console`, and `platform`. `boot.rs` owns the UEFI handoff; `kernel.rs` owns the privileged
bootstrap boundary and `platform::runtime` contains its composition, health gating, and top-level
coordination loop; `main.rs` declares modules only.

Assembly-visible GDT, TSS, IDT, and user-transition storage uses a layout-transparent writable
cell. Access is restricted to raw pointers under the bootstrap single-CPU invariant; scalar state
shared with interrupt paths uses atomics. SMP requires replacing these cells with per-CPU state.

The ring model is architectural, not a direct representation of CPU privilege levels.

Cargo package edges are checked against this inward ring direction by
`scripts/arch-deps.py --check`; boot and test assembly exceptions are listed in
that checker rather than becoming implicit imports.

ABI v4 keeps endpoint ownership in the canonical service `ServiceSpec::endpoints` descriptor list;
Core maps that bounded list through generic endpoint identity and lifetime records. The ABI-v4 named
page fields are populated only by the address-space compatibility adapter. Remote transport pages are
isolated in the Remote protocol module, and proof state is owned by the test-hook proof module rather
than the platform composition root.

Deferred endpoint evolution now has isolated bounded primitives: `logos_abi::endpoint_v5` defines a
generation-bound endpoint table, while `logos_core` provides binary manifest validation, owner-scoped
resource leases, explicit-loss event queues, and an opt-in cooperative poll runtime. These are not
implicitly wired into the ABI-v4 boot path; the existing scheduler and typed pages remain active.
Network service ingress is admitted through its bounded event reactor before dispatch, while pending
operations remain one service-owned state object. Storage read selections and replace transactions
hold caller-owned, generation-checked leases; `Cancel` reclaims only that caller's active work, and
service replacement reconstructs the bounded pool. The cooperative poll runtime remains deferred to
Core scheduler integration rather than creating a second service-local scheduler.

The current Remote migration checkpoint is deliberately partial: `RemoteRuntime` owns RemoteState,
local trust commands through one `local_command` path, transport reset/start state, protected
control loading, the enrollment gate, and one bounded generation-bound input operation slot.
Production and test-driven Terminal input share that path; mutable RemoteState is not exposed to
callers. The composition root still owns Gateway endpoint bindings, the large Remote request
polling loop, protected persistence context, deadlines, and replacement composition. ABI v4 remains
unfrozen until the Network and Remote QEMU proofs are green.

### Async-first state and scheduling rule

Long-lived or externally-driven work lives in bounded state owned by the subsystem that can advance
it. Commands, completions, events, and timers cause explicit state transitions; readiness and
completion are authoritative state, while notifications are bounded, coalesced hints. Subsystems
may report a runnable task or changed endpoint, but Core/platform composition owns waking and running
tasks. Blocking APIs are allowed when they wrap the stateful primitive or preserve a real durability
or security boundary. Generations, deadlines, and cancellation belong to the operation state so
replacement and stale completion remain deterministic. See [ADR-0028](adr/0028-async-first-subsystem-state.md).

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

Native service binaries share the minimal `logos-service-rt` crate for their PE entry point and
panic handler. The runtime has no firmware dependency; service protocols remain in `logos-abi`
and portable policy types remain in `logos-core`. See [ADR-0012](adr/0012-service-runtime-boundary.md).

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

### Versioned module boundaries

Modules expose goal-oriented operations through published contracts. Consumers bind to a protocol identifier and supported version range, not to an implementation crate, concrete service type, or module milestone label.

An implementation may be replaced without coordinated consumer changes while it preserves the selected contract and its behavioral proofs. An incompatible contract is a new version discovered and negotiated at the provider boundary. Temporary adapters or parallel versions are required only when a documented migration or rollback needs them.

System capability milestones compose exact module slices and add integration proofs; they do not make those modules share ownership or advance versions together. This permits work such as Sessions v2, Platform v3, and Persistence v2 to proceed independently until a published contract changes. See [ADR-0016](adr/0016-capability-slices.md).

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

Console v1 has a bounded terminal model with scrollback, history, selection, search, and rendering. The normal terminal runs as an independently loaded Ring-3 service through a bounded Core bootstrap gate; command/session dispatch remains bootstrap code and has no recovery role.

The terminal model stores bounded UTF-8, edits only on character boundaries, and owns cursor position independently of rendering.

The terminal model owns caret visibility. The Ring-3 renderer redraws through the implemented bounded `foundation.display` presentation contract.

Terminal editing provides insert, delete, character-safe navigation, and Ctrl+left/right word navigation without giving the renderer ownership of the input buffer.

Terminal layout reads display dimensions on each redraw, wrapping cells to the current usable width without retaining display-mode state.

Terminal scrollback is fixed-capacity and owned by the terminal model; persistence remains a separate storage contract.

Command history traverses that bounded terminal state through up/down input events; it has no persistence dependency.

Terminal selection is UTF-8-boundary validated and exposes borrowed selected bytes, ready for a future clipboard service without coupling one into the terminal.

Terminal search validates UTF-8 queries and searches the visible buffer before bounded scrollback, returning byte ranges on character boundaries.

Terminal output is a separate bounded line model; rendering consumes that model plus the editor state, and command results never write framebuffer pixels directly.

Terminal redraw retains no display-service state: rendering the same model on a replacement display service reproduces the output and editor.

The normal terminal model lives in the no-std `logos-terminal` crate and is loaded from its staged
native payload. Core's normal-mode loop is disabled; it routes the capability-only Input, Display,
and Session contracts while retaining its direct recovery-console code.

Each native service has its own Core-owned address space. Only service code, stack, IPC buffers,
and granted endpoints are mapped there; raw devices and kernel memory are never service mappings.

### Native service address-space bootstrap

Core creates a fresh PML4 for each service, retains the current supervisor-only kernel mappings,
and assigns a free slot to that service. Validated PE sections, a two-page stack, and the shared
service context are user-accessible; the gate stack and inherited mappings remain supervisor-only.
Core owns every page-table and service frame and releases all of them if setup fails or the service
exits.

The loader accepts only a bounded PE32+ image with a valid DOS/PE header, image bounds, executable
entry RVA, in-bounds sections, and in-bounds base-relocation directory. It copies each section into
Core-owned frames, applies only AMD64 `DIR64` relocations for the service virtual base, maps section
write/execute permissions, and marks data and stack pages non-executable.

Before any service entry, Core installs a kernel GDT and bootstrap TSS. The TSS supplies a
Core-owned ring-0 stack for faults and gates entered from Ring 3; service selectors have DPL 3 and
cannot reuse the kernel selectors. The bootstrap is single-CPU until multicore support is in scope.
Each service space maps that stack supervisor-only before entry and restores the bootstrap TSS stack
after return.

The first transition proof switches to the service CR3, enters a user-mapped probe with `iretq`,
and returns only through a DPL-3 gate on the TSS stack. Core restores its original CR3 and stack
before resuming. A native terminal entry may use that path only after it has an explicit IPC ABI.

Native service headers carry the transport ABI, service name, protocol major/minor, and native
entry function. The loader requires an exact transport ABI and protocol major, and accepts a
payload minor version only when it supports the manifest's required minor. The loader
derives that entry's RVA after firmware relocation and accepts it only when it lies in an executable
PE section. Core maps a versioned service-context page and passes its user virtual address to the
entry. `Ready` resumes Ring 3 after Core acknowledges it; `ReadInput` captures a Core-owned service
frame and Core supplies one typed input byte before resuming it; `Complete` returns control to Core.
`SubmitCommand` copies at most 256 bytes into the same context page, suspends the service at a
Core-owned frame, and resumes it only after Core has copied and replied to the request.
`PresentPixel` supplies bounded coordinates and color only; Core validates them and writes the
framebuffer. Core defers display and syscall requests until their capability checks complete.

A long-lived native service is suspended only at a Core gate. Core saves its registers and `iretq`
frame on Core-owned memory, then later restores that frame on the service's supervisor stack. A
service cannot select its return frame, kernel stack, CR3, or device mappings.

Core models each loaded native terminal as a task with its address space, entry, context page, and
blocked/completed state. Scheduler integration wakes that task only after Core has written a valid
response into its context page. `ReadInput` blocks on the scheduler input event. The bootstrap
terminal repeats `ReadInput`, `ClearDisplay`, and `PresentText` until Core delivers Escape, which requests a clean
`Complete` return.

`PresentText` carries at most 256 bytes plus bounded pixel coordinates and color. Core validates that
request and rasterizes it through the Core-owned framebuffer; native services never map the display.
The QEMU native-terminal proof injects a PS/2 key through QMP, then Core normalizes and delivers it
through the task's input endpoint before resuming Ring 3.

### Platform v1 Input contract

The first bootstrap-gate replacement is `foundation.input` v1. It carries one bounded typed input
event (`Text`, `Backspace`, `Enter`, or `Escape`) and accepts one typed layout request (`Qwerty` or
`Azerty`). Core retains the PS/2 driver and recovery input path, enforces the terminal's explicit
Input capability, and never exposes PS/2 ports or scancodes to the terminal.

### Platform v1 Display contract

`foundation.display` v1 begins with typed RGB presentation values, bounded coordinates, and bounded
text. Core owns framebuffer validation and writes; the terminal receives no framebuffer mapping.
Core defers each presentation request and authorizes it with the terminal session's explicit Display
capability. A denial or malformed presentation stops the normal terminal and enters recovery.

### Platform v1 Session contract

`foundation.session` v1 gates every typed terminal syscall with an explicit Session capability.
Core brokers each request through independent Terminal client and Sessions server pages. Sessions requests an
individually capability-gated privileged effect, receives a typed `EffectResult`, formats the
bounded reply, and returns it to the terminal. A missing Session capability is rejected before
dispatch. Versioned requests and effect results live in `logos-abi`; Core retains no normal-command
registry or terminal reply switch. Recovery remains a direct Core path.

Core's fixed four-slot native scheduler owns Terminal, Sessions, Store, and Network task instances
and keeps their immutable boot-staged images separate. Replacement quarantines the failed task,
builds a fresh address space, atomically installs it with a new generation, then releases the old
space. The generic scheduler remains responsible for Core and driver work.

Native-service restart replaces the whole service address space; it never reuses potentially
corrupted code, stack, heap, context, or mapped pages. Ring-3 faults and explicit service panics
are contained at the Core gate, cancel outstanding work, invalidate the instance generation, and
restart according to the service manifest. Terminal gets one immediate retry before recovery;
Sessions, Store, and Network degrade after bounded retries while local terminal operations remain
available. Core faults and uncooperative service loops remain fatal and deferred respectively.

The recovery console can manually reset the retry budget and replace Terminal or Sessions; normal
mode resumes only after Terminal completes its ready handshake. Missing or incompatible Terminal
enters recovery. Missing Sessions, Store, or Network leaves local terminal operation available with
commands unavailable, in-memory history, or offline networking respectively.

See [ADR-0001](adr/0001-terminal-service-boundary.md), [ADR-0003](adr/0003-native-service-payload-contract.md), [ADR-0004](adr/0004-native-service-address-spaces.md), [ADR-0005](adr/0005-native-service-suspension.md), and [ADR-0017](adr/0017-native-service-fault-restart.md).

In normal mode, the terminal is the sole PS/2 input consumer. Recovery input is activated only after startup selects recovery mode.

The Core effect executor authorizes a live recovery handoff with an explicit recovery capability. The handoff stops normal input processing before recovery activates its direct input/output path.

The local session has a stable local principal and session identifier. Its capability context is explicit and revocable; commands authorize against that context rather than a global grant.

Remote operation replaces only the input/output transport:

```text
remote client
    <-> remote gateway
    <-> session manager
    <-> shell / command registry
```

### Remote Foundation v1

The Gateway is an optional Ring-3 payload. It owns one bounded TCP attachment and relays only
authenticated typed requests to Sessions. System trust policy derives the machine static key and
protected-store key from the UEFI root, completes Noise IK, and never maps long-term key material
into Gateway. Sessions owns the durable sequence journal: it writes `Pending` before a remote
effect and `Complete` before the reply, replaying only the exact completed request after reconnect.
Network owns TCP state; Core continues to own NIC DMA and validates every Gateway page, capability,
generation, deadline, and relay transition. See ADR-0018.

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

The Persistence v1 native storage contract is based on capability-scoped namespaces containing
named byte objects. Replacing an object creates an immutable version; callers may read the current
or immediately previous version. Streams and general transactions remain future contracts.

Core retains VirtIO block DMA, interrupts, timeout/reset, and generation-tagged shared-page
ownership until Ring-1 driver isolation can enforce those resources directly. Shared pages are
quota-bound, non-executable, owner-checked, temporarily lendable, and reclaimed on service exit.

The restartable Ring-2 Storage service owns the on-disk policy and is coordinated by the concrete
`platform::storage::StorageRuntime`, which owns Store rebinding, relay state, one bounded operation
record, Block dispatch, and a single-slot Block wake handoff. The old synchronous persistence
wrappers remain a composition compatibility boundary until loan/durability transition proofs close.
Store client and server pages are never shared; Core copies validated requests and replies between
them. Transfer handles name Core-owned loan records, and loans are returned on success, denial,
timeout, cancellation, fault, and replacement. Two alternating checksummed
superblocks select one of two append-only arenas. A replace becomes visible only after its payload,
flush, commit sector, and final flush complete. Recovery ignores incomplete or corrupt records.
Compaction copies live current/previous versions to the inactive arena before switching the
superblock generation. See [ADR-0013](adr/0013-persistence-v1-boundary.md).

Remote Foundation adds a protected namespace over the same Store contract. The trust owner derives
separate device and storage keys from the UEFI root, seeds bounded ephemeral-key generation from
firmware entropy, and wipes the bootstrap root. It seals the enrollment and remote-session objects
before Store replacement and opens them only after authenticated readback. Authentication failure
is degraded remote state, never permission to use a previous version.

A file view may provide:

- paths;
- directories;
- files;
- metadata;
- streams;
- atomic rename and replace.

The compatibility view must not force every native service to use path-based APIs internally.

## 12. Networking model

Applications consume bounded datagram interfaces through the Network service. The bootstrap path
is typed and asynchronous at the ABI boundary, but its current client implementation still uses a
single global transaction; a scalable socket architecture is not yet complete.

They do not own network drivers.

Network v1 uses the boundary accepted in [ADR-0015](adr/0015-network-v1-boundary.md). Core owns
VirtIO negotiation, DMA queues and bounce buffers, interrupts, timeout, reset, and reclamation. The
typed `NetworkDevicePage` and `NetworkEventPage` carry device requests/replies, one bounded event,
deadlines, generations, and validated DMA handles; `ControlPage` supplies only lifecycle
notifications. `platform::network::NetworkRuntime` owns this device-facing lifecycle while Core
continues to own physical pages, mappings, queues, and capability checks.
For passive TCP, Core stamps the calling native-service owner into the trusted delivery context;
the public `NetworkRequest` never carries an owner. The Network service applies that owner to
listener, accepted-stream, read, write, and close operations.
The Ring-2 Network service owns Ethernet, ARP, IPv4, ICMP echo, DHCP, UDP, and generation-tagged datagram
endpoints. Clients receive no raw-frame access. The hermetic QEMU peer supplies independent DHCP, ARP,
ICMP, UDP, malformed-frame, cancellation, timeout, and reconnect proofs.

Terminal and Gateway use typed, generation-bound Network client pages. They receive no Network
device/event endpoints; `platform::network::NetworkRuntime` owns the single active association,
readiness cache, and completion target. Once the Network service is bound and idle, NetworkRuntime
submits `Status` directly to the Network server endpoint; Terminal is not a readiness dependency.
`platform::runtime` retains only top-level polling and composition.

Gateway waits for TCP send capacity through the typed `AwaitWritable` request. Network owns the
deferred request and completes it from its device/event loop; Gateway never waits on a Network
event endpoint it does not own.

Every production Network completion wakes and runs its blocked caller task for both successful and
error statuses. QEMU white-box requests use an explicit test-only probe target, so test completion
cannot encode Terminal as a fake caller or weaken the production scheduler invariant. The intended
next architecture is queue-based: application writes enqueue to connection-owned TX buffers,
bounded Network work produces segments, NIC completion reclaims buffers, and RX processing fills
bounded per-connection buffers and publishes readiness. Network must not wake or run Gateway or
Remote as part of protocol processing; scheduler composition belongs above the Network service.

Bind, send, and receive authority are separate. Each grant carries an exact protocol/local-port or
protocol/remote-IPv4-and-port scope; wildcard and CIDR policy remain future firewall work. Network
failure does not block local boot, and a driver or service generation change invalidates endpoints
before configuration is reacquired.

Policy is separated from mechanism:

- driver: packets and device state;
- network service: protocols and connections;
- firewall: allow/deny policy;
- identity/trust: peer authentication;
- session gateway: remote LogOS protocol.

Remote Foundation currently consumes the bootstrap TCP slice but does not constitute a production
TCP architecture proof. A genuine TCP stream proof must pass through the Network service without
Remote. Only after that path is green may Gateway and `logosctl` be used as the next vertical proof.

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
