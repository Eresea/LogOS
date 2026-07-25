# LogOS Subsystem Naming Register

> **Status:** Living naming and scope registry
> **Updated:** 2026-07-24

## 1. Purpose

Names are architectural tools.

A good subsystem name:

- makes its scope easier to remember;
- discourages unrelated responsibilities from accumulating;
- avoids binding the design to one hardware device, storage medium, protocol, or implementation;
- remains meaningful when the implementation changes;
- is distinct from application, project, and AI terminology already in use.

LogOS uses two related names:

1. a **canonical namespace**, used in contracts, manifests, resource references, logs, and code;
2. a **short name**, used in documentation and discussion.

The canonical namespace prioritizes precision. The short name prioritizes recognition.

Example:

```text
canonical namespace: core.fabric
short name: Fabric
scope: physical-page ownership, virtual mappings, address-space permissions
```

## 2. Naming rules

### Required

1. Prefer a singular noun or compact noun phrase.
2. Name the responsibility, not the current implementation.
3. Avoid physical-medium names for abstract services.
4. Avoid names already overloaded in LogOS or the wider project ecosystem.
5. Avoid names that imply authority the subsystem does not possess.
6. Avoid names whose scope cannot be stated in one sentence.
7. Keep canonical namespaces stable after public contracts depend on them.
8. Use suffixes such as `service`, `manager`, or `daemon` only when they remove ambiguity.
9. Record rejected alternatives when the distinction matters.

## 3. Naming lifecycle

Each entry has one status:

- **Reserved** — accepted and should not be reused.
- **Working** — preferred current name; may still change before stable contracts.
- **Candidate** — under evaluation.
- **Deprecated** — retained only for migration/history.
- **Rejected** — intentionally not used.

A name becomes **Reserved** when:

- its scope is documented;
- its canonical namespace is used by a versioned public contract;
- no known collision exists;
- the name still fits at least two plausible future implementations.

## 4. Current register

| Namespace               | Short name           | Ring | Status    | Scope                                                                         |
| ----------------------- | -------------------- | ---: | --------- | ----------------------------------------------------------------------------- |
| `core`                  | **Core**             |    0 | Reserved  | Privileged kernel mechanisms and global invariants                            |
| `core.fabric`           | **Fabric**           |    0 | Working   | Physical pages, virtual mappings, address spaces, permissions, reclamation    |
| `core.authority`        | **Authority**        |    0 | Working   | Capability tables, checks, delegation primitives, and enforcement             |
| `core.channels`         | **Channels**         |    0 | Working   | Bounded IPC transport, correlation, wake-up, cancellation, and backpressure   |
| `core.scheduler`        | **Scheduler**        |    0 | Reserved  | Ready, blocked, wake-up, and execution scheduling                             |
| `core.clock`            | **Clock**            |    0 | Working   | Monotonic deadlines, timers, and wake-up; not civil time                      |
| `core.trace`            | **Trace**            |    0 | Reserved  | Bounded bootstrap, lifecycle, fault, and recovery diagnostics                 |
| `core.recovery`         | **Recovery Console** |    0 | Reserved  | Minimal kernel-owned diagnostics and recovery interface                       |
| `foundation.devices`    | **Devices**          |    1 | Working   | Discovery, binding, ownership, reset, and device-independent exposure         |
| `foundation.input`      | **Input**            |    1 | Working   | Physical/logical keys, layouts, modifiers, repeat, composition, pointers      |
| `foundation.display`    | **Display**          |    1 | Working   | Display modes, scanout, framebuffer/surface presentation; no window policy    |
| `foundation.text`       | **Glyph**            |    1 | Candidate | Font loading, metrics, shaping, fallback, and rasterization                   |
| `foundation.block`      | **Block**            |    1 | Working   | Asynchronous block-device access and recovery                                 |
| `foundation.netdev`     | **Net Device**       |    1 | Working   | Network-device packet transport and device lifecycle                          |
| `system.supervisor`     | **Supervisor**       |    2 | Working   | Service manifests, dependencies, health, restart, quiesce, and recovery       |
| `system.identity`       | **Identity**         |    2 | Working   | Machine, user, service, application, and agent principals                     |
| `system.secrets`        | **Vault**            |    2 | Working   | Protected secret storage, scoped access, rotation, and revocation             |
| `system.time`           | **Time**             |    2 | Working   | Civil time, synchronization, trust state, and calendars                       |
| `system.store`          | **Store**            |    2 | Working   | Persistent objects, streams, versions, transactions, namespaces, and quotas   |
| `system.network`        | **Network**          |    2 | Working   | IP, transport protocols, DNS, connections, policy integration                 |
| `system.audit`          | **Audit**            |    2 | Working   | Durable security-relevant action records                                      |
| `system.update`         | **Update**           |    2 | Working   | Package validation, staging, activation, health gates, and rollback           |
| `session.manager`       | **Sessions**         |    3 | Working   | Interactive session identity, capabilities, environment, lifetime, and resume |
| `session.commands`      | **Commands**         |    3 | Working   | Command descriptors, discovery, schemas, invocation, and typed results        |
| `session.shell`         | **Shell**            |    3 | Working   | Syntax, variables, pipelines, jobs, cancellation, and automation              |
| `session.terminal`      | **Slate**            |    3 | Candidate | Terminal model, cursor, editing, scrollback, selection, and rendering         |
| `session.remote`        | **Gateway**          |    3 | Candidate | Authenticated remote transport, multiplexing, resume, and client bridging     |
| `runtime.wasm`          | **WASM Runtime**     |    4 | Working   | WASM validation, instantiation, host interfaces, quotas, and lifecycle        |
| `runtime.package`       | **Packages**         |    4 | Working   | Signed application bundles, manifests, dependencies, and versions             |
| `runtime.app`           | **Application**      |    4 | Working   | Sandboxed executable identity and owned resources                             |
| `runtime.workspace`     | **Workspace**        |    4 | Working   | User-scoped grouping of resources, applications, sessions, and authority      |
| `runtime.tools`         | **Tool Registry**    |    4 | Working   | Typed operations exposed to software and authorized AI agents                 |
| `experience.compositor` | **Compositor**       |    5 | Working   | Surface composition, focus, input routing, presentation, and capture policy   |
| `experience.shell`      | **Desktop Shell**    |    5 | Working   | Login, launcher, notifications, settings, and workspace UX                    |

