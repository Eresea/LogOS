# Applications

> **Status:** Applications v1 planned

## Goal

Run portable applications as capability-isolated WASM components without making the runtime part of kernel correctness.

## V1 — WASM execution

- [ ] Validate/load WASM components through versioned native LogOS WIT interfaces; WASI remains compatibility-only.
- [ ] Capability-scoped imports; memory, CPU, storage, and network quotas; cancellation and asynchronous host operations.
- [ ] Application identity, health, lifecycle, crash isolation, and restart policy.
- [ ] Basic install, run, persist, stop, and remove operations plus a Rust SDK.

## V2 — Packages and workspaces

- [ ] Signed packages, atomic application update/rollback, state migration, and typed inter-application interfaces.
- [ ] Workspace identity, metadata, resources, applications, sessions, and scoped capability grants.
- [ ] Export, import, snapshot, and recovery without granting machine-wide authority.
- [ ] Package and state garbage collection.

## V3 — AI-addressable applications

- [ ] Typed tool descriptors expose schemas, capabilities, side effects, and approval policy.
- [ ] Human approval for sensitive effects; no implicit agent privilege.
- [ ] Auditable agent sessions and bounded decision records without private model reasoning.
- [ ] Re-approval when untrusted content would trigger privileged effects.
- [ ] Sensitive-value labels with explicit audited declassification before remote-model access.
- [ ] Optional compensating actions backed by Store versions where reversal exists.

## Exit criteria

A WASM application can be installed, run, communicate, persist, stop, and be removed using only granted interfaces. Failure remains isolated and resource limits remain enforceable.

See [Architecture](architecture.md#ring-4--runtime), [AI addressability](architecture.md#14-ai-addressability), and [Security](security.md).
