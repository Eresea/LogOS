# LogOS Naming Register

> **Status:** Optional reference; consult when adding or renaming a subsystem

Names should describe a stable ownership boundary, not an implementation detail or an aspirational
feature. [Architecture](architecture.md) is the source of truth for what each boundary owns.

## Rules

- Prefer one canonical namespace and one short human name.
- Name the invariant, resource, or policy the component owns.
- Do not use a name that implies authority the component must not have.
- Keep `Core`, `Foundation`, `System`, `Sessions`, `Runtime`, and `Experience` for the architectural
  rings; do not use them as generic product labels.
- A rename requires updating active docs, source identifiers, tests, and relevant ADR references.

## Current register

| Canonical namespace | Short name | Scope |
| --- | --- | --- |
| `core` | Core | privileged mechanisms, capabilities, memory, scheduling, IPC, recovery |
| `foundation.input` | Input | typed device-facing input |
| `foundation.display` | Display | typed device-facing presentation |
| `foundation.block` | Block | bounded block-device access |
| `foundation.network` | Network device | NIC/DMA and device events |
| `system.platform` | Platform | identity, capabilities, supervision, health, restart |
| `system.store` | Store / Persistence | bounded named objects and recovery |
| `system.network` | Network service | packet, datagram, and stream protocol state |
| `system.remote` | Remote / Gateway | trust, enrollment, and structured attachment |
| `session.terminal` | Terminal | local command and presentation client |
| `session.sessions` | Sessions | session lifecycle and typed effects |
| `runtime.wasm` | Runtime | sandboxed application execution |
| `experience` | Experience | compositor and graphical clients |

## Reserved vocabulary

Use `service` for a replaceable supervised payload, `driver` for hardware-facing code, `endpoint` for
a typed capability-scoped transport, `page` for a mapped ABI page, `generation` for replacement-safe
identity, and `proof` for an automated contract check. Avoid introducing `manager`, `engine`, `hub`,
or `orchestrator` unless ownership and failure behavior are explicit.

## Review questions

1. Is this a new boundary or a feature of an existing one?
2. What does the name own, and what must it never own?
3. Does the name remain correct if implementation changes?
4. Is the capability and failure scope clear from the name?
5. Does the change need an ADR because it crosses rings or changes a public contract?