## 5. Names requiring deliberate review

## `core.fabric` — Fabric

### Intended scope

- page ownership;
- address spaces;
- virtual mappings;
- mapping permissions;
- reclamation;
- possibly shared-memory objects at the lowest mechanism level.

### Why it fits

It describes the memory/address-space substrate without colliding with AI memory, persistent storage, or physical RAM.

### Risks

“Fabric” can also describe communication interconnects. Keep IPC under `core.channels` to preserve the distinction.

### Alternatives considered

- `Address`
- `Space`
- `Pages`
- `Memory`
- `Mapping`

Current preference: **Fabric**.

## `foundation.text` — Glyph

### Intended scope

- fonts;
- glyph metrics;
- shaping;
- fallback;
- rasterization.

### Exclusions

- terminal scrollback;
- shell parsing;
- document layout;
- widgets;
- compositor policy.

### Alternatives considered

- `Text`
- `Type`
- `Font`
- `Script`

Current preference: **Glyph**, pending implementation experience.

## `session.terminal` — Slate

### Intended scope

- editable terminal model;
- caret;
- selection;
- scrollback;
- cell/line layout;
- terminal rendering state.

### Exclusions

- command parsing;
- process/job control;
- session authentication;
- font rasterization;
- display ownership.

### Why it fits

A slate is a presentation and interaction surface, not the command system itself.

### Alternatives considered

- `Terminal`
- `Console`
- `Canvas`
- `Screen`

Current preference: **Slate** as the short name; canonical namespace remains `session.terminal`.

## `session.remote` — Gateway

### Intended scope

- authenticate clients;
- negotiate protocol versions;
- multiplex and resume sessions;
- transport commands, events, resources, logs, and files.

### Exclusions

- network stack;
- authorization policy ownership;
- shell parsing;
- service supervision.

### Alternatives considered

- `Bridge`
- `Relay`
- `Portal`
- `Remote`

Current preference: **Gateway**, pending protocol design.

## 6. Reserved vocabulary

These words have specific meanings and should not be reused casually.

| Term                 | Meaning                                                                 |
| -------------------- | ----------------------------------------------------------------------- |
| **Core**             | Ring 0 kernel only                                                      |
| **Capability**       | Explicit authority over an operation or resource                        |
| **Principal**        | Identified actor                                                        |
| **Resource**         | Addressable object, service, device, session, application, or model     |
| **Service**          | Independently supervised native or runtime-hosted component             |
| **Application**      | Sandboxed user-facing or background workload                            |
| **Session**          | Identity and capability context for interactive or automated operations |
| **Workspace**        | Grouping of resources, applications, sessions, and scoped authority     |
| **Command**          | Discoverable typed operation                                            |
| **Shell**            | Syntax, pipelines, variables, and jobs                                  |
| **Terminal**         | Interactive textual model and renderer                                  |
| **Recovery Console** | Minimal kernel-owned fallback                                           |
| **Store**            | Native persistence service                                              |
| **File**             | Compatibility/user-facing stream abstraction; not all stored data       |
| **Trace**            | Bounded diagnostic event stream                                         |
| **Audit**            | Durable security-relevant record                                        |
| **Tool**             | Typed operation exposed to an automation client or agent                |

## 7. Resource namespace guidance

Preferred user-facing resource references:

```text
system:/health
service:/supervisor
service:/network
device:/pci/0000:00:03.0
store:/workspaces/main
session:/current
workspace:/main
app:/editor
model:/local/default
```

Rules:

- the scheme identifies the resource domain;
- a resource reference is not necessarily a path or file;
- references should be serializable in command results;
- authorization is checked when resolving or acting on a reference;
- display aliases may change, stable identifiers must not.

## 8. Naming decision template

Use this template when introducing a subsystem.

```md
## <canonical namespace> — <short name>

**Status:** Candidate | Working | Reserved

### One-sentence scope

<The subsystem owns...>

### Owns

- ...

### Does not own

- ...

### Inputs

- ...

### Outputs

- ...

### Required capabilities

- ...

### Failure boundary

- ...

### Why the name fits

...

### Alternatives rejected

- `<name>` — <reason>
```

## 9. Rename policy

Before a public contract is stable:

- rename freely when scope becomes clearer;
- update the register and architecture annex;
- avoid compatibility aliases unless needed for persisted data.

After a public contract is stable:

- keep the canonical namespace;
- a short display name may change;
- protocol identifiers change only through versioned migration;
- deprecations must name the replacement and removal condition.

## 10. Naming review in development

Every new roadmap item or major implementation change should include a naming check:

1. Is this a new subsystem or only a feature of an existing one?
2. Does the proposed name still fit if the implementation changes?
3. Does it collide with AI memory, storage, networking, UI, or existing project names?
4. Can its scope be remembered from the name alone?
5. Can its exclusions be stated clearly?
6. Should the canonical namespace differ from the short name?
7. Does the name encourage accidental scope growth?

Naming is complete only when the boundary is clear.
